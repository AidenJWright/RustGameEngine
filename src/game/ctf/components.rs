//! Runtime-only components for capture the flag.

use crate::ecs::component::Component;

/// Marks a player entity and stores its team/player id: 1 or 2.
#[derive(Debug, Clone)]
pub struct PlayerMarker {
    pub id: u8,
}

impl Component for PlayerMarker {}

/// Marks a flag entity and records which player owns it.
#[derive(Debug, Clone)]
pub struct Flag {
    pub owner_id: u8,
}

impl Component for Flag {}

/// Axis-aligned wall collider. Values are half-extents in world units.
#[derive(Debug, Clone)]
pub struct Wall {
    pub w: f32,
    pub h: f32,
}

impl Component for Wall {}
