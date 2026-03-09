//! Rollback and correction helpers for host-authoritative multiplayer.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use crate::components::Transform;
use crate::ecs::world::World;

use super::net_types::{NetworkTick, Snapshot};
use super::net_types::EntityStatePacket;

/// A compact deterministic state hash used for divergence checks.
pub type FrameHash = u64;

/// Deterministically hash all `Transform` component data in a world.
pub fn state_hash(world: &World, _tick: NetworkTick) -> FrameHash {
    let mut transforms: Vec<_> = world
        .query::<Transform>()
        .map(|(entity, transform)| (entity, transform.position.x, transform.position.y, transform.position.z, transform.rotation, transform.scale.x, transform.scale.y, transform.scale.z))
        .collect();

    transforms.sort_by(|(a, ..), (b, ..)| {
        match a.index.cmp(&b.index) {
            std::cmp::Ordering::Equal => a.generation.cmp(&b.generation),
            other => other,
        }
    });

    let mut hasher = DefaultHasher::new();
    transforms.into_iter().for_each(|(entity, x, y, z, rot, sx, sy, sz)| {
        entity.index.hash(&mut hasher);
        entity.generation.hash(&mut hasher);
        x.to_bits().hash(&mut hasher);
        y.to_bits().hash(&mut hasher);
        z.to_bits().hash(&mut hasher);
        rot.to_bits().hash(&mut hasher);
        sx.to_bits().hash(&mut hasher);
        sy.to_bits().hash(&mut hasher);
        sz.to_bits().hash(&mut hasher);
    });

    hasher.finish()
}

/// Build a full transform snapshot from authoritative state.
pub fn capture_snapshot(world: &World, tick: NetworkTick) -> Snapshot {
    let mut entities = Vec::new();

    let mut list: Vec<_> = world
        .query::<Transform>()
        .map(|(entity, transform)| EntityStatePacket::from((entity, transform)))
        .collect();

    list.sort_by(|a, b| {
        match a.entity.0.cmp(&b.entity.0) {
            std::cmp::Ordering::Equal => a.entity.1.cmp(&b.entity.1),
            other => other,
        }
    });

    entities.extend(list);

    Snapshot { tick, entities }
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

    entities
        .into_iter()
        .for_each(|entity| {
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
    use crate::math::Vec3;

    #[test]
    fn hashing_is_deterministic_for_the_same_state() {
        let mut a = World::new();
        let e1 = a.spawn();
        a.insert(e1, Transform { position: Vec3::new(1.0, 2.0, 3.0), ..Transform::identity() });

        let mut b = World::new();
        let e2 = b.spawn();
        b.insert(e2, Transform { position: Vec3::new(1.0, 2.0, 3.0), ..Transform::identity() });

        let a_hash = state_hash(&a, 1);
        let b_hash = state_hash(&b, 1);
        assert_eq!(a_hash, b_hash);
    }
}
