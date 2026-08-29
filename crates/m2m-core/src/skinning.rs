//! Skinning weight solver.
//!
//! Replaces the legacy rigid nearest-bone approach
//! (`legacy/src/lib/solvers/WeightCalculator.ts:71-80`) with geodesic voxel
//! binding. See `memory/architecture.md` §3 for why, and
//! `docs/algorithms/geodesic-voxel-binding.md` once P1/R-2 lands.

/// Maximum bones influencing a single vertex.
///
/// Four is the GPU skinning limit that glTF, FBX, and every real-time engine
/// assume; exceeding it silently truncates downstream.
pub const MAX_INFLUENCES: usize = 4;

/// Per-vertex skinning weights: bone indices and their normalised influences.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkinWeights {
    /// Bone index per influence, `MAX_INFLUENCES` per vertex.
    pub indices: Vec<u16>,
    /// Influence weight per bone, `MAX_INFLUENCES` per vertex, summing to 1.0.
    pub weights: Vec<f32>,
}

impl SkinWeights {
    /// Allocates zeroed weights for `vertex_count` vertices.
    pub fn zeroed(vertex_count: usize) -> Self {
        Self {
            indices: vec![0; vertex_count * MAX_INFLUENCES],
            weights: vec![0.0; vertex_count * MAX_INFLUENCES],
        }
    }

    /// Number of vertices these weights cover.
    pub fn vertex_count(&self) -> usize {
        self.weights.len() / MAX_INFLUENCES
    }

    /// Returns the index of the first vertex whose weights do not sum to 1.0
    /// within `tolerance`, or `None` if every vertex is normalised.
    ///
    /// This is invariant 1 from `memory/test.md` §3 and is checked in debug
    /// builds after every solve.
    pub fn first_unnormalised(&self, tolerance: f32) -> Option<usize> {
        self.weights
            .chunks_exact(MAX_INFLUENCES)
            .position(|w| (w.iter().sum::<f32>() - 1.0).abs() > tolerance)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zeroed_has_correct_shape() {
        let w = SkinWeights::zeroed(10);
        assert_eq!(w.vertex_count(), 10);
        assert_eq!(w.indices.len(), 10 * MAX_INFLUENCES);
    }

    #[test]
    fn detects_unnormalised_vertex() {
        let mut w = SkinWeights::zeroed(2);
        w.weights[0] = 1.0; // vertex 0 sums to 1.0
                            // vertex 1 is left summing to 0.0
        assert_eq!(w.first_unnormalised(1e-5), Some(1));
    }

    #[test]
    fn accepts_normalised_weights() {
        let mut w = SkinWeights::zeroed(1);
        w.weights[..4].copy_from_slice(&[0.5, 0.3, 0.15, 0.05]);
        assert_eq!(w.first_unnormalised(1e-5), None);
    }
}
