//! Type-erased global resource map.
//!
//! Resources are singletons stored by type — e.g. `DeltaTime`, `InputState`.
//! Access is O(1) via `TypeId` lookup and `Any` downcast.

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// A resource must be `Any + Send + Sync + 'static`.
pub trait Resource: Any + Send + Sync + 'static {}

/// Blanket: every suitable type is automatically a resource.
impl<T: Any + Send + Sync + 'static> Resource for T {}

/// Global singleton store, keyed by [`TypeId`].
#[derive(Default)]
pub struct Resources {
    map: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl Resources {
    /// Create an empty resource store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or replace a resource of type `T`.
    pub fn insert<T: Resource>(&mut self, value: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(value));
    }

    /// Get an immutable reference to resource `T`, or `None` if absent.
    pub fn get<T: Resource>(&self) -> Option<&T> {
        self.map
            .get(&TypeId::of::<T>())?
            .downcast_ref::<T>()
    }

    /// Get a mutable reference to resource `T`, or `None` if absent.
    pub fn get_mut<T: Resource>(&mut self) -> Option<&mut T> {
        self.map
            .get_mut(&TypeId::of::<T>())?
            .downcast_mut::<T>()
    }

    /// Remove and return resource `T`.
    pub fn remove<T: Resource>(&mut self) -> Option<T> {
        self.map
            .remove(&TypeId::of::<T>())
            .and_then(|b| b.downcast::<T>().ok())
            .map(|b| *b)
    }
}

// ---------------------------------------------------------------------------
// Built-in engine resources
// ---------------------------------------------------------------------------

/// Frame delta time in seconds.
#[derive(Debug, Clone, Copy, Default)]
pub struct DeltaTime(pub f32);

/// Total elapsed time since engine start, in seconds.
#[derive(Debug, Clone, Copy, Default)]
pub struct ElapsedTime(pub f32);
