//! Typed component storage and a type-erased registry.
//!
//! **Storage choice**: `Vec`-based sparse set keyed on `Entity::index`.
//! - O(1) insert/remove/lookup at the cost of memory proportional to the
//!   highest entity index (sparse). For dense worlds this is ideal; for worlds
//!   with many short-lived entities a true sparse-set (dense array + sparse
//!   index map) would be more cache-friendly for iteration.

use std::any::{Any, TypeId};
use std::collections::HashMap;

use super::entity::Entity;

// ---------------------------------------------------------------------------
// Marker trait
// ---------------------------------------------------------------------------

/// Marker trait that all component types must implement.
///
/// Components are **plain data** — no methods, no logic.
/// Implement via `#[derive(Debug, Clone)]` + `impl Component for Foo {}`.
pub trait Component: Any + Send + Sync + std::fmt::Debug + Clone + 'static {}

// ---------------------------------------------------------------------------
// Type-erased storage trait
// ---------------------------------------------------------------------------

/// Object-safe interface over a concrete [`ComponentStorage<T>`].
///
/// Used to store heterogeneous component storages in a single `HashMap`.
pub trait AnyComponentStorage: Any + Send + Sync {
    /// Remove the component for this entity (if present).
    fn remove_entity(&mut self, entity: Entity);
    /// Expose as `&dyn Any` for downcasting.
    fn as_any(&self) -> &dyn Any;
    /// Expose as `&mut dyn Any` for downcasting.
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

// ---------------------------------------------------------------------------
// Concrete typed storage
// ---------------------------------------------------------------------------

/// Sparse-Vec storage for components of type `T`.
///
/// The outer `Vec` is indexed by `Entity::index`; each slot holds an
/// `Option<(generation, T)>` so stale handles are rejected.
#[derive(Debug)]
pub struct ComponentStorage<T: Component> {
    /// `data[entity.index]` = `Some((generation, component))` when live.
    data: Vec<Option<(u32, T)>>,
}

impl<T: Component> Default for ComponentStorage<T> {
    fn default() -> Self {
        Self { data: Vec::new() }
    }
}

impl<T: Component> ComponentStorage<T> {
    /// Create an empty storage.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace the component for `entity`.
    pub fn insert(&mut self, entity: Entity, component: T) {
        let idx = entity.index as usize;
        if idx >= self.data.len() {
            self.data.resize_with(idx + 1, || None);
        }
        self.data[idx] = Some((entity.generation, component));
    }

    /// Remove the component for `entity`. Returns `true` if it existed.
    pub fn remove(&mut self, entity: Entity) -> bool {
        if let Some(slot) = self.data.get_mut(entity.index as usize) {
            if slot
                .as_ref()
                .map(|(g, _)| *g == entity.generation)
                .unwrap_or(false)
            {
                *slot = None;
                return true;
            }
        }
        false
    }

    /// Get an immutable reference to the component for `entity`.
    pub fn get(&self, entity: Entity) -> Option<&T> {
        self.data
            .get(entity.index as usize)?
            .as_ref()
            .filter(|(g, _)| *g == entity.generation)
            .map(|(_, c)| c)
    }

    /// Get a mutable reference to the component for `entity`.
    pub fn get_mut(&mut self, entity: Entity) -> Option<&mut T> {
        self.data
            .get_mut(entity.index as usize)?
            .as_mut()
            .filter(|(g, _)| *g == entity.generation)
            .map(|(_, c)| c)
    }

    /// Iterate over all live `(Entity, &T)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (Entity, &T)> {
        self.data.iter().enumerate().filter_map(|(idx, slot)| {
            slot.as_ref()
                .map(|(gen, comp)| (Entity::new(idx as u32, *gen), comp))
        })
    }

    /// Iterate over all live `(Entity, &mut T)` pairs.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (Entity, &mut T)> {
        self.data.iter_mut().enumerate().filter_map(|(idx, slot)| {
            slot.as_mut()
                .map(|(gen, comp)| (Entity::new(idx as u32, *gen), comp))
        })
    }

    /// Collect entity indices (for intersection queries).
    pub fn entity_indices(&self) -> impl Iterator<Item = Entity> + '_ {
        self.iter().map(|(e, _)| e)
    }
}

impl<T: Component> AnyComponentStorage for ComponentStorage<T> {
    fn remove_entity(&mut self, entity: Entity) {
        self.remove(entity);
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_mut(&mut self) -> &mut dyn Any {
        self
    }
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Holds one [`ComponentStorage<T>`] per component type, keyed by [`TypeId`].
#[derive(Default)]
pub struct ComponentRegistry {
    storages: HashMap<TypeId, Box<dyn AnyComponentStorage>>,
}

impl ComponentRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Get or create the storage for component type `T`.
    pub fn storage<T: Component>(&mut self) -> &mut ComponentStorage<T> {
        self.storages
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(ComponentStorage::<T>::new()))
            .as_any_mut()
            .downcast_mut::<ComponentStorage<T>>()
            .expect("type mismatch in component registry — this is a bug")
    }

    /// Read-only access to storage for component type `T`.
    pub fn storage_ref<T: Component>(&self) -> Option<&ComponentStorage<T>> {
        self.storages
            .get(&TypeId::of::<T>())?
            .as_any()
            .downcast_ref::<ComponentStorage<T>>()
    }

    /// Remove all components for `entity` across every registered storage.
    pub fn remove_all_for(&mut self, entity: Entity) {
        self.storages
            .values_mut()
            .for_each(|s| s.remove_entity(entity));
    }
}
