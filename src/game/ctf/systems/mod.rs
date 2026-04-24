//! Capture-the-flag systems.

pub mod flag_carry;
pub mod flag_pickup;
pub mod hud;
pub mod tagging;
pub mod wall_collision;
pub mod win_condition;

pub use flag_carry::FlagCarrySystem;
pub use flag_pickup::FlagPickupSystem;
pub use tagging::TaggingSystem;
pub use wall_collision::WallCollisionSystem;
pub use win_condition::{StopOnWinSystem, WinConditionSystem};

fn distance_sq(ax: f32, ay: f32, bx: f32, by: f32) -> f32 {
    let dx = ax - bx;
    let dy = ay - by;
    dx * dx + dy * dy
}

fn is_playing(world: &crate::ecs::world::World) -> bool {
    world
        .resource::<crate::game::ctf::resources::GameState>()
        .map_or(true, |state| {
            matches!(
                state.phase,
                crate::game::ctf::resources::GamePhase::Playing
            )
        })
}
