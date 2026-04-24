//! Scene-backed CTF setup helpers.

use crate::components::{Camera, Shape, Tag, Transform, Velocity};
use crate::ecs::entity::Entity;
use crate::ecs::world::World;
use crate::game::ctf::components::{Flag, PlayerMarker, Wall};
use crate::game::ctf::resources::{
    CarrierState, ControlState, CtfInputState, CtfSyncState, EntityRefs, GameState, PlayerControl,
};
use crate::multiplayer::matchmaking::{CtfSlot, CtfSlotAssignment};

/// Attach CTF runtime metadata to scene-authored CTF entities.
pub fn setup_ctf_scene_entities(
    world: &mut World,
    assignments: &[CtfSlotAssignment],
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
    require_saved_camera(world);

    let refs = EntityRefs {
        players,
        player_spawns,
        red_flag,
        blue_flag,
        red_flag_spawn,
        blue_flag_spawn,
    };

    world.insert_resource(GameState::default());
    world.insert_resource(CarrierState::default());
    world.insert_resource(build_control_state(assignments));
    world.insert_resource(CtfInputState::default());
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
                vec![assignment.primary_slot]
            };
            PlayerControl {
                client_id: assignment.client_id,
                primary_slot: assignment.primary_slot,
                selected_slot: assignment.primary_slot,
                owned_slots,
                tag_down: false,
                switch_down: false,
            }
        })
        .collect();

    ControlState { controls }
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
        reload_scene(&mut world, "assets/ctf_scene.json").expect("load CTF scene");
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
        );

        assert_eq!(world.query::<PlayerMarker>().count(), 4);
        assert_eq!(world.query::<Flag>().count(), 2);
        assert_eq!(world.query::<Wall>().count(), 16);
        assert_eq!(world.query::<Camera>().count(), 1);
    }

    #[test]
    fn single_human_team_controls_both_team_slots() {
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
                primary_slot: CtfSlot::Blue2,
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
        assert_eq!(red.owned_slots, vec![CtfSlot::Red1, CtfSlot::Red2]);
        assert_eq!(blue.owned_slots, vec![CtfSlot::Blue1]);
    }
}
