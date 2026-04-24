//! Capture-the-flag game logic.

pub mod components;
pub mod resources;
pub mod systems;

/// Width of the CTF arena in world units.
pub const ARENA_WIDTH: f32 = 1280.0;
/// Height of the CTF arena in world units.
pub const ARENA_HEIGHT: f32 = 720.0;
/// Midline separating the two territories.
pub const MIDLINE_X: f32 = ARENA_WIDTH * 0.5;
/// Player collision radius. Triangles are rendered in a 40 px box.
pub const PLAYER_RADIUS: f32 = 20.0;
/// Flag collision/render radius.
pub const FLAG_RADIUS: f32 = 12.0;
/// Maximum distance at which a defender can tag a carrier.
pub const TAG_RANGE: f32 = 40.0;
/// Offset applied to a carried flag so it remains visible below its carrier.
pub const FLAG_CARRY_OFFSET_Y: f32 = 25.0;
/// Movement speed for both local players.
pub const PLAYER_SPEED: f32 = 220.0;
/// Raw `KeysPressed` discriminant for Space.
pub const P1_TAG_KEY: u32 = 8;
/// Raw `KeysPressed` discriminant for Return.
pub const P2_TAG_KEY: u32 = 9;
