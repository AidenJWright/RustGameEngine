//! Application runners — shared `AppCore` plus game and editor loops.

pub mod core;
pub mod editor_runner;
pub mod game_runner;

pub use core::AppCore;
pub use editor_runner::EditorRunner;
pub use game_runner::GameRunner;
