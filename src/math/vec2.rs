//! 2-component float vector.

use std::ops::{Add, Mul, Neg, Sub};
use serde::{Deserialize, Serialize};

/// A 2D vector with `f32` components.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec2 {
    pub x: f32,
    pub y: f32,
}

impl Vec2 {
    /// Construct from components.
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    /// Zero vector.
    pub const ZERO: Self = Self::new(0.0, 0.0);

    /// Unit X.
    pub const X: Self = Self::new(1.0, 0.0);

    /// Unit Y.
    pub const Y: Self = Self::new(0.0, 1.0);

    /// Dot product.
    pub fn dot(self, rhs: Self) -> f32 {
        self.x * rhs.x + self.y * rhs.y
    }

    /// Squared length (avoids sqrt).
    pub fn length_squared(self) -> f32 {
        self.dot(self)
    }

    /// Euclidean length.
    pub fn length(self) -> f32 {
        self.length_squared().sqrt()
    }

    /// Unit vector in the same direction. Returns ZERO for a zero vector.
    pub fn normalize(self) -> Self {
        let len = self.length();
        if len > f32::EPSILON {
            Self::new(self.x / len, self.y / len)
        } else {
            Self::ZERO
        }
    }

    /// Component-wise absolute value.
    pub fn abs(self) -> Self {
        Self::new(self.x.abs(), self.y.abs())
    }
}

impl Add for Vec2 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y)
    }
}

impl Sub for Vec2 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y)
    }
}

impl Mul<f32> for Vec2 {
    type Output = Self;
    fn mul(self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s)
    }
}

impl Neg for Vec2 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y)
    }
}
