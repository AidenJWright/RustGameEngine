//! Rollback and correction helpers for host-authoritative multiplayer.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::components::{Camera, Transform};
use crate::ecs::world::World;
use crate::game::ctf::resources::{
    AutoMovePath, AutoMoveState, CarrierState, ControlState, FlagMotion, FlagMotionState,
    GamePhase, GameState,
};
use crate::multiplayer::matchmaking::{CtfSlot, CtfSlotAssignment};

use super::net_types::{
    CtfAutoMovePathSnapshot, CtfFlagMotionSnapshot, CtfSnapshotState, EntityStatePacket,
};
use super::net_types::{NetworkTick, Snapshot};

/// A compact deterministic state hash used for divergence checks.
pub type FrameHash = u64;

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
    let mut transforms: Vec<_> = world
        .query::<Transform>()
        .filter(|(entity, _)| world.get::<Camera>(*entity).is_none())
        .map(|(entity, transform)| {
            (
                entity,
                transform.position.x,
                transform.position.y,
                transform.position.z,
                transform.rotation,
                transform.scale.x,
                transform.scale.y,
                transform.scale.z,
            )
        })
        .collect();

    transforms.sort_by(|(a, ..), (b, ..)| match a.index.cmp(&b.index) {
        std::cmp::Ordering::Equal => a.generation.cmp(&b.generation),
        other => other,
    });

    let mut hasher = DefaultHasher::new();
    transforms
        .into_iter()
        .for_each(|(entity, x, y, z, rot, sx, sy, sz)| {
            entity.index.hash(&mut hasher);
            entity.generation.hash(&mut hasher);
            canonicalize(x).to_bits().hash(&mut hasher);
            canonicalize(y).to_bits().hash(&mut hasher);
            canonicalize(z).to_bits().hash(&mut hasher);
            canonicalize(rot).to_bits().hash(&mut hasher);
            canonicalize(sx).to_bits().hash(&mut hasher);
            canonicalize(sy).to_bits().hash(&mut hasher);
            canonicalize(sz).to_bits().hash(&mut hasher);
        });

    if let Some(state) = capture_ctf_snapshot_state(world) {
        state.winner.hash(&mut hasher);
        state.red_flag_carrier.hash(&mut hasher);
        state.blue_flag_carrier.hash(&mut hasher);
        state
            .selected_slots
            .iter()
            .for_each(|assignment| {
                assignment.client_id.hash(&mut hasher);
                assignment.primary_slot.hash(&mut hasher);
            });
        hash_motion_snapshot(&mut hasher, state.red_flag_motion);
        hash_motion_snapshot(&mut hasher, state.blue_flag_motion);
        state.auto_paths.iter().for_each(|path| {
            path.slot.hash(&mut hasher);
            path.next_index.hash(&mut hasher);
            path.waypoints.len().hash(&mut hasher);
            path.waypoints.iter().for_each(|(x, y)| {
                canonicalize(*x).to_bits().hash(&mut hasher);
                canonicalize(*y).to_bits().hash(&mut hasher);
            });
        });
    }

    hasher.finish()
}

/// Build a full transform snapshot from authoritative state.
///
/// Camera entities are excluded from snapshots — each player manages their
/// own camera independently.
pub fn capture_snapshot(world: &World, tick: NetworkTick) -> Snapshot {
    let mut entities = Vec::new();

    let mut list: Vec<_> = world
        .query::<Transform>()
        .filter(|(entity, _)| world.get::<Camera>(*entity).is_none())
        .map(|(entity, transform)| EntityStatePacket::from((entity, transform)))
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
    let mut target = std::collections::HashMap::<(u32, u32), &EntityStatePacket>::new();
    snapshot.entities.iter().for_each(|entity| {
        target.insert(entity.entity, entity);
    });

    let entities: Vec<_> = world
        .query::<Transform>()
        .map(|(entity, _)| entity)
        .collect();

    entities.into_iter().for_each(|entity| {
        if let Some(state) = target.remove(&(entity.index, entity.generation)) {
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

    if let Some(ctf) = &snapshot.ctf {
        world.insert_resource(GameState {
            phase: ctf
                .winner
                .map_or(GamePhase::Playing, GamePhase::Won),
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
    let flag_motion = world.resource::<FlagMotionState>().copied().unwrap_or_default();
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

fn restore_auto_paths(
    paths: &[CtfAutoMovePathSnapshot],
) -> [Option<AutoMovePath>; CtfSlot::COUNT] {
    let mut restored = std::array::from_fn(|_| None);
    for path in paths {
        restored[path.slot.index()] = Some(AutoMovePath {
            waypoints: path.waypoints.clone(),
            next_index: path.next_index,
        });
    }
    restored
}

fn hash_motion_snapshot(
    hasher: &mut DefaultHasher,
    motion: Option<CtfFlagMotionSnapshot>,
) {
    motion.is_some().hash(hasher);
    if let Some(motion) = motion {
        canonicalize(motion.dir_x).to_bits().hash(hasher);
        canonicalize(motion.dir_y).to_bits().hash(hasher);
        canonicalize(motion.remaining_distance).to_bits().hash(hasher);
    }
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
}
