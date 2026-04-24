//! `PlayerInput` component — configurable keyboard bindings for player movement.
//!
//! Attach this to a player entity in the editor to control which keys drive
//! movement.  The `PlayerInputSystem` reads this component alongside the
//! `KeysPressed` resource to produce `Velocity` updates each frame.

use serde::{Deserialize, Serialize};

use crate::ecs::component::Component;

/// A key that can be bound to a player action.
///
/// Variants correspond to physical keys.  `None` disables the binding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfigKey {
    None,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    W,
    A,
    S,
    D,
    R,
    Space,
    Return,
}

impl ConfigKey {
    /// All variants, in display order — used to build editor combo boxes.
    pub const ALL: &'static [Self] = &[
        Self::None,
        Self::ArrowLeft,
        Self::ArrowRight,
        Self::ArrowUp,
        Self::ArrowDown,
        Self::W,
        Self::A,
        Self::S,
        Self::D,
        Self::R,
        Self::Space,
        Self::Return,
    ];

    /// Human-readable label shown in the editor dropdown.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "None",
            Self::ArrowLeft => "Arrow Left",
            Self::ArrowRight => "Arrow Right",
            Self::ArrowUp => "Arrow Up",
            Self::ArrowDown => "Arrow Down",
            Self::W => "W",
            Self::A => "A",
            Self::S => "S",
            Self::D => "D",
            Self::R => "R",
            Self::Space => "Space",
            Self::Return => "Return",
        }
    }

    /// Index into `ConfigKey::ALL` — used by the editor combo selection.
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|k| *k == self).unwrap_or(0)
    }
}

/// A single directional axis with a negative and a positive key binding.
///
/// Axis value is in `[-1, 1]`.  Both keys pressed simultaneously yield `0`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InputAxis {
    /// Key that produces a negative (`-1.0`) contribution.
    pub negative: ConfigKey,
    /// Key that produces a positive (`+1.0`) contribution.
    pub positive: ConfigKey,
}

impl InputAxis {
    /// Arrow-key horizontal axis (Left = negative, Right = positive).
    pub fn arrows_horizontal() -> Self {
        Self {
            negative: ConfigKey::ArrowLeft,
            positive: ConfigKey::ArrowRight,
        }
    }

    /// Arrow-key vertical axis (Up = negative, Down = positive).
    pub fn arrows_vertical() -> Self {
        Self {
            negative: ConfigKey::ArrowUp,
            positive: ConfigKey::ArrowDown,
        }
    }

    /// WASD horizontal axis.
    pub fn wasd_horizontal() -> Self {
        Self {
            negative: ConfigKey::A,
            positive: ConfigKey::D,
        }
    }

    /// WASD vertical axis.
    pub fn wasd_vertical() -> Self {
        Self {
            negative: ConfigKey::W,
            positive: ConfigKey::S,
        }
    }
}

/// Configures which keys drive a player entity's movement.
///
/// Attach this to a player entity to enable the configurable input system.
/// The `PlayerInputSystem` reads this alongside the `KeysPressed` resource
/// to write `Velocity` each frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlayerInput {
    /// Horizontal movement axis (negative = left, positive = right).
    pub horizontal: InputAxis,
    /// Vertical movement axis (negative = up, positive = down).
    pub vertical: InputAxis,
    /// Movement speed in world units per second.
    pub speed: f32,
}

impl Default for PlayerInput {
    fn default() -> Self {
        Self {
            horizontal: InputAxis::arrows_horizontal(),
            vertical: InputAxis::arrows_vertical(),
            speed: 220.0,
        }
    }
}

impl Component for PlayerInput {}
