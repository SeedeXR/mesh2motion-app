//! Mesh representation.
//!
//! Vertex data is stored **structure-of-arrays**: the solver sweeps positions
//! far more often than it touches normals or UVs, and keeping them in separate
//! contiguous buffers keeps those sweeps cache-dense.

use crate::{CoreError, Result};
use glam::Vec3;

/// A triangle mesh: positions plus a triangle index buffer.
#[derive(Debug, Clone, Default)]
pub struct Mesh {
    /// Vertex positions, one per vertex.
    pub positions: Vec<Vec3>,
    /// Triangle indices, three per triangle, each indexing `positions`.
    pub indices: Vec<u32>,
}

impl Mesh {
    /// Builds a mesh from flat `[x, y, z, x, y, z, ...]` positions and triangle indices.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::InvalidMesh`] if `positions` is empty or not a multiple
    /// of 3, if `indices` is not a multiple of 3, or if any index is out of range.
    pub fn from_flat(positions: &[f32], indices: &[u32]) -> Result<Self> {
        if positions.is_empty() {
            return Err(CoreError::InvalidMesh("no vertices".into()));
        }
        if positions.len() % 3 != 0 {
            return Err(CoreError::InvalidMesh(format!(
                "position buffer length {} is not a multiple of 3",
                positions.len()
            )));
        }
        if indices.len() % 3 != 0 {
            return Err(CoreError::InvalidMesh(format!(
                "index buffer length {} is not a multiple of 3",
                indices.len()
            )));
        }

        let vertex_count = positions.len() / 3;
        // Bounds-check before building: an out-of-range index would otherwise
        // become a panic deep inside the solver, far from the malformed file
        // that caused it.
        if let Some(&bad) = indices.iter().find(|&&i| i as usize >= vertex_count) {
            return Err(CoreError::InvalidMesh(format!(
                "index {bad} out of range for {vertex_count} vertices"
            )));
        }

        Ok(Self {
            positions: positions
                .chunks_exact(3)
                .map(|c| Vec3::new(c[0], c[1], c[2]))
                .collect(),
            indices: indices.to_vec(),
        })
    }

    /// Number of vertices.
    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    /// Number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Axis-aligned bounding box as `(min, max)`.
    ///
    /// Returns `None` for an empty mesh.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let first = *self.positions.first()?;
        Some(
            self.positions
                .iter()
                .fold((first, first), |(lo, hi), &p| (lo.min(p), hi.max(p))),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tri() -> (Vec<f32>, Vec<u32>) {
        (
            vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            vec![0, 1, 2],
        )
    }

    #[test]
    fn builds_from_flat_buffers() {
        let (p, i) = tri();
        let m = Mesh::from_flat(&p, &i).expect("valid triangle");
        assert_eq!(m.vertex_count(), 3);
        assert_eq!(m.triangle_count(), 1);
    }

    #[test]
    fn rejects_empty() {
        assert!(Mesh::from_flat(&[], &[]).is_err());
    }

    #[test]
    fn rejects_ragged_positions() {
        assert!(Mesh::from_flat(&[0.0, 1.0], &[]).is_err());
    }

    #[test]
    fn rejects_ragged_indices() {
        let (p, _) = tri();
        assert!(Mesh::from_flat(&p, &[0, 1]).is_err());
    }

    #[test]
    fn rejects_out_of_range_index() {
        let (p, _) = tri();
        assert!(Mesh::from_flat(&p, &[0, 1, 99]).is_err());
    }

    #[test]
    fn computes_bounds() {
        let (p, i) = tri();
        let m = Mesh::from_flat(&p, &i).unwrap();
        let (lo, hi) = m.bounds().unwrap();
        assert_eq!(lo, Vec3::ZERO);
        assert_eq!(hi, Vec3::new(1.0, 1.0, 0.0));
    }
}
