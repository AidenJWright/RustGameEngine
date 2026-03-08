//! `Transform` component — position, rotation, scale in 3D.

use super::vec3::Vec3;
use super::mat4::Mat4;

/// Spatial transform: position (Vec3), rotation around Z (radians), scale (Vec3).
///
/// This is a **component** — plain data, no methods beyond `into_matrix`.
#[derive(Debug, Clone)]
pub struct Transform {
    /// World-space position.
    pub position: Vec3,
    /// Rotation angle around the Z axis in radians.
    pub rotation: f32,
    /// Per-axis scale factor.
    pub scale: Vec3,
}

impl Transform {
    /// Identity transform at the origin.
    pub fn identity() -> Self {
        Self {
            position: Vec3::ZERO,
            rotation: 0.0,
            scale: Vec3::new(1.0, 1.0, 1.0),
        }
    }

    /// Build the TRS matrix: T * R * S.
    pub fn into_matrix(&self) -> Mat4 {
        let t = Mat4::translate(self.position);
        let r = Mat4::rotate_z(self.rotation);
        let s = Mat4::scale(self.scale);
        t.mul(r.mul(s))
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}
