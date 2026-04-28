//! Scene-backed CTF setup helpers.

use crate::components::{Camera, Shape, Tag, Transform, Velocity};
use crate::ecs::entity::Entity;
use crate::ecs::world::World;
use crate::game::ctf::components::{Flag, PlayerMarker, TeamRestrictedZone, Wall};
use crate::game::ctf::nav::build_navigation_grid;
use crate::game::ctf::resources::{
    AutoMoveState, CarrierState, ControlState, CtfInputState, CtfPointerState, CtfRestartState,
    CtfSyncState, EntityRefs, FlagMotionState, GameState, PlayerControl,
};
use crate::multiplayer::matchmaking::{CtfSlot, CtfSlotAssignment, MapSize};

/// Attach CTF runtime metadata to scene-authored CTF entities.
pub fn setup_ctf_scene_entities(
    world: &mut World,
    assignments: &[CtfSlotAssignment],
    map_size: MapSize,
) -> EntityRefs {
    let red_flag = find_tag(world, "ctf_flag_red");
    let blue_flag = find_tag(world, "ctf_flag_blue");
    let players = std::array::from_fn(|idx| find_tag(world, CtfSlot::ALL[idx].tag()));
    let player_spawns = std::array::from_fn(|idx| transform_xy(world, players[idx]));
    let red_flag_spawn = transform_xy(world, red_flag);
    let blue_flag_spawn = transform_xy(world, blue_flag);

    world.insert(red_flag, Flag { owner_id: 1 });
    world.insert(blue_flag, Flag { owner_id: 2 });

    for slot in CtfSlot::ALL {
        let entity = players[slot.index()];
        world.insert(entity, PlayerMarker { slot });
        if world.get::<Velocity>(entity).is_none() {
            world.insert(entity, Velocity { dx: 0.0, dy: 0.0 });
        }
    }

    attach_wall_components(world);
    attach_team_restricted_zones(world);
    require_saved_camera(world);

    let refs = EntityRefs {
        players,
        player_spawns,
        red_flag,
        blue_flag,
        red_flag_spawn,
        blue_flag_spawn,
    };

    let (arena_w, arena_h) = map_size.dimensions();
    world.insert_resource(GameState::default());
    world.insert_resource(CarrierState::default());
    world.insert_resource(FlagMotionState::default());
    world.insert_resource(AutoMoveState::default());
    world.insert_resource(build_navigation_grid(&wall_rects(world), arena_w, arena_h));
    world.insert_resource(map_size);
    world.insert_resource(CtfRestartState {
        selected_map_size: map_size,
        pending_reload: None,
    });
    world.insert_resource(build_control_state(assignments));
    world.insert_resource(CtfInputState::default());
    world.insert_resource(CtfPointerState::default());
    world.insert_resource(CtfSyncState::default());
    world.insert_resource(refs.clone());
    refs
}

/// Find the saved CTF camera entity.
pub fn ctf_camera_entity(world: &World) -> Entity {
    find_tag(world, "ctf_camera")
}

fn build_control_state(assignments: &[CtfSlotAssignment]) -> ControlState {
    let red_count = assignments
        .iter()
        .filter(|assignment| assignment.primary_slot.is_red())
        .count();
    let blue_count = assignments.len().saturating_sub(red_count);

    let controls = assignments
        .iter()
        .map(|assignment| {
            let team_count = if assignment.primary_slot.is_red() {
                red_count
            } else {
                blue_count
            };
            let owned_slots = if team_count == 1 {
                assignment.primary_slot.teammate_slots().to_vec()
            } else {
                assignment.primary_slot.control_pair().to_vec()
            };
            PlayerControl {
                client_id: assignment.client_id,
                primary_slot: assignment.primary_slot,
                selected_slot: assignment.primary_slot,
                owned_slots,
                tag_down: false,
                switch_down: false,
                restart_down: false,
            }
        })
        .collect();

    ControlState { controls }
}

fn attach_team_restricted_zones(world: &mut World) {
    let zone_entities: Vec<(Entity, u8)> = world
        .query::<Tag>()
        .filter_map(|(entity, tag)| match tag.as_str() {
            "ctf_restricted_red_zone" => Some((entity, 1)),
            "ctf_restricted_blue_zone" => Some((entity, 2)),
            _ => None,
        })
        .collect();

    for (entity, team_id) in zone_entities {
        if let Some(Shape::Rect { width, height }) = world.get::<Shape>(entity).cloned() {
            world.insert(
                entity,
                TeamRestrictedZone {
                    team_id,
                    w: width * 0.5,
                    h: height * 0.5,
                },
            );
        }
    }
}

fn attach_wall_components(world: &mut World) {
    let wall_entities: Vec<Entity> = world
        .query::<Tag>()
        .filter(|(_, tag)| tag.as_str() == "wall")
        .map(|(entity, _)| entity)
        .collect();

    for entity in wall_entities {
        if let Some(Shape::Rect { width, height }) = world.get::<Shape>(entity).cloned() {
            world.insert(
                entity,
                Wall {
                    w: width * 0.5,
                    h: height * 0.5,
                },
            );
        }
    }
}

fn wall_rects(world: &World) -> Vec<(f32, f32, f32, f32)> {
    world
        .query2::<Transform, Wall>()
        .map(|(_, transform, wall)| (transform.position.x, transform.position.y, wall.w, wall.h))
        .collect()
}

fn require_saved_camera(world: &World) {
    let camera = ctf_camera_entity(world);
    assert!(
        world.get::<Camera>(camera).is_some(),
        "ctf_camera must have a Camera component in the saved scene"
    );
}

fn find_tag(world: &World, needle: &str) -> Entity {
    world
        .query::<Tag>()
        .find(|(_, tag)| tag.as_str() == needle)
        .map(|(entity, _)| entity)
        .unwrap_or_else(|| panic!("missing required CTF scene tag `{needle}`"))
}

fn transform_xy(world: &World, entity: Entity) -> (f32, f32) {
    let transform = world
        .get::<Transform>(entity)
        .unwrap_or_else(|| panic!("entity {entity} is missing Transform"));
    (transform.position.x, transform.position.y)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scene::reload_scene;

    #[test]
    fn resolves_scene_authored_ctf_entities() {
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

        assert_eq!(world.query::<PlayerMarker>().count(), 8);
        assert_eq!(world.query::<Flag>().count(), 2);
        assert_eq!(world.query::<Wall>().count(), 16);
        assert_eq!(world.query::<Camera>().count(), 1);
    }

    #[test]
    fn single_human_team_controls_all_team_slots() {
        let controls = build_control_state(&[
            CtfSlotAssignment {
                client_id: 1,
                primary_slot: CtfSlot::Red1,
            },
            CtfSlotAssignment {
                client_id: 2,
                primary_slot: CtfSlot::Blue1,
            },
            CtfSlotAssignment {
                client_id: 3,
                primary_slot: CtfSlot::Blue3,
            },
        ]);

        let red = controls
            .controls
            .iter()
            .find(|control| control.client_id == 1)
            .expect("red control");
        let blue = controls
            .controls
            .iter()
            .find(|control| control.client_id == 2)
            .expect("blue control");
        assert_eq!(
            red.owned_slots,
            vec![CtfSlot::Red1, CtfSlot::Red2, CtfSlot::Red3, CtfSlot::Red4]
        );
        assert_eq!(blue.owned_slots, vec![CtfSlot::Blue1, CtfSlot::Blue2]);
    }
}
