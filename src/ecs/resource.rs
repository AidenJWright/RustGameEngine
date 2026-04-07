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
        self.map.get(&TypeId::of::<T>())?.downcast_ref::<T>()
    }

    /// Get a mutable reference to resource `T`, or `None` if absent.
    pub fn get_mut<T: Resource>(&mut self) -> Option<&mut T> {
        self.map.get_mut(&TypeId::of::<T>())?.downcast_mut::<T>()
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

/// Set of currently held keys, updated by the game loop each frame.
///
/// Systems can read this resource to implement key-driven behaviour without
/// coupling to the winit event loop directly.
///
/// Keys are stored as `u32` discriminants matching the engine `KeyCode` enum
/// to avoid a cross-module dependency on `crate::platform` from within `ecs`.
/// The helper methods on `KeysPressed` use `crate::platform::KeyCode` which is
/// re-exported from the engine root.
#[derive(Debug, Default, Clone)]
pub struct KeysPressed {
    /// Raw set of pressed key discriminants.
    pub held: std::collections::HashSet<u32>,
}

impl KeysPressed {
    /// Record a key as pressed.
    pub fn press(&mut self, discriminant: u32) {
        self.held.insert(discriminant);
    }

    /// Record a key as released.
    pub fn release(&mut self, discriminant: u32) {
        self.held.remove(&discriminant);
    }

    /// Test whether a key is currently held.
    pub fn is_held(&self, discriminant: u32) -> bool {
        self.held.contains(&discriminant)
    }
}
