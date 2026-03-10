//! Scene serialisation — save/load the ECS world to/from JSON.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::components::{Color, Health, Shape, Tag, Transform, Velocity};
use crate::ecs::entity::Entity;
use crate::ecs::world::World;
use crate::systems::sinusoid::SinusoidComponent;

// ---------------------------------------------------------------------------
// Entity ID encoding
// ---------------------------------------------------------------------------

fn encode(e: Entity) -> u64 {
    ((e.index as u64) << 32) | (e.generation as u64)
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// All component state for one entity, with every field optional.
///
/// During save, fields are `None` when the entity doesn't have that component.
/// During load, `None` fields are simply not inserted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EntityData {
    /// Stable reference ID (index << 32 | generation) — used only for
    /// parent/child reconstruction; runtime IDs will differ after load.
    pub id: u64,
    /// Encoded ID of the parent entity, or `None` for roots.
    pub parent: Option<u64>,
    pub tag: Option<Tag>,
    pub transform: Option<Transform>,
    pub color: Option<Color>,
    pub shape: Option<Shape>,
    pub velocity: Option<Velocity>,
    pub health: Option<Health>,
    pub sinusoid: Option<SinusoidComponent>,
}

/// Top-level scene file format.
#[derive(Debug, Serialize, Deserialize)]
pub struct SceneData {
    pub version: u32,
    pub entities: Vec<EntityData>,
}

// ---------------------------------------------------------------------------
// Save
// ---------------------------------------------------------------------------

/// Serialise the entire scene (all entities reachable from root entities in
/// the scene tree) to a pretty-printed JSON file at `path`.
///
/// Entities are written in depth-first order so that parents always appear
/// before their children, which makes loading safe with a single pass.
pub fn save_scene(world: &World, path: &str) -> std::io::Result<()> {
    let mut entities = Vec::new();

    let roots: Vec<Entity> = world.scene_tree().root_entities().collect();
    for root in roots {
        world.scene_tree().walk_depth_first(root, |entity, _| {
            let parent = world.scene_tree().parent(entity).map(encode);
            entities.push(EntityData {
                id: encode(entity),
                parent,
                tag: world.get::<Tag>(entity).cloned(),
                transform: world.get::<Transform>(entity).cloned(),
                color: world.get::<Color>(entity).cloned(),
                shape: world.get::<Shape>(entity).cloned(),
                velocity: world.get::<Velocity>(entity).cloned(),
                health: world.get::<Health>(entity).cloned(),
                sinusoid: world.get::<SinusoidComponent>(entity).cloned(),
            });
        });
    }

    let scene = SceneData {
        version: 1,
        entities,
    };
    let json = serde_json::to_string_pretty(&scene)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(path, json)
}

// ---------------------------------------------------------------------------
// Load
// ---------------------------------------------------------------------------

/// Deserialise a scene JSON file and spawn its entities into `world`.
///
/// Entities are loaded additively (existing entities are not cleared).
/// The saved ID → new [`Entity`] mapping is built as we go; since the file
/// is written depth-first, a parent is always encountered before its children.
pub fn load_scene(world: &mut World, path: &str) -> std::io::Result<()> {
    let json = std::fs::read_to_string(path)?;
    let scene: SceneData = serde_json::from_str(&json)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;

    let mut id_map: HashMap<u64, Entity> = HashMap::new();

    for data in &scene.entities {
        let entity = match data.parent {
            None => world.spawn(),
            Some(pid) => {
                let parent = *id_map.get(&pid).ok_or_else(|| {
                    std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("scene: parent id {pid} not yet spawned"),
                    )
                })?;
                world.spawn_child(parent)
            }
        };
        id_map.insert(data.id, entity);

        if let Some(c) = data.tag.clone() {
            world.insert(entity, c);
        }
        if let Some(c) = data.transform.clone() {
            world.insert(entity, c);
        }
        if let Some(c) = data.color.clone() {
            world.insert(entity, c);
        }
        if let Some(c) = data.shape.clone() {
            world.insert(entity, c);
        }
        if let Some(c) = data.velocity.clone() {
            world.insert(entity, c);
        }
        if let Some(c) = data.health.clone() {
            world.insert(entity, c);
        }
        if let Some(c) = data.sinusoid.clone() {
            world.insert(entity, c);
        }
    }

    Ok(())
}
