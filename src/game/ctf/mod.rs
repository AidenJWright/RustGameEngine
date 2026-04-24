//! Capture-the-flag game logic.

pub mod components;
pub mod nav;
pub mod resources;
pub mod setup;
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
/// Movement speed for keyboard-controlled CTF players.
pub const PLAYER_SPEED: f32 = 220.0 * 1.5;
/// Movement speed for point-and-click auto movement.
pub const AUTO_MOVE_SPEED: f32 = PLAYER_SPEED * 0.95;
/// Thrown flags travel this many tagging radii.
pub const FLAG_THROW_DISTANCE_MULTIPLIER: f32 = 5.0;
/// Thrown flags travel this many player-speed units per second.
pub const FLAG_THROW_SPEED_MULTIPLIER: f32 = 3.0;
/// Maximum distance a thrown flag travels before stopping.
pub const FLAG_THROW_DISTANCE: f32 = TAG_RANGE * FLAG_THROW_DISTANCE_MULTIPLIER;
/// Thrown flag speed in world units per second.
pub const FLAG_THROW_SPEED: f32 = PLAYER_SPEED * FLAG_THROW_SPEED_MULTIPLIER;
/// Maximum distance at which a defender can stop their moving flag.
pub const FLAG_KNOCKDOWN_RANGE: f32 = TAG_RANGE;
/// Navigation grid cell size for point-and-click movement.
pub const PATH_GRID_CELL_SIZE: f32 = PLAYER_RADIUS;
/// Distance at which an auto-move waypoint is considered reached.
pub const PATH_WAYPOINT_RADIUS: f32 = 4.0;
/// Raw `KeysPressed` discriminant for Space.
pub const P1_TAG_KEY: u32 = 8;
/// Raw `KeysPressed` discriminant for Return.
pub const P2_TAG_KEY: u32 = 9;
/// Raw `KeysPressed` discriminant for Left Shift.
pub const SWITCH_KEY: u32 = 10;
/// Raw `KeysPressed` discriminant for R.
pub const RESTART_KEY: u32 = 11;
/// `InputFrame::action_bits` flag for tagging.
pub const ACTION_TAG: u8 = 0b0000_0001;
/// `InputFrame::action_bits` flag for teammate slot switching.
pub const ACTION_SWITCH: u8 = 0b0000_0010;
/// `InputFrame::action_bits` flag for restarting the CTF match.
pub const ACTION_RESTART: u8 = 0b0000_0100;
