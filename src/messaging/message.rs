//! Loop phase messages used by the game loop to drive systems.

/// The phase of the game loop in which a system runs.
///
/// Systems subscribe to a phase and are dispatched in priority order within
/// that phase each frame: `First → Update → Last`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum LoopPhase {
    /// Earliest phase — input handling, time updates.
    First,
    /// Main simulation phase — movement, AI, physics.
    Update,
    /// Latest phase — rendering prep, cleanup.
    Last,
}
