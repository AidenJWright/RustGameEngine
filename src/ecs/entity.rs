//! Entity allocation with generation counters to safely handle recycling.
//!
//! **Tradeoff**: Using a free-list with generations means `Entity` IDs are
//! stable references even after deletion — stale handles are detected via the
//! generation mismatch rather than index re-use.

use std::collections::VecDeque;

/// A unique handle to an entity.
///
/// Combines an index (which slot in the allocator) and a generation counter
/// so that a handle referencing a despawned entity is distinguishable from a
/// new entity that reuses the same slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Entity {
    /// Slot index in the allocator.
    pub(crate) index: u32,
    /// Generation — incremented on each recycle of this slot.
    pub(crate) generation: u32,
}

impl Entity {
    /// Construct an entity directly (used by allocator and tests).
    pub fn new(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }
}

impl std::fmt::Display for Entity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Entity({}:{})", self.index, self.generation)
    }
}

/// Allocates and recycles [`Entity`] handles.
///
/// Internally keeps a free-list of recycled slots and bumps the generation
/// counter each time a slot is re-issued, making old handles detectably stale.
#[derive(Debug, Default)]
pub struct EntityAllocator {
    /// Next fresh index to issue when the free-list is empty.
    next_index: u32,
    /// Generation counter per slot — indexed by `Entity::index`.
    generations: Vec<u32>,
    /// Slots available for reuse (populated by `free`).
    free_list: VecDeque<u32>,
}

impl EntityAllocator {
    /// Create a new allocator with no live entities.
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a fresh [`Entity`].
    pub fn allocate(&mut self) -> Entity {
        if let Some(index) = self.free_list.pop_front() {
            // Re-issue a recycled slot; generation was already bumped on free.
            let generation = self.generations[index as usize];
            Entity { index, generation }
        } else {
            // Issue a brand-new slot.
            let index = self.next_index;
            self.next_index += 1;
            self.generations.push(0);
            Entity { index, generation: 0 }
        }
    }

    /// Free an entity slot, bumping its generation so old handles are stale.
    ///
    /// Returns `false` if the entity was already freed (generation mismatch).
    pub fn free(&mut self, entity: Entity) -> bool {
        let gen = self.generations.get_mut(entity.index as usize);
        match gen {
            Some(g) if *g == entity.generation => {
                *g += 1; // bump — makes existing handles to this slot stale
                self.free_list.push_back(entity.index);
                true
            }
            _ => false,
        }
    }

    /// Check whether an entity handle is still live.
    pub fn is_alive(&self, entity: Entity) -> bool {
        self.generations
            .get(entity.index as usize)
            .map(|&g| g == entity.generation)
            .unwrap_or(false)
    }
}
