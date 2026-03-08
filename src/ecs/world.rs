//! The central container — owns all entities, components, resources, and the scene tree.

use super::component::{Component, ComponentRegistry};
use super::entity::{Entity, EntityAllocator};
use super::resource::{Resource, Resources};
use super::scene_tree::SceneTree;

/// Owns all ECS state: entities, components, resources, and the scene hierarchy.
///
/// The `World` is the **only** place that stores live data. Systems receive a
/// reference to it and express mutations through a [`super::command_buffer::CommandBuffer`].
#[derive(Default)]
pub struct World {
    pub(crate) allocator: EntityAllocator,
    pub(crate) registry: ComponentRegistry,
    pub(crate) resources: Resources,
    pub(crate) scene_tree: SceneTree,
}

impl World {
    /// Create an empty world.
    pub fn new() -> Self {
        Self::default()
    }

    // -----------------------------------------------------------------------
    // Entity lifecycle
    // -----------------------------------------------------------------------

    /// Spawn a new root entity (no parent in the scene tree).
    pub fn spawn(&mut self) -> Entity {
        let entity = self.allocator.allocate();
        self.scene_tree.register_root(entity);
        entity
    }

    /// Spawn a new entity as a child of `parent` in the scene tree.
    ///
    /// Panics if `parent` is not alive.
    pub fn spawn_child(&mut self, parent: Entity) -> Entity {
        assert!(
            self.allocator.is_alive(parent),
            "spawn_child: parent {parent} is not alive"
        );
        let child = self.allocator.allocate();
        self.scene_tree.register_root(child);
        self.scene_tree.attach(child, parent);
        child
    }

    /// Despawn `entity` and **recursively** all its children.
    ///
    /// Silently ignores already-dead entities.
    pub fn despawn(&mut self, entity: Entity) {
        if !self.allocator.is_alive(entity) {
            return;
        }
        // Collect the entire subtree first (depth-first) so we have a
        // stable list even as we remove tree entries.
        let subtree = self.scene_tree.collect_subtree(entity);
        subtree.iter().for_each(|&e| {
            self.registry.remove_all_for(e);
            self.scene_tree.remove_entity(e);
            self.allocator.free(e);
        });
    }

    // -----------------------------------------------------------------------
    // Component access
    // -----------------------------------------------------------------------

    /// Attach component `T` to `entity`.
    pub fn insert<T: Component>(&mut self, entity: Entity, component: T) {
        self.registry.storage::<T>().insert(entity, component);
    }

    /// Remove component `T` from `entity`. Returns `true` if it was present.
    pub fn remove<T: Component>(&mut self, entity: Entity) -> bool {
        self.registry.storage::<T>().remove(entity)
    }

    /// Immutable reference to component `T` on `entity`.
    pub fn get<T: Component>(&self, entity: Entity) -> Option<&T> {
        self.registry.storage_ref::<T>()?.get(entity)
    }

    /// Mutable reference to component `T` on `entity`.
    pub fn get_mut<T: Component>(&mut self, entity: Entity) -> Option<&mut T> {
        self.registry.storage::<T>().get_mut(entity)
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    /// Iterate all `(Entity, &T)` pairs that have component `T`.
    pub fn query<T: Component>(&self) -> impl Iterator<Item = (Entity, &T)> {
        self.registry
            .storage_ref::<T>()
            .into_iter()
            .flat_map(|s| s.iter())
    }

    /// Iterate all `(Entity, &mut T)` pairs that have component `T`.
    pub fn query_mut<T: Component>(&mut self) -> impl Iterator<Item = (Entity, &mut T)> {
        self.registry.storage::<T>().iter_mut()
    }

    /// Iterate entities that have **both** `A` and `B`.
    ///
    /// Implemented by collecting the smaller entity set then filtering by presence
    /// in the other store — straightforward intersection without extra allocations
    /// when the sets are small.
    pub fn query2<A: Component, B: Component>(
        &self,
    ) -> impl Iterator<Item = (Entity, &A, &B)> {
        // Collect entity set for A, then filter to those that also have B.
        let a_store = self.registry.storage_ref::<A>();
        let b_store = self.registry.storage_ref::<B>();
        match (a_store, b_store) {
            (Some(a), Some(b)) => {
                // Safety: we hold shared refs to two *different* storages;
                // they are distinct TypeId keys in the registry HashMap.
                let pairs: Vec<(Entity, &A, &B)> = a
                    .iter()
                    .filter_map(|(e, a_comp)| b.get(e).map(|b_comp| (e, a_comp, b_comp)))
                    .collect();
                pairs.into_iter()
            }
            _ => Vec::new().into_iter(),
        }
    }

    /// Iterate entities that have **all three** of `A`, `B`, and `C`.
    pub fn query3<A: Component, B: Component, C: Component>(
        &self,
    ) -> impl Iterator<Item = (Entity, &A, &B, &C)> {
        let a_store = self.registry.storage_ref::<A>();
        let b_store = self.registry.storage_ref::<B>();
        let c_store = self.registry.storage_ref::<C>();
        match (a_store, b_store, c_store) {
            (Some(a), Some(b), Some(c)) => {
                let triples: Vec<(Entity, &A, &B, &C)> = a
                    .iter()
                    .filter_map(|(e, a_comp)| {
                        b.get(e)
                            .and_then(|b_comp| c.get(e).map(|c_comp| (e, a_comp, b_comp, c_comp)))
                    })
                    .collect();
                triples.into_iter()
            }
            _ => Vec::new().into_iter(),
        }
    }

    // -----------------------------------------------------------------------
    // Resources
    // -----------------------------------------------------------------------

    /// Insert or replace a global resource of type `T`.
    pub fn insert_resource<T: Resource>(&mut self, value: T) {
        self.resources.insert(value);
    }

    /// Get an immutable reference to resource `T`.
    pub fn resource<T: Resource>(&self) -> Option<&T> {
        self.resources.get::<T>()
    }

    /// Get a mutable reference to resource `T`.
    pub fn resource_mut<T: Resource>(&mut self) -> Option<&mut T> {
        self.resources.get_mut::<T>()
    }

    // -----------------------------------------------------------------------
    // Scene tree
    // -----------------------------------------------------------------------

    /// Read access to the scene hierarchy.
    pub fn scene_tree(&self) -> &SceneTree {
        &self.scene_tree
    }

    /// Mutable access (used internally and by demo setup).
    pub fn scene_tree_mut(&mut self) -> &mut SceneTree {
        &mut self.scene_tree
    }
}
