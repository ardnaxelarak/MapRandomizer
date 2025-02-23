pub mod escape_timer;
mod run_speed;

use crate::{
    game_data::{
        self, BlueOption, BounceMovementType, Capacity, DoorOrientation, DoorPtrPair,
        EntranceCondition, ExitCondition, FlagId, Float, GModeMobility, GModeMode, HubLocation,
        Item, ItemId, ItemLocationId, Link, LinkIdx, LinksDataGroup, MainEntranceCondition, Map,
        NodeId, Physics, Requirement, RoomGeometryRoomIdx, RoomId, SparkPosition, StartLocation,
        TemporaryBlueDirection, VertexId, VertexKey,
    },
    traverse::{
        apply_link, apply_requirement, get_bireachable_idxs, get_one_way_reachable_idx,
        get_spoiler_route, traverse, GlobalState, LocalState, LockedDoorData, TraverseResult,
        IMPOSSIBLE_LOCAL_STATE, NUM_COST_METRICS,
    },
};
use anyhow::{bail, Result};
use hashbrown::{HashMap, HashSet};
use log::info;
use rand::SeedableRng;
use rand::{seq::SliceRandom, Rng};
use run_speed::{
    get_extra_run_speed_tiles, get_max_extra_run_speed, get_shortcharge_max_extra_run_speed,
    get_shortcharge_min_extra_run_speed,
};
use serde_derive::{Deserialize, Serialize};
use std::{cmp::min, convert::TryFrom, hash::Hash, iter, time::SystemTime};
use strum::VariantNames;

use crate::game_data::GameData;

use self::escape_timer::SpoilerEscape;

// Once there are fewer than 20 item locations remaining to be filled, key items will be
// placed as quickly as possible. This helps prevent generation failures particularly on lower
// difficulty settings where some item locations may never be accessible (e.g. Main Street Missile).
const KEY_ITEM_FINISH_THRESHOLD: usize = 20;

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum ProgressionRate {
    Slow,
    Uniform,
    Fast,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum ItemPlacementStyle {
    Neutral,
    Forced,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum ItemPriorityStrength {
    Moderate,
    Heavy,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum DoorLocksSize {
    Small,
    Large,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum ItemMarkers {
    Simple,
    Majors,
    Uniques,
    ThreeTiered,
    FourTiered,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum ItemDotChange {
    Fade,
    Disappear,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum Objective {
    Kraid,
    Phantoon,
    Draygon,
    Ridley,
    SporeSpawn,
    Crocomire,
    Botwoon,
    GoldenTorizo,
    MetroidRoom1,
    MetroidRoom2,
    MetroidRoom3,
    MetroidRoom4,
    BombTorizo,
    BowlingStatue,
    AcidChozoStatue,
    PitRoom,
    BabyKraidRoom,
    PlasmaRoom,
    MetalPiratesRoom,
}

impl Objective {
    pub fn get_all() -> &'static [Objective] {
        use Objective::*;
        &[
            Kraid,
            Phantoon,
            Draygon,
            Ridley,
            SporeSpawn,
            Crocomire,
            Botwoon,
            GoldenTorizo,
            MetroidRoom1,
            MetroidRoom2,
            MetroidRoom3,
            MetroidRoom4,
            BombTorizo,
            BowlingStatue,
            AcidChozoStatue,
            PitRoom,
            BabyKraidRoom,
            PlasmaRoom,
            MetalPiratesRoom,
        ]
    }
    pub fn get_flag_name(&self) -> &'static str {
        use Objective::*;
        match self {
            Kraid => "f_DefeatedKraid",
            Phantoon => "f_DefeatedPhantoon",
            Draygon => "f_DefeatedDraygon",
            Ridley => "f_DefeatedRidley",
            SporeSpawn => "f_DefeatedSporeSpawn",
            Crocomire => "f_DefeatedCrocomire",
            Botwoon => "f_DefeatedBotwoon",
            GoldenTorizo => "f_DefeatedGoldenTorizo",
            MetroidRoom1 => "f_KilledMetroidRoom1",
            MetroidRoom2 => "f_KilledMetroidRoom2",
            MetroidRoom3 => "f_KilledMetroidRoom3",
            MetroidRoom4 => "f_KilledMetroidRoom4",
            BombTorizo => "f_DefeatedBombTorizo",
            BowlingStatue => "f_UsedBowlingStatue",
            AcidChozoStatue => "f_UsedAcidChozoStatue",
            PitRoom => "f_ClearedPitRoom",
            BabyKraidRoom => "f_ClearedBabyKraidRoom",
            PlasmaRoom => "f_ClearedPlasmaRoom",
            MetalPiratesRoom => "f_ClearedMetalPiratesRoom",
        }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum DoorsMode {
    Blue,
    Ammo,
    Beam,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum StartLocationMode {
    Ship,
    Random,
    Escape,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum AreaAssignment {
    Standard,
    Random,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum WallJump {
    Vanilla,
    Collectible,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum EtankRefill {
    Disabled,
    Vanilla,
    Full,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum MapsRevealed {
    No,
    Partial,
    Full,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum MapStationReveal {
    Partial,
    Full,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum SaveAnimals {
    No,
    Maybe,
    Yes,
}

#[derive(Clone, Copy, Serialize, Deserialize, Debug, PartialEq)]
pub enum MotherBrainFight {
    Vanilla,
    Short,
    Skip,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct DebugOptions {
    pub new_game_extra: bool,
    pub extended_spoiler: bool,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct ItemPriorityGroup {
    pub name: String,
    pub items: Vec<String>,
}

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct DifficultyConfig {
    pub name: Option<String>,
    pub tech: Vec<String>,
    pub notable_strats: Vec<String>,
    pub shine_charge_tiles: f32,
    pub heated_shine_charge_tiles: f32,
    pub shinecharge_leniency_frames: Capacity,
    pub speed_ball_tiles: f32,
    pub progression_rate: ProgressionRate,
    pub random_tank: bool,
    pub spazer_before_plasma: bool,
    pub stop_item_placement_early: bool,
    pub item_pool: Vec<(Item, usize)>,
    pub starting_items: Vec<(Item, usize)>,
    pub item_placement_style: ItemPlacementStyle,
    pub item_priority_strength: ItemPriorityStrength,
    pub item_priorities: Vec<ItemPriorityGroup>,
    pub semi_filler_items: Vec<Item>,
    pub filler_items: Vec<Item>,
    pub early_filler_items: Vec<Item>,
    pub resource_multiplier: f32,
    pub gate_glitch_leniency: Capacity,
    pub door_stuck_leniency: Capacity,
    pub escape_timer_multiplier: f32,
    pub phantoon_proficiency: f32,
    pub draygon_proficiency: f32,
    pub ridley_proficiency: f32,
    pub botwoon_proficiency: f32,
    pub mother_brain_proficiency: f32,
    // Quality-of-life options:
    pub supers_double: bool,
    pub mother_brain_fight: MotherBrainFight,
    pub escape_movement_items: bool,
    pub escape_refill: bool,
    pub escape_enemies_cleared: bool,
    pub mark_map_stations: bool,
    pub room_outline_revealed: bool,
    pub transition_letters: bool,
    pub door_locks_size: DoorLocksSize,
    pub item_markers: ItemMarkers,
    pub item_dot_change: ItemDotChange,
    pub all_items_spawn: bool,
    pub acid_chozo: bool,
    pub buffed_drops: bool,
    pub fast_elevators: bool,
    pub fast_doors: bool,
    pub fast_pause_menu: bool,
    pub respin: bool,
    pub infinite_space_jump: bool,
    pub momentum_conservation: bool,
    // Game variations:
    pub objectives: Vec<Objective>,
    pub doors_mode: DoorsMode,
    pub start_location_mode: StartLocationMode,
    pub save_animals: SaveAnimals,
    pub early_save: bool,
    pub area_assignment: AreaAssignment,
    pub wall_jump: WallJump,
    pub etank_refill: EtankRefill,
    pub maps_revealed: MapsRevealed,
    pub map_station_reveal: MapStationReveal,
    pub energy_free_shinesparks: bool,
    pub vanilla_map: bool,
    pub ultra_low_qol: bool,
    // Presets:
    pub skill_assumptions_preset: Option<String>,
    pub item_progression_preset: Option<String>,
    pub quality_of_life_preset: Option<String>,
    // Debug:
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debug_options: Option<DebugOptions>,
}

// Includes preprocessing specific to the map:
pub struct Randomizer<'a> {
    pub map: &'a Map,
    pub toilet_intersections: Vec<RoomGeometryRoomIdx>,
    pub locked_door_data: &'a LockedDoorData,
    pub game_data: &'a GameData,
    pub difficulty_tiers: &'a [DifficultyConfig],
    pub base_links_data: &'a LinksDataGroup,
    pub seed_links_data: LinksDataGroup,
    pub initial_items_remaining: Vec<usize>, // Corresponds to GameData.items_isv (one count per distinct item name)
}

#[derive(Clone)]
struct ItemLocationState {
    pub placed_item: Option<Item>,
    pub collected: bool,
    pub reachable: bool,
    pub bireachable: bool,
    pub bireachable_vertex_id: Option<VertexId>,
    pub difficulty_tier: Option<usize>,
}

#[derive(Clone)]
struct FlagLocationState {
    pub reachable: bool,
    pub reachable_vertex_id: Option<VertexId>,
    pub bireachable: bool,
    pub bireachable_vertex_id: Option<VertexId>,
}

#[derive(Clone)]
struct DoorState {
    pub bireachable: bool,
    pub bireachable_vertex_id: Option<VertexId>,
}

#[derive(Clone)]
struct SaveLocationState {
    pub bireachable: bool,
}

#[derive(Clone)]
struct DebugData {
    global_state: GlobalState,
    forward: TraverseResult,
    reverse: TraverseResult,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BeamType {
    Charge,
    Ice,
    Wave,
    Spazer,
    Plasma,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DoorType {
    Blue,
    Red,
    Green,
    Yellow,
    Gray,
    Beam(BeamType),
}

#[derive(Clone, Copy)]
pub struct LockedDoor {
    pub src_ptr_pair: DoorPtrPair,
    pub dst_ptr_pair: DoorPtrPair,
    pub door_type: DoorType,
    pub bidirectional: bool, // if true, the door is locked on both sides, with a shared state
}

#[derive(Clone)]
// State that changes over the course of item placement attempts
struct RandomizationState {
    step_num: usize,
    start_location: StartLocation,
    hub_location: HubLocation,
    item_precedence: Vec<Item>, // An ordering of the 21 distinct item names. The game will prioritize placing key items earlier in the list.
    save_location_state: Vec<SaveLocationState>, // Corresponds to GameData.item_locations (one record for each of 100 item locations)
    item_location_state: Vec<ItemLocationState>, // Corresponds to GameData.item_locations (one record for each of 100 item locations)
    flag_location_state: Vec<FlagLocationState>, // Corresponds to GameData.flag_locations
    door_state: Vec<DoorState>,                  // Corresponds to LockedDoorData.locked_doors
    items_remaining: Vec<usize>, // Corresponds to GameData.items_isv (one count for each of 21 distinct item names)
    global_state: GlobalState,
    debug_data: Option<DebugData>,
    previous_debug_data: Option<DebugData>,
    key_visited_vertices: HashSet<usize>,
}

pub struct Randomization {
    pub difficulty: DifficultyConfig,
    pub map: Map,
    pub toilet_intersections: Vec<RoomGeometryRoomIdx>,
    pub locked_door_data: LockedDoorData,
    pub item_placement: Vec<Item>,
    pub start_location: StartLocation,
    pub starting_items: Vec<(Item, usize)>, // (item type, count), only for non-zero counts
    pub spoiler_log: SpoilerLog,
    pub seed: usize,
    pub display_seed: usize,
    pub seed_name: String,
}

struct SelectItemsOutput {
    key_items: Vec<Item>,
    other_items: Vec<Item>,
}

struct VertexInfo {
    area_name: String,
    room_id: usize,
    room_name: String,
    room_coords: (usize, usize),
    node_name: String,
    node_id: usize,
}

pub fn randomize_map_areas(map: &mut Map, seed: usize) {
    let mut rng_seed = [0u8; 32];
    rng_seed[..8].copy_from_slice(&seed.to_le_bytes());
    let mut rng = rand::rngs::StdRng::from_seed(rng_seed);

    let mut area_mapping: Vec<usize> = (0..6).collect();
    area_mapping.shuffle(&mut rng);

    let mut subarea_mapping: Vec<Vec<usize>> = vec![(0..2).collect(); 6];
    for i in 0..6 {
        subarea_mapping[i].shuffle(&mut rng);
    }

    for i in 0..map.area.len() {
        map.area[i] = area_mapping[map.area[i]];
        map.subarea[i] = subarea_mapping[map.area[i]][map.subarea[i]];
    }
}

fn compute_run_frames(tiles: f32) -> Capacity {
    assert!(tiles >= 0.0);
    let frames = if tiles <= 7.0 {
        9.0 + 4.0 * tiles
    } else if tiles <= 16.0 {
        15.0 + 3.0 * tiles
    } else if tiles <= 42.0 {
        32.0 + 2.0 * tiles
    } else {
        47.0 + 64.0 / 39.0 * tiles
    };
    frames.ceil() as Capacity
}

fn remove_some_duplicates<T: Clone + PartialEq + Eq + Hash>(
    x: &[T],
    dup_set: &HashSet<T>,
) -> Vec<T> {
    let mut out: Vec<T> = vec![];
    let mut seen_set: HashSet<T> = HashSet::new();
    for e in x {
        if seen_set.contains(e) {
            continue;
        }
        if dup_set.contains(e) {
            seen_set.insert(e.clone());
        }
        out.push(e.clone());
    }
    out
}

struct Preprocessor<'a> {
    game_data: &'a GameData,
    door_map: HashMap<(RoomId, NodeId), (RoomId, NodeId)>,
    difficulty: &'a DifficultyConfig,
}

fn compute_shinecharge_frames(
    other_runway_length: f32,
    runway_length: f32,
) -> (Capacity, Capacity) {
    let combined_length = other_runway_length + runway_length;
    if combined_length > 31.3 {
        // Dash can be held the whole time:
        let total_time = compute_run_frames(combined_length);
        let other_time = compute_run_frames(other_runway_length);
        return (other_time, total_time - other_time);
    }
    // Combined runway is too short to hold dash the whole time. A shortcharge is needed:
    let total_time = 85.0; // 85 frames to charge a shinespark (assuming a good enough 1-tap)
    let initial_speed = 0.125;
    let acceleration =
        2.0 * (combined_length - initial_speed * total_time) / (total_time * total_time);
    let other_time =
        (f32::sqrt(initial_speed * initial_speed + 2.0 * acceleration * other_runway_length)
            - initial_speed)
            / acceleration;
    let other_time = other_time.ceil() as Capacity;
    (other_time, total_time as Capacity - other_time)
}

impl<'a> Preprocessor<'a> {
    pub fn new(game_data: &'a GameData, map: &'a Map, difficulty: &'a DifficultyConfig) -> Self {
        let mut door_map: HashMap<(RoomId, NodeId), (RoomId, NodeId)> = HashMap::new();
        for &((src_exit_ptr, src_entrance_ptr), (dst_exit_ptr, dst_entrance_ptr), _bidirectional) in
            &map.doors
        {
            let (src_room_id, src_node_id) =
                game_data.door_ptr_pair_map[&(src_exit_ptr, src_entrance_ptr)];
            let (dst_room_id, dst_node_id) =
                game_data.door_ptr_pair_map[&(dst_exit_ptr, dst_entrance_ptr)];
            door_map.insert((src_room_id, src_node_id), (dst_room_id, dst_node_id));
            door_map.insert((dst_room_id, dst_node_id), (src_room_id, src_node_id));

            if (dst_room_id, dst_node_id) == (32, 1) {
                // West Ocean bottom left door, West Ocean Bridge left door
                door_map.insert((32, 7), (src_room_id, src_node_id));
            }
            if (src_room_id, src_node_id) == (32, 5) {
                // West Ocean bottom right door, West Ocean Bridge right door
                door_map.insert((32, 8), (dst_room_id, dst_node_id));
            }
        }
        Preprocessor {
            game_data,
            door_map,
            difficulty,
        }
    }

    fn add_door_links(
        &self,
        src_room_id: usize,
        src_node_id: usize,
        dst_room_id: usize,
        dst_node_id: usize,
        is_toilet: bool,
        door_links: &mut Vec<Link>,
    ) {
        let empty_vec_exits = vec![];
        let empty_vec_entrances = vec![];
        for (src_vertex_id, exit_condition) in self
            .game_data
            .node_exit_conditions
            .get(&(src_room_id, src_node_id))
            .unwrap_or(&empty_vec_exits)
        {
            for (dst_vertex_id, entrance_condition) in self
                .game_data
                .node_entrance_conditions
                .get(&(dst_room_id, dst_node_id))
                .unwrap_or(&empty_vec_entrances)
            {
                if entrance_condition.through_toilet == game_data::ToiletCondition::Yes
                    && !is_toilet
                {
                    // The strat requires passing through the Toilet, which is not the case here.
                    continue;
                } else if entrance_condition.through_toilet == game_data::ToiletCondition::No
                    && is_toilet
                {
                    // The strat requires not passing through the Toilet, but here it does.
                    continue;
                }
                let req_opt = self.get_cross_room_reqs(
                    exit_condition,
                    src_room_id,
                    src_node_id,
                    entrance_condition,
                    dst_room_id,
                    dst_node_id,
                    is_toilet,
                );
                let exit_with_shinecharge = self.game_data.does_leave_shinecharged(exit_condition);
                let enter_with_shinecharge =
                    self.game_data.does_come_in_shinecharged(entrance_condition);
                let carry_shinecharge = exit_with_shinecharge || enter_with_shinecharge;

                // if (src_room_id, src_node_id) == (155, 5) {
                //     println!(
                //         "({}, {}, {:?}) -> ({}, {}, {:?}): {:?}",
                //         src_room_id,
                //         src_node_id,
                //         exit_condition,
                //         dst_room_id,
                //         dst_node_id,
                //         entrance_condition,
                //         req_opt
                //     );
                // }
                if let Some(req) = req_opt {
                    door_links.push(Link {
                        from_vertex_id: *src_vertex_id,
                        to_vertex_id: *dst_vertex_id,
                        requirement: req,
                        start_with_shinecharge: carry_shinecharge,
                        end_with_shinecharge: carry_shinecharge,
                        notable_strat_name: None,
                        strat_name: "Base (Cross Room)".to_string(),
                        strat_notes: vec![],
                    });
                }
            }
        }
    }

    pub fn get_all_door_links(&self) -> Vec<Link> {
        let mut door_links = vec![];
        for (&(src_room_id, src_node_id), &(dst_room_id, dst_node_id)) in self.door_map.iter() {
            self.add_door_links(
                src_room_id,
                src_node_id,
                dst_room_id,
                dst_node_id,
                false,
                &mut door_links,
            );
            if src_room_id == 321 {
                // Create links that skip over the Toilet:
                let src_node_id = if src_node_id == 1 { 2 } else { 1 };
                let (src_room_id, src_node_id) = *self.door_map.get(&(321, src_node_id)).unwrap();
                self.add_door_links(
                    src_room_id,
                    src_node_id,
                    dst_room_id,
                    dst_node_id,
                    true,
                    &mut door_links,
                );
            }
        }
        let extra_door_links: Vec<((usize, usize), (usize, usize))> = vec![
            ((220, 2), (322, 2)), // East Pants Room right door, Pants Room right door
            ((32, 7), (32, 1)),   // West Ocean bottom left door, West Ocean Bridge left door
            ((32, 8), (32, 5)),   // West Ocean bottom right door, West Ocean Bridge right door
        ];
        for ((src_room_id, src_node_id), (dst_other_room_id, dst_other_node_id)) in extra_door_links
        {
            let (dst_room_id, dst_node_id) = self.door_map[&(dst_other_room_id, dst_other_node_id)];
            self.add_door_links(
                src_room_id,
                src_node_id,
                dst_room_id,
                dst_node_id,
                false,
                &mut door_links,
            )
        }
        door_links
    }

    fn get_cross_room_reqs(
        &self,
        exit_condition: &ExitCondition,
        _exit_room_id: RoomId,
        _exit_node_id: NodeId,
        entrance_condition: &EntranceCondition,
        entrance_room_id: RoomId,
        entrance_node_id: NodeId,
        is_toilet: bool,
    ) -> Option<Requirement> {
        match &entrance_condition.main {
            MainEntranceCondition::ComeInNormally {} => {
                self.get_come_in_normally_reqs(exit_condition)
            }
            MainEntranceCondition::ComeInRunning {
                speed_booster,
                min_tiles,
                max_tiles,
            } => self.get_come_in_running_reqs(
                exit_condition,
                *speed_booster,
                min_tiles.get(),
                max_tiles.get(),
            ),
            MainEntranceCondition::ComeInJumping {
                speed_booster,
                min_tiles,
                max_tiles,
            } => self.get_come_in_running_reqs(
                exit_condition,
                *speed_booster,
                min_tiles.get(),
                max_tiles.get(),
            ),
            MainEntranceCondition::ComeInSpaceJumping {
                speed_booster,
                min_tiles,
                max_tiles,
            } => self.get_come_in_space_jumping_reqs(
                exit_condition,
                *speed_booster,
                min_tiles.get(),
                max_tiles.get(),
            ),
            MainEntranceCondition::ComeInShinecharging {
                effective_length,
                min_tiles,
                heated,
            } => self.get_come_in_shinecharging_reqs(
                exit_condition,
                effective_length.get(),
                min_tiles.get(),
                *heated,
            ),
            MainEntranceCondition::ComeInGettingBlueSpeed {
                effective_length,
                min_tiles,
                heated,
                min_extra_run_speed,
                max_extra_run_speed,
            } => self.get_come_in_getting_blue_speed_reqs(
                exit_condition,
                effective_length.get(),
                min_tiles.get(),
                *heated,
                min_extra_run_speed.get(),
                max_extra_run_speed.get(),
            ),
            MainEntranceCondition::ComeInShinecharged {} => {
                self.get_come_in_shinecharged_reqs(exit_condition)
            }
            MainEntranceCondition::ComeInShinechargedJumping {} => {
                self.get_come_in_shinecharged_jumping_reqs(exit_condition)
            }
            MainEntranceCondition::ComeInWithSpark { position } => {
                self.get_come_in_with_spark_reqs(exit_condition, *position)
            }
            MainEntranceCondition::ComeInStutterShinecharging { min_tiles } => {
                self.get_come_in_stutter_shinecharging_reqs(exit_condition, min_tiles.get())
            }
            MainEntranceCondition::ComeInWithBombBoost {} => {
                self.get_come_in_with_bomb_boost_reqs(exit_condition)
            }
            MainEntranceCondition::ComeInWithDoorStuckSetup {
                heated,
                door_orientation,
            } => self.get_come_in_with_door_stuck_setup_reqs(
                exit_condition,
                *heated,
                *door_orientation,
            ),
            MainEntranceCondition::ComeInSpeedballing {
                effective_runway_length,
                heated,
            } => {
                // TODO: once flash suit logic is ready, handle this differently
                self.get_come_in_speedballing_reqs(
                    exit_condition,
                    effective_runway_length.get(),
                    *heated,
                )
            }
            MainEntranceCondition::ComeInWithTemporaryBlue { direction } => {
                self.get_come_in_with_temporary_blue_reqs(exit_condition, *direction)
            }
            MainEntranceCondition::ComeInSpinning {
                unusable_tiles,
                min_extra_run_speed,
                max_extra_run_speed,
            } => self.get_come_in_spinning_reqs(
                exit_condition,
                unusable_tiles.get(),
                min_extra_run_speed.get(),
                max_extra_run_speed.get(),
            ),
            MainEntranceCondition::ComeInBlueSpinning {
                unusable_tiles,
                min_extra_run_speed,
                max_extra_run_speed,
            } => self.get_come_in_blue_spinning_reqs(
                exit_condition,
                unusable_tiles.get(),
                min_extra_run_speed.get(),
                max_extra_run_speed.get(),
            ),
            MainEntranceCondition::ComeInWithMockball {
                adjacent_min_tiles,
                remote_and_landing_min_tiles,
            } => self.get_come_in_with_mockball_reqs(
                exit_condition,
                adjacent_min_tiles.get(),
                remote_and_landing_min_tiles
                    .into_iter()
                    .map(|(a, b)| (a.get(), b.get()))
                    .collect(),
            ),
            MainEntranceCondition::ComeInWithSpringBallBounce {
                adjacent_min_tiles,
                remote_and_landing_min_tiles,
                movement_type,
            } => self.get_come_in_with_spring_ball_bounce_reqs(
                exit_condition,
                adjacent_min_tiles.get(),
                remote_and_landing_min_tiles
                    .into_iter()
                    .map(|(a, b)| (a.get(), b.get()))
                    .collect(),
                *movement_type,
            ),
            MainEntranceCondition::ComeInWithBlueSpringBallBounce {
                min_extra_run_speed,
                max_extra_run_speed,
                min_landing_tiles,
                movement_type,
            } => self.get_come_in_with_blue_spring_ball_bounce_reqs(
                exit_condition,
                min_extra_run_speed.get(),
                max_extra_run_speed.get(),
                min_landing_tiles.get(),
                *movement_type,
            ),
            MainEntranceCondition::ComeInWithRMode {} => {
                self.get_come_in_with_r_mode_reqs(exit_condition)
            }
            MainEntranceCondition::ComeInWithGMode {
                mode,
                morphed,
                mobility,
            } => self.get_come_in_with_g_mode_reqs(
                exit_condition,
                entrance_room_id,
                entrance_node_id,
                *mode,
                *morphed,
                *mobility,
                is_toilet,
            ),
            MainEntranceCondition::ComeInWithStoredFallSpeed {
                fall_speed_in_tiles,
            } => self.get_come_in_with_stored_fall_speed_reqs(exit_condition, *fall_speed_in_tiles),
            MainEntranceCondition::ComeInWithWallJumpBelow { min_height } => {
                self.get_come_in_with_wall_jump_below_reqs(exit_condition, min_height.get())
            }
            MainEntranceCondition::ComeInWithSpaceJumpBelow {} => {
                self.get_come_in_with_space_jump_below_reqs(exit_condition)
            }
            MainEntranceCondition::ComeInWithPlatformBelow {
                min_height,
                max_height,
                max_left_position,
                min_right_position,
            } => self.get_come_in_with_platform_below_reqs(
                exit_condition,
                min_height.get(),
                max_height.get(),
                max_left_position.get(),
                min_right_position.get(),
            ),
            MainEntranceCondition::ComeInWithGrappleTeleport { block_positions } => {
                self.get_come_in_with_grapple_teleport_reqs(exit_condition, block_positions)
            }
        }
    }

    fn get_come_in_normally_reqs(&self, exit_condition: &ExitCondition) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveNormally {} => Some(Requirement::Free),
            _ => None,
        }
    }

    fn get_come_in_running_reqs(
        &self,
        exit_condition: &ExitCondition,
        speed_booster: Option<bool>,
        min_tiles: f32,
        max_tiles: f32,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveWithRunway {
                effective_length,
                heated,
                physics,
                from_exit_node,
            } => {
                let effective_length = effective_length.get();
                if effective_length < min_tiles {
                    return None;
                }
                let mut reqs: Vec<Requirement> = vec![];
                if speed_booster == Some(true) {
                    reqs.push(Requirement::Item(Item::SpeedBooster as ItemId));
                }
                if speed_booster == Some(false) {
                    reqs.push(Requirement::Tech(
                        self.game_data.tech_isv.index_by_key["canDisableEquipment"],
                    ));
                }
                if *physics != Some(Physics::Air) {
                    reqs.push(Requirement::Item(Item::Gravity as ItemId));
                    // TODO: in sm-json-data, add physics property to leaveWithRunway schema (for door nodes with multiple possible physics)
                }
                if *heated {
                    let heat_frames = if *from_exit_node {
                        compute_run_frames(min_tiles) * 2 + 20
                    } else {
                        if effective_length > max_tiles {
                            // 10 heat frames to position after stopping on a dime, before resuming running
                            compute_run_frames(effective_length - max_tiles)
                                + compute_run_frames(max_tiles)
                                + 10
                        } else {
                            compute_run_frames(effective_length)
                        }
                    };
                    reqs.push(Requirement::HeatFrames(heat_frames as Capacity));
                }
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_come_in_space_jumping_reqs(
        &self,
        exit_condition: &ExitCondition,
        speed_booster: Option<bool>,
        min_tiles: f32,
        _max_tiles: f32,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveSpaceJumping {
                remote_runway_length,
                blue,
                ..
            } => {
                // TODO: Take into account any exit constraints on min_extra_run_speed and max_extra_run_speed.
                // Currently there might not be any scenarios where this matters, but that could change?
                // It is awkward because for a non-blue entrance strat like this, the constraints are measured in tiles rather
                // than run speed, though we could convert between the two.
                let remote_runway_length = remote_runway_length.get();
                if *blue == BlueOption::Yes {
                    return None;
                }
                if remote_runway_length < min_tiles {
                    return None;
                }
                let mut reqs: Vec<Requirement> = vec![Requirement::Item(Item::SpaceJump as ItemId)];
                if speed_booster == Some(true) {
                    reqs.push(Requirement::Item(Item::SpeedBooster as ItemId));
                }
                if speed_booster == Some(false) {
                    reqs.push(Requirement::Tech(
                        self.game_data.tech_isv.index_by_key["canDisableEquipment"],
                    ));
                }
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_cross_room_shortcharge_heat_frames(
        &self,
        from_exit_node: bool,
        entrance_length: f32,
        exit_length: f32,
        entrance_heated: bool,
        exit_heated: bool,
    ) -> Capacity {
        let mut total_heat_frames = 0;
        if from_exit_node {
            // Runway in the exiting room starts and ends at the door so we need to run both directions:
            if entrance_heated && exit_heated {
                // Both rooms are heated. Heat frames are optimized by minimizing runway usage in the source room.
                // But since the shortcharge difficulty is not known here, we conservatively assume up to 33 tiles
                // of the combined runway may need to be used. (TODO: Instead add a Requirement enum case to handle this more accurately.)
                let other_runway_length =
                    f32::max(0.0, f32::min(exit_length, 33.0 - entrance_length));
                let heat_frames_1 = compute_run_frames(other_runway_length) + 20;
                let heat_frames_2 = Capacity::max(
                    85,
                    compute_run_frames(other_runway_length + entrance_length),
                );
                // Add 5 lenience frames (partly to account for the possibility of some inexactness in our calculations)
                total_heat_frames += heat_frames_1 + heat_frames_2 + 5;
            } else if !entrance_heated && exit_heated {
                // Only the destination room is heated. Heat frames are optimized by using the full runway in
                // the source room.
                let (_, heat_frames) = compute_shinecharge_frames(exit_length, entrance_length);
                total_heat_frames += heat_frames + 5;
            } else if entrance_heated && !exit_heated {
                // Only the source room is heated. As in the first case above, heat frames are optimized by
                // minimizing runway usage in the source room. (TODO: Use new Requirement enum case.)
                let other_runway_length =
                    f32::max(0.0, f32::min(exit_length, 33.0 - entrance_length));
                let heat_frames_1 = compute_run_frames(other_runway_length) + 20;
                let (heat_frames_2, _) =
                    compute_shinecharge_frames(other_runway_length, entrance_length);
                total_heat_frames += heat_frames_1 + heat_frames_2 + 5;
            }
        } else if entrance_heated || exit_heated {
            // Runway in the other room starts at a different node and runs toward the door. The full combined
            // runway is used.
            let (frames_1, frames_2) = compute_shinecharge_frames(exit_length, entrance_length);
            total_heat_frames += 5;
            if exit_heated {
                // Heat frames for source room
                total_heat_frames += frames_1;
            }
            if entrance_heated {
                // Heat frames for destination room
                total_heat_frames += frames_2;
            }
        }
        total_heat_frames
    }

    fn add_run_speed_reqs(
        &self,
        exit_runway_length: f32,
        exit_min_extra_run_speed: f32,
        exit_max_extra_run_speed: f32,
        exit_heated: bool,
        entrance_min_extra_run_speed: f32,
        entrance_max_extra_run_speed: f32,
        reqs: &mut Vec<Requirement>,
    ) -> bool {
        let shortcharge_min_speed =
            get_shortcharge_min_extra_run_speed(self.difficulty.shine_charge_tiles);
        let shortcharge_max_speed_opt = get_shortcharge_max_extra_run_speed(
            self.difficulty.shine_charge_tiles,
            exit_runway_length,
        );
        let exit_min_speed = f32::max(entrance_min_extra_run_speed, shortcharge_min_speed);
        let exit_max_speed = f32::min(
            entrance_max_extra_run_speed,
            shortcharge_max_speed_opt.unwrap_or(-1.0),
        );
        let overall_min_speed = f32::max(exit_min_speed, exit_min_extra_run_speed);
        let overall_max_speed = f32::min(exit_max_speed, exit_max_extra_run_speed);
        if overall_min_speed > overall_max_speed {
            return false;
        }

        if exit_heated {
            let exit_min_speed = f32::max(
                entrance_min_extra_run_speed,
                get_shortcharge_min_extra_run_speed(self.difficulty.heated_shine_charge_tiles),
            );
            let exit_max_speed = f32::min(
                entrance_max_extra_run_speed,
                get_shortcharge_max_extra_run_speed(
                    self.difficulty.heated_shine_charge_tiles,
                    exit_runway_length,
                )
                .unwrap_or(-1.0),
            );
            let overall_min_speed = f32::max(exit_min_speed, exit_min_extra_run_speed);
            let overall_max_speed = f32::min(exit_max_speed, exit_max_extra_run_speed);
            if overall_min_speed > overall_max_speed {
                reqs.push(Requirement::Item(Item::Varia as usize));
            }
        }
        true
    }

    fn get_come_in_getting_blue_speed_reqs(
        &self,
        exit_condition: &ExitCondition,
        mut runway_length: f32,
        min_tiles: f32,
        runway_heated: bool,
        min_extra_run_speed: f32,
        max_extra_run_speed: f32,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveWithRunway {
                effective_length,
                heated,
                physics,
                from_exit_node,
            } => {
                let mut effective_length = effective_length.get();
                if effective_length < min_tiles {
                    return None;
                }
                if runway_length < 0.0 {
                    // TODO: remove this hack: strats with negative runway length here coming in should use comeInBlueSpinning instead.
                    // add a test on the sm-json-data side to enforce this.
                    effective_length += runway_length;
                    runway_length = 0.0;
                }

                let mut reqs: Vec<Requirement> = vec![];
                let combined_runway_length = effective_length + runway_length;

                if !self.add_run_speed_reqs(
                    combined_runway_length,
                    0.0,
                    7.0,
                    *heated || runway_heated,
                    min_extra_run_speed,
                    max_extra_run_speed,
                    &mut reqs,
                ) {
                    return None;
                }

                reqs.push(Requirement::make_blue_speed(
                    combined_runway_length,
                    runway_heated || *heated,
                ));
                if *physics != Some(Physics::Air) {
                    reqs.push(Requirement::Item(Item::Gravity as ItemId));
                }
                if *heated || runway_heated {
                    let heat_frames = self.get_cross_room_shortcharge_heat_frames(
                        *from_exit_node,
                        runway_length,
                        effective_length,
                        runway_heated,
                        *heated,
                    );
                    reqs.push(Requirement::HeatFrames(heat_frames));
                }
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_come_in_shinecharging_reqs(
        &self,
        exit_condition: &ExitCondition,
        mut runway_length: f32,
        min_tiles: f32,
        runway_heated: bool,
    ) -> Option<Requirement> {
        // TODO: Remove min_tiles here, after strats have been correctly split off using "comeInGettingBlueSpeed".
        match exit_condition {
            ExitCondition::LeaveWithRunway {
                effective_length,
                heated,
                physics,
                from_exit_node,
            } => {
                let mut effective_length = effective_length.get();
                if effective_length < min_tiles {
                    return None;
                }
                if runway_length < 0.0 {
                    // TODO: remove this hack: strats with negative runway length here coming in should use comeInBlueSpinning instead.
                    // add a test on the sm-json-data side to enforce this.
                    effective_length += runway_length;
                    runway_length = 0.0;
                }

                let mut reqs: Vec<Requirement> = vec![];
                let combined_runway_length = effective_length + runway_length;
                reqs.push(Requirement::make_shinecharge(
                    combined_runway_length,
                    runway_heated || *heated,
                ));
                if *physics != Some(Physics::Air) {
                    reqs.push(Requirement::Item(Item::Gravity as ItemId));
                }
                if *heated || runway_heated {
                    let heat_frames = self.get_cross_room_shortcharge_heat_frames(
                        *from_exit_node,
                        runway_length,
                        effective_length,
                        runway_heated,
                        *heated,
                    );
                    reqs.push(Requirement::HeatFrames(heat_frames));
                }
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_come_in_speedballing_reqs(
        &self,
        exit_condition: &ExitCondition,
        mut runway_length: f32,
        runway_heated: bool,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveWithRunway {
                effective_length,
                heated,
                physics,
                from_exit_node,
            } => {
                let mut effective_length = effective_length.get();
                if runway_length < 0.0 {
                    // TODO: remove this hack: strats with negative runway length here coming in should use comeInBlueSpinning instead.
                    // add a test on the sm-json-data side to enforce this.
                    effective_length += runway_length;
                    runway_length = 0.0;
                }

                let mut reqs: Vec<Requirement> = vec![Requirement::Tech(
                    self.game_data.tech_isv.index_by_key["canSpeedball"],
                )];
                let combined_runway_length = effective_length + runway_length;
                reqs.push(Requirement::SpeedBall {
                    used_tiles: Float::new(combined_runway_length),
                    heated: *heated || runway_heated,
                });
                if *physics != Some(Physics::Air) {
                    reqs.push(Requirement::Item(Item::Gravity as ItemId));
                }
                if *heated || runway_heated {
                    // Speedball would technically have slightly different heat frames (compared to a shortcharge) since you no longer
                    // gaining run speed while in the air, but this is a small enough difference to neglect for now. There should be
                    // enough lenience in the heat frame calculation already to account for it.
                    let heat_frames = self.get_cross_room_shortcharge_heat_frames(
                        *from_exit_node,
                        runway_length,
                        effective_length,
                        runway_heated,
                        *heated,
                    );
                    reqs.push(Requirement::HeatFrames(heat_frames));
                }
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_come_in_spinning_reqs(
        &self,
        exit_condition: &ExitCondition,
        unusable_tiles: f32,
        entrance_min_extra_run_speed: f32,
        entrance_max_extra_run_speed: f32,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveSpinning {
                remote_runway_length,
                blue,
                heated: _,
                min_extra_run_speed,
                max_extra_run_speed,
            } => {
                let remote_runway_length = remote_runway_length.get();
                let min_extra_run_speed = min_extra_run_speed.get();
                let max_extra_run_speed = max_extra_run_speed.get();
                let runway_max_speed = get_max_extra_run_speed(remote_runway_length);

                let overall_max_extra_run_speed = f32::min(
                    max_extra_run_speed,
                    f32::min(entrance_max_extra_run_speed, runway_max_speed),
                );
                let overall_min_extra_run_speed =
                    f32::max(min_extra_run_speed, entrance_min_extra_run_speed);

                if overall_min_extra_run_speed > overall_max_extra_run_speed {
                    return None;
                }
                if *blue == BlueOption::Yes {
                    return None;
                }
                Some(Requirement::Free)
            }
            ExitCondition::LeaveWithRunway {
                effective_length,
                heated,
                physics,
                from_exit_node,
            } => {
                let effective_length = effective_length.get();
                let mut reqs: Vec<Requirement> = vec![];

                if *physics != Some(Physics::Air) {
                    reqs.push(Requirement::Item(Item::Gravity as ItemId));
                }

                let min_tiles = get_extra_run_speed_tiles(entrance_min_extra_run_speed);
                let max_tiles = get_extra_run_speed_tiles(entrance_max_extra_run_speed);

                if min_tiles > effective_length - unusable_tiles {
                    return None;
                }

                if *heated {
                    let heat_frames = if *from_exit_node {
                        compute_run_frames(min_tiles + unusable_tiles) * 2 + 20
                    } else {
                        if max_tiles < effective_length - unusable_tiles {
                            // 10 heat frames to position after stopping on a dime, before resuming running
                            compute_run_frames(effective_length - unusable_tiles - max_tiles)
                                + compute_run_frames(max_tiles + unusable_tiles)
                                + 10
                        } else {
                            compute_run_frames(effective_length)
                        }
                    };
                    reqs.push(Requirement::HeatFrames(heat_frames as Capacity));
                }
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_come_in_blue_spinning_reqs(
        &self,
        exit_condition: &ExitCondition,
        unusable_tiles: f32,
        entrance_min_extra_run_speed: f32,
        entrance_max_extra_run_speed: f32,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveSpinning {
                remote_runway_length,
                blue,
                heated,
                min_extra_run_speed,
                max_extra_run_speed,
            } => {
                let mut reqs: Vec<Requirement> = vec![];

                if !self.add_run_speed_reqs(
                    remote_runway_length.get(),
                    min_extra_run_speed.get(),
                    max_extra_run_speed.get(),
                    *heated,
                    entrance_min_extra_run_speed,
                    entrance_max_extra_run_speed,
                    &mut reqs,
                ) {
                    return None;
                }
                if *blue == BlueOption::No {
                    return None;
                }
                Some(Requirement::make_shinecharge(
                    remote_runway_length.get(),
                    *heated,
                ))
            }
            ExitCondition::LeaveWithRunway {
                effective_length,
                heated,
                physics,
                from_exit_node,
            } => {
                let effective_length = effective_length.get();
                let mut reqs: Vec<Requirement> = vec![];

                if !self.add_run_speed_reqs(
                    effective_length,
                    0.0,
                    7.0,
                    *heated,
                    entrance_min_extra_run_speed,
                    entrance_max_extra_run_speed,
                    &mut reqs,
                ) {
                    return None;
                }

                reqs.push(Requirement::make_shinecharge(
                    effective_length - unusable_tiles,
                    *heated,
                ));
                if *physics != Some(Physics::Air) {
                    reqs.push(Requirement::Item(Item::Gravity as ItemId));
                }
                if *from_exit_node {
                    // Runway in the other room starts and ends at the door so we need to run both directions:
                    if *heated {
                        // Shortcharge difficulty is not known here, so we conservatively assume up to 33 tiles
                        // of runway may need to be used. (TODO: Instead add a Requirement enum case to handle this more accurately.)
                        let other_runway_length = f32::min(effective_length, 33.0 + unusable_tiles);
                        let heat_frames_1 = compute_run_frames(other_runway_length) + 20;
                        let (heat_frames_2, _) =
                            compute_shinecharge_frames(other_runway_length, 0.0);
                        reqs.push(Requirement::HeatFrames(heat_frames_1 + heat_frames_2 + 5));
                    }
                } else if *heated {
                    // Runway in the other room starts at a different node and runs toward the door. The full combined
                    // runway is used.
                    let (frames_1, _) = compute_shinecharge_frames(effective_length, 0.0);
                    let heat_frames = frames_1 + 5;
                    reqs.push(Requirement::HeatFrames(heat_frames));
                }
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_come_in_with_mockball_reqs(
        &self,
        exit_condition: &ExitCondition,
        adjacent_min_tiles: f32,
        remote_and_landing_min_tiles: Vec<(f32, f32)>,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveWithMockball {
                remote_runway_length,
                landing_runway_length,
                blue,
                ..
            } => {
                // TODO: Take into account any exit constraints on min_extra_run_speed and max_extra_run_speed.
                let remote_runway_length = remote_runway_length.get();
                let landing_runway_length = landing_runway_length.get();

                if *blue == BlueOption::Yes {
                    return None;
                }
                if !remote_and_landing_min_tiles
                    .iter()
                    .any(|(r, d)| *r <= remote_runway_length && *d <= landing_runway_length)
                {
                    return None;
                }
                Some(Requirement::make_and(vec![
                    Requirement::Tech(self.game_data.tech_isv.index_by_key["canMockball"]),
                    Requirement::Item(Item::Morph as ItemId),
                ]))
            }
            ExitCondition::LeaveWithRunway {
                effective_length,
                heated,
                physics,
                from_exit_node,
            } => {
                let effective_length = effective_length.get();
                if effective_length < adjacent_min_tiles {
                    return None;
                }
                let mut reqs: Vec<Requirement> = vec![];
                if *physics != Some(Physics::Air) {
                    reqs.push(Requirement::Item(Item::Gravity as ItemId));
                    // TODO: in sm-json-data, add physics property to leaveWithRunway schema (for door nodes with multiple possible physics)
                }
                if *heated {
                    let heat_frames = if *from_exit_node {
                        compute_run_frames(adjacent_min_tiles) * 2 + 20
                    } else {
                        compute_run_frames(effective_length)
                    };
                    reqs.push(Requirement::HeatFrames(heat_frames));
                }
                reqs.push(Requirement::Tech(
                    self.game_data.tech_isv.index_by_key["canMockball"],
                ));
                reqs.push(Requirement::Item(Item::Morph as ItemId));
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_come_in_with_spring_ball_bounce_reqs(
        &self,
        exit_condition: &ExitCondition,
        adjacent_min_tiles: f32,
        remote_and_landing_min_tiles: Vec<(f32, f32)>,
        exit_movement_type: BounceMovementType,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveWithMockball {
                remote_runway_length,
                landing_runway_length,
                blue,
                ..
            } => {
                // TODO: Take into account any exit constraints on min_extra_run_speed and max_extra_run_speed.
                // Currently there might not be any scenarios where this matters, but that could change?
                // It is awkward because for a non-blue entrance strat like this, the constraints are measured in tiles rather
                // than run speed, though we could convert between the two.
                let remote_runway_length = remote_runway_length.get();
                let landing_runway_length = landing_runway_length.get();
                if *blue == BlueOption::Yes {
                    return None;
                }
                if !remote_and_landing_min_tiles
                    .iter()
                    .any(|(r, d)| *r <= remote_runway_length && *d <= landing_runway_length)
                {
                    return None;
                }
                if exit_movement_type == BounceMovementType::Uncontrolled {
                    return None;
                }
                let mut reqs: Vec<Requirement> = vec![];
                reqs.push(Requirement::Tech(
                    self.game_data.tech_isv.index_by_key["canMockball"],
                ));
                reqs.push(Requirement::Tech(
                    self.game_data.tech_isv.index_by_key["canSpringBallBounce"],
                ));
                reqs.push(Requirement::Item(Item::Morph as ItemId));
                reqs.push(Requirement::Item(Item::SpringBall as ItemId));
                Some(Requirement::make_and(reqs))
            }
            ExitCondition::LeaveWithSpringBallBounce {
                remote_runway_length,
                landing_runway_length,
                blue,
                movement_type,
                ..
            } => {
                // TODO: Take into account any exit constraints on min_extra_run_speed and max_extra_run_speed.
                let remote_runway_length = remote_runway_length.get();
                let landing_runway_length = landing_runway_length.get();
                if *blue == BlueOption::Yes {
                    return None;
                }
                if !remote_and_landing_min_tiles
                    .iter()
                    .any(|(r, d)| *r <= remote_runway_length && *d <= landing_runway_length)
                {
                    return None;
                }
                if *movement_type != exit_movement_type
                    && *movement_type != BounceMovementType::Any
                    && exit_movement_type != BounceMovementType::Any
                {
                    return None;
                }
                let mut reqs: Vec<Requirement> = vec![];
                if *movement_type == BounceMovementType::Controlled
                    || exit_movement_type == BounceMovementType::Controlled
                {
                    reqs.push(Requirement::Tech(
                        self.game_data.tech_isv.index_by_key["canMockball"],
                    ));
                }
                reqs.push(Requirement::Tech(
                    self.game_data.tech_isv.index_by_key["canSpringBallBounce"],
                ));
                reqs.push(Requirement::Item(Item::Morph as ItemId));
                reqs.push(Requirement::Item(Item::SpringBall as ItemId));
                Some(Requirement::make_and(reqs))
            }
            ExitCondition::LeaveWithRunway {
                effective_length,
                heated,
                physics,
                from_exit_node,
            } => {
                let effective_length = effective_length.get();
                if effective_length < adjacent_min_tiles {
                    return None;
                }
                let mut reqs: Vec<Requirement> = vec![];
                if *physics != Some(Physics::Air) {
                    reqs.push(Requirement::Item(Item::Gravity as ItemId));
                }
                if *heated {
                    let heat_frames = if *from_exit_node {
                        compute_run_frames(adjacent_min_tiles) * 2 + 20
                    } else {
                        compute_run_frames(effective_length)
                    };
                    reqs.push(Requirement::HeatFrames(heat_frames));
                }
                if exit_movement_type == BounceMovementType::Controlled {
                    reqs.push(Requirement::Tech(
                        self.game_data.tech_isv.index_by_key["canMockball"],
                    ));
                }
                reqs.push(Requirement::Tech(
                    self.game_data.tech_isv.index_by_key["canSpringBallBounce"],
                ));
                reqs.push(Requirement::Item(Item::Morph as ItemId));
                reqs.push(Requirement::Item(Item::SpringBall as ItemId));
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_come_in_with_blue_spring_ball_bounce_reqs(
        &self,
        exit_condition: &ExitCondition,
        entrance_min_extra_run_speed: f32,
        entrance_max_extra_run_speed: f32,
        min_landing_tiles: f32,
        entrance_movement_type: BounceMovementType,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveWithMockball {
                remote_runway_length,
                landing_runway_length,
                blue,
                heated,
                min_extra_run_speed,
                max_extra_run_speed,
            } => {
                let remote_runway_length = remote_runway_length.get();
                let landing_runway_length = landing_runway_length.get();
                if *blue == BlueOption::Yes {
                    return None;
                }
                if entrance_movement_type == BounceMovementType::Uncontrolled {
                    return None;
                }
                if landing_runway_length < min_landing_tiles {
                    return None;
                }
                let mut reqs: Vec<Requirement> = vec![];

                if !self.add_run_speed_reqs(
                    remote_runway_length,
                    min_extra_run_speed.get(),
                    max_extra_run_speed.get(),
                    *heated,
                    entrance_min_extra_run_speed,
                    entrance_max_extra_run_speed,
                    &mut reqs,
                ) {
                    return None;
                }

                reqs.push(Requirement::Tech(
                    self.game_data.tech_isv.index_by_key["canMockball"],
                ));
                reqs.push(Requirement::Tech(
                    self.game_data.tech_isv.index_by_key["canSpringBallBounce"],
                ));
                reqs.push(Requirement::Item(Item::SpeedBooster as ItemId));
                reqs.push(Requirement::Item(Item::Morph as ItemId));
                reqs.push(Requirement::Item(Item::SpringBall as ItemId));
                Some(Requirement::make_and(reqs))
            }
            ExitCondition::LeaveWithSpringBallBounce {
                remote_runway_length,
                landing_runway_length,
                blue,
                heated,
                movement_type,
                min_extra_run_speed,
                max_extra_run_speed,
            } => {
                let remote_runway_length = remote_runway_length.get();
                let landing_runway_length = landing_runway_length.get();
                if *blue == BlueOption::Yes {
                    return None;
                }
                if landing_runway_length < min_landing_tiles {
                    return None;
                }
                if *movement_type != entrance_movement_type
                    && *movement_type != BounceMovementType::Any
                    && entrance_movement_type != BounceMovementType::Any
                {
                    return None;
                }
                let mut reqs: Vec<Requirement> = vec![];

                if !self.add_run_speed_reqs(
                    remote_runway_length,
                    min_extra_run_speed.get(),
                    max_extra_run_speed.get(),
                    *heated,
                    entrance_min_extra_run_speed,
                    entrance_max_extra_run_speed,
                    &mut reqs,
                ) {
                    return None;
                }

                if *movement_type == BounceMovementType::Controlled
                    || entrance_movement_type == BounceMovementType::Controlled
                {
                    reqs.push(Requirement::Tech(
                        self.game_data.tech_isv.index_by_key["canMockball"],
                    ));
                }
                reqs.push(Requirement::Tech(
                    self.game_data.tech_isv.index_by_key["canSpringBallBounce"],
                ));
                reqs.push(Requirement::Item(Item::SpeedBooster as ItemId));
                reqs.push(Requirement::Item(Item::Morph as ItemId));
                reqs.push(Requirement::Item(Item::SpringBall as ItemId));
                Some(Requirement::make_and(reqs))
            }
            ExitCondition::LeaveWithRunway {
                effective_length,
                heated,
                physics,
                from_exit_node,
            } => {
                let effective_length = effective_length.get();
                let mut reqs: Vec<Requirement> = vec![];

                if !self.add_run_speed_reqs(
                    effective_length,
                    0.0,
                    7.0,
                    *heated,
                    entrance_min_extra_run_speed,
                    entrance_max_extra_run_speed,
                    &mut reqs,
                ) {
                    return None;
                }

                if *physics != Some(Physics::Air) {
                    reqs.push(Requirement::Item(Item::Gravity as ItemId));
                }
                if *heated {
                    let heat_frames = if *from_exit_node {
                        // For now, be conservative by assuming we use the whole runway. This could be refined later:
                        compute_run_frames(effective_length) * 2 + 20
                    } else {
                        compute_run_frames(effective_length)
                    };
                    reqs.push(Requirement::HeatFrames(heat_frames));
                }
                if entrance_movement_type == BounceMovementType::Controlled {
                    reqs.push(Requirement::Tech(
                        self.game_data.tech_isv.index_by_key["canMockball"],
                    ));
                }
                reqs.push(Requirement::Tech(
                    self.game_data.tech_isv.index_by_key["canSpringBallBounce"],
                ));
                reqs.push(Requirement::Item(Item::SpeedBooster as ItemId));
                reqs.push(Requirement::Item(Item::Morph as ItemId));
                reqs.push(Requirement::Item(Item::SpringBall as ItemId));
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_come_in_shinecharged_reqs(&self, exit_condition: &ExitCondition) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveShinecharged { .. } => Some(Requirement::Free),
            ExitCondition::LeaveWithRunway {
                effective_length,
                heated,
                physics,
                from_exit_node,
            } => {
                let effective_length = effective_length.get();
                let mut reqs: Vec<Requirement> = vec![];
                reqs.push(Requirement::make_shinecharge(effective_length, *heated));
                reqs.push(Requirement::ShineChargeFrames(10)); // Assume shinecharge is obtained 10 frames before going through door.
                if *physics != Some(Physics::Air) {
                    reqs.push(Requirement::Item(Item::Gravity as ItemId));
                }
                if *heated {
                    if *from_exit_node {
                        let runway_length = f32::min(33.0, effective_length);
                        let run_frames = compute_run_frames(runway_length);
                        let heat_frames_1 = run_frames + 20;
                        let heat_frames_2 = Capacity::max(85, run_frames);
                        reqs.push(Requirement::HeatFrames(heat_frames_1 + heat_frames_2 + 15));
                    } else {
                        let heat_frames = Capacity::max(85, compute_run_frames(effective_length));
                        reqs.push(Requirement::HeatFrames(heat_frames + 5));
                    }
                }
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_come_in_shinecharged_jumping_reqs(
        &self,
        exit_condition: &ExitCondition,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveShinecharged { physics } => {
                if *physics != Some(Physics::Air) {
                    return None;
                }
                Some(Requirement::Free)
            }
            ExitCondition::LeaveWithRunway {
                effective_length,
                heated,
                physics,
                from_exit_node,
            } => {
                let effective_length = effective_length.get();
                let mut reqs: Vec<Requirement> = vec![];
                reqs.push(Requirement::make_shinecharge(effective_length, *heated));
                reqs.push(Requirement::ShineChargeFrames(10)); // Assume shinecharge is obtained 10 frames before going through door.
                if *physics != Some(Physics::Air) {
                    reqs.push(Requirement::Item(Item::Gravity as ItemId));
                }
                if *heated {
                    if *from_exit_node {
                        let runway_length = f32::min(33.0, effective_length);
                        let run_frames = compute_run_frames(runway_length);
                        let heat_frames_1 = run_frames + 20;
                        let heat_frames_2 = Capacity::max(85, run_frames);
                        reqs.push(Requirement::HeatFrames(heat_frames_1 + heat_frames_2 + 15));
                    } else {
                        let heat_frames = Capacity::max(85, compute_run_frames(effective_length));
                        reqs.push(Requirement::HeatFrames(heat_frames + 5));
                    }
                }
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_come_in_stutter_shinecharging_reqs(
        &self,
        exit_condition: &ExitCondition,
        min_tiles: f32,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveWithRunway {
                effective_length,
                heated,
                physics,
                from_exit_node,
            } => {
                let effective_length = effective_length.get();
                if *physics != Some(Physics::Air) {
                    return None;
                }
                if effective_length < min_tiles {
                    return None;
                }
                let mut reqs: Vec<Requirement> = vec![];
                reqs.push(Requirement::Tech(
                    self.game_data.tech_isv.index_by_key["canStutterWaterShineCharge"],
                ));
                reqs.push(Requirement::Item(Item::SpeedBooster as ItemId));
                if *heated {
                    let heat_frames = if *from_exit_node {
                        compute_run_frames(min_tiles) * 2 + 20
                    } else {
                        compute_run_frames(effective_length)
                    };
                    reqs.push(Requirement::HeatFrames(heat_frames));
                }
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_come_in_with_spark_reqs(
        &self,
        exit_condition: &ExitCondition,
        come_in_position: SparkPosition,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveWithSpark { position } => {
                if *position == come_in_position
                    || *position == SparkPosition::Any
                    || come_in_position == SparkPosition::Any
                {
                    Some(Requirement::Free)
                } else {
                    None
                }
            }
            ExitCondition::LeaveShinecharged { .. } => {
                // Shinecharge frames are handled through Requirement::ShineChargeFrames
                Some(Requirement::Free)
            }
            ExitCondition::LeaveWithRunway {
                effective_length,
                heated,
                physics,
                from_exit_node,
            } => {
                let effective_length = effective_length.get();
                let mut reqs: Vec<Requirement> = vec![];
                reqs.push(Requirement::make_shinecharge(effective_length, *heated));
                if *physics != Some(Physics::Air) {
                    reqs.push(Requirement::Item(Item::Gravity as ItemId));
                }
                if *heated {
                    if *from_exit_node {
                        let runway_length = f32::min(33.0, effective_length);
                        let run_frames = compute_run_frames(runway_length);
                        let heat_frames_1 = run_frames + 20;
                        let heat_frames_2 = Capacity::max(85, run_frames);
                        reqs.push(Requirement::HeatFrames(heat_frames_1 + heat_frames_2 + 5));
                    } else {
                        let heat_frames = Capacity::max(85, compute_run_frames(effective_length));
                        reqs.push(Requirement::HeatFrames(heat_frames + 5));
                    }
                }
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_come_in_with_temporary_blue_reqs(
        &self,
        exit_condition: &ExitCondition,
        exit_direction: TemporaryBlueDirection,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveWithTemporaryBlue { direction } => {
                if *direction != exit_direction
                    && *direction != TemporaryBlueDirection::Any
                    && exit_direction != TemporaryBlueDirection::Any
                {
                    return None;
                }
                let mut reqs: Vec<Requirement> = vec![];
                reqs.push(Requirement::Tech(
                    self.game_data.tech_isv.index_by_key["canTemporaryBlue"],
                ));
                Some(Requirement::make_and(reqs))
            }
            ExitCondition::LeaveWithRunway {
                effective_length,
                heated,
                physics,
                from_exit_node,
            } => {
                let effective_length = effective_length.get();
                let mut reqs: Vec<Requirement> = vec![];
                reqs.push(Requirement::Tech(
                    self.game_data.tech_isv.index_by_key["canTemporaryBlue"],
                ));
                reqs.push(Requirement::make_shinecharge(effective_length, *heated));
                if *physics != Some(Physics::Air) {
                    reqs.push(Requirement::Item(Item::Gravity as ItemId));
                }
                if *heated {
                    let heat_frames_temp_blue = 200;
                    if *from_exit_node {
                        let runway_length = f32::min(33.0, effective_length);
                        let run_frames = compute_run_frames(runway_length);
                        let heat_frames_1 = run_frames + 20;
                        let heat_frames_2 = Capacity::max(85, run_frames);
                        reqs.push(Requirement::HeatFrames(
                            heat_frames_1 + heat_frames_2 + heat_frames_temp_blue + 15,
                        ));
                    } else {
                        let heat_frames = Capacity::max(85, compute_run_frames(effective_length));
                        reqs.push(Requirement::HeatFrames(
                            heat_frames + heat_frames_temp_blue + 5,
                        ));
                    }
                }
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_come_in_with_bomb_boost_reqs(
        &self,
        exit_condition: &ExitCondition,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveWithRunway {
                effective_length,
                heated,
                physics,
                from_exit_node,
            } => {
                let effective_length = effective_length.get();
                let mut reqs: Vec<Requirement> = vec![];
                if *physics != Some(Physics::Air) {
                    return None;
                }
                reqs.push(Requirement::And(vec![
                    Requirement::Item(Item::Morph as ItemId),
                    Requirement::Or(vec![
                        Requirement::Item(Item::Bombs as ItemId),
                        Requirement::PowerBombs(1),
                    ]),
                ]));
                if *heated {
                    let mut heat_frames = 100;
                    if *from_exit_node {
                        heat_frames += compute_run_frames(effective_length);
                    }
                    reqs.push(Requirement::HeatFrames(heat_frames));
                }
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_come_in_with_door_stuck_setup_reqs(
        &self,
        exit_condition: &ExitCondition,
        entrance_heated: bool,
        door_orientation: DoorOrientation,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveWithRunway {
                heated,
                physics,
                from_exit_node,
                ..
            } => {
                if !from_exit_node {
                    return None;
                }
                let mut reqs: Vec<Requirement> = vec![];
                reqs.push(Requirement::Tech(
                    self.game_data.tech_isv.index_by_key["canStationarySpinJump"],
                ));
                if door_orientation == DoorOrientation::Right {
                    reqs.push(Requirement::Tech(
                        self.game_data.tech_isv.index_by_key["canRightSideDoorStuck"],
                    ));
                    if *physics != Some(Physics::Air) {
                        reqs.push(Requirement::Or(vec![
                            Requirement::Item(Item::Gravity as ItemId),
                            Requirement::Tech(
                                self.game_data.tech_isv.index_by_key
                                    ["canRightSideDoorStuckFromWater"],
                            ),
                        ]));
                    }
                }
                let mut heat_frames_per_attempt = 0;
                if *heated {
                    heat_frames_per_attempt += 100;
                }
                if entrance_heated {
                    heat_frames_per_attempt += 50;
                }
                if heat_frames_per_attempt > 0 {
                    reqs.push(Requirement::HeatFrames(heat_frames_per_attempt));
                    reqs.push(Requirement::HeatedDoorStuckLeniency {
                        heat_frames: heat_frames_per_attempt,
                    })
                }
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_come_in_with_r_mode_reqs(&self, exit_condition: &ExitCondition) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveWithGModeSetup { .. } => {
                let mut reqs: Vec<Requirement> = vec![];
                reqs.push(Requirement::Tech(
                    self.game_data.tech_isv.index_by_key["canEnterRMode"],
                ));
                reqs.push(Requirement::Item(Item::XRayScope as ItemId));
                reqs.push(Requirement::ReserveTrigger {
                    min_reserve_energy: 1,
                    max_reserve_energy: 400,
                });
                Some(Requirement::make_and(reqs))
            }
            _ => None,
        }
    }

    fn get_come_in_with_g_mode_reqs(
        &self,
        exit_condition: &ExitCondition,
        entrance_room_id: RoomId,
        entrance_node_id: NodeId,
        mut mode: GModeMode,
        entrance_morphed: bool,
        mobility: GModeMobility,
        is_toilet: bool,
    ) -> Option<Requirement> {
        if is_toilet {
            // Take into account that obtaining direct G-mode in the Toilet is not possible.
            match mode {
                GModeMode::Any => {
                    mode = GModeMode::Indirect;
                }
                GModeMode::Direct => {
                    return None;
                }
                GModeMode::Indirect => {}
            }
        }

        let empty_vec = vec![];
        let regain_mobility_vec = self
            .game_data
            .node_gmode_regain_mobility
            .get(&(entrance_room_id, entrance_node_id))
            .unwrap_or(&empty_vec);
        match exit_condition {
            ExitCondition::LeaveWithGModeSetup { knockback } => {
                if mode == GModeMode::Indirect {
                    return None;
                }
                let mut reqs: Vec<Requirement> = vec![];
                reqs.push(Requirement::Tech(
                    self.game_data.tech_isv.index_by_key["canEnterGMode"],
                ));
                if entrance_morphed {
                    reqs.push(Requirement::Or(vec![
                        Requirement::Tech(
                            self.game_data.tech_isv.index_by_key["canArtificialMorph"],
                        ),
                        Requirement::Item(Item::Morph as ItemId),
                    ]));
                }
                reqs.push(Requirement::Item(Item::XRayScope as ItemId));

                let mobile_req = if *knockback {
                    Requirement::ReserveTrigger {
                        min_reserve_energy: 1,
                        max_reserve_energy: 4,
                    }
                } else {
                    Requirement::Never
                };
                let immobile_req = if regain_mobility_vec.len() > 0 {
                    let mut immobile_req_or_vec: Vec<Requirement> = Vec::new();
                    for (regain_mobility_link, _) in regain_mobility_vec {
                        immobile_req_or_vec.push(Requirement::make_and(vec![
                            Requirement::Tech(
                                self.game_data.tech_isv.index_by_key["canEnterGModeImmobile"],
                            ),
                            Requirement::ReserveTrigger {
                                min_reserve_energy: 1,
                                max_reserve_energy: 400,
                            },
                            regain_mobility_link.requirement.clone(),
                        ]));
                    }
                    Requirement::make_or(immobile_req_or_vec)
                } else {
                    Requirement::Never
                };

                match mobility {
                    GModeMobility::Any => {
                        reqs.push(Requirement::make_or(vec![mobile_req, immobile_req]));
                    }
                    GModeMobility::Mobile => {
                        reqs.push(mobile_req);
                    }
                    GModeMobility::Immobile => {
                        reqs.push(immobile_req);
                    }
                }

                Some(Requirement::make_and(reqs))
            }
            ExitCondition::LeaveWithGMode { morphed } => {
                if mode == GModeMode::Direct {
                    return None;
                }
                if !morphed && entrance_morphed {
                    Some(Requirement::Item(Item::Morph as ItemId))
                } else {
                    Some(Requirement::Free)
                }
            }
            _ => None,
        }
    }

    fn get_come_in_with_stored_fall_speed_reqs(
        &self,
        exit_condition: &ExitCondition,
        fall_speed_in_tiles_needed: i32,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveWithStoredFallSpeed {
                fall_speed_in_tiles,
            } => {
                if *fall_speed_in_tiles != fall_speed_in_tiles_needed {
                    return None;
                }
                return Some(Requirement::Tech(
                    self.game_data.tech_isv.index_by_key["canMoonfall"],
                ));
            }
            _ => None,
        }
    }

    fn get_come_in_with_wall_jump_below_reqs(
        &self,
        exit_condition: &ExitCondition,
        min_height: f32,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveWithDoorFrameBelow { height, .. } => {
                let height = height.get();
                if height < min_height {
                    return None;
                }
                return Some(Requirement::Walljump);
            }
            _ => None,
        }
    }

    fn get_come_in_with_space_jump_below_reqs(
        &self,
        exit_condition: &ExitCondition,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveWithDoorFrameBelow { heated, .. } => {
                let mut reqs_and_vec = vec![];

                reqs_and_vec.push(Requirement::Item(
                    self.game_data.item_isv.index_by_key["SpaceJump"],
                ));
                if *heated {
                    reqs_and_vec.push(Requirement::HeatFrames(30));
                }
                return Some(Requirement::make_and(reqs_and_vec));
            }
            _ => None,
        }
    }

    fn get_come_in_with_grapple_teleport_reqs(
        &self,
        exit_condition: &ExitCondition,
        entrance_block_positions: &[(u16, u16)],
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveWithGrappleTeleport { block_positions } => {
                let entrance_block_positions_set: HashSet<(u16, u16)> =
                    entrance_block_positions.iter().copied().collect();
                if block_positions
                    .iter()
                    .any(|x| entrance_block_positions_set.contains(x))
                {
                    Some(Requirement::make_and(vec![
                        Requirement::Tech(
                            self.game_data.tech_isv.index_by_key["canGrappleTeleport"],
                        ),
                        Requirement::Item(self.game_data.item_isv.index_by_key["Grapple"]),
                    ]))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn get_come_in_with_platform_below_reqs(
        &self,
        exit_condition: &ExitCondition,
        min_height: f32,
        max_height: f32,
        max_left_position: f32,
        min_right_position: f32,
    ) -> Option<Requirement> {
        match exit_condition {
            ExitCondition::LeaveWithPlatformBelow {
                height,
                left_position,
                right_position,
            } => {
                let height = height.get();
                let left_position = left_position.get();
                let right_position = right_position.get();
                if height < min_height || height > max_height {
                    return None;
                }
                if left_position > max_left_position {
                    return None;
                }
                if right_position < min_right_position {
                    return None;
                }
                return Some(Requirement::Free);
            }
            _ => None,
        }
    }
}

fn get_randomizable_doors(
    game_data: &GameData,
    difficulty: &DifficultyConfig,
) -> HashSet<DoorPtrPair> {
    // Doors which we do not want to randomize:
    let mut non_randomizable_doors: HashSet<DoorPtrPair> = vec![
        // Gray doors - Pirate rooms:
        (0x18B7A, 0x18B62), // Pit Room left
        (0x18B86, 0x18B92), // Pit Room right
        (0x19192, 0x1917A), // Baby Kraid left
        (0x1919E, 0x191AA), // Baby Kraid right
        (0x1A558, 0x1A54C), // Plasma Room
        (0x19A32, 0x19966), // Metal Pirates left
        (0x19A3E, 0x19A1A), // Metal Pirates right
        // Gray doors - Bosses:
        (0x191CE, 0x191B6), // Kraid left
        (0x191DA, 0x19252), // Kraid right
        (0x1A2C4, 0x1A2AC), // Phantoon
        (0x1A978, 0x1A924), // Draygon left
        (0x1A96C, 0x1A840), // Draygon right
        (0x198B2, 0x19A62), // Ridley left
        (0x198BE, 0x198CA), // Ridley right
        (0x1AA8C, 0x1AAE0), // Mother Brain left
        (0x1AA80, 0x1AAC8), // Mother Brain right
        // Gray doors - Minibosses:
        (0x18BAA, 0x18BC2), // Bomb Torizo
        (0x18E56, 0x18E3E), // Spore Spawn bottom
        (0x193EA, 0x193D2), // Crocomire top
        (0x1A90C, 0x1A774), // Botwoon left
        (0x19882, 0x19A86), // Golden Torizo right
        // Save stations:
        (0x189BE, 0x1899A), // Crateria Save Room
        (0x19006, 0x18D12), // Green Brinstar Main Shaft Save Room
        (0x19012, 0x18F52), // Etecoon Save Room
        (0x18FD6, 0x18DF6), // Big Pink Save Room
        (0x1926A, 0x190D2), // Caterpillar Save Room
        (0x1925E, 0x19186), // Warehouse Save Room
        (0x1A828, 0x1A744), // Aqueduct Save Room
        (0x1A888, 0x1A7EC), // Draygon Save Room left
        (0x1A87C, 0x1A930), // Draygon Save Room right
        (0x1A5F4, 0x1A588), // Forgotten Highway Save Room
        (0x1A324, 0x1A354), // Glass Tunnel Save Room
        (0x19822, 0x193BA), // Crocomire Save Room
        (0x19462, 0x19456), // Post Crocomire Save Room
        (0x1982E, 0x19702), // Lower Norfair Elevator Save Room
        (0x19816, 0x192FA), // Frog Savestation left
        (0x1980A, 0x197DA), // Frog Savestation right
        (0x197CE, 0x1959A), // Bubble Mountain Save Room
        (0x19AB6, 0x19A0E), // Red Kihunter Shaft Save Room
        (0x1A318, 0x1A240), // Wrecked Ship Save Room
        (0x1AAD4, 0x1AABC), // Lower Tourian Save Room
        // Map stations:
        (0x18C2E, 0x18BDA), // Crateria Map Room
        (0x18D72, 0x18D36), // Brinstar Map Room
        (0x197C2, 0x19306), // Norfair Map Room
        (0x1A5E8, 0x1A51C), // Maridia Map Room
        (0x1A2B8, 0x1A2A0), // Wrecked Ship Map Room
        (0x1AB40, 0x1A99C), // Tourian Map Room (Upper Tourian Save Room)
        // Refill stations:
        (0x18D96, 0x18D7E), // Green Brinstar Missile Refill Room
        (0x18F6A, 0x18DBA), // Dachora Energy Refill Room
        (0x191FE, 0x1904E), // Sloaters Refill
        (0x1A894, 0x1A8F4), // Maridia Missile Refill Room
        (0x1A930, 0x1A87C), // Maridia Health Refill Room
        (0x19786, 0x19756), // Nutella Refill left
        (0x19792, 0x1976E), // Nutella Refill right
        (0x1920A, 0x191C2), // Kraid Recharge Station
        (0x198A6, 0x19A7A), // Golden Torizo Energy Recharge
        (0x1AA74, 0x1AA68), // Tourian Recharge Room
        // Pants room interior door
        (0x1A7A4, 0x1A78C), // Left door
        (0x1A78C, 0x1A7A4), // Right door
        // Items: (to avoid an interaction in map tiles between doors disappearing and items disappearing)
        (0x18FA6, 0x18EDA), // First Missile Room
        (0x18FFA, 0x18FEE), // Billy Mays Room
        (0x18D66, 0x18D5A), // Brinstar Reserve Tank Room
        (0x18F3A, 0x18F5E), // Etecoon Energy Tank Room (top left door)
        (0x18F5E, 0x18F3A), // Etecoon Supers Room
        (0x18E02, 0x18E62), // Big Pink (top door to Pink Brinstar Power Bomb Room)
        (0x18FCA, 0x18FBE), // Hopper Energy Tank Room
        (0x19132, 0x19126), // Spazer Room
        (0x19162, 0x1914A), // Warehouse Energy Tank Room
        (0x19252, 0x191DA), // Varia Suit Room
        (0x18ADE, 0x18A36), // The Moat (left door)
        (0x18C9A, 0x18C82), // The Final Missile
        (0x18BE6, 0x18C3A), // Terminator Room (left door)
        (0x18B0E, 0x18952), // Gauntlet Energy Tank Room (right door)
        (0x1A924, 0x1A978), // Space Jump Room
        (0x19A62, 0x198B2), // Ridley Tank Room
        (0x199D2, 0x19A9E), // Lower Norfair Escape Power Bomb Room (left door)
        (0x199DE, 0x199C6), // Lower Norfair Escape Power Bomb Room (top door)
        (0x19876, 0x1983A), // Golden Torizo's Room (left door)
        (0x19A86, 0x19882), // Screw Attack Room (left door)
        (0x1941A, 0x192D6), // Hi Jump Energy Tank Room (right door)
        (0x193F6, 0x19426), // Hi Jump Boots Room
        (0x1929A, 0x19732), // Cathedral (right door)
        (0x1953A, 0x19552), // Green Bubbles Missile Room
        (0x195B2, 0x195BE), // Speed Booster Hall
        (0x195BE, 0x195B2), // Speed Booster Room
        (0x1962A, 0x1961E), // Wave Beam Room
        (0x1935A, 0x1937E), // Ice Beam Room
        (0x1938A, 0x19336), // Crumble Shaft (top right door)
        (0x19402, 0x192E2), // Crocomire Escape (left door)
        (0x1946E, 0x1943E), // Post Crocomire Power Bomb Room
        (0x19516, 0x194DA), // Grapple Beam Room (bottom right door)
        (0x1A2E8, 0x1A210), // Wrecked Ship West Super Room
        (0x1A300, 0x18A06), // Gravity Suit Room (left door)
        (0x1A30C, 0x1A1A4), // Gravity Suit Room (right door)
    ]
    .into_iter()
    .map(|(x, y)| (Some(x), Some(y)))
    .collect();

    // Avoid placing an ammo door on a tile with an objective "X", as it looks bad.
    for i in difficulty.objectives.iter() {
        use Objective::*;
        match i {
            SporeSpawn => {
                non_randomizable_doors.insert((Some(0x18E4A), Some(0x18D2A)));
            }
            Crocomire => {
                non_randomizable_doors.insert((Some(0x193DE), Some(0x19432)));
            }
            Botwoon => {
                non_randomizable_doors.insert((Some(0x1A918), Some(0x1A84C)));
            }
            GoldenTorizo => {
                non_randomizable_doors.insert((Some(0x19876), Some(0x1983A)));
            }
            MetroidRoom1 => {
                non_randomizable_doors.insert((Some(0x1A9B4), Some(0x1A9C0))); // left
                non_randomizable_doors.insert((Some(0x1A9A8), Some(0x1A984))); // right
            }
            MetroidRoom2 => {
                non_randomizable_doors.insert((Some(0x1A9C0), Some(0x1A9B4))); // top right
                non_randomizable_doors.insert((Some(0x1A9CC), Some(0x1A9D8))); // bottom right
            }
            MetroidRoom3 => {
                non_randomizable_doors.insert((Some(0x1A9D8), Some(0x1A9CC))); // left
                non_randomizable_doors.insert((Some(0x1A9E4), Some(0x1A9F0))); // right
            }
            MetroidRoom4 => {
                non_randomizable_doors.insert((Some(0x1A9F0), Some(0x1A9E4))); // left
                non_randomizable_doors.insert((Some(0x1A9FC), Some(0x1AA08))); // bottom
            }
            _ => {} // All other tiles have gray doors and are excluded above.
        }
    }

    let mut out: Vec<DoorPtrPair> = vec![];
    for room in &game_data.room_geometry {
        for door in &room.doors {
            let pair = (door.exit_ptr, door.entrance_ptr);
            let has_door_cap = door.offset.is_some();
            if has_door_cap && !non_randomizable_doors.contains(&pair) {
                out.push(pair);
            }
        }
    }
    out.into_iter().collect()
}

fn get_randomizable_door_connections(
    game_data: &GameData,
    map: &Map,
    difficulty: &DifficultyConfig,
) -> Vec<(DoorPtrPair, DoorPtrPair)> {
    let doors = get_randomizable_doors(game_data, difficulty);
    let mut out: Vec<(DoorPtrPair, DoorPtrPair)> = vec![];
    for (src_door_ptr_pair, dst_door_ptr_pair, _bidirectional) in &map.doors {
        if doors.contains(src_door_ptr_pair) && doors.contains(dst_door_ptr_pair) {
            out.push((*src_door_ptr_pair, *dst_door_ptr_pair));
        }
    }
    out
}

pub fn randomize_doors(
    game_data: &GameData,
    map: &Map,
    difficulty: &DifficultyConfig,
    seed: usize,
) -> LockedDoorData {
    let mut rng_seed = [0u8; 32];
    rng_seed[..8].copy_from_slice(&seed.to_le_bytes());
    let mut rng = rand::rngs::StdRng::from_seed(rng_seed);

    let get_loc = |ptr_pair: DoorPtrPair| -> (RoomGeometryRoomIdx, usize, usize) {
        let (room_idx, door_idx) = game_data.room_and_door_idxs_by_door_ptr_pair[&ptr_pair];
        let room = &game_data.room_geometry[room_idx];
        let door = &room.doors[door_idx];
        (room_idx, door.x, door.y)
    };
    let mut used_locs: HashSet<(RoomGeometryRoomIdx, usize, usize)> = HashSet::new();
    let mut used_beam_rooms: HashSet<RoomGeometryRoomIdx> = HashSet::new();
    let mut door_types = vec![];

    match difficulty.doors_mode {
        DoorsMode::Blue => {}
        DoorsMode::Ammo => {
            let red_doors_cnt = 30;
            let green_doors_cnt = 15;
            let yellow_doors_cnt = 10;
            door_types.extend(vec![DoorType::Red; red_doors_cnt]);
            door_types.extend(vec![DoorType::Green; green_doors_cnt]);
            door_types.extend(vec![DoorType::Yellow; yellow_doors_cnt]);
        }
        DoorsMode::Beam => {
            let red_doors_cnt = 18;
            let green_doors_cnt = 10;
            let yellow_doors_cnt = 7;
            let beam_door_each_cnt = 4;
            door_types.extend(vec![DoorType::Red; red_doors_cnt]);
            door_types.extend(vec![DoorType::Green; green_doors_cnt]);
            door_types.extend(vec![DoorType::Yellow; yellow_doors_cnt]);
            door_types.extend(vec![DoorType::Beam(BeamType::Charge); beam_door_each_cnt]);
            door_types.extend(vec![DoorType::Beam(BeamType::Ice); beam_door_each_cnt]);
            door_types.extend(vec![DoorType::Beam(BeamType::Wave); beam_door_each_cnt]);
            door_types.extend(vec![DoorType::Beam(BeamType::Spazer); beam_door_each_cnt]);
            door_types.extend(vec![DoorType::Beam(BeamType::Plasma); beam_door_each_cnt]);
        }
    };
    let door_conns = get_randomizable_door_connections(game_data, map, difficulty);
    let mut locked_doors: Vec<LockedDoor> = vec![];
    let total_cnt = door_types.len();
    let idxs = rand::seq::index::sample(&mut rng, door_conns.len(), total_cnt);
    for (i, idx) in idxs.into_iter().enumerate() {
        let conn = &door_conns[idx];
        let door = LockedDoor {
            src_ptr_pair: conn.0,
            dst_ptr_pair: conn.1,
            door_type: door_types[i],
            bidirectional: true,
        };

        // Make sure we don't put two ammo doors in the same tile (since that would interfere
        // with the mechanism for making the doors disappear from the map).
        let src_loc = get_loc(door.src_ptr_pair);
        let dst_loc = get_loc(door.dst_ptr_pair);
        if used_locs.contains(&src_loc) || used_locs.contains(&dst_loc) {
            continue;
        }
        if let DoorType::Beam(_) = door_types[i] {
            let src_room_idx = src_loc.0;
            let dst_room_idx = dst_loc.0;
            if used_beam_rooms.contains(&src_room_idx) || used_beam_rooms.contains(&dst_room_idx) {
                continue;
            }
            used_beam_rooms.insert(src_room_idx);
            used_beam_rooms.insert(dst_room_idx);
        }
        used_locs.insert(src_loc);
        used_locs.insert(dst_loc);
        locked_doors.push(door);
    }

    let mut locked_door_node_map: HashMap<(RoomId, NodeId), usize> = HashMap::new();
    for (i, door) in locked_doors.iter().enumerate() {
        let (src_room_id, src_node_id) = game_data.door_ptr_pair_map[&door.src_ptr_pair];
        locked_door_node_map.insert((src_room_id, src_node_id), i);
        if door.bidirectional {
            let (dst_room_id, dst_node_id) = game_data.door_ptr_pair_map[&door.dst_ptr_pair];
            locked_door_node_map.insert((dst_room_id, dst_node_id), i);
        }
    }

    // Homing Geemer Room left door -> West Ocean Bridge left door
    if let Some(&idx) = locked_door_node_map.get(&(313, 1)) {
        locked_door_node_map.insert((32, 7), idx);
    }

    // Homing Geemer Room right door -> West Ocean Bridge right door
    if let Some(&idx) = locked_door_node_map.get(&(313, 2)) {
        locked_door_node_map.insert((32, 8), idx);
    }

    // Pants Room right door -> East Pants Room right door
    if let Some(&idx) = locked_door_node_map.get(&(322, 2)) {
        locked_door_node_map.insert((220, 2), idx);
    }

    let mut locked_door_vertex_ids = vec![vec![]; locked_doors.len()];
    for (&(room_id, node_id), vertex_ids) in &game_data.node_door_unlock {
        if let Some(&locked_door_idx) = locked_door_node_map.get(&(room_id, node_id)) {
            locked_door_vertex_ids[locked_door_idx].extend(vertex_ids);
        }
    }

    LockedDoorData {
        locked_doors,
        locked_door_node_map,
        locked_door_vertex_ids,
    }
}

fn is_req_possible(req: &Requirement, tech_active: &[bool], strats_active: &[bool]) -> bool {
    match req {
        Requirement::Tech(tech_id) => tech_active[*tech_id],
        Requirement::Strat(strat_id) => strats_active[*strat_id],
        Requirement::And(reqs) => reqs
            .iter()
            .all(|x| is_req_possible(x, tech_active, strats_active)),
        Requirement::Or(reqs) => reqs
            .iter()
            .any(|x| is_req_possible(x, tech_active, strats_active)),
        _ => true,
    }
}

pub fn filter_links(
    links: &[Link],
    game_data: &GameData,
    difficulty: &DifficultyConfig,
) -> Vec<Link> {
    let mut out = vec![];
    let tech_vec = get_tech_vec(game_data, difficulty);
    let strat_vec = get_strat_vec(game_data, difficulty);
    for link in links {
        if is_req_possible(&link.requirement, &tech_vec, &strat_vec) {
            out.push(link.clone())
        }
    }
    out
}

fn get_tech_vec(game_data: &GameData, difficulty: &DifficultyConfig) -> Vec<bool> {
    let tech_set: HashSet<String> = difficulty.tech.iter().map(|x| x.clone()).collect();
    game_data
        .tech_isv
        .keys
        .iter()
        .map(|x| tech_set.contains(x))
        .collect()
}

fn get_strat_vec(game_data: &GameData, difficulty: &DifficultyConfig) -> Vec<bool> {
    let strat_set: HashSet<String> = difficulty
        .notable_strats
        .iter()
        .map(|x| x.clone())
        .collect();
    game_data
        .notable_strat_isv
        .keys
        .iter()
        .map(|x| strat_set.contains(x))
        .collect()
}

fn ensure_enough_tanks(initial_items_remaining: &mut [usize], difficulty: &DifficultyConfig) {
    // Give an extra tank to two, compared to what may be needed for Ridley, for lenience:
    if difficulty.ridley_proficiency < 0.3 {
        while initial_items_remaining[Item::ETank as usize]
            + initial_items_remaining[Item::ReserveTank as usize]
            < 11
        {
            initial_items_remaining[Item::ETank as usize] += 1;
        }
    } else if difficulty.ridley_proficiency < 0.8 {
        while initial_items_remaining[Item::ETank as usize]
            + initial_items_remaining[Item::ReserveTank as usize]
            < 9
        {
            initial_items_remaining[Item::ETank as usize] += 1;
        }
    } else if difficulty.ridley_proficiency < 0.9 {
        while initial_items_remaining[Item::ETank as usize]
            + initial_items_remaining[Item::ReserveTank as usize]
            < 7
        {
            initial_items_remaining[Item::ETank as usize] += 1;
        }
    } else {
        // Give enough tanks for Mother Brain:
        while initial_items_remaining[Item::ETank as usize]
            + initial_items_remaining[Item::ReserveTank as usize]
            < 3
        {
            initial_items_remaining[Item::ETank as usize] += 1;
        }
    }
}

pub fn strip_name(s: &str) -> String {
    let mut out = String::new();
    for word in s.split_inclusive(|x: char| !x.is_ascii_alphabetic()) {
        let capitalized_word = word[0..1].to_ascii_uppercase() + &word[1..];
        let stripped_word: String = capitalized_word
            .chars()
            .filter(|x| x.is_ascii_alphanumeric())
            .collect();
        out += &stripped_word;
    }
    out
}

impl<'r> Randomizer<'r> {
    pub fn new(
        map: &'r Map,
        locked_door_data: &'r LockedDoorData,
        difficulty_tiers: &'r [DifficultyConfig],
        game_data: &'r GameData,
        base_links_data: &'r LinksDataGroup,
    ) -> Randomizer<'r> {
        let preprocessor = Preprocessor::new(game_data, map, &difficulty_tiers[0]);
        let preprocessed_seed_links: Vec<Link> = preprocessor.get_all_door_links();
        info!(
            "{} base links, {} door links",
            base_links_data.links.len(),
            preprocessed_seed_links.len()
        );

        let mut initial_items_remaining: Vec<usize> = vec![1; game_data.item_isv.keys.len()];
        initial_items_remaining[Item::Nothing as usize] = 0;
        initial_items_remaining[Item::WallJump as usize] =
            if difficulty_tiers[0].wall_jump == WallJump::Collectible {
                1
            } else {
                0
            };
        initial_items_remaining[Item::Super as usize] = 10;
        initial_items_remaining[Item::PowerBomb as usize] = 10;
        initial_items_remaining[Item::ETank as usize] = 14;
        initial_items_remaining[Item::ReserveTank as usize] = 4;
        initial_items_remaining[Item::Missile as usize] =
            game_data.item_locations.len() - initial_items_remaining.iter().sum::<usize>();

        for &(item, cnt) in &difficulty_tiers[0].item_pool {
            initial_items_remaining[item as usize] = cnt;
        }

        ensure_enough_tanks(&mut initial_items_remaining, &difficulty_tiers[0]);

        if initial_items_remaining.iter().sum::<usize>() > game_data.item_locations.len() {
            initial_items_remaining[Item::Missile as usize] -=
                initial_items_remaining.iter().sum::<usize>() - game_data.item_locations.len();
        }

        for &(item, cnt) in &difficulty_tiers[0].starting_items {
            initial_items_remaining[item as usize] -=
                usize::min(cnt, initial_items_remaining[item as usize]);
        }

        assert!(initial_items_remaining.iter().sum::<usize>() <= game_data.item_locations.len());
        initial_items_remaining[Item::Nothing as usize] =
            game_data.item_locations.len() - initial_items_remaining.iter().sum::<usize>();

        let toilet_intersections = Self::get_toilet_intersections(map, game_data);

        Randomizer {
            map,
            toilet_intersections,
            locked_door_data,
            initial_items_remaining,
            game_data,
            base_links_data,
            seed_links_data: LinksDataGroup::new(
                preprocessed_seed_links,
                game_data.vertex_isv.keys.len(),
                base_links_data.links.len(),
            ),
            difficulty_tiers,
        }
    }

    pub fn get_toilet_intersections(map: &Map, game_data: &GameData) -> Vec<RoomGeometryRoomIdx> {
        let mut out = vec![];
        let toilet_pos = map.rooms[game_data.toilet_room_idx];
        for room_idx in 0..map.rooms.len() {
            let room_map = &game_data.room_geometry[room_idx].map;
            let room_pos = map.rooms[room_idx];
            let room_height = room_map.len() as isize;
            let room_width = room_map[0].len() as isize;
            let rel_pos_x = (toilet_pos.0 as isize) - (room_pos.0 as isize);
            let rel_pos_y = (toilet_pos.1 as isize) - (room_pos.1 as isize);

            if rel_pos_x >= 0 && rel_pos_x < room_width {
                for y in 2..8 {
                    let y1 = rel_pos_y + y;
                    if y1 >= 0 && y1 < room_height && room_map[y1 as usize][rel_pos_x as usize] == 1
                    {
                        out.push(room_idx);
                        break;
                    }
                }
            }
        }
        out
    }

    pub fn get_link(&self, idx: usize) -> &Link {
        let base_links_len = self.base_links_data.links.len();
        if idx < base_links_len {
            &self.base_links_data.links[idx]
        } else {
            &self.seed_links_data.links[idx - base_links_len]
        }
    }

    fn get_initial_flag_vec(&self) -> Vec<bool> {
        let mut flag_vec = vec![false; self.game_data.flag_isv.keys.len()];
        let tourian_open_idx = self.game_data.flag_isv.index_by_key["f_TourianOpen"];
        flag_vec[tourian_open_idx] = true;
        if self.difficulty_tiers[0].all_items_spawn {
            let all_items_spawn_idx = self.game_data.flag_isv.index_by_key["f_AllItemsSpawn"];
            flag_vec[all_items_spawn_idx] = true;
        }
        if self.difficulty_tiers[0].acid_chozo {
            let acid_chozo_without_space_jump_idx =
                self.game_data.flag_isv.index_by_key["f_AcidChozoWithoutSpaceJump"];
            flag_vec[acid_chozo_without_space_jump_idx] = true;
        }
        flag_vec
    }

    fn update_reachability(&self, state: &mut RandomizationState) {
        let num_vertices = self.game_data.vertex_isv.keys.len();
        let start_vertex_id = self.game_data.vertex_isv.index_by_key[&VertexKey {
            room_id: state.hub_location.room_id,
            node_id: state.hub_location.node_id,
            obstacle_mask: 0,
            actions: vec![],
        }];
        let mut forward = traverse(
            &self.base_links_data,
            &self.seed_links_data,
            None,
            &state.global_state,
            LocalState::new(),
            num_vertices,
            start_vertex_id,
            false,
            &self.difficulty_tiers[0],
            self.game_data,
            &self.locked_door_data,
        );
        let mut reverse = traverse(
            &self.base_links_data,
            &self.seed_links_data,
            None,
            &state.global_state,
            LocalState::new(),
            num_vertices,
            start_vertex_id,
            true,
            &self.difficulty_tiers[0],
            self.game_data,
            &self.locked_door_data,
        );
        for (i, vertex_ids) in self.game_data.item_vertex_ids.iter().enumerate() {
            // Clear out any previous bireachable markers (because in rare cases a previously bireachable
            // vertex can become no longer "bireachable" due to the imperfect cost heuristic used for
            // resource management.)
            state.item_location_state[i].bireachable = false;
            state.item_location_state[i].bireachable_vertex_id = None;

            for &v in vertex_ids {
                if forward.cost[v].iter().any(|&x| f32::is_finite(x)) {
                    state.item_location_state[i].reachable = true;
                    if !state.item_location_state[i].bireachable
                        && get_bireachable_idxs(&state.global_state, v, &mut forward, &mut reverse)
                            .is_some()
                    {
                        state.item_location_state[i].bireachable = true;
                        state.item_location_state[i].bireachable_vertex_id = Some(v);
                    }
                }
            }
        }
        for (i, vertex_ids) in self.game_data.flag_vertex_ids.iter().enumerate() {
            // Clear out any previous bireachable markers (because in rare cases a previously bireachable
            // vertex can become no longer "bireachable" due to the imperfect cost heuristic used for
            // resource management.)
            state.flag_location_state[i].reachable = false;
            state.flag_location_state[i].reachable_vertex_id = None;
            state.flag_location_state[i].bireachable = false;
            state.flag_location_state[i].bireachable_vertex_id = None;

            for &v in vertex_ids {
                if forward.cost[v].iter().any(|&x| f32::is_finite(x)) {
                    if !state.flag_location_state[i].reachable {
                        state.flag_location_state[i].reachable = true;
                        state.flag_location_state[i].reachable_vertex_id = Some(v);
                    }
                    if !state.flag_location_state[i].bireachable
                        && get_bireachable_idxs(&state.global_state, v, &mut forward, &mut reverse)
                            .is_some()
                    {
                        state.flag_location_state[i].bireachable = true;
                        state.flag_location_state[i].bireachable_vertex_id = Some(v);
                    }
                }
            }
        }
        for (i, vertex_ids) in self
            .locked_door_data
            .locked_door_vertex_ids
            .iter()
            .enumerate()
        {
            // Clear out any previous bireachable markers (because in rare cases a previously bireachable
            // vertex can become no longer "bireachable" due to the imperfect cost heuristic used for
            // resource management.)
            state.door_state[i].bireachable = false;
            state.door_state[i].bireachable_vertex_id = None;

            for &v in vertex_ids {
                if forward.cost[v].iter().any(|&x| f32::is_finite(x)) {
                    if !state.door_state[i].bireachable
                        && get_bireachable_idxs(&state.global_state, v, &mut forward, &mut reverse)
                            .is_some()
                    {
                        state.door_state[i].bireachable = true;
                        state.door_state[i].bireachable_vertex_id = Some(v);
                    }
                }
            }
        }
        for (i, (room_id, node_id)) in self.game_data.save_locations.iter().enumerate() {
            state.save_location_state[i].bireachable = false;
            let vertex_id = self.game_data.vertex_isv.index_by_key[&VertexKey {
                room_id: *room_id,
                node_id: *node_id,
                obstacle_mask: 0,
                actions: vec![],
            }];
            if get_bireachable_idxs(&state.global_state, vertex_id, &mut forward, &mut reverse)
                .is_some()
            {
                state.save_location_state[i].bireachable = true;
            }
        }

        // Store TraverseResults to use for constructing spoiler log
        state.debug_data = Some(DebugData {
            global_state: state.global_state.clone(),
            forward,
            reverse,
        });
    }

    // Determine how many key items vs. filler items to place on this step.
    fn determine_item_split(
        &self,
        state: &RandomizationState,
        num_bireachable: usize,
        num_oneway_reachable: usize,
    ) -> (usize, usize) {
        let num_items_to_place = num_bireachable + num_oneway_reachable;
        let filtered_item_precedence: Vec<Item> = state
            .item_precedence
            .iter()
            .copied()
            .filter(|&item| {
                state.items_remaining[item as usize] == self.initial_items_remaining[item as usize]
            })
            .collect();
        let num_key_items_remaining = filtered_item_precedence.len();
        let num_items_remaining: usize = state.items_remaining.iter().sum();
        let mut num_key_items_to_place = match self.difficulty_tiers[0].progression_rate {
            ProgressionRate::Slow => 1,
            ProgressionRate::Uniform => usize::max(
                1,
                f32::round(
                    (num_key_items_remaining as f32) / (num_items_remaining as f32)
                        * (num_items_to_place as f32),
                ) as usize,
            ),
            ProgressionRate::Fast => usize::max(
                1,
                f32::round(
                    2.0 * (num_key_items_remaining as f32) / (num_items_remaining as f32)
                        * (num_items_to_place as f32),
                ) as usize,
            ),
        };

        // If we're at the end, dump as many key items as possible:
        if !self.difficulty_tiers[0].stop_item_placement_early
            && num_items_remaining < num_items_to_place + KEY_ITEM_FINISH_THRESHOLD
        {
            num_key_items_to_place = num_key_items_remaining;
        }

        // But we can't place more key items than we have unfilled bireachable item locations:
        num_key_items_to_place = min(
            num_key_items_to_place,
            min(num_bireachable, num_key_items_remaining),
        );

        let num_filler_items_to_place = num_items_to_place - num_key_items_to_place;

        (num_key_items_to_place, num_filler_items_to_place)
    }

    fn select_filler_items<R: Rng>(
        &self,
        state: &RandomizationState,
        num_bireachable_filler_items_to_select: usize,
        num_one_way_reachable_filler_items_to_select: usize,
        rng: &mut R,
    ) -> Vec<Item> {
        // In the future we might do something different with how bireachable locations are filled vs. one-way,
        // but for now they are just lumped together:
        let num_filler_items_to_select =
            num_bireachable_filler_items_to_select + num_one_way_reachable_filler_items_to_select;
        let expansion_item_set: HashSet<Item> =
            [Item::ETank, Item::ReserveTank, Item::Super, Item::PowerBomb]
                .into_iter()
                .collect();
        let mut item_types_to_prioritize: Vec<Item> = vec![];
        let mut item_types_to_mix: Vec<Item> = vec![Item::Missile, Item::Nothing];
        let mut item_types_to_delay: Vec<Item> = vec![];
        let mut item_types_to_extra_delay: Vec<Item> = vec![];

        for &item in &state.item_precedence {
            if item == Item::Missile
                || item == Item::Nothing
                || state.items_remaining[item as usize] == 0
            {
                continue;
            }
            if self.difficulty_tiers[0].early_filler_items.contains(&item)
                && state.items_remaining[item as usize]
                    == self.initial_items_remaining[item as usize]
            {
                item_types_to_prioritize.push(item);
                item_types_to_mix.push(item);
            } else if self.difficulty_tiers[0].filler_items.contains(&item)
                || (self.difficulty_tiers[0].semi_filler_items.contains(&item)
                    && state.items_remaining[item as usize]
                        < self.initial_items_remaining[item as usize])
            {
                item_types_to_mix.push(item);
            } else if expansion_item_set.contains(&item) {
                item_types_to_delay.push(item);
            } else {
                item_types_to_extra_delay.push(item);
            }
        }
        let mut items_to_mix: Vec<Item> = Vec::new();
        for &item in &item_types_to_mix {
            let mut cnt = state.items_remaining[item as usize];
            if item_types_to_prioritize.contains(&item) {
                cnt -= 1;
            }
            for _ in 0..cnt {
                items_to_mix.push(item);
            }
        }
        let mut items_to_delay: Vec<Item> = Vec::new();
        for &item in &item_types_to_delay {
            for _ in 0..state.items_remaining[item as usize] {
                items_to_delay.push(item);
            }
        }
        let mut items_to_extra_delay: Vec<Item> = Vec::new();
        for &item in &item_types_to_extra_delay {
            for _ in 0..state.items_remaining[item as usize] {
                if self.difficulty_tiers[0].stop_item_placement_early {
                    // When using "Stop item placement early", place extra Nothing items rather than dumping key items.
                    // It could sometimes result in failure due to not leaving enough places to put needed key items,
                    // but this is an acceptable risk and shouldn't happen too often.
                    items_to_extra_delay.push(Item::Nothing);
                } else {
                    items_to_extra_delay.push(item);
                }
            }
        }
        items_to_mix.shuffle(rng);
        let mut items_to_place: Vec<Item> = item_types_to_prioritize;
        items_to_place.extend(items_to_mix);
        items_to_place.extend(items_to_delay);
        items_to_place.extend(items_to_extra_delay);
        if self.difficulty_tiers[0].spazer_before_plasma {
            self.apply_spazer_plasma_priority(&mut items_to_place);
        }
        items_to_place = items_to_place[0..num_filler_items_to_select].to_vec();
        items_to_place
    }

    fn select_key_items(
        &self,
        state: &RandomizationState,
        num_key_items_to_select: usize,
        attempt_num: usize,
    ) -> Option<Vec<Item>> {
        if num_key_items_to_select >= 1 {
            let mut unplaced_items: Vec<Item> = vec![];
            let mut placed_items: Vec<Item> = vec![];
            let mut additional_items: Vec<Item> = vec![];

            for &item in &state.item_precedence {
                if state.items_remaining[item as usize] > 0
                    || (self.difficulty_tiers[0].stop_item_placement_early && item == Item::Nothing)
                {
                    if self.difficulty_tiers[0].progression_rate == ProgressionRate::Slow {
                        // With Slow progression, items that have been placed before (e.g. an ETank) are treated like any other
                        // item, still keeping their same position in the key item priority
                        unplaced_items.push(item);
                    } else {
                        // With Uniform and Fast progression, items that have been placed before get put in last priority:
                        if state.items_remaining[item as usize]
                            == self.initial_items_remaining[item as usize]
                        {
                            unplaced_items.push(item);
                        } else {
                            placed_items.push(item);
                        }
                    }

                    if state.items_remaining[item as usize] >= 2 {
                        let cnt = state.items_remaining[item as usize] - 1;
                        for _ in 0..cnt {
                            additional_items.push(item);
                        }
                    }
                }
            }

            let cnt_different_items_remaining = unplaced_items.len() + placed_items.len();
            let mut remaining_items: Vec<Item> = vec![];
            remaining_items.extend(unplaced_items);
            remaining_items.extend(placed_items);
            remaining_items.extend(additional_items);

            if attempt_num > 0
                && num_key_items_to_select - 1 + attempt_num >= cnt_different_items_remaining
            {
                return None;
            }

            // If we will be placing `k` key items, we let the first `k - 1` items to place remain fixed based on the
            // item precedence order, while we vary the last key item across attempts (to try to find some choice that
            // will expand the set of bireachable item locations).
            let mut key_items_to_place: Vec<Item> = vec![];
            key_items_to_place.extend(remaining_items[0..(num_key_items_to_select - 1)].iter());
            key_items_to_place.push(remaining_items[num_key_items_to_select - 1 + attempt_num]);
            assert!(key_items_to_place.len() == num_key_items_to_select);
            return Some(key_items_to_place);
        } else {
            if attempt_num > 0 {
                return None;
            } else {
                return Some(vec![]);
            }
        }
    }

    fn get_init_traverse(
        &self,
        state: &RandomizationState,
        init_traverse: Option<&TraverseResult>,
    ) -> Option<TraverseResult> {
        if let Some(init) = init_traverse {
            let mut out = init.clone();
            for v in 0..init.local_states.len() {
                if !state.key_visited_vertices.contains(&v) {
                    out.local_states[v] = [IMPOSSIBLE_LOCAL_STATE; NUM_COST_METRICS];
                    out.cost[v] = [f32::INFINITY; NUM_COST_METRICS];
                    out.start_trail_ids[v] = [-1; NUM_COST_METRICS];
                }
            }
            Some(out)
        } else {
            None
        }
    }

    fn find_hard_location(
        &self,
        state: &RandomizationState,
        bireachable_locations: &[ItemLocationId],
        init_traverse: Option<&TraverseResult>,
    ) -> (usize, usize) {
        // For forced mode, we prioritize placing a key item at a location that is inaccessible at
        // lower difficulty tiers. This function returns an index into `bireachable_locations`, identifying
        // a location with the hardest possible difficulty to reach.
        let num_vertices = self.game_data.vertex_isv.keys.len();
        let start_vertex_id = self.game_data.vertex_isv.index_by_key[&VertexKey {
            room_id: state.hub_location.room_id,
            node_id: state.hub_location.node_id,
            obstacle_mask: 0,
            actions: vec![],
        }];

        for tier in 1..self.difficulty_tiers.len() {
            let difficulty = &self.difficulty_tiers[tier];
            let mut tmp_global = state.global_state.clone();
            tmp_global.tech = get_tech_vec(&self.game_data, difficulty);
            tmp_global.notable_strats = get_strat_vec(&self.game_data, difficulty);

            let traverse_result = traverse(
                &self.base_links_data,
                &self.seed_links_data,
                self.get_init_traverse(state, init_traverse),
                &tmp_global,
                LocalState::new(),
                num_vertices,
                start_vertex_id,
                false,
                difficulty,
                self.game_data,
                self.locked_door_data,
            );

            for (i, &item_location_id) in bireachable_locations.iter().enumerate() {
                let mut is_reachable = false;
                for &v in &self.game_data.item_vertex_ids[item_location_id] {
                    if traverse_result.cost[v].iter().any(|&x| f32::is_finite(x)) {
                        is_reachable = true;
                    }
                }
                if !is_reachable {
                    return (i, tier - 1);
                }
            }
        }
        return (0, self.difficulty_tiers.len() - 1);
    }

    fn place_items(
        &self,
        attempt_num_rando: usize,
        state: &RandomizationState,
        new_state: &mut RandomizationState,
        bireachable_locations: &[ItemLocationId],
        other_locations: &[ItemLocationId],
        key_items_to_place: &[Item],
        other_items_to_place: &[Item],
    ) {
        info!(
            "[attempt {attempt_num_rando}] Placing {:?}, {:?}",
            key_items_to_place, other_items_to_place
        );

        let num_items_remaining: usize = state.items_remaining.iter().sum();
        let num_items_to_place: usize = key_items_to_place.len() + other_items_to_place.len();
        let skip_hard_placement = !self.difficulty_tiers[0].stop_item_placement_early
            && num_items_remaining < num_items_to_place + KEY_ITEM_FINISH_THRESHOLD;

        let mut new_bireachable_locations: Vec<ItemLocationId> = bireachable_locations.to_vec();
        if self.difficulty_tiers.len() > 1 && !skip_hard_placement {
            let traverse_result = match state.previous_debug_data.as_ref() {
                Some(x) => Some(&x.forward),
                None => None,
            };
            for i in 0..key_items_to_place.len() {
                let (hard_idx, tier) = if key_items_to_place.len() > 1 {
                    // We're placing more than one key item in this step. Obtaining some of them could help make
                    // others easier to obtain. So we use "new_state" to try to find locations that are still hard to
                    // reach even with the new items.
                    self.find_hard_location(
                        new_state,
                        &new_bireachable_locations[i..],
                        traverse_result,
                    )
                } else {
                    // We're only placing one key item in this step. Try to find a location that is hard to reach
                    // without already having the new item.
                    self.find_hard_location(state, &new_bireachable_locations[i..], traverse_result)
                };
                info!(
                    "[attempt {attempt_num_rando}] {:?} in tier {} (of {})",
                    key_items_to_place[i],
                    tier,
                    self.difficulty_tiers.len()
                );

                let hard_loc = new_bireachable_locations[i + hard_idx];
                new_bireachable_locations.swap(i, i + hard_idx);

                // Mark the vertices along the path to the newly chosen hard location. Vertices that are
                // easily accessible from along this path are then discouraged from being chosen later
                // as hard locations (since the point of forced mode is to ensure unique hard strats
                // are required; we don't want it to be the same hard strat over and over again).
                let hard_vertex_id = state.item_location_state[hard_loc]
                    .bireachable_vertex_id
                    .unwrap();
                new_state.item_location_state[hard_loc].difficulty_tier = Some(tier);
                let forward = &state.debug_data.as_ref().unwrap().forward;
                let reverse = &state.debug_data.as_ref().unwrap().reverse;
                let (forward_cost_idx, _) =
                    get_bireachable_idxs(&state.global_state, hard_vertex_id, forward, reverse)
                        .unwrap();
                let route = get_spoiler_route(
                    &state.debug_data.as_ref().unwrap().forward,
                    hard_vertex_id,
                    forward_cost_idx,
                );
                for &link_idx in &route {
                    let vertex_id = self.get_link(link_idx as usize).to_vertex_id;
                    new_state.key_visited_vertices.insert(vertex_id);
                }
            }
        }

        let mut all_locations: Vec<ItemLocationId> = Vec::new();
        all_locations.extend(new_bireachable_locations);
        all_locations.extend(other_locations);
        let mut all_items_to_place: Vec<Item> = Vec::new();
        all_items_to_place.extend(key_items_to_place);
        all_items_to_place.extend(other_items_to_place);
        assert!(all_locations.len() == all_items_to_place.len());
        for (&loc, &item) in iter::zip(&all_locations, &all_items_to_place) {
            new_state.item_location_state[loc].placed_item = Some(item);
        }
    }

    fn finish(&self, attempt_num_rando: usize, state: &mut RandomizationState) {
        let mut remaining_items: Vec<Item> = Vec::new();
        for item_id in 0..self.game_data.item_isv.keys.len() {
            for _ in 0..state.items_remaining[item_id] {
                remaining_items.push(Item::try_from(item_id).unwrap());
            }
        }
        if self.difficulty_tiers[0].stop_item_placement_early {
            info!(
                "[attempt {attempt_num_rando}] Finishing without {:?}",
                remaining_items
            );
            for item_loc_state in &mut state.item_location_state {
                if item_loc_state.placed_item.is_none() || !item_loc_state.bireachable {
                    item_loc_state.placed_item = Some(Item::Nothing);
                }
            }
        } else {
            info!(
                "[attempt {attempt_num_rando}] Finishing with {:?}",
                remaining_items
            );
            let mut idx = 0;
            for item_loc_state in &mut state.item_location_state {
                if item_loc_state.placed_item.is_none() {
                    item_loc_state.placed_item = Some(remaining_items[idx]);
                    idx += 1;
                }
            }
            assert!(idx == remaining_items.len());
        }
    }

    fn provides_progression(
        &self,
        old_state: &RandomizationState,
        new_state: &mut RandomizationState,
        key_items: &[Item],
        filler_items: &[Item],
        placed_uncollected_bireachable_items: &[Item],
        num_unplaced_bireachable: usize,
    ) -> bool {
        // Collect all the items that would be collectible in this scenario:
        // 1) Items that were already placed on an earlier step; this is only applicable to filler items
        // (normally Missiles) on Slow progression, which became one-way reachable on an earlier step but are now
        // bireachable.
        // 2) Key items,
        // 3) Other items
        for &item in placed_uncollected_bireachable_items.iter().chain(
            key_items
                .iter()
                .chain(filler_items.iter())
                .take(num_unplaced_bireachable),
        ) {
            new_state.global_state.collect(item, self.game_data);
        }

        self.update_reachability(new_state);
        let num_bireachable = new_state
            .item_location_state
            .iter()
            .filter(|x| x.bireachable)
            .count();
        let num_reachable = new_state
            .item_location_state
            .iter()
            .filter(|x| x.reachable)
            .count();
        let num_one_way_reachable = num_reachable - num_bireachable;

        // Maximum acceptable number of one-way-reachable items. This is to try to avoid extreme
        // cases where the player would gain access to very large areas that they cannot return from:
        let one_way_reachable_limit = 20;

        // Check if all items are already bireachable. It isn't necessary for correctness to check this case,
        // but it speeds up the last step, where no further progress is possible (meaning there is no point
        // trying a bunch of possible key items to place to try to make more progress.
        let all_items_bireachable = num_bireachable == new_state.item_location_state.len();

        let gives_expansion = if all_items_bireachable {
            true
        } else {
            iter::zip(
                &new_state.item_location_state,
                &old_state.item_location_state,
            )
            .any(|(n, o)| n.bireachable && !o.reachable)
        };

        let is_beatable = self.is_game_beatable(&new_state);

        (num_one_way_reachable < one_way_reachable_limit && gives_expansion) || is_beatable
    }

    fn multi_attempt_select_items<R: Rng + Clone>(
        &self,
        attempt_num_rando: usize,
        state: &RandomizationState,
        placed_uncollected_bireachable_items: &[Item],
        num_unplaced_bireachable: usize,
        num_unplaced_oneway_reachable: usize,
        rng: &mut R,
    ) -> (SelectItemsOutput, RandomizationState) {
        let (num_key_items_to_select, num_filler_items_to_select) = self.determine_item_split(
            state,
            num_unplaced_bireachable,
            num_unplaced_oneway_reachable,
        );
        let num_bireachable_filler_items_to_select =
            num_unplaced_bireachable - num_key_items_to_select;
        let num_one_way_reachable_filler_items_to_select =
            num_filler_items_to_select - num_bireachable_filler_items_to_select;
        let selected_filler_items = self.select_filler_items(
            state,
            num_bireachable_filler_items_to_select,
            num_one_way_reachable_filler_items_to_select,
            rng,
        );

        let mut new_state_filler: RandomizationState = RandomizationState {
            step_num: state.step_num,
            start_location: state.start_location.clone(),
            hub_location: state.hub_location.clone(),
            item_precedence: state.item_precedence.clone(),
            item_location_state: state.item_location_state.clone(),
            flag_location_state: state.flag_location_state.clone(),
            save_location_state: state.save_location_state.clone(),
            door_state: state.door_state.clone(),
            items_remaining: state.items_remaining.clone(),
            global_state: state.global_state.clone(),
            debug_data: None,
            previous_debug_data: None,
            key_visited_vertices: HashSet::new(),
        };
        for &item in &selected_filler_items {
            // We check if items_remaining is positive, only because with "Stop item placement early" there
            // could be extra (unplanned) Nothing items placed.
            if new_state_filler.items_remaining[item as usize] > 0 {
                new_state_filler.items_remaining[item as usize] -= 1;
            }
        }

        let mut attempt_num = 0;
        let mut selected_key_items = self
            .select_key_items(&new_state_filler, num_key_items_to_select, attempt_num)
            .unwrap();

        loop {
            let mut new_state: RandomizationState = new_state_filler.clone();
            for &item in &selected_key_items {
                if new_state.items_remaining[item as usize] > 0 {
                    new_state.items_remaining[item as usize] -= 1;
                }
            }

            if self.provides_progression(
                &state,
                &mut new_state,
                &selected_key_items,
                &selected_filler_items,
                &placed_uncollected_bireachable_items,
                num_unplaced_bireachable,
            ) {
                let selection = SelectItemsOutput {
                    key_items: selected_key_items,
                    other_items: selected_filler_items,
                };
                return (selection, new_state);
            }

            if let Some(new_selected_key_items) =
                self.select_key_items(&new_state_filler, num_key_items_to_select, attempt_num)
            {
                selected_key_items = new_selected_key_items;
            } else {
                info!("[attempt {attempt_num_rando}] Exhausted key item placement attempts");
                if self.difficulty_tiers[0].stop_item_placement_early {
                    for x in &mut selected_key_items {
                        *x = Item::Nothing;
                    }
                    new_state = new_state_filler;
                    for &item in &selected_key_items {
                        if new_state.items_remaining[item as usize] > 0 {
                            new_state.items_remaining[item as usize] -= 1;
                        }
                    }
                    let _ = self.provides_progression(
                        &state,
                        &mut new_state,
                        &selected_key_items,
                        &selected_filler_items,
                        &placed_uncollected_bireachable_items,
                        num_unplaced_bireachable,
                    );
                }
                let selection = SelectItemsOutput {
                    key_items: selected_key_items,
                    other_items: selected_filler_items,
                };
                return (selection, new_state);
            }
            attempt_num += 1;
        }
    }

    fn step<R: Rng + Clone>(
        &self,
        attempt_num_rando: usize,
        state: &mut RandomizationState,
        rng: &mut R,
    ) -> (SpoilerSummary, SpoilerDetails, bool) {
        let orig_global_state = state.global_state.clone();
        let mut spoiler_flag_summaries: Vec<SpoilerFlagSummary> = Vec::new();
        let mut spoiler_flag_details: Vec<SpoilerFlagDetails> = Vec::new();
        let mut spoiler_door_summaries: Vec<SpoilerDoorSummary> = Vec::new();
        let mut spoiler_door_details: Vec<SpoilerDoorDetails> = Vec::new();
        loop {
            let mut any_update = false;
            for (i, &flag_id) in self.game_data.flag_ids.iter().enumerate() {
                if state.global_state.flags[flag_id] {
                    continue;
                }
                if state.flag_location_state[i].reachable
                    && flag_id == self.game_data.mother_brain_defeated_flag_id
                {
                    // f_DefeatedMotherBrain flag is special in that we only require one-way reachability for it:
                    any_update = true;
                    let flag_vertex_id = state.flag_location_state[i].reachable_vertex_id.unwrap();
                    spoiler_flag_summaries.push(self.get_spoiler_flag_summary(
                        &state,
                        flag_vertex_id,
                        flag_id,
                    ));
                    spoiler_flag_details.push(self.get_spoiler_flag_details_one_way(
                        &state,
                        flag_vertex_id,
                        flag_id,
                    ));
                    state.global_state.flags[flag_id] = true;
                } else if state.flag_location_state[i].bireachable {
                    any_update = true;
                    let flag_vertex_id =
                        state.flag_location_state[i].bireachable_vertex_id.unwrap();
                    spoiler_flag_summaries.push(self.get_spoiler_flag_summary(
                        &state,
                        flag_vertex_id,
                        flag_id,
                    ));
                    spoiler_flag_details.push(self.get_spoiler_flag_details(
                        &state,
                        flag_vertex_id,
                        flag_id,
                    ));
                    state.global_state.flags[flag_id] = true;
                }
            }
            for i in 0..self.locked_door_data.locked_doors.len() {
                if state.global_state.doors_unlocked[i] {
                    continue;
                }
                if state.door_state[i].bireachable {
                    any_update = true;
                    let door_vertex_id = state.door_state[i].bireachable_vertex_id.unwrap();
                    spoiler_door_summaries.push(self.get_spoiler_door_summary(door_vertex_id, i));
                    spoiler_door_details.push(self.get_spoiler_door_details(
                        &state,
                        door_vertex_id,
                        i,
                    ));
                    state.global_state.doors_unlocked[i] = true;
                }
            }
            if any_update {
                self.update_reachability(state);
            } else {
                break;
            }
        }

        if self.difficulty_tiers[0].stop_item_placement_early && self.is_game_beatable(state) {
            info!("Stopping early");
            self.update_reachability(state);
            let spoiler_summary = self.get_spoiler_summary(
                &orig_global_state,
                state,
                &state,
                spoiler_flag_summaries,
                spoiler_door_summaries,
            );
            let spoiler_details = self.get_spoiler_details(
                &orig_global_state,
                state,
                &state,
                spoiler_flag_details,
                spoiler_door_details,
            );
            state.previous_debug_data = state.debug_data.clone();
            return (spoiler_summary, spoiler_details, true);
        }

        let mut placed_uncollected_bireachable_loc: Vec<ItemLocationId> = Vec::new();
        let mut placed_uncollected_bireachable_items: Vec<Item> = Vec::new();
        let mut unplaced_bireachable: Vec<ItemLocationId> = Vec::new();
        let mut unplaced_oneway_reachable: Vec<ItemLocationId> = Vec::new();
        for (i, item_location_state) in state.item_location_state.iter().enumerate() {
            if let Some(item) = item_location_state.placed_item {
                if !item_location_state.collected && item_location_state.bireachable {
                    placed_uncollected_bireachable_loc.push(i);
                    placed_uncollected_bireachable_items.push(item);
                }
            } else {
                if item_location_state.bireachable {
                    unplaced_bireachable.push(i);
                } else if item_location_state.reachable {
                    unplaced_oneway_reachable.push(i);
                }
            }
        }
        unplaced_bireachable.shuffle(rng);
        unplaced_oneway_reachable.shuffle(rng);
        let (selection, mut new_state) = self.multi_attempt_select_items(
            attempt_num_rando,
            &state,
            &placed_uncollected_bireachable_items,
            unplaced_bireachable.len(),
            unplaced_oneway_reachable.len(),
            rng,
        );
        new_state.previous_debug_data = state.debug_data.clone();
        new_state.key_visited_vertices = state.key_visited_vertices.clone();

        // Mark the newly collected items that were placed on earlier steps:
        for &loc in &placed_uncollected_bireachable_loc {
            new_state.item_location_state[loc].collected = true;
        }

        // Place the new items:
        // We place items in all newly reachable locations (bireachable as
        // well as one-way-reachable locations). One-way-reachable locations are filled only
        // with filler items, to reduce the possibility of them being usable to break from the
        // intended logical sequence.
        self.place_items(
            attempt_num_rando,
            &state,
            &mut new_state,
            &unplaced_bireachable,
            &unplaced_oneway_reachable,
            &selection.key_items,
            &selection.other_items,
        );

        // Mark the newly placed bireachable items as collected:
        for &loc in &unplaced_bireachable {
            new_state.item_location_state[loc].collected = true;
        }

        let spoiler_summary = self.get_spoiler_summary(
            &orig_global_state,
            state,
            &new_state,
            spoiler_flag_summaries,
            spoiler_door_summaries,
        );
        let spoiler_details = self.get_spoiler_details(
            &orig_global_state,
            state,
            &new_state,
            spoiler_flag_details,
            spoiler_door_details,
        );
        *state = new_state;
        (spoiler_summary, spoiler_details, false)
    }

    fn get_seed_name(&self, seed: usize) -> String {
        let t = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut rng_seed = [0u8; 32];
        rng_seed[..8].copy_from_slice(&seed.to_le_bytes());
        rng_seed[8..24].copy_from_slice(&t.to_le_bytes());
        let mut rng = rand::rngs::StdRng::from_seed(rng_seed);
        // Leave out vowels and characters that could read like vowels, to minimize the chance
        // of forming words.
        let alphabet = "256789BCDFGHJKLMNPQRSTVWXYZbcdfghjkmnpqrstvwxyz";
        let mut out: String = String::new();
        let num_chars = 11;
        for _ in 0..num_chars {
            let i = rng.gen_range(0..alphabet.len());
            let c = alphabet.as_bytes()[i] as char;
            out.push(c);
        }
        out
    }

    fn get_randomization(
        &self,
        state: &RandomizationState,
        spoiler_summaries: Vec<SpoilerSummary>,
        spoiler_details: Vec<SpoilerDetails>,
        mut debug_data_vec: Vec<DebugData>,
        seed: usize,
        display_seed: usize,
    ) -> Result<Randomization> {
        // Compute the first step on which each node becomes reachable/bireachable:
        let mut node_reachable_step: HashMap<(RoomId, NodeId), usize> = HashMap::new();
        let mut node_bireachable_step: HashMap<(RoomId, NodeId), usize> = HashMap::new();
        let mut map_tile_reachable_step: HashMap<(RoomId, (usize, usize)), usize> = HashMap::new();
        let mut map_tile_bireachable_step: HashMap<(RoomId, (usize, usize)), usize> =
            HashMap::new();

        for (step, debug_data) in debug_data_vec.iter_mut().enumerate() {
            for (
                v,
                VertexKey {
                    room_id, node_id, ..
                },
            ) in self.game_data.vertex_isv.keys.iter().enumerate()
            {
                if node_bireachable_step.contains_key(&(*room_id, *node_id)) {
                    continue;
                }
                if get_bireachable_idxs(
                    &debug_data.global_state,
                    v,
                    &mut debug_data.forward,
                    &mut debug_data.reverse,
                )
                .is_some()
                {
                    node_bireachable_step.insert((*room_id, *node_id), step);
                    let room_ptr = self.game_data.room_ptr_by_id[room_id];
                    let room_idx = self.game_data.room_idx_by_ptr[&room_ptr];
                    if let Some(coords) = self.game_data.node_tile_coords.get(&(*room_id, *node_id))
                    {
                        for (x, y) in coords.iter().copied() {
                            let key = (room_idx, (x, y));
                            if !map_tile_bireachable_step.contains_key(&key) {
                                map_tile_bireachable_step.insert(key, step);
                            }
                        }
                    }
                }

                if node_reachable_step.contains_key(&(*room_id, *node_id)) {
                    continue;
                }
                if debug_data.forward.cost[v]
                    .iter()
                    .any(|&x| f32::is_finite(x))
                {
                    node_reachable_step.insert((*room_id, *node_id), step);
                    let room_ptr = self.game_data.room_ptr_by_id[room_id];
                    let room_idx = self.game_data.room_idx_by_ptr[&room_ptr];
                    if let Some(coords) = self.game_data.node_tile_coords.get(&(*room_id, *node_id))
                    {
                        for (x, y) in coords.iter().copied() {
                            let key = (room_idx, (x, y));
                            if !map_tile_reachable_step.contains_key(&key) {
                                map_tile_reachable_step.insert(key, step);
                            }
                        }
                    }
                }
            }
        }

        let item_placement: Vec<Item> = state
            .item_location_state
            .iter()
            .map(|x| x.placed_item.unwrap())
            .collect();
        let spoiler_all_items = state
            .item_location_state
            .iter()
            .enumerate()
            .map(|(i, x)| {
                let (r, n) = self.game_data.item_locations[i];
                let item_vertex_info = self.get_vertex_info_by_id(r, n);
                let location = SpoilerLocation {
                    area: item_vertex_info.area_name,
                    room: item_vertex_info.room_name,
                    node: item_vertex_info.node_name,
                    coords: item_vertex_info.room_coords,
                };
                let item = x.placed_item.unwrap();
                SpoilerItemLoc {
                    item: Item::VARIANTS[item as usize].to_string(),
                    location,
                }
            })
            .collect();
        let spoiler_all_rooms = self
            .map
            .rooms
            .iter()
            .enumerate()
            .zip(self.game_data.room_geometry.iter())
            .map(|((room_idx, c), g)| {
                let room = g.name.clone();
                let short_name = strip_name(&room);
                let map = if room_idx == self.game_data.toilet_room_idx {
                    vec![vec![1; 1]; 10]
                } else {
                    g.map.clone()
                };
                let height = map.len();
                let width = map[0].len();
                let mut map_reachable_step: Vec<Vec<u8>> = vec![vec![255; width]; height];
                let mut map_bireachable_step: Vec<Vec<u8>> = vec![vec![255; width]; height];
                for y in 0..height {
                    for x in 0..width {
                        if map[y][x] != 0 {
                            let key = (room_idx, (x, y));
                            if let Some(step) = map_tile_reachable_step.get(&key) {
                                map_reachable_step[y][x] = *step as u8;
                            }
                            if let Some(step) = map_tile_bireachable_step.get(&key) {
                                map_bireachable_step[y][x] = *step as u8;
                            }
                        }
                    }
                }
                SpoilerRoomLoc {
                    room,
                    short_name,
                    map,
                    map_reachable_step,
                    map_bireachable_step,
                    coords: *c,
                }
            })
            .collect();
        let spoiler_escape =
            escape_timer::compute_escape_data(self.game_data, self.map, &self.difficulty_tiers[0])?;
        let spoiler_log = SpoilerLog {
            item_priority: state
                .item_precedence
                .iter()
                .map(|x| format!("{:?}", x))
                .collect(),
            summary: spoiler_summaries,
            escape: spoiler_escape,
            details: spoiler_details,
            all_items: spoiler_all_items,
            all_rooms: spoiler_all_rooms,
        };

        Ok(Randomization {
            difficulty: self.difficulty_tiers[0].clone(),
            map: self.map.clone(),
            toilet_intersections: self.toilet_intersections.clone(),
            locked_door_data: self.locked_door_data.clone(),
            item_placement,
            spoiler_log,
            seed,
            display_seed,
            seed_name: self.get_seed_name(seed),
            start_location: state.start_location.clone(),
            starting_items: self.difficulty_tiers[0].starting_items.clone(),
        })
    }

    fn get_item_precedence<R: Rng>(
        &self,
        item_priorities: &[ItemPriorityGroup],
        item_priority_strength: ItemPriorityStrength,
        rng: &mut R,
    ) -> Vec<Item> {
        let mut item_precedence: Vec<Item> = Vec::new();
        if self.difficulty_tiers[0].progression_rate == ProgressionRate::Slow {
            // With slow progression, prioritize placing nothing and missiles over other key items.
            item_precedence.push(Item::Nothing);
            item_precedence.push(Item::Missile);
        }
        match item_priority_strength {
            ItemPriorityStrength::Moderate => {
                assert!(item_priorities.len() == 3);
                let mut items = vec![];
                for (i, priority_group) in item_priorities.iter().enumerate() {
                    for item_name in &priority_group.items {
                        items.push(item_name.clone());
                        if i != 1 {
                            // Include a second copy of Early and Late items:
                            items.push(item_name.clone());
                        }
                    }
                }
                items.shuffle(rng);

                // Remove the later copy of each "Early" item
                items = remove_some_duplicates(
                    &items,
                    &item_priorities[0].items.iter().cloned().collect(),
                );

                // Remove the earlier copy of each "Late" item
                items.reverse();
                items = remove_some_duplicates(
                    &items,
                    &item_priorities[2].items.iter().cloned().collect(),
                );
                items.reverse();

                for item_name in &items {
                    let item_idx = self.game_data.item_isv.index_by_key[item_name];
                    item_precedence.push(Item::try_from(item_idx).unwrap());
                }
            }
            ItemPriorityStrength::Heavy => {
                for priority_group in item_priorities {
                    let mut items = priority_group.items.clone();
                    items.shuffle(rng);
                    for item_name in &items {
                        let item_idx = self.game_data.item_isv.index_by_key[item_name];
                        item_precedence.push(Item::try_from(item_idx).unwrap());
                    }
                }
            }
        }
        if self.difficulty_tiers[0].progression_rate != ProgressionRate::Slow {
            // With Normal and Uniform progression, prioritize all other key items over missiles
            // and nothing.
            item_precedence.push(Item::Missile);
            item_precedence.push(Item::Nothing);
        }
        item_precedence
    }

    fn rerandomize_tank_precedence<R: Rng>(&self, item_precedence: &mut [Item], rng: &mut R) {
        if rng.gen_bool(0.5) {
            return;
        }
        let etank_idx = item_precedence
            .iter()
            .position(|&x| x == Item::ETank)
            .unwrap();
        let reserve_idx = item_precedence
            .iter()
            .position(|&x| x == Item::ReserveTank)
            .unwrap();
        item_precedence[etank_idx] = Item::ReserveTank;
        item_precedence[reserve_idx] = Item::ETank;
    }

    fn apply_spazer_plasma_priority(&self, item_precedence: &mut [Item]) {
        let spazer_idx_opt = item_precedence.iter().position(|&x| x == Item::Spazer);
        let plasma_idx_opt = item_precedence.iter().position(|&x| x == Item::Plasma);
        if spazer_idx_opt.is_none() || plasma_idx_opt.is_none() {
            return;
        }
        let spazer_idx = spazer_idx_opt.unwrap();
        let plasma_idx = plasma_idx_opt.unwrap();
        if plasma_idx < spazer_idx {
            item_precedence[plasma_idx] = Item::Spazer;
            item_precedence[spazer_idx] = Item::Plasma;
        }
    }

    pub fn determine_start_location<R: Rng>(
        &self,
        attempt_num_rando: usize,
        num_attempts: usize,
        rng: &mut R,
    ) -> Result<(StartLocation, HubLocation)> {
        if self.difficulty_tiers[0].start_location_mode == StartLocationMode::Ship {
            let mut ship_start = StartLocation::default();
            ship_start.name = "Ship".to_string();
            ship_start.room_id = 8;
            ship_start.node_id = 5;
            ship_start.door_load_node_id = Some(2);
            ship_start.x = 72.0;
            ship_start.y = 69.5;

            let mut ship_hub = HubLocation::default();
            ship_hub.name = "Ship".to_string();
            ship_hub.room_id = 8;
            ship_hub.node_id = 5;

            return Ok((ship_start, ship_hub));
        }
        for i in 0..num_attempts {
            info!("[attempt {attempt_num_rando}] start location attempt {}", i);
            let start_loc_idx = rng.gen_range(0..self.game_data.start_locations.len());
            let start_loc = self.game_data.start_locations[start_loc_idx].clone();

            info!("[attempt {attempt_num_rando}] start: {:?}", start_loc);
            let num_vertices = self.game_data.vertex_isv.keys.len();
            let start_vertex_id = self.game_data.vertex_isv.index_by_key[&VertexKey {
                room_id: start_loc.room_id,
                node_id: start_loc.node_id,
                obstacle_mask: 0,
                actions: vec![],
            }];
            let global = self.get_initial_global_state();
            let local = apply_requirement(
                &start_loc.requires_parsed.as_ref().unwrap(),
                &global,
                LocalState::new(),
                false,
                &self.difficulty_tiers[0],
                self.game_data,
                &self.locked_door_data,
            );
            if local.is_none() {
                continue;
            }
            let forward = traverse(
                &self.base_links_data,
                &self.seed_links_data,
                None,
                &global,
                local.unwrap(),
                num_vertices,
                start_vertex_id,
                false,
                &self.difficulty_tiers[0],
                self.game_data,
                self.locked_door_data,
            );
            let forward0 = traverse(
                &self.base_links_data,
                &self.seed_links_data,
                None,
                &global,
                LocalState::new(),
                num_vertices,
                start_vertex_id,
                false,
                &self.difficulty_tiers[0],
                self.game_data,
                self.locked_door_data,
            );
            let reverse = traverse(
                &self.base_links_data,
                &self.seed_links_data,
                None,
                &global,
                LocalState::new(),
                num_vertices,
                start_vertex_id,
                true,
                &self.difficulty_tiers[0],
                self.game_data,
                self.locked_door_data,
            );

            // We require several conditions for a start location to be valid with a given hub location:
            // 1) The hub location must be one-way reachable from the start location, including initial start location
            // requirements (e.g. including requirements to reach the starting node from the actual start location, which
            // may not be at a node)
            // 2) The starting node (not the actual start location) must be bireachable from the hub location
            // (ie. there must be a logical round-trip path from the hub to the starting node and back)
            // 3) Any logical requirements on the hub must be satisfied.
            for hub in &self.game_data.hub_locations {
                let hub_vertex_id = self.game_data.vertex_isv.index_by_key[&VertexKey {
                    room_id: hub.room_id,
                    node_id: hub.node_id,
                    obstacle_mask: 0,
                    actions: vec![],
                }];
                if forward.cost[hub_vertex_id]
                    .iter()
                    .any(|&x| f32::is_finite(x))
                    && get_bireachable_idxs(&global, hub_vertex_id, &forward0, &reverse).is_some()
                {
                    let local = apply_requirement(
                        &hub.requires_parsed.as_ref().unwrap(),
                        &global,
                        LocalState::new(),
                        false,
                        &self.difficulty_tiers[0],
                        self.game_data,
                        &self.locked_door_data,
                    );
                    if local.is_some() {
                        return Ok((start_loc, hub.clone()));
                    }
                }
            }
        }
        bail!("[attempt {attempt_num_rando}] Failed to find start location.")
    }

    fn get_initial_global_state(&self) -> GlobalState {
        let items = vec![false; self.game_data.item_isv.keys.len()];
        let weapon_mask = self.game_data.get_weapon_mask(&items);
        let mut global = GlobalState {
            tech: get_tech_vec(&self.game_data, &self.difficulty_tiers[0]),
            notable_strats: get_strat_vec(&self.game_data, &self.difficulty_tiers[0]),
            items: items,
            flags: self.get_initial_flag_vec(),
            doors_unlocked: vec![false; self.locked_door_data.locked_doors.len()],
            max_energy: 99,
            max_reserves: 0,
            max_missiles: 0,
            max_supers: 0,
            max_power_bombs: 0,
            weapon_mask: weapon_mask,
        };
        for &(item, cnt) in &self.difficulty_tiers[0].starting_items {
            for _ in 0..cnt {
                global.collect(item, self.game_data);
            }
        }
        global
    }

    pub fn dummy_randomize(&self, seed: usize, display_seed: usize) -> Result<Randomization> {
        // For the "Escape" start location mode, item placement is irrelevant since you start
        // with all items collected.
        let spoiler_escape =
            escape_timer::compute_escape_data(self.game_data, self.map, &self.difficulty_tiers[0])?;
        let spoiler_all_rooms = self
            .map
            .rooms
            .iter()
            .zip(self.game_data.room_geometry.iter())
            .map(|(c, g)| {
                let room = g.name.clone();
                let short_name = strip_name(&room);
                let height = g.map.len();
                let width = g.map[0].len();
                let map_reachable_step: Vec<Vec<u8>> = vec![vec![255; width]; height];
                let map_bireachable_step: Vec<Vec<u8>> = vec![vec![255; width]; height];
                SpoilerRoomLoc {
                    room,
                    short_name,
                    map: g.map.clone(),
                    map_reachable_step,
                    map_bireachable_step,
                    coords: *c,
                }
            })
            .collect();

        let starting_items: Vec<(Item, usize)> = vec![
            (Item::ETank, 14),
            (Item::Missile, 46),
            (Item::Super, 10),
            (Item::PowerBomb, 10),
            (Item::Bombs, 1),
            (Item::Charge, 1),
            (Item::Ice, 1),
            (Item::HiJump, 1),
            (Item::SpeedBooster, 1),
            (Item::Wave, 1),
            (Item::Spazer, 1),
            (Item::SpringBall, 1),
            (Item::Varia, 1),
            (Item::Gravity, 1),
            (Item::XRayScope, 1),
            (Item::Plasma, 1),
            (Item::Grapple, 1),
            (Item::SpaceJump, 1),
            (Item::ScrewAttack, 1),
            (Item::Morph, 1),
            (Item::ReserveTank, 4),
        ];

        let spoiler_log = SpoilerLog {
            item_priority: vec![],
            summary: vec![],
            escape: spoiler_escape,
            details: vec![],
            all_items: vec![],
            all_rooms: spoiler_all_rooms,
        };
        Ok(Randomization {
            difficulty: self.difficulty_tiers[0].clone(),
            map: self.map.clone(),
            toilet_intersections: self.toilet_intersections.clone(),
            locked_door_data: self.locked_door_data.clone(),
            item_placement: vec![Item::Nothing; 100],
            spoiler_log,
            seed,
            seed_name: self.get_seed_name(seed),
            display_seed,
            start_location: StartLocation::default(),
            starting_items,
        })
    }

    fn is_game_beatable(&self, state: &RandomizationState) -> bool {
        for (i, &flag_id) in self.game_data.flag_ids.iter().enumerate() {
            if flag_id == self.game_data.mother_brain_defeated_flag_id
                && state.flag_location_state[i].reachable
            {
                return true;
            }
        }
        return false;
    }

    pub fn randomize(
        &self,
        attempt_num_rando: usize,
        seed: usize,
        display_seed: usize,
    ) -> Result<Randomization> {
        if self.difficulty_tiers[0].start_location_mode == StartLocationMode::Escape {
            return self.dummy_randomize(seed, display_seed);
        }
        let mut rng_seed = [0u8; 32];
        rng_seed[..8].copy_from_slice(&seed.to_le_bytes());
        let mut rng = rand::rngs::StdRng::from_seed(rng_seed);
        let initial_global_state = self.get_initial_global_state();
        let initial_item_location_state = ItemLocationState {
            placed_item: None,
            collected: false,
            reachable: false,
            bireachable: false,
            bireachable_vertex_id: None,
            difficulty_tier: None,
        };
        let initial_flag_location_state = FlagLocationState {
            reachable: false,
            reachable_vertex_id: None,
            bireachable: false,
            bireachable_vertex_id: None,
        };
        let initial_save_location_state = SaveLocationState { bireachable: false };
        let initial_door_state = DoorState {
            bireachable: false,
            bireachable_vertex_id: None,
        };
        let num_attempts_start_location = 10;
        let (start_location, hub_location) = self.determine_start_location(
            attempt_num_rando,
            num_attempts_start_location,
            &mut rng,
        )?;
        let mut item_precedence: Vec<Item> = self.get_item_precedence(
            &self.difficulty_tiers[0].item_priorities,
            self.difficulty_tiers[0].item_priority_strength,
            &mut rng,
        );
        if self.difficulty_tiers[0].spazer_before_plasma {
            self.apply_spazer_plasma_priority(&mut item_precedence);
        }
        info!(
            "[attempt {attempt_num_rando}] Item precedence: {:?}",
            item_precedence
        );
        let mut state = RandomizationState {
            step_num: 1,
            item_precedence,
            start_location,
            hub_location,
            item_location_state: vec![
                initial_item_location_state;
                self.game_data.item_locations.len()
            ],
            flag_location_state: vec![initial_flag_location_state; self.game_data.flag_ids.len()],
            save_location_state: vec![
                initial_save_location_state;
                self.game_data.save_locations.len()
            ],
            door_state: vec![initial_door_state; self.locked_door_data.locked_doors.len()],
            items_remaining: self.initial_items_remaining.clone(),
            global_state: initial_global_state,
            debug_data: None,
            previous_debug_data: None,
            key_visited_vertices: HashSet::new(),
        };
        self.update_reachability(&mut state);
        if !state.item_location_state.iter().any(|x| x.bireachable) {
            bail!("[attempt {attempt_num_rando}] No initially bireachable item locations");
        }
        let mut spoiler_summary_vec: Vec<SpoilerSummary> = Vec::new();
        let mut spoiler_details_vec: Vec<SpoilerDetails> = Vec::new();
        let mut debug_data_vec: Vec<DebugData> = Vec::new();
        loop {
            if self.difficulty_tiers[0].random_tank {
                self.rerandomize_tank_precedence(&mut state.item_precedence, &mut rng);
            }
            let (spoiler_summary, spoiler_details, is_early_stop) =
                self.step(attempt_num_rando, &mut state, &mut rng);
            let cnt_collected = state
                .item_location_state
                .iter()
                .filter(|x| x.collected)
                .count();
            let cnt_placed = state
                .item_location_state
                .iter()
                .filter(|x| x.placed_item.is_some())
                .count();
            let cnt_reachable = state
                .item_location_state
                .iter()
                .filter(|x| x.reachable)
                .count();
            let cnt_bireachable = state
                .item_location_state
                .iter()
                .filter(|x| x.bireachable)
                .count();
            info!("[attempt {attempt_num_rando}] step={0}, bireachable={cnt_bireachable}, reachable={cnt_reachable}, placed={cnt_placed}, collected={cnt_collected}", state.step_num);

            let any_progress = spoiler_summary.items.len() > 0 || spoiler_summary.flags.len() > 0;
            spoiler_summary_vec.push(spoiler_summary);
            spoiler_details_vec.push(spoiler_details);
            debug_data_vec.push(state.previous_debug_data.as_ref().unwrap().clone());

            if is_early_stop {
                break;
            }

            if !any_progress {
                // No further progress was made on the last step. So we are done with this attempt: either we have
                // succeeded or we have failed.

                if !self.is_game_beatable(&state) {
                    bail!("[attempt {attempt_num_rando}] Attempt failed: Game not beatable");
                }

                if !self.difficulty_tiers[0].stop_item_placement_early {
                    // Check that at least one instance of each item can be collected.
                    for i in 0..self.initial_items_remaining.len() {
                        if self.initial_items_remaining[i] > 0 && !state.global_state.items[i] {
                            bail!("[attempt {attempt_num_rando}] Attempt failed: Key items not all collectible, missing {:?}",
                                  Item::try_from(i).unwrap());
                        }
                    }

                    // Check that Phantoon can be defeated. This is to rule out the possibility that Phantoon may be locked
                    // behind Bowling Alley.
                    let phantoon_flag_id =
                        self.game_data.flag_isv.index_by_key["f_DefeatedPhantoon"];
                    let mut phantoon_defeated = false;
                    for (i, flag_id) in self.game_data.flag_ids.iter().enumerate() {
                        if *flag_id == phantoon_flag_id && state.flag_location_state[i].bireachable
                        {
                            phantoon_defeated = true;
                        }
                    }

                    if !phantoon_defeated {
                        bail!(
                            "[attempt {attempt_num_rando}] Attempt failed: Phantoon not defeated"
                        );
                    }
                }

                // Success:
                break;
            }

            if state.step_num == 1 && self.difficulty_tiers[0].early_save {
                if !state.save_location_state.iter().any(|x| x.bireachable) {
                    bail!(
                        "[attempt {attempt_num_rando}] Attempt failed: no accessible save location"
                    );
                }
            }
            state.step_num += 1;
        }
        self.finish(attempt_num_rando, &mut state);
        self.get_randomization(
            &state,
            spoiler_summary_vec,
            spoiler_details_vec,
            debug_data_vec,
            seed,
            display_seed,
        )
    }
}

// Spoiler log ---------------------------------------------------------

#[derive(Serialize, Deserialize, Debug)]
pub struct SpoilerRouteEntry {
    area: String,
    room: String,
    node: String,
    short_room: String,
    from_node_id: usize,
    to_node_id: usize,
    obstacles_bitmask: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    coords: Option<(usize, usize)>,
    strat_name: String,
    short_strat_name: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    strat_notes: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    energy_used: Option<Capacity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reserves_used: Option<Capacity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    missiles_used: Option<Capacity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    supers_used: Option<Capacity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    power_bombs_used: Option<Capacity>,
}

#[derive(Serialize, Deserialize)]
pub struct SpoilerLocation {
    pub area: String,
    pub room: String,
    pub node: String,
    pub coords: (usize, usize),
}

#[derive(Serialize, Deserialize)]
pub struct SpoilerStartState {
    max_energy: Capacity,
    max_reserves: Capacity,
    max_missiles: Capacity,
    max_supers: Capacity,
    max_power_bombs: Capacity,
    items: Vec<String>,
    flags: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct SpoilerItemDetails {
    item: String,
    location: SpoilerLocation,
    difficulty: Option<String>,
    obtain_route: Vec<SpoilerRouteEntry>,
    return_route: Vec<SpoilerRouteEntry>,
}

#[derive(Serialize, Deserialize)]
pub struct SpoilerFlagDetails {
    flag: String,
    location: SpoilerLocation,
    obtain_route: Vec<SpoilerRouteEntry>,
    return_route: Vec<SpoilerRouteEntry>,
}

#[derive(Serialize, Deserialize)]
pub struct SpoilerDoorDetails {
    door_type: String,
    location: SpoilerLocation,
    obtain_route: Vec<SpoilerRouteEntry>,
    return_route: Vec<SpoilerRouteEntry>,
}

#[derive(Serialize, Deserialize)]
pub struct SpoilerDetails {
    step: usize,
    start_state: SpoilerStartState,
    flags: Vec<SpoilerFlagDetails>,
    doors: Vec<SpoilerDoorDetails>,
    items: Vec<SpoilerItemDetails>,
}

#[derive(Serialize, Deserialize)]
pub struct SpoilerItemLoc {
    pub item: String,
    pub location: SpoilerLocation,
}
#[derive(Serialize, Deserialize)]
pub struct SpoilerRoomLoc {
    // here temporarily, most likely, since these can be baked into the web UI
    room: String,
    short_name: String,
    map: Vec<Vec<u8>>,
    map_reachable_step: Vec<Vec<u8>>,
    map_bireachable_step: Vec<Vec<u8>>,
    coords: (usize, usize),
}

#[derive(Serialize, Deserialize)]
pub struct SpoilerItemSummary {
    pub item: String,
    pub location: SpoilerLocation,
}

#[derive(Serialize, Deserialize)]
pub struct SpoilerFlagSummary {
    flag: String,
}

#[derive(Serialize, Deserialize)]
pub struct SpoilerDoorSummary {
    door_type: String,
    location: SpoilerLocation,
}

#[derive(Serialize, Deserialize)]
pub struct SpoilerSummary {
    pub step: usize,
    pub flags: Vec<SpoilerFlagSummary>,
    pub doors: Vec<SpoilerDoorSummary>,
    pub items: Vec<SpoilerItemSummary>,
}

#[derive(Serialize, Deserialize)]
pub struct SpoilerLog {
    pub item_priority: Vec<String>,
    pub summary: Vec<SpoilerSummary>,
    pub escape: SpoilerEscape,
    pub details: Vec<SpoilerDetails>,
    pub all_items: Vec<SpoilerItemLoc>,
    pub all_rooms: Vec<SpoilerRoomLoc>,
}

impl<'a> Randomizer<'a> {
    fn get_vertex_info(&self, vertex_id: usize) -> VertexInfo {
        let VertexKey {
            room_id, node_id, ..
        } = self.game_data.vertex_isv.keys[vertex_id];
        self.get_vertex_info_by_id(room_id, node_id)
    }
    fn get_vertex_info_by_id(&self, room_id: RoomId, node_id: NodeId) -> VertexInfo {
        let room_ptr = self.game_data.room_ptr_by_id[&room_id];
        let room_idx = self.game_data.room_idx_by_ptr[&room_ptr];
        let area = self.map.area[room_idx];
        let room_coords = self.map.rooms[room_idx];
        VertexInfo {
            area_name: self.game_data.area_names[area].clone(),
            room_name: self.game_data.room_json_map[&room_id]["name"]
                .as_str()
                .unwrap()
                .to_string(),
            room_id,
            room_coords,
            node_name: self.game_data.node_json_map[&(room_id, node_id)]["name"]
                .as_str()
                .unwrap()
                .to_string(),
            node_id,
        }
    }

    fn get_spoiler_start_state(&self, global_state: &GlobalState) -> SpoilerStartState {
        let mut items: Vec<String> = Vec::new();
        for i in 0..self.game_data.item_isv.keys.len() {
            if global_state.items[i] {
                items.push(self.game_data.item_isv.keys[i].to_string());
            }
        }
        let mut flags: Vec<String> = Vec::new();
        for i in 0..self.game_data.flag_isv.keys.len() {
            if global_state.flags[i] {
                flags.push(self.game_data.flag_isv.keys[i].to_string());
            }
        }
        SpoilerStartState {
            max_energy: global_state.max_energy,
            max_reserves: global_state.max_reserves,
            max_missiles: global_state.max_missiles,
            max_supers: global_state.max_supers,
            max_power_bombs: global_state.max_power_bombs,
            items: items,
            flags: flags,
        }
    }

    fn get_spoiler_route(
        &self,
        global_state: &GlobalState,
        mut local_state: LocalState,
        link_idxs: &[LinkIdx],
        difficulty: &DifficultyConfig,
        reverse: bool,
    ) -> Vec<SpoilerRouteEntry> {
        let mut route: Vec<SpoilerRouteEntry> = Vec::new();
        for &link_idx in link_idxs {
            let link = self.get_link(link_idx as usize);
            let raw_link = self.get_link(link_idx as usize);
            let sublinks = vec![raw_link.clone()];

            let new_local_state_opt = apply_link(
                &link,
                &global_state,
                local_state,
                reverse,
                difficulty,
                self.game_data,
                &self.locked_door_data,
            );
            if new_local_state_opt.is_none() {
                panic!("Failed applying requirement in spoiler route: reverse={}, local_state={:?}, requirement={:?}", reverse, local_state, link.requirement);
            }
            let new_local_state = new_local_state_opt.unwrap();
            let sublinks_ordered: Vec<&Link> = if reverse {
                sublinks.iter().rev().collect()
            } else {
                sublinks.iter().collect()
            };
            for (i, link) in sublinks_ordered.iter().enumerate() {
                let last = i == sublinks.len() - 1;
                let from_vertex_info = self.get_vertex_info(link.from_vertex_id);
                let to_vertex_info = self.get_vertex_info(link.to_vertex_id);
                let VertexKey {
                    obstacle_mask: to_obstacles_mask,
                    ..
                } = self.game_data.vertex_isv.keys[link.to_vertex_id];
                let door_coords = self
                    .game_data
                    .node_coords
                    .get(&(to_vertex_info.room_id, to_vertex_info.node_id))
                    .map(|x| *x);
                let coords = door_coords.map(|(x, y)| {
                    (
                        x + to_vertex_info.room_coords.0,
                        y + to_vertex_info.room_coords.1,
                    )
                });

                let spoiler_entry = SpoilerRouteEntry {
                    area: to_vertex_info.area_name,
                    short_room: strip_name(&to_vertex_info.room_name),
                    room: to_vertex_info.room_name,
                    node: to_vertex_info.node_name,
                    from_node_id: from_vertex_info.node_id,
                    to_node_id: to_vertex_info.node_id,
                    obstacles_bitmask: to_obstacles_mask,
                    coords,
                    strat_name: link.strat_name.clone(),
                    short_strat_name: strip_name(&link.strat_name),
                    strat_notes: link.strat_notes.clone(),
                    energy_used: if last {
                        Some(new_local_state.energy_used)
                    } else {
                        Some(local_state.energy_used)
                    },
                    reserves_used: if last {
                        Some(new_local_state.reserves_used)
                    } else {
                        Some(local_state.reserves_used)
                    },
                    missiles_used: if last {
                        Some(new_local_state.missiles_used)
                    } else {
                        Some(local_state.missiles_used)
                    },
                    supers_used: if last {
                        Some(new_local_state.supers_used)
                    } else {
                        Some(local_state.supers_used)
                    },
                    power_bombs_used: if last {
                        Some(new_local_state.power_bombs_used)
                    } else {
                        Some(local_state.power_bombs_used)
                    },
                };
                route.push(spoiler_entry);
            }
            local_state = new_local_state;
        }

        if reverse {
            route.reverse();
        }

        // Remove repeated resource values, to reduce clutter in the spoiler view:
        for i in (0..(route.len() - 1)).rev() {
            if route[i + 1].energy_used == route[i].energy_used {
                route[i + 1].energy_used = None;
            }
            if route[i + 1].reserves_used == route[i].reserves_used {
                route[i + 1].reserves_used = None;
            }
            if route[i + 1].missiles_used == route[i].missiles_used {
                route[i + 1].missiles_used = None;
            }
            if route[i + 1].supers_used == route[i].supers_used {
                route[i + 1].supers_used = None;
            }
            if route[i + 1].power_bombs_used == route[i].power_bombs_used {
                route[i + 1].power_bombs_used = None;
            }
        }
        if route[0].energy_used == Some(0) {
            route[0].energy_used = None;
        }
        if route[0].reserves_used == Some(0) {
            route[0].reserves_used = None;
        }
        if route[0].missiles_used == Some(0) {
            route[0].missiles_used = None;
        }
        if route[0].supers_used == Some(0) {
            route[0].supers_used = None;
        }
        if route[0].power_bombs_used == Some(0) {
            route[0].power_bombs_used = None;
        }

        route
    }

    fn get_spoiler_route_birectional(
        &self,
        state: &RandomizationState,
        vertex_id: usize,
    ) -> (Vec<SpoilerRouteEntry>, Vec<SpoilerRouteEntry>) {
        let forward = &state.debug_data.as_ref().unwrap().forward;
        let reverse = &state.debug_data.as_ref().unwrap().reverse;
        let global_state = &state.debug_data.as_ref().unwrap().global_state;
        let (forward_cost_idx, reverse_cost_idx) =
            get_bireachable_idxs(global_state, vertex_id, forward, reverse).unwrap();
        let forward_link_idxs: Vec<LinkIdx> =
            get_spoiler_route(forward, vertex_id, forward_cost_idx);
        let reverse_link_idxs: Vec<LinkIdx> =
            get_spoiler_route(reverse, vertex_id, reverse_cost_idx);
        let obtain_route = self.get_spoiler_route(
            global_state,
            LocalState::new(),
            &forward_link_idxs,
            &self.difficulty_tiers[0],
            false,
        );
        let return_route = self.get_spoiler_route(
            global_state,
            LocalState::new(),
            &reverse_link_idxs,
            &self.difficulty_tiers[0],
            true,
        );
        (obtain_route, return_route)
    }

    fn get_spoiler_route_one_way(
        &self,
        state: &RandomizationState,
        vertex_id: usize,
    ) -> Vec<SpoilerRouteEntry> {
        let forward = &state.debug_data.as_ref().unwrap().forward;
        let global_state = &state.debug_data.as_ref().unwrap().global_state;
        let forward_cost_idx = get_one_way_reachable_idx(vertex_id, forward).unwrap();
        let forward_link_idxs: Vec<LinkIdx> =
            get_spoiler_route(forward, vertex_id, forward_cost_idx);
        let obtain_route = self.get_spoiler_route(
            global_state,
            LocalState::new(),
            &forward_link_idxs,
            &self.difficulty_tiers[0],
            false,
        );
        obtain_route
    }

    fn get_spoiler_item_details(
        &self,
        state: &RandomizationState,
        item_vertex_id: usize,
        item: Item,
        tier: Option<usize>,
    ) -> SpoilerItemDetails {
        let (obtain_route, return_route) =
            self.get_spoiler_route_birectional(state, item_vertex_id);
        let item_vertex_info = self.get_vertex_info(item_vertex_id);
        SpoilerItemDetails {
            item: Item::VARIANTS[item as usize].to_string(),
            location: SpoilerLocation {
                area: item_vertex_info.area_name,
                room: item_vertex_info.room_name,
                node: item_vertex_info.node_name,
                coords: item_vertex_info.room_coords,
            },
            difficulty: if let Some(tier) = tier {
                self.difficulty_tiers[tier].name.clone()
            } else {
                None
            },
            obtain_route: obtain_route,
            return_route: return_route,
        }
    }

    fn get_spoiler_item_summary(
        &self,
        _state: &RandomizationState,
        item_vertex_id: usize,
        item: Item,
    ) -> SpoilerItemSummary {
        let item_vertex_info = self.get_vertex_info(item_vertex_id);
        SpoilerItemSummary {
            item: Item::VARIANTS[item as usize].to_string(),
            location: SpoilerLocation {
                area: item_vertex_info.area_name,
                room: item_vertex_info.room_name,
                node: item_vertex_info.node_name,
                coords: item_vertex_info.room_coords,
            },
        }
    }

    fn get_spoiler_flag_details(
        &self,
        state: &RandomizationState,
        flag_vertex_id: usize,
        flag_id: FlagId,
    ) -> SpoilerFlagDetails {
        let (obtain_route, return_route) =
            self.get_spoiler_route_birectional(state, flag_vertex_id);
        let flag_vertex_info = self.get_vertex_info(flag_vertex_id);
        SpoilerFlagDetails {
            flag: self.game_data.flag_isv.keys[flag_id].to_string(),
            location: SpoilerLocation {
                area: flag_vertex_info.area_name,
                room: flag_vertex_info.room_name,
                node: flag_vertex_info.node_name,
                coords: flag_vertex_info.room_coords,
            },
            obtain_route: obtain_route,
            return_route: return_route,
        }
    }

    fn get_spoiler_flag_details_one_way(
        &self,
        state: &RandomizationState,
        flag_vertex_id: usize,
        flag_id: FlagId,
    ) -> SpoilerFlagDetails {
        // This is for a one-way reachable flag, used for f_DefeatedMotherBrain:
        let obtain_route = self.get_spoiler_route_one_way(state, flag_vertex_id);
        let flag_vertex_info = self.get_vertex_info(flag_vertex_id);
        SpoilerFlagDetails {
            flag: self.game_data.flag_isv.keys[flag_id].to_string(),
            location: SpoilerLocation {
                area: flag_vertex_info.area_name,
                room: flag_vertex_info.room_name,
                node: flag_vertex_info.node_name,
                coords: flag_vertex_info.room_coords,
            },
            obtain_route: obtain_route,
            return_route: vec![],
        }
    }

    fn get_door_type_name(door_type: DoorType) -> String {
        match door_type {
            DoorType::Blue => "blue",
            DoorType::Red => "red",
            DoorType::Green => "green",
            DoorType::Yellow => "yellow",
            DoorType::Gray => "gray",
            DoorType::Beam(beam) => match beam {
                BeamType::Charge => "charge",
                BeamType::Ice => "ice",
                BeamType::Wave => "wave",
                BeamType::Spazer => "spazer",
                BeamType::Plasma => "plasma",
            },
        }
        .to_string()
    }

    fn get_spoiler_door_details(
        &self,
        state: &RandomizationState,
        unlock_vertex_id: usize,
        locked_door_idx: usize,
    ) -> SpoilerDoorDetails {
        let (obtain_route, return_route) =
            self.get_spoiler_route_birectional(state, unlock_vertex_id);
        let locked_door = &self.locked_door_data.locked_doors[locked_door_idx];
        let (room_id, node_id) = self.game_data.door_ptr_pair_map[&locked_door.src_ptr_pair];
        let door_vertex_id = self.game_data.vertex_isv.index_by_key[&VertexKey {
            room_id,
            node_id,
            obstacle_mask: 0,
            actions: vec![],
        }];
        let door_vertex_info = self.get_vertex_info(door_vertex_id);
        SpoilerDoorDetails {
            door_type: Self::get_door_type_name(
                self.locked_door_data.locked_doors[locked_door_idx].door_type,
            ),
            location: SpoilerLocation {
                area: door_vertex_info.area_name,
                room: door_vertex_info.room_name,
                node: door_vertex_info.node_name,
                coords: door_vertex_info.room_coords,
            },
            obtain_route: obtain_route,
            return_route: return_route,
        }
    }

    fn get_spoiler_flag_summary(
        &self,
        _state: &RandomizationState,
        _flag_vertex_id: usize,
        flag_id: FlagId,
    ) -> SpoilerFlagSummary {
        SpoilerFlagSummary {
            flag: self.game_data.flag_isv.keys[flag_id].to_string(),
        }
    }

    fn get_spoiler_door_summary(
        &self,
        _unlock_vertex_id: usize,
        locked_door_idx: usize,
    ) -> SpoilerDoorSummary {
        let locked_door = &self.locked_door_data.locked_doors[locked_door_idx];
        let (room_id, node_id) = self.game_data.door_ptr_pair_map[&locked_door.src_ptr_pair];
        let door_vertex_id = self.game_data.vertex_isv.index_by_key[&VertexKey {
            room_id,
            node_id,
            obstacle_mask: 0,
            actions: vec![],
        }];
        let door_vertex_info = self.get_vertex_info(door_vertex_id);
        SpoilerDoorSummary {
            door_type: Self::get_door_type_name(
                self.locked_door_data.locked_doors[locked_door_idx].door_type,
            ),
            location: SpoilerLocation {
                area: door_vertex_info.area_name,
                room: door_vertex_info.room_name,
                node: door_vertex_info.node_name,
                coords: door_vertex_info.room_coords,
            },
        }
    }

    fn get_spoiler_details(
        &self,
        orig_global_state: &GlobalState, // Global state before acquiring new flags
        state: &RandomizationState,      // State after acquiring new flags but not new items
        new_state: &RandomizationState,  // State after acquiring new flags and new items
        spoiler_flag_details: Vec<SpoilerFlagDetails>,
        spoiler_door_details: Vec<SpoilerDoorDetails>,
    ) -> SpoilerDetails {
        let mut items: Vec<SpoilerItemDetails> = Vec::new();
        for i in 0..self.game_data.item_locations.len() {
            if let Some(item) = new_state.item_location_state[i].placed_item {
                if item == Item::Nothing {
                    continue;
                }
                if !state.item_location_state[i].collected
                    && new_state.item_location_state[i].collected
                {
                    let item_vertex_id =
                        state.item_location_state[i].bireachable_vertex_id.unwrap();
                    let tier = new_state.item_location_state[i].difficulty_tier;
                    items.push(self.get_spoiler_item_details(state, item_vertex_id, item, tier));
                }
            }
        }
        SpoilerDetails {
            step: state.step_num,
            start_state: self.get_spoiler_start_state(orig_global_state),
            items,
            flags: spoiler_flag_details,
            doors: spoiler_door_details,
        }
    }

    fn get_spoiler_summary(
        &self,
        _orig_global_state: &GlobalState, // Global state before acquiring new flags
        state: &RandomizationState,       // State after acquiring new flags but not new items
        new_state: &RandomizationState,   // State after acquiring new flags and new items
        spoiler_flag_summaries: Vec<SpoilerFlagSummary>,
        spoiler_door_summaries: Vec<SpoilerDoorSummary>,
    ) -> SpoilerSummary {
        let mut items: Vec<SpoilerItemSummary> = Vec::new();
        for i in 0..self.game_data.item_locations.len() {
            if let Some(item) = new_state.item_location_state[i].placed_item {
                if item == Item::Nothing {
                    continue;
                }
                if !state.item_location_state[i].collected
                    && new_state.item_location_state[i].collected
                {
                    let item_vertex_id =
                        state.item_location_state[i].bireachable_vertex_id.unwrap();
                    items.push(self.get_spoiler_item_summary(state, item_vertex_id, item));
                }
            }
        }
        SpoilerSummary {
            step: state.step_num,
            items,
            flags: spoiler_flag_summaries,
            doors: spoiler_door_summaries,
        }
    }
}
