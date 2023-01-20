use std::path::Path;

use crate::{
    game_data::{GameData, Item, Map, NodePtr},
    randomize::Randomization,
};
use anyhow::{ensure, Context, Result};
use hashbrown::HashMap;
use ips;
use std::iter;

const NUM_AREAS: usize = 6;

fn snes2pc(addr: usize) -> usize {
    addr >> 1 & 0x3F8000 | addr & 0x7FFF
}

#[derive(Clone)]
pub struct Rom {
    pub data: Vec<u8>,
}

impl Rom {
    pub fn load(path: &Path) -> Result<Self> {
        let data = std::fs::read(path)
            .with_context(|| format!("Unable to load ROM at path {}", path.display()))?;
        Ok(Rom { data })
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        std::fs::write(path, &self.data)
            .with_context(|| format!("Unable to save ROM at path {}", path.display()))?;
        Ok(())
    }

    pub fn read_u8(&self, addr: usize) -> Result<isize> {
        ensure!(addr + 1 <= self.data.len(), "read_u8 address out of bounds");
        Ok(self.data[addr] as isize)
    }

    pub fn read_u16(&self, addr: usize) -> Result<isize> {
        ensure!(
            addr + 2 <= self.data.len(),
            "read_u16 address out of bounds"
        );
        let b0 = self.data[addr] as isize;
        let b1 = self.data[addr + 1] as isize;
        Ok(b0 | b1 << 8)
    }

    pub fn read_u24(&self, addr: usize) -> Result<isize> {
        ensure!(
            addr + 3 <= self.data.len(),
            "read_u24 address out of bounds"
        );
        let b0 = self.data[addr] as isize;
        let b1 = self.data[addr + 1] as isize;
        let b2 = self.data[addr + 2] as isize;
        Ok(b0 | b1 << 8 | b2 << 16)
    }

    pub fn read_n(&self, addr: usize, n: usize) -> Result<&[u8]> {
        ensure!(addr + n <= self.data.len(), "read_n address out of bounds");
        Ok(&self.data[addr..(addr + n)])
    }

    pub fn write_u8(&mut self, addr: usize, x: isize) -> Result<()> {
        ensure!(
            addr + 1 <= self.data.len(),
            "write_u8 address out of bounds"
        );
        ensure!(x >= 0 && x <= 0xFF, "write_u8 data does not fit");
        self.data[addr] = x as u8;
        Ok(())
    }

    pub fn write_u16(&mut self, addr: usize, x: isize) -> Result<()> {
        ensure!(
            addr + 2 <= self.data.len(),
            "write_u16 address out of bounds"
        );
        ensure!(x >= 0 && x <= 0xFFFF, "write_u16 data does not fit");
        self.data[addr] = (x & 0xFF) as u8;
        self.data[addr + 1] = (x >> 8) as u8;
        Ok(())
    }

    pub fn write_u24(&mut self, addr: usize, x: isize) -> Result<()> {
        ensure!(
            addr + 3 <= self.data.len(),
            "write_u24 address out of bounds"
        );
        ensure!(x >= 0 && x <= 0xFFFFFF, "write_u24 data does not fit");
        self.data[addr] = (x & 0xFF) as u8;
        self.data[addr + 1] = ((x >> 8) & 0xFF) as u8;
        self.data[addr + 2] = (x >> 16) as u8;
        Ok(())
    }

    pub fn write_n(&mut self, addr: usize, x: &[u8]) -> Result<()> {
        ensure!(
            addr + x.len() <= self.data.len(),
            "write_n address out of bounds"
        );
        self.data[addr..(addr + x.len())].copy_from_slice(x);
        Ok(())
    }
}

pub struct Patcher<'a> {
    pub orig_rom: &'a mut Rom,
    pub rom: &'a mut Rom,
    pub randomization: &'a Randomization,
    pub game_data: &'a GameData,
    pub map: &'a Map,
}

fn xy_to_map_offset(x: isize, y: isize) -> isize {
    let y1 = y + 1;
    if x < 32 {
        (y1 * 32 + x) * 2
    } else {
        ((y1 + 32) * 32 + x - 32) * 2
    }
}

fn item_to_plm_type(item: Item, orig_plm_type: isize) -> isize {
    let item_id = item as isize;
    let old_item_id = ((orig_plm_type - 0xEED7) / 4) % 21;
    orig_plm_type + (item_id - old_item_id) * 4
}

impl<'a> Patcher<'a> {
    fn apply_ips_patch(&mut self, patch_path: &Path) -> Result<()> {
        let patch_data = std::fs::read(&patch_path)
            .with_context(|| format!("Unable to read patch {}", patch_path.display()))?;
        let patch = ips::Patch::parse(&patch_data)
            .with_context(|| format!("Unable to parse patch {}", patch_path.display()))?;
        for hunk in patch.hunks() {
            self.rom.write_n(hunk.offset(), hunk.payload())?;
        }
        Ok(())
    }

    fn apply_ips_patches(&mut self) -> Result<()> {
        let patches_dir = Path::new("../patches/ips/");
        let mut patches = vec![
            "mb_barrier",
            "mb_barrier_clear",
            "hud_expansion_opaque",
            "gray_doors",
            "vanilla_bugfixes",
            "music",
            "crateria_sky_fixed",
            "everest_tube",
            "sandfalls",
            "saveload",
            "map_area",
            "elevators_speed",
            "boss_exit",
            "itemsounds",
            "progressive_suits",
            "disable_map_icons",
            "escape",
            "mother_brain_no_drain",
            "tourian_map",
            "tourian_eye_door",
            "no_explosions_before_escape",
            "escape_room_1",
            "unexplore",
            "max_ammo_display",
            "missile_refill_all",
            "sound_effect_disables",
            "title_map_animation",
            "fast_reload",
        ];
        patches.push("new_game_extra");
        // "new_game_extra' if args.debug else 'new_game",
        for patch_name in patches {
            let patch_path = patches_dir.join(patch_name.to_string() + ".ips");
            self.apply_ips_patch(&patch_path)?;
        }
        Ok(())
    }

    fn place_items(&mut self) -> Result<()> {
        for (&item, &loc) in iter::zip(
            &self.randomization.item_placement,
            &self.game_data.item_locations,
        ) {
            let item_plm_ptr = self.game_data.node_ptr_map[&loc];
            let orig_plm_type = self.orig_rom.read_u16(item_plm_ptr)?;
            let new_plm_type = item_to_plm_type(item, orig_plm_type);
            self.rom.write_u16(item_plm_ptr, new_plm_type)?;
        }
        Ok(())
    }

    fn connect_door_pair(
        &mut self,
        src_exit_ptr: Option<usize>,
        src_entrance_ptr: Option<usize>,
        dst_exit_ptr: Option<usize>,
        dst_entrance_ptr: Option<usize>,
    ) -> Result<()> {
        if src_exit_ptr.is_some() && dst_entrance_ptr.is_some() {
            let door_data = self.orig_rom.read_n(dst_entrance_ptr.unwrap(), 12)?;
            self.rom.write_n(src_exit_ptr.unwrap(), door_data)?;
        }
        if dst_exit_ptr.is_some() && src_entrance_ptr.is_some() {
            let door_data = self.orig_rom.read_n(src_entrance_ptr.unwrap(), 12)?;
            self.rom.write_n(dst_exit_ptr.unwrap(), door_data)?;
        }
        Ok(())
    }

    fn connect_doors(&mut self) -> Result<()> {
        for &((src_exit_ptr, src_entrance_ptr), (dst_exit_ptr, dst_entrance_ptr), _bidirectional) in
            &self.randomization.map.doors
        {
            self.connect_door_pair(
                src_exit_ptr,
                src_entrance_ptr,
                dst_exit_ptr,
                dst_entrance_ptr,
            )?;
        }
        Ok(())
    }

    fn fix_save_stations(&mut self) -> Result<()> {
        let save_station_ptrs = vec![
            0x44C5, 0x44D3, 0x45CF, 0x45DD, 0x45EB, 0x45F9, 0x4607, 0x46D9, 0x46E7, 0x46F5, 0x4703,
            0x4711, 0x471F, 0x481B, 0x4917, 0x4925, 0x4933, 0x4941, 0x4A2F, 0x4A3D,
        ];

        let mut orig_door_map: HashMap<NodePtr, NodePtr> = HashMap::new();
        let mut new_door_map: HashMap<NodePtr, NodePtr> = HashMap::new();
        for &((src_exit_ptr, src_entrance_ptr), (dst_exit_ptr, dst_entrance_ptr), _bidirectional) in
            &self.randomization.map.doors
        {
            if src_exit_ptr.is_some() && src_entrance_ptr.is_some() {
                orig_door_map.insert(src_exit_ptr.unwrap(), src_entrance_ptr.unwrap());
            }
            if dst_exit_ptr.is_some() && dst_entrance_ptr.is_some() {
                orig_door_map.insert(dst_exit_ptr.unwrap(), dst_entrance_ptr.unwrap());
            }
            if src_exit_ptr.is_some() && dst_exit_ptr.is_some() {
                new_door_map.insert(src_exit_ptr.unwrap(), dst_exit_ptr.unwrap());
                new_door_map.insert(dst_exit_ptr.unwrap(), src_exit_ptr.unwrap());
            }
        }

        for ptr in save_station_ptrs {
            let orig_entrance_door_ptr = (self.orig_rom.read_u16(ptr + 2)? + 0x10000) as NodePtr;
            let exit_door_ptr = orig_door_map[&orig_entrance_door_ptr];
            let entrance_door_ptr = new_door_map[&exit_door_ptr];
            self.rom
                .write_u16(ptr + 2, (entrance_door_ptr & 0xFFFF) as isize)?;
        }
        Ok(())
    }

    fn write_map_tilemaps(&mut self) -> Result<()> {
        let area_map_ptrs: Vec<isize> = vec![
            0x1A9000,  // Crateria
            0x1A8000,  // Brinstar
            0x1AA000,  // Norfair
            0x1AB000,  // Wrecked ship
            0x1AC000,  // Maridia
            0x1AD000,  // Tourian
        ];

        // Determine upper-left corner of each area:
        let mut area_map_min_x = [isize::MAX; NUM_AREAS];
        let mut area_map_min_y = [isize::MAX; NUM_AREAS];
        for i in 0..self.map.area.len() {
            let area = self.map.area[i];
            let x = self.map.rooms[i].0 as isize;
            let y = self.map.rooms[i].1 as isize;
            if x < area_map_min_x[area] {
                area_map_min_x[area] = x;
            }
            if y < area_map_min_y[area] {
                area_map_min_y[area] = y;
            }
        }

        // Clear all map tilemap data:
        for area_ptr in &area_map_ptrs {
            for i in 0..(64 * 32) {
                self.rom.write_u16((area_ptr + i * 2) as usize, 0x001F)?;
            }
        }

        // Write new map tilemap data (and room X & Y map position) by room:
        for (i, room) in self.game_data.room_geometry.iter().enumerate() {
            let orig_area = self.orig_rom.read_u8(room.rom_address + 1)? as usize;
            let orig_base_x = self.orig_rom.read_u8(room.rom_address + 2)?;
            let orig_base_y = self.orig_rom.read_u8(room.rom_address + 3)?;
            let orig_base_ptr = area_map_ptrs[orig_area];
            let new_area = self.map.area[i];
            let new_base_ptr = area_map_ptrs[new_area];
            let new_base_x = self.map.rooms[i].0 as isize - area_map_min_x[new_area] + 2;
            let new_base_y = self.map.rooms[i].1 as isize - area_map_min_y[new_area] + 1;
            self.rom.write_u8(room.rom_address + 2, new_base_x)?;
            self.rom.write_u8(room.rom_address + 3, new_base_y)?;
            for y in 0..room.map.len() {
                for x in 0..room.map[0].len() {
                    if room.map[y][x] == 0 {
                        continue;
                    }
                    let orig_x = orig_base_x + x as isize;
                    let orig_y = orig_base_y + y as isize;
                    let orig_offset = xy_to_map_offset(orig_x as isize, orig_y as isize);
                    let orig_ptr = (orig_base_ptr + orig_offset) as usize;
                    let new_x = new_base_x + x as isize;
                    let new_y = new_base_y + y as isize;
                    let new_offset = xy_to_map_offset(new_x, new_y);
                    let new_ptr = (new_base_ptr + new_offset) as usize;
                    let data = self.orig_rom.read_u16(orig_ptr)?;
                    self.rom.write_u16(new_ptr, data)?;
                }
            }
        }
        Ok(())
    }

    fn write_map_areas(&mut self) -> Result<()> {
        let mut room_index_area_hashmaps: Vec<HashMap<usize, usize>> =
            vec![HashMap::new(); NUM_AREAS];
        for (i, room) in self.game_data.room_geometry.iter().enumerate() {
            let room_index = self.orig_rom.read_u8(room.rom_address)? as usize;
            let orig_room_area = self.orig_rom.read_u8(room.rom_address + 1)? as usize;
            assert!(!room_index_area_hashmaps[orig_room_area].contains_key(&room_index));
            let new_area = self.map.area[i];
            room_index_area_hashmaps[orig_room_area].insert(room_index, new_area);
        }

        // Handle twin rooms:
        let aqueduct_room_idx = self.game_data.room_idx_by_name["Aqueduct"];
        room_index_area_hashmaps[4].insert(0x18, self.map.area[aqueduct_room_idx]); // Set Toilet to same map area as Aqueduct
        let pants_room_idx = self.game_data.room_idx_by_name["Pants Room"];
        room_index_area_hashmaps[4].insert(0x25, self.map.area[pants_room_idx]); // Set East Pants Room to same area as Pants Room
        let west_ocean_room_idx = self.game_data.room_idx_by_name["West Ocean"];
        room_index_area_hashmaps[0].insert(0x11, self.map.area[west_ocean_room_idx]); // Set Homing Geemer Room to same area as West Ocean

        // Write the information about each room's map area to some free space in bank 0x8F
        // which will be read by the `map_area` patch.
        let area_data_base_ptr = snes2pc(0x8FE99B);
        let mut area_data_ptr_pc = area_data_base_ptr + 2 * NUM_AREAS;
        for area in 0..NUM_AREAS {
            // Write pointer to the start of the table for the given area:
            let area_data_ptr_snes = (area_data_ptr_pc & 0x7FFF) | 0x8000;
            self.rom
                .write_u16(area_data_base_ptr + 2 * area, area_data_ptr_snes as isize)?;

            // Write the table contents:
            for (&room_index, &new_area) in &room_index_area_hashmaps[area] {
                self.rom
                    .write_u8(area_data_ptr_pc + room_index, new_area as isize)?;
            }

            // Advance the pointer keeping track of the next available free space:
            area_data_ptr_pc += room_index_area_hashmaps[area].keys().max().unwrap() + 1;
        }
        assert!(area_data_ptr_pc <= snes2pc(0x8FEB00));
        Ok(())
    }
}

pub fn make_rom(
    base_rom_path: &Path,
    randomization: &Randomization,
    game_data: &GameData,
) -> Result<Rom> {
    let mut orig_rom = Rom::load(base_rom_path)?;
    let mut rom = orig_rom.clone();
    let mut patcher = Patcher {
        orig_rom: &mut orig_rom,
        rom: &mut rom,
        randomization,
        game_data,
        map: &randomization.map,
    };
    patcher.apply_ips_patches()?;
    patcher.place_items()?;
    patcher.connect_doors()?;
    patcher.fix_save_stations()?;
    patcher.write_map_tilemaps()?;
    patcher.write_map_areas()?;
    Ok(rom)
}
