//! Rollback and correction helpers for mixed-authority multiplayer.

use std::collections::{hash_map::DefaultHasher, HashMap, VecDeque};
use std::hash::{Hash, Hasher};

use crate::components::{Camera, Transform};
use crate::ecs::entity::Entity;
use crate::ecs::world::World;
use crate::game::ctf::resources::{
    AutoMovePath, AutoMoveState, CarrierState, ControlState, EntityRefs, FlagMotion,
    FlagMotionState, GamePhase, GameState,
};
use crate::multiplayer::matchmaking::{CtfSlot, CtfSlotAssignment};

use super::net_types::{
    CtfAutoMovePathSnapshot, CtfFlagMotionSnapshot, CtfSnapshotState, EntityStatePacket,
    NetworkEntityId, StableEntityId,
};
use super::net_types::{NetworkTick, Snapshot};

/// A compact deterministic state hash used for divergence checks.
pub type FrameHash = u64;

/// Number of simulation frames rendered behind the latest processed frame.
pub const INTERPOLATION_DELAY_FRAMES: NetworkTick = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SnapshotEntityKey {
    Stable(StableEntityId),
    Entity(NetworkEntityId),
}

fn snapshot_entity_key(state: &EntityStatePacket) -> SnapshotEntityKey {
    state.stable_id.map_or(
        SnapshotEntityKey::Entity(state.entity),
        SnapshotEntityKey::Stable,
    )
}

/// Rolling snapshot buffer used for delayed rendering and smooth corrections.
#[derive(Debug, Clone)]
pub struct FrameInterpolationBuffer {
    delay: NetworkTick,
    frames: VecDeque<Snapshot>,
}

impl Default for FrameInterpolationBuffer {
    fn default() -> Self {
        Self::new(INTERPOLATION_DELAY_FRAMES)
    }
}

impl FrameInterpolationBuffer {
    /// Create a buffer that displays `delay` frames behind the latest snapshot.
    pub fn new(delay: NetworkTick) -> Self {
        Self {
            delay,
            frames: VecDeque::new(),
        }
    }

    /// Configured frame delay.
    pub fn delay(&self) -> NetworkTick {
        self.delay
    }

    /// Record a simulated frame, replacing an older snapshot for the same tick.
    pub fn push_snapshot(&mut self, snapshot: Snapshot) {
        self.insert_snapshot(snapshot);
        self.trim_to_delay();
    }

    /// Oldest retained snapshot, which is the frame rendered to the player.
    pub fn display_snapshot(&self) -> Option<&Snapshot> {
        self.frames.front()
    }

    /// Latest retained snapshot, which corresponds to the processed world.
    pub fn latest_snapshot(&self) -> Option<&Snapshot> {
        self.frames.back()
    }

    /// Return a retained snapshot for an exact simulation tick.
    pub fn snapshot_for_tick(&self, tick: NetworkTick) -> Option<&Snapshot> {
        self.frames
            .iter()
            .rev()
            .find(|snapshot| snapshot.tick == tick)
    }

    /// Return a deterministic hash for a retained simulation tick.
    pub fn hash_for_tick(&self, tick: NetworkTick) -> Option<FrameHash> {
        self.snapshot_for_tick(tick).map(snapshot_hash)
    }

    /// Return a deterministic hash for part of a retained simulation tick.
    pub fn authority_hash_for_tick<F>(&self, tick: NetworkTick, accepts: F) -> Option<FrameHash>
    where
        F: Fn(NetworkEntityId) -> bool,
    {
        self.snapshot_for_tick(tick)
            .map(|snapshot| snapshot_authority_hash(snapshot, accepts))
    }

    /// Iterate retained snapshots from oldest to newest.
    pub fn snapshots(&self) -> impl Iterator<Item = &Snapshot> {
        self.frames.iter()
    }

    /// Replace a corrected frame and rebuild buffered in-between frames.
    ///
    /// If the display frame is tick 7 and the authoritative correction is tick
    /// 10, ticks 8 and 9 become linear blends from the displayed frame to the
    /// authoritative frame. Simulation state can jump to tick 10 immediately,
    /// while rendering walks through the rebuilt buffer.
    pub fn reconcile(&mut self, authoritative: Snapshot) {
        self.reconcile_authority(authoritative, |_| true);
    }

    /// Replace a corrected authority subset and rebuild buffered in-between frames.
    pub fn reconcile_authority<F>(&mut self, authoritative: Snapshot, accepts: F)
    where
        F: Fn(NetworkEntityId) -> bool,
    {
        let correction_tick = authoritative.tick;
        let target_entities = authoritative
            .entities
            .iter()
            .filter(|entity| accepts(entity.entity))
            .cloned()
            .collect::<Vec<_>>();

        if target_entities.is_empty() {
            return;
        }

        let anchor = self
            .snapshot_at_or_before(correction_tick.saturating_sub(self.delay))
            .or_else(|| self.display_snapshot())
            .cloned();

        let Some(anchor) = anchor else {
            self.push_snapshot(Snapshot {
                entities: target_entities,
                ..authoritative
            });
            return;
        };

        if correction_tick <= anchor.tick {
            let mut corrected = self
                .snapshot_for_tick(correction_tick)
                .cloned()
                .unwrap_or_else(|| Snapshot {
                    tick: correction_tick,
                    entities: anchor.entities.clone(),
                    ctf: anchor.ctf.clone(),
                });
            merge_authority_entities(&mut corrected, &target_entities);
            self.push_snapshot(corrected);
            return;
        }

        let mut rebuilt: Vec<Snapshot> = self
            .frames
            .iter()
            .filter(|snapshot| snapshot.tick < anchor.tick || snapshot.tick > correction_tick)
            .cloned()
            .collect();

        rebuilt.push(anchor.clone());
        for tick in (anchor.tick + 1)..correction_tick {
            rebuilt.push(interpolate_authority_snapshot(
                self.snapshot_for_tick(tick),
                &anchor,
                &target_entities,
                correction_tick,
                tick,
            ));
        }
        rebuilt.push(interpolate_authority_snapshot(
            self.snapshot_for_tick(correction_tick),
            &anchor,
            &target_entities,
            correction_tick,
            correction_tick,
        ));

        rebuilt.sort_by_key(|snapshot| snapshot.tick);
        self.frames.clear();
        for snapshot in rebuilt {
            self.insert_snapshot(snapshot);
        }
        self.trim_to_delay();
    }

    fn snapshot_at_or_before(&self, tick: NetworkTick) -> Option<&Snapshot> {
        self.frames
            .iter()
            .rev()
            .find(|snapshot| snapshot.tick <= tick)
    }

    fn insert_snapshot(&mut self, snapshot: Snapshot) {
        if let Some(index) = self
            .frames
            .iter()
            .position(|existing| existing.tick == snapshot.tick)
        {
            self.frames[index] = snapshot;
        } else {
            self.frames.push_back(snapshot);
        }
        self.frames
            .make_contiguous()
            .sort_by_key(|snapshot| snapshot.tick);
    }

    fn trim_to_delay(&mut self) {
        let capacity = self.delay as usize + 1;
        while self.frames.len() > capacity {
            self.frames.pop_front();
        }
    }
}

/// Canonicalize an `f32` before hashing to eliminate bit-pattern ambiguity.
///
/// Two cases produce multiple distinct bit patterns for logically equivalent
/// values and must be collapsed before calling `to_bits()`:
///
/// - **Signed zero**: `+0.0` and `-0.0` compare equal under IEEE 754 but have
///   different bit patterns (`0x00000000` vs `0x80000000`).  Both are mapped to
///   `+0.0` so that peers which arrive at zero via different arithmetic paths
///   still agree on the hash.
///
/// - **NaN**: any bit pattern with a saturated exponent and non-zero mantissa is
///   a NaN, giving thousands of distinct encodings that all represent "not a
///   number".  All are mapped to `f32::NAN` (canonical quiet NaN, `0x7FC00000`)
///   so that a single corrupted float does not produce an unpredictable hash.
///
/// All other finite and infinite values are returned unchanged.
fn canonicalize(v: f32) -> f32 {
    if v.is_nan() {
        f32::NAN
    } else if v == 0.0 {
        0.0_f32
    } else {
        v
    }
}

/// Deterministically hash all `Transform` component data in a world.
///
/// Camera entities are excluded because each player has an independent camera
/// — including them would cause spurious desync detections.
pub fn state_hash(world: &World, _tick: NetworkTick) -> FrameHash {
    snapshot_hash(&capture_snapshot(world, _tick))
}

/// Deterministically hash a captured snapshot.
pub fn snapshot_hash(snapshot: &Snapshot) -> FrameHash {
    snapshot_authority_hash(snapshot, |_| true)
}

/// Deterministically hash a filtered entity subset from a captured snapshot.
pub fn snapshot_authority_hash<F>(snapshot: &Snapshot, accepts: F) -> FrameHash
where
    F: Fn(NetworkEntityId) -> bool,
{
    let mut entities = snapshot.entities.clone();
    entities.retain(|entity| accepts(entity.entity));
    entities.sort_by(|a, b| match a.entity.0.cmp(&b.entity.0) {
        std::cmp::Ordering::Equal => a.entity.1.cmp(&b.entity.1),
        other => other,
    });

    let filtered_len = entities.len();
    let mut hasher = DefaultHasher::new();
    entities.into_iter().for_each(|entity| {
        entity.entity.0.hash(&mut hasher);
        entity.entity.1.hash(&mut hasher);
        entity.stable_id.hash(&mut hasher);
        canonicalize(entity.position.0).to_bits().hash(&mut hasher);
        canonicalize(entity.position.1).to_bits().hash(&mut hasher);
        canonicalize(entity.position.2).to_bits().hash(&mut hasher);
        canonicalize(entity.rotation).to_bits().hash(&mut hasher);
        canonicalize(entity.scale.0).to_bits().hash(&mut hasher);
        canonicalize(entity.scale.1).to_bits().hash(&mut hasher);
        canonicalize(entity.scale.2).to_bits().hash(&mut hasher);
    });

    if filtered_len == 0 {
        0_u8.hash(&mut hasher);
    }

    if filtered_len == snapshot.entities.len() {
        if let Some(state) = &snapshot.ctf {
            hash_ctf_snapshot(&mut hasher, state);
        }
    }

    hasher.finish()
}

/// Build a full transform snapshot from authoritative state.
///
/// Camera entities are excluded from snapshots — each player manages their
/// own camera independently.
pub fn capture_snapshot(world: &World, tick: NetworkTick) -> Snapshot {
    let mut entities = Vec::new();
    let stable_ids = ctf_stable_entity_ids(world);

    let mut list: Vec<_> = world
        .query::<Transform>()
        .filter(|(entity, _)| world.get::<Camera>(*entity).is_none())
        .map(|(entity, transform)| {
            let mut packet = EntityStatePacket::from((entity, transform));
            packet.stable_id = stable_ids.get(&packet.entity).copied();
            packet
        })
        .collect();

    list.sort_by(|a, b| match a.entity.0.cmp(&b.entity.0) {
        std::cmp::Ordering::Equal => a.entity.1.cmp(&b.entity.1),
        other => other,
    });

    entities.extend(list);

    Snapshot {
        tick,
        entities,
        ctf: capture_ctf_snapshot_state(world),
    }
}

/// Overwrite local transform state from an authoritative snapshot.
pub fn apply_snapshot(world: &mut World, snapshot: &Snapshot) {
    apply_snapshot_authority(world, snapshot, |_| true);

    if let Some(ctf) = &snapshot.ctf {
        world.insert_resource(GameState {
            phase: ctf.winner.map_or(GamePhase::Playing, GamePhase::Won),
        });
        world.insert_resource(CarrierState {
            red_flag_carrier: ctf.red_flag_carrier,
            blue_flag_carrier: ctf.blue_flag_carrier,
        });
        world.insert_resource(FlagMotionState {
            red: ctf.red_flag_motion.map(restore_motion),
            blue: ctf.blue_flag_motion.map(restore_motion),
        });
        world.insert_resource(AutoMoveState {
            paths: restore_auto_paths(&ctf.auto_paths),
        });
        if let Some(mut controls) = world.resource::<ControlState>().cloned() {
            for assignment in &ctf.selected_slots {
                if let Some(control) = controls
                    .controls
                    .iter_mut()
                    .find(|control| control.client_id == assignment.client_id)
                {
                    control.selected_slot = assignment.primary_slot;
                }
            }
            world.insert_resource(controls);
        }
    }
}

/// Overwrite transform state for a filtered authority subset.
pub fn apply_snapshot_authority<F>(world: &mut World, snapshot: &Snapshot, accepts: F)
where
    F: Fn(NetworkEntityId) -> bool,
{
    let mut target = std::collections::HashMap::<(u32, u32), &EntityStatePacket>::new();
    let mut stable_target = std::collections::HashMap::<StableEntityId, &EntityStatePacket>::new();
    snapshot.entities.iter().for_each(|entity| {
        if accepts(entity.entity) {
            if let Some(stable_id) = entity.stable_id {
                stable_target.insert(stable_id, entity);
            } else {
                target.insert(entity.entity, entity);
            }
        }
    });
    let local_stable_ids = ctf_stable_entity_ids(world);

    let entities: Vec<_> = world
        .query::<Transform>()
        .map(|(entity, _)| entity)
        .collect();

    entities.into_iter().for_each(|entity| {
        let entity_id = (entity.index, entity.generation);
        let state = local_stable_ids
            .get(&entity_id)
            .and_then(|stable_id| stable_target.remove(stable_id))
            .or_else(|| target.remove(&entity_id));
        if let Some(state) = state {
            let transform = Transform {
                position: crate::math::Vec3::new(
                    state.position.0,
                    state.position.1,
                    state.position.2,
                ),
                rotation: state.rotation,
                scale: crate::math::Vec3::new(state.scale.0, state.scale.1, state.scale.2),
            };
            world.insert(entity, transform);
        }
    });
}

fn ctf_stable_entity_ids(world: &World) -> HashMap<NetworkEntityId, StableEntityId> {
    let mut stable_ids = HashMap::new();
    let Some(refs) = world.resource::<EntityRefs>() else {
        return stable_ids;
    };

    for slot in CtfSlot::ALL {
        stable_ids.insert(
            network_entity_id(refs.player(slot)),
            StableEntityId::CtfPlayer(slot),
        );
    }
    stable_ids.insert(network_entity_id(refs.red_flag), StableEntityId::CtfRedFlag);
    stable_ids.insert(
        network_entity_id(refs.blue_flag),
        StableEntityId::CtfBlueFlag,
    );
    stable_ids
}

/// Look up an entity transform inside a snapshot.
pub fn snapshot_transform(snapshot: &Snapshot, entity: Entity) -> Option<Transform> {
    snapshot
        .entities
        .iter()
        .find(|state| state.entity == network_entity_id(entity))
        .map(packet_transform)
}

/// Convert an ECS entity handle into the wire-stable network id.
pub fn network_entity_id(entity: Entity) -> NetworkEntityId {
    (entity.index, entity.generation)
}

fn capture_ctf_snapshot_state(world: &World) -> Option<CtfSnapshotState> {
    let carrier = world.resource::<CarrierState>().copied()?;
    let winner = world
        .resource::<GameState>()
        .and_then(|state| match state.phase {
            GamePhase::Playing => None,
            GamePhase::Won(id) => Some(id),
        });
    let mut selected_slots = world
        .resource::<ControlState>()
        .map(|controls| {
            controls
                .controls
                .iter()
                .map(|control| CtfSlotAssignment {
                    client_id: control.client_id,
                    primary_slot: control.selected_slot,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    selected_slots.sort_by_key(|assignment| assignment.client_id);
    let flag_motion = world
        .resource::<FlagMotionState>()
        .copied()
        .unwrap_or_default();
    let auto_paths = world
        .resource::<AutoMoveState>()
        .map(capture_auto_paths)
        .unwrap_or_default();

    Some(CtfSnapshotState {
        winner,
        red_flag_carrier: carrier.red_flag_carrier,
        blue_flag_carrier: carrier.blue_flag_carrier,
        selected_slots,
        red_flag_motion: flag_motion.red.map(capture_motion),
        blue_flag_motion: flag_motion.blue.map(capture_motion),
        auto_paths,
    })
}

fn capture_motion(motion: FlagMotion) -> CtfFlagMotionSnapshot {
    CtfFlagMotionSnapshot {
        dir_x: motion.dir_x,
        dir_y: motion.dir_y,
        remaining_distance: motion.remaining_distance,
    }
}

fn restore_motion(motion: CtfFlagMotionSnapshot) -> FlagMotion {
    FlagMotion {
        dir_x: motion.dir_x,
        dir_y: motion.dir_y,
        remaining_distance: motion.remaining_distance,
    }
}

fn capture_auto_paths(auto_move: &AutoMoveState) -> Vec<CtfAutoMovePathSnapshot> {
    let mut paths = Vec::new();
    for slot in crate::multiplayer::matchmaking::CtfSlot::ALL {
        if let Some(path) = &auto_move.paths[slot.index()] {
            paths.push(CtfAutoMovePathSnapshot {
                slot,
                waypoints: path.waypoints.clone(),
                next_index: path.next_index,
            });
        }
    }
    paths
}

fn restore_auto_paths(paths: &[CtfAutoMovePathSnapshot]) -> [Option<AutoMovePath>; CtfSlot::COUNT] {
    let mut restored = std::array::from_fn(|_| None);
    for path in paths {
        restored[path.slot.index()] = Some(AutoMovePath {
            waypoints: path.waypoints.clone(),
            next_index: path.next_index,
        });
    }
    restored
}

fn hash_motion_snapshot(hasher: &mut DefaultHasher, motion: Option<CtfFlagMotionSnapshot>) {
    motion.is_some().hash(hasher);
    if let Some(motion) = motion {
        canonicalize(motion.dir_x).to_bits().hash(hasher);
        canonicalize(motion.dir_y).to_bits().hash(hasher);
        canonicalize(motion.remaining_distance)
            .to_bits()
            .hash(hasher);
    }
}

fn hash_ctf_snapshot(hasher: &mut DefaultHasher, state: &CtfSnapshotState) {
    state.winner.hash(hasher);
    state.red_flag_carrier.hash(hasher);
    state.blue_flag_carrier.hash(hasher);
    state.selected_slots.iter().for_each(|assignment| {
        assignment.client_id.hash(hasher);
        assignment.primary_slot.hash(hasher);
    });
    hash_motion_snapshot(hasher, state.red_flag_motion);
    hash_motion_snapshot(hasher, state.blue_flag_motion);
    state.auto_paths.iter().for_each(|path| {
        path.slot.hash(hasher);
        path.next_index.hash(hasher);
        path.waypoints.len().hash(hasher);
        path.waypoints.iter().for_each(|(x, y)| {
            canonicalize(*x).to_bits().hash(hasher);
            canonicalize(*y).to_bits().hash(hasher);
        });
    });
}

fn interpolate_authority_snapshot(
    existing: Option<&Snapshot>,
    anchor: &Snapshot,
    targets: &[EntityStatePacket],
    target_tick: NetworkTick,
    tick: NetworkTick,
) -> Snapshot {
    let mut snapshot = existing.cloned().unwrap_or_else(|| Snapshot {
        tick,
        entities: anchor.entities.clone(),
        ctf: anchor.ctf.clone(),
    });
    snapshot.tick = tick;

    let span = target_tick.saturating_sub(anchor.tick).max(1);
    let step = tick.saturating_sub(anchor.tick).min(span);
    let alpha = step as f32 / span as f32;

    for target in targets {
        let state = anchor
            .entities
            .iter()
            .find(|anchor_state| snapshot_entity_key(anchor_state) == snapshot_entity_key(target))
            .map_or_else(
                || target.clone(),
                |anchor_state| interpolate_entity(anchor_state, target, alpha),
            );
        upsert_entity_state(&mut snapshot, state);
    }

    snapshot
        .entities
        .sort_by(|a, b| match a.entity.0.cmp(&b.entity.0) {
            std::cmp::Ordering::Equal => a.entity.1.cmp(&b.entity.1),
            other => other,
        });
    snapshot
}

fn merge_authority_entities(snapshot: &mut Snapshot, entities: &[EntityStatePacket]) {
    for entity in entities {
        upsert_entity_state(snapshot, entity.clone());
    }
    snapshot
        .entities
        .sort_by(|a, b| match a.entity.0.cmp(&b.entity.0) {
            std::cmp::Ordering::Equal => a.entity.1.cmp(&b.entity.1),
            other => other,
        });
}

fn upsert_entity_state(snapshot: &mut Snapshot, state: EntityStatePacket) {
    if let Some(existing) = snapshot
        .entities
        .iter_mut()
        .find(|existing| snapshot_entity_key(existing) == snapshot_entity_key(&state))
    {
        let local_entity = existing.entity;
        *existing = state;
        if existing.stable_id.is_some() {
            existing.entity = local_entity;
        }
    } else {
        snapshot.entities.push(state);
    }
}

fn interpolate_entity(
    anchor: &EntityStatePacket,
    target: &EntityStatePacket,
    alpha: f32,
) -> EntityStatePacket {
    EntityStatePacket {
        entity: anchor.entity,
        stable_id: target.stable_id.or(anchor.stable_id),
        position: (
            lerp(anchor.position.0, target.position.0, alpha),
            lerp(anchor.position.1, target.position.1, alpha),
            lerp(anchor.position.2, target.position.2, alpha),
        ),
        rotation: lerp(anchor.rotation, target.rotation, alpha),
        scale: (
            lerp(anchor.scale.0, target.scale.0, alpha),
            lerp(anchor.scale.1, target.scale.1, alpha),
            lerp(anchor.scale.2, target.scale.2, alpha),
        ),
    }
}

fn packet_transform(state: &EntityStatePacket) -> Transform {
    Transform {
        position: crate::math::Vec3::new(state.position.0, state.position.1, state.position.2),
        rotation: state.rotation,
        scale: crate::math::Vec3::new(state.scale.0, state.scale.1, state.scale.2),
    }
}

fn lerp(a: f32, b: f32, alpha: f32) -> f32 {
    a + (b - a) * alpha
}

/// Quick delta decision helper for mismatch recovery.
pub fn needs_correction(local: FrameHash, remote: FrameHash) -> bool {
    local != remote
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Transform;
    use crate::ecs::world::World;
    use crate::game::ctf::resources::{
        AutoMovePath, AutoMoveState, CarrierState, ControlState, FlagMotion, FlagMotionState,
        GamePhase, GameState,
    };
    use crate::game::ctf::setup::setup_ctf_scene_entities;
    use crate::math::Vec3;
    use crate::multiplayer::matchmaking::{CtfSlot, CtfSlotAssignment, MapSize};
    use crate::scene::reload_scene;

    #[test]
    fn hashing_is_deterministic_for_the_same_state() {
        let mut a = World::new();
        let e1 = a.spawn();
        a.insert(
            e1,
            Transform {
                position: Vec3::new(1.0, 2.0, 3.0),
                ..Transform::identity()
            },
        );

        let mut b = World::new();
        let e2 = b.spawn();
        b.insert(
            e2,
            Transform {
                position: Vec3::new(1.0, 2.0, 3.0),
                ..Transform::identity()
            },
        );

        let a_hash = state_hash(&a, 1);
        let b_hash = state_hash(&b, 1);
        assert_eq!(a_hash, b_hash);
    }

    #[test]
    fn snapshot_hash_matches_world_hash() {
        let mut world = World::new();
        let entity = world.spawn();
        world.insert(
            entity,
            Transform {
                position: Vec3::new(4.0, 5.0, 6.0),
                rotation: 0.25,
                scale: Vec3::new(2.0, 3.0, 4.0),
            },
        );

        let snapshot = capture_snapshot(&world, 42);

        assert_eq!(snapshot_hash(&snapshot), state_hash(&world, 42));
    }

    #[test]
    fn interpolation_buffer_displays_three_frames_behind_latest() {
        let mut buffer = FrameInterpolationBuffer::default();

        for tick in 7..=10 {
            buffer.push_snapshot(test_snapshot(tick, tick as f32));
        }

        assert_eq!(buffer.display_snapshot().expect("display frame").tick, 7);
        assert_eq!(buffer.latest_snapshot().expect("latest frame").tick, 10);
    }

    #[test]
    fn reconcile_rebuilds_buffered_frames_between_display_and_authority() {
        let mut buffer = FrameInterpolationBuffer::default();

        for tick in 7..=10 {
            buffer.push_snapshot(test_snapshot(tick, 0.0));
        }

        buffer.reconcile(test_snapshot(10, 30.0));

        let positions = buffer
            .snapshots()
            .map(|snapshot| (snapshot.tick, snapshot.entities[0].position.0))
            .collect::<Vec<_>>();

        assert_eq!(positions, vec![(7, 0.0), (8, 10.0), (9, 20.0), (10, 30.0)]);
    }

    #[test]
    fn reconcile_authority_preserves_unowned_entities() {
        let mut buffer = FrameInterpolationBuffer::default();

        for tick in 7..=10 {
            buffer.push_snapshot(test_dual_snapshot(tick, 100.0, 0.0));
        }

        buffer.reconcile_authority(test_dual_snapshot(10, 999.0, 30.0), |entity| {
            entity == (2, 0)
        });

        let final_frame = buffer.snapshot_for_tick(10).expect("corrected frame");
        let local = final_frame
            .entities
            .iter()
            .find(|entity| entity.entity == (1, 0))
            .expect("local entity");
        let remote = final_frame
            .entities
            .iter()
            .find(|entity| entity.entity == (2, 0))
            .expect("remote entity");

        assert_eq!(local.position.0, 100.0);
        assert_eq!(remote.position.0, 30.0);
    }

    #[test]
    fn ctf_snapshot_round_trips_resource_state() {
        let mut world = World::new();
        reload_scene(&mut world, "assets/ctf_arena_small.json").expect("load CTF scene");
        setup_ctf_scene_entities(
            &mut world,
            &[
                CtfSlotAssignment {
                    client_id: 1,
                    primary_slot: CtfSlot::Red1,
                },
                CtfSlotAssignment {
                    client_id: 2,
                    primary_slot: CtfSlot::Blue1,
                },
            ],
            MapSize::Small,
        );

        world.insert_resource(GameState {
            phase: GamePhase::Won(1),
        });
        world.insert_resource(CarrierState {
            red_flag_carrier: Some(CtfSlot::Blue1),
            blue_flag_carrier: Some(CtfSlot::Red2),
        });
        world.insert_resource(FlagMotionState {
            red: Some(FlagMotion {
                dir_x: 1.0,
                dir_y: 0.0,
                remaining_distance: 120.0,
            }),
            blue: None,
        });
        let mut paths = std::array::from_fn(|_| None);
        paths[CtfSlot::Red1.index()] = Some(AutoMovePath {
            waypoints: vec![(220.0, 320.0), (280.0, 360.0)],
            next_index: 1,
        });
        world.insert_resource(AutoMoveState { paths });
        if let Some(mut controls) = world.resource::<ControlState>().cloned() {
            controls.controls[0].selected_slot = CtfSlot::Red2;
            world.insert_resource(controls);
        }

        let snapshot = capture_snapshot(&world, 99);

        world.insert_resource(GameState::default());
        world.insert_resource(CarrierState::default());
        world.insert_resource(FlagMotionState::default());
        world.insert_resource(AutoMoveState::default());
        if let Some(mut controls) = world.resource::<ControlState>().cloned() {
            controls.controls[0].selected_slot = CtfSlot::Red1;
            world.insert_resource(controls);
        }

        apply_snapshot(&mut world, &snapshot);

        assert!(matches!(
            world.resource::<GameState>().expect("game state").phase,
            GamePhase::Won(1)
        ));
        let carrier = world.resource::<CarrierState>().expect("carrier");
        assert_eq!(carrier.red_flag_carrier, Some(CtfSlot::Blue1));
        assert_eq!(carrier.blue_flag_carrier, Some(CtfSlot::Red2));
        let motion = world.resource::<FlagMotionState>().expect("flag motion");
        assert_eq!(
            motion.red,
            Some(FlagMotion {
                dir_x: 1.0,
                dir_y: 0.0,
                remaining_distance: 120.0,
            })
        );
        let auto_move = world.resource::<AutoMoveState>().expect("auto move");
        assert_eq!(
            auto_move.paths[CtfSlot::Red1.index()],
            Some(AutoMovePath {
                waypoints: vec![(220.0, 320.0), (280.0, 360.0)],
                next_index: 1,
            })
        );
        let controls = world.resource::<ControlState>().expect("controls");
        assert_eq!(controls.controls[0].selected_slot, CtfSlot::Red2);
    }

    #[test]
    fn stable_entity_ids_apply_across_different_ecs_ids() {
        let mut world = World::new();
        let players = std::array::from_fn(|_| world.spawn());
        let red_flag = world.spawn();
        let blue_flag = world.spawn();
        world.insert(players[CtfSlot::Red1.index()], Transform::identity());
        world.insert_resource(EntityRefs {
            players,
            player_spawns: [(0.0, 0.0); CtfSlot::COUNT],
            red_flag,
            blue_flag,
            red_flag_spawn: (0.0, 0.0),
            blue_flag_spawn: (0.0, 0.0),
        });

        let snapshot = Snapshot {
            tick: 7,
            entities: vec![EntityStatePacket {
                entity: (999, 0),
                stable_id: Some(StableEntityId::CtfPlayer(CtfSlot::Red1)),
                position: (42.0, 0.0, 0.0),
                rotation: 0.0,
                scale: (1.0, 1.0, 1.0),
            }],
            ctf: None,
        };

        apply_snapshot_authority(&mut world, &snapshot, |_| true);
        let transform = world
            .get::<Transform>(players[CtfSlot::Red1.index()])
            .expect("red1 transform");
        assert_eq!(transform.position.x, 42.0);
    }

    #[test]
    fn interpolation_buffer_reconciles_stable_ids_without_remote_id_leak() {
        let mut buffer = FrameInterpolationBuffer::new(0);
        buffer.push_snapshot(Snapshot {
            tick: 7,
            entities: vec![EntityStatePacket {
                entity: (1, 0),
                stable_id: Some(StableEntityId::CtfPlayer(CtfSlot::Red1)),
                position: (0.0, 0.0, 0.0),
                rotation: 0.0,
                scale: (1.0, 1.0, 1.0),
            }],
            ctf: None,
        });

        buffer.reconcile_authority(
            Snapshot {
                tick: 7,
                entities: vec![EntityStatePacket {
                    entity: (999, 0),
                    stable_id: Some(StableEntityId::CtfPlayer(CtfSlot::Red1)),
                    position: (42.0, 0.0, 0.0),
                    rotation: 0.0,
                    scale: (1.0, 1.0, 1.0),
                }],
                ctf: None,
            },
            |_| true,
        );

        let snapshot = buffer.snapshot_for_tick(7).expect("corrected snapshot");
        assert_eq!(snapshot.entities.len(), 1);
        assert_eq!(snapshot.entities[0].entity, (1, 0));
        assert_eq!(snapshot.entities[0].position.0, 42.0);
    }

    fn test_snapshot(tick: NetworkTick, x: f32) -> Snapshot {
        Snapshot {
            tick,
            entities: vec![EntityStatePacket {
                entity: (1, 0),
                stable_id: None,
                position: (x, 0.0, 0.0),
                rotation: 0.0,
                scale: (1.0, 1.0, 1.0),
            }],
            ctf: None,
        }
    }

    fn test_dual_snapshot(tick: NetworkTick, local_x: f32, remote_x: f32) -> Snapshot {
        Snapshot {
            tick,
            entities: vec![
                EntityStatePacket {
                    entity: (1, 0),
                    stable_id: None,
                    position: (local_x, 0.0, 0.0),
                    rotation: 0.0,
                    scale: (1.0, 1.0, 1.0),
                },
                EntityStatePacket {
                    entity: (2, 0),
                    stable_id: None,
                    position: (remote_x, 0.0, 0.0),
                    rotation: 0.0,
                    scale: (1.0, 1.0, 1.0),
                },
            ],
            ctf: None,
        }
    }
}
