//! Scene tree — parent/child entity relationships.
//!
//! Every entity is either a root (no parent) or attached to exactly one parent.
//! The tree is stored as two side-by-side maps for O(1) parent and child lookups.

use std::collections::HashMap;

use super::entity::Entity;

/// Stores the scene hierarchy as explicit parent/child maps.
///
/// All mutations go through [`World`]; external code should use the read-only
/// accessors below.
#[derive(Debug, Default)]
pub struct SceneTree {
    /// `parent_of[child]` = the parent entity (or `None` for roots).
    parent_of: HashMap<Entity, Option<Entity>>,
    /// `children_of[parent]` = ordered list of child entities.
    children_of: HashMap<Entity, Vec<Entity>>,
}

impl SceneTree {
    /// Create an empty scene tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new entity as a root (called by `World::spawn`).
    pub(crate) fn register_root(&mut self, entity: Entity) {
        self.parent_of.insert(entity, None);
        self.children_of.entry(entity).or_default();
    }

    /// Attach `child` under `parent`.
    ///
    /// If `child` was previously attached elsewhere it is detached first.
    pub fn attach(&mut self, child: Entity, parent: Entity) {
        // Detach from old parent if any.
        self.detach(child);
        self.parent_of.insert(child, Some(parent));
        self.children_of.entry(parent).or_default().push(child);
        // Ensure child has its own children list.
        self.children_of.entry(child).or_default();
    }

    /// Detach `child` from its current parent, making it a root.
    pub fn detach(&mut self, child: Entity) {
        if let Some(maybe_parent) = self.parent_of.get(&child).copied() {
            if let Some(parent) = maybe_parent {
                if let Some(siblings) = self.children_of.get_mut(&parent) {
                    siblings.retain(|&e| e != child);
                }
            }
        }
        self.parent_of.insert(child, None);
    }

    /// Return the children of `parent` as a slice (empty if none registered).
    pub fn children(&self, parent: Entity) -> &[Entity] {
        self.children_of
            .get(&parent)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// Return the parent of `child`, or `None` if it is a root.
    pub fn parent(&self, child: Entity) -> Option<Entity> {
        self.parent_of.get(&child).copied().flatten()
    }

    /// Iterate over all entities with no parent.
    pub fn root_entities(&self) -> impl Iterator<Item = Entity> + '_ {
        self.parent_of
            .iter()
            .filter_map(|(&e, &p)| if p.is_none() { Some(e) } else { None })
    }

    /// Depth-first traversal starting at `root`.
    ///
    /// `f` receives each entity and its depth (0 = root).
    /// Uses an explicit stack to avoid recursion overflow on deep trees.
    pub fn walk_depth_first(&self, root: Entity, mut f: impl FnMut(Entity, usize)) {
        // Stack entries: (entity, depth)
        let mut stack: Vec<(Entity, usize)> = vec![(root, 0)];
        while let Some((entity, depth)) = stack.pop() {
            f(entity, depth);
            // Push children in reverse order so they are visited left-to-right.
            if let Some(children) = self.children_of.get(&entity) {
                children
                    .iter()
                    .rev()
                    .for_each(|&child| stack.push((child, depth + 1)));
            }
        }
    }

    /// Remove all tree entries for `entity` (does NOT recurse — callers must
    /// collect children first via [`SceneTree::collect_subtree`]).
    pub(crate) fn remove_entity(&mut self, entity: Entity) {
        // Detach from parent's child list.
        if let Some(Some(parent)) = self.parent_of.remove(&entity) {
            if let Some(siblings) = self.children_of.get_mut(&parent) {
                siblings.retain(|&e| e != entity);
            }
        }
        self.children_of.remove(&entity);
    }

    /// Collect `root` and all its descendants in depth-first order.
    ///
    /// Used by `World::despawn` to recursively despawn subtrees.
    pub fn collect_subtree(&self, root: Entity) -> Vec<Entity> {
        let mut result = Vec::new();
        self.walk_depth_first(root, |e, _| result.push(e));
        result
    }
}
