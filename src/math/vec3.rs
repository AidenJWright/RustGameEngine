//! 3-component float vector.

use serde::{Deserialize, Serialize};
use std::ops::{Add, Mul, Neg, Sub};

/// A 3D vector with `f32` components.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

impl Vec3 {
    /// Construct from components.
    pub const fn new(x: f32, y: f32, z: f32) -> Self {
        Self { x, y, z }
    }

    /// Zero vector.
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);
    /// Unit X.
    pub const X: Self = Self::new(1.0, 0.0, 0.0);
    /// Unit Y.
    pub const Y: Self = Self::new(0.0, 1.0, 0.0);
    /// Unit Z.
    pub const Z: Self = Self::new(0.0, 0.0, 1.0);

    /// Dot product.
    pub fn dot(self, rhs: Self) -> f32 {
        self.x * rhs.x + self.y * rhs.y + self.z * rhs.z
    }

    /// Squared length.
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
            Self::new(self.x / len, self.y / len, self.z / len)
        } else {
            Self::ZERO
        }
    }

    /// Cross product: `self × rhs`.
    pub fn cross(self, rhs: Self) -> Self {
        Self::new(
            self.y * rhs.z - self.z * rhs.y,
            self.z * rhs.x - self.x * rhs.z,
            self.x * rhs.y - self.y * rhs.x,
        )
    }

    /// Construct from an array `[x, y, z]`.
    pub fn from_array(a: [f32; 3]) -> Self {
        Self::new(a[0], a[1], a[2])
    }

    /// Convert to array.
    pub fn to_array(self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }
}

impl Add for Vec3 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub for Vec3 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

impl Mul<f32> for Vec3 {
    type Output = Self;
    fn mul(self, s: f32) -> Self {
        Self::new(self.x * s, self.y * s, self.z * s)
    }
}

impl Neg for Vec3 {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.x, -self.y, -self.z)
    }
}
