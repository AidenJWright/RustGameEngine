//! Column-major 4×4 float matrix — identity, translate, rotate_z, scale, multiply.

use super::vec3::Vec3;

/// A column-major 4×4 matrix.
///
/// `cols[i]` is the i-th column stored as `[f32; 4]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mat4 {
    /// 4 columns, each 4 floats.
    pub cols: [[f32; 4]; 4],
}

impl Mat4 {
    /// Identity matrix.
    pub const IDENTITY: Self = Self {
        cols: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };

    /// Translation matrix.
    pub fn translate(t: Vec3) -> Self {
        let mut m = Self::IDENTITY;
        m.cols[3][0] = t.x;
        m.cols[3][1] = t.y;
        m.cols[3][2] = t.z;
        m
    }

    /// Rotation around the Z axis by `angle` radians (right-hand rule).
    pub fn rotate_z(angle: f32) -> Self {
        let (sin, cos) = angle.sin_cos();
        let mut m = Self::IDENTITY;
        m.cols[0][0] = cos;
        m.cols[0][1] = sin;
        m.cols[1][0] = -sin;
        m.cols[1][1] = cos;
        m
    }

    /// Uniform scale matrix.
    pub fn scale(s: Vec3) -> Self {
        let mut m = Self::IDENTITY;
        m.cols[0][0] = s.x;
        m.cols[1][1] = s.y;
        m.cols[2][2] = s.z;
        m
    }

    /// Matrix multiplication: `self * rhs`.
    pub fn mul(self, rhs: Self) -> Self {
        let mut out = [[0.0f32; 4]; 4];
        for col in 0..4 {
            for row in 0..4 {
                out[col][row] = (0..4).map(|k| self.cols[k][row] * rhs.cols[col][k]).sum();
            }
        }
        Self { cols: out }
    }

    /// Flatten to a `[f32; 16]` in column-major order (ready for GPU upload).
    pub fn to_cols_array(self) -> [f32; 16] {
        let c = self.cols;
        [
            c[0][0], c[0][1], c[0][2], c[0][3], c[1][0], c[1][1], c[1][2], c[1][3], c[2][0],
            c[2][1], c[2][2], c[2][3], c[3][0], c[3][1], c[3][2], c[3][3],
        ]
    }
}

impl Default for Mat4 {
    fn default() -> Self {
        Self::IDENTITY
    }
}
