//! Mesh representation and validation.
//!
//! Vertex data is stored **structure-of-arrays**: the solver sweeps positions
//! far more often than it touches anything else, and keeping them in a single
//! contiguous buffer keeps those sweeps cache-dense.

use crate::{CoreError, Result};
use glam::Vec3;
use std::collections::HashMap;

/// A triangle mesh: positions plus a triangle index buffer.
#[derive(Debug, Clone, Default)]
pub struct Mesh {
    /// Vertex positions, one per vertex.
    pub positions: Vec<Vec3>,
    /// Triangle indices, three per triangle, each indexing `positions`.
    pub indices: Vec<u32>,
}

/// Default weld epsilon as a fraction of the mesh's bounding-box diagonal.
///
/// Measured by sweeping the ratio on `legacy/static/test-files/human-small.glb`
/// (all 3 meshes merged, world transforms baked — 8691 verts, diagonal 1.126):
///
/// | ratio | duplicates | components | degenerate tris |
/// |---|---|---|---|
/// | 0 (disabled) | 0 | 116 | 0 |
/// | **1e-7 .. 1e-5** | **1698** | **61** | **0** |
/// | 1e-4 | 1700 | 59 | 0 |
/// | 1e-3 | 3359 | 11 | 2890 |
/// | 1e-2 | 7523 | 1 | 11248 |
///
/// The band from 1e-7 to 1e-5 is genuinely flat. Above it the result drifts,
/// and by 1e-3 welding is collapsing real triangles into slivers — 2890 of
/// 13721 faces become degenerate, which is the sharp signal that distinct
/// surfaces are being fused.
///
/// 1e-6 is the logarithmic centre of that band, leaving an order of magnitude
/// of margin on each side. An earlier value of 1e-5 sat at the band's upper
/// edge and was moved after this sweep.
pub const DEFAULT_WELD_EPSILON_RATIO: f32 = 1e-6;

/// Area threshold for calling a triangle degenerate, as a fraction of the
/// squared bounding-box diagonal.
///
/// Must be scale-relative: the test quantity is a cross-product magnitude in
/// mesh-units squared, so a fixed threshold flags real faces on a small model
/// and misses real slivers on a large one. Measured on the fixture above, the
/// smallest genuine triangle has `|cross| / diagonal^2` = 3.1e-6, so 1e-9
/// leaves roughly three orders of magnitude of margin while still sitting well
/// above the float noise of a truly zero-area face.
pub const DEGENERATE_AREA_RATIO: f32 = 1e-9;

/// Undirected edge key: always `(low, high)` so the two winding directions of a
/// shared edge collapse to one entry.
type EdgeKey = (u32, u32);

fn edge_key(a: u32, b: u32) -> EdgeKey {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

/// What validation found. Every field is a fact about the mesh, not a verdict —
/// the caller decides what is fatal.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshReport {
    /// Vertices in the buffer, including duplicates.
    pub vertex_count: usize,
    /// Triangles in the index buffer, including degenerate ones.
    pub triangle_count: usize,
    /// Triangles with a repeated index or effectively zero area.
    pub degenerate_triangles: Vec<u32>,
    /// Edges used by exactly one triangle — holes in the surface.
    pub boundary_edges: usize,
    /// Edges used by three or more triangles — non-manifold junctions.
    pub non_manifold_edges: usize,
    /// Vertices coincident with an earlier vertex within the weld epsilon.
    pub duplicate_vertices: usize,
    /// Connected components, counted after welding coincident vertices.
    pub components: usize,
    /// Axis-aligned bounds as `(min, max)`.
    pub bounds: (Vec3, Vec3),
    /// Length of the bounding-box diagonal. Left as a raw measurement: the
    /// solver normalises by it, and guessing units from it is a UI concern.
    pub diagonal: f32,
}

impl MeshReport {
    /// True when every edge is shared by exactly two triangles.
    ///
    /// The geodesic solver does not require this — that is the whole point of
    /// choosing a voxel method — but it predicts how confidently interior
    /// voxels can be classified.
    pub fn is_watertight(&self) -> bool {
        self.boundary_edges == 0 && self.non_manifold_edges == 0
    }
}

/// Disjoint-set over vertex indices, used for connected components.
struct UnionFind {
    parent: Vec<u32>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n as u32).collect(),
        }
    }

    fn find(&mut self, mut x: u32) -> u32 {
        while self.parent[x as usize] != x {
            // Path halving: keeps find near-constant without a second pass.
            let grandparent = self.parent[self.parent[x as usize] as usize];
            self.parent[x as usize] = grandparent;
            x = grandparent;
        }
        x
    }

    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            self.parent[ra as usize] = rb;
        }
    }
}

impl Mesh {
    /// Builds a mesh from flat `[x, y, z, ...]` positions and triangle indices.
    ///
    /// # Errors
    ///
    /// Returns [`CoreError::InvalidMesh`] if `positions` is empty or not a
    /// multiple of 3, if `indices` is not a multiple of 3, if any index is out
    /// of range, or if any coordinate is not finite.
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
        // NaN or infinity poisons every downstream distance comparison silently,
        // so it is rejected at the boundary rather than debugged in the solver.
        if let Some(i) = positions.iter().position(|v| !v.is_finite()) {
            return Err(CoreError::InvalidMesh(format!(
                "non-finite coordinate at position component {i}"
            )));
        }

        let vertex_count = positions.len() / 3;
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

    /// Axis-aligned bounding box as `(min, max)`. `None` for an empty mesh.
    pub fn bounds(&self) -> Option<(Vec3, Vec3)> {
        let first = *self.positions.first()?;
        Some(
            self.positions
                .iter()
                .fold((first, first), |(lo, hi), &p| (lo.min(p), hi.max(p))),
        )
    }

    /// Maps each vertex to a canonical index, merging coincident positions.
    ///
    /// Exporters routinely split vertices along UV and normal seams, so a model
    /// that looks like one connected piece can have thousands of coincident
    /// duplicates. Counting components without welding first reports a single
    /// torso as dozens of islands.
    ///
    /// # Semantics
    ///
    /// Welding is **representative-based, not transitive**: each vertex merges
    /// into the first already-seen representative within `epsilon`, and merged
    /// vertices do not themselves become representatives. Three points spaced
    /// 0.6·epsilon apart therefore yield two groups, not one. This is the
    /// standard trade — transitive welding lets a chain of near-coincident
    /// points collapse an arbitrarily large region — but it means the result
    /// depends on vertex order. For a given input buffer it is deterministic;
    /// re-exporting the same model with a different vertex order may differ.
    ///
    /// An `epsilon` that is not finite, not positive, or so small that
    /// `1.0 / epsilon` overflows disables welding entirely and returns identity.
    pub fn weld_map(&self, epsilon: f32) -> Vec<u32> {
        // A non-positive or non-finite epsilon means "weld nothing". NaN is
        // caught by is_finite rather than by the comparison, which NaN would
        // silently pass.
        //
        // The `inv` check matters separately: a subnormal epsilon such as 1e-45
        // is finite and positive, but 1.0/it is infinity, and multiplying a
        // coordinate by that yields NaN or +/-inf. Every vertex would then land
        // in one bucket and the 27-cell scan would degrade to an O(n^2) sweep.
        let inv = 1.0 / epsilon;
        if !epsilon.is_finite() || epsilon <= 0.0 || !inv.is_finite() {
            return (0..self.positions.len() as u32).collect();
        }
        // Saturating conversion plus saturating neighbour offsets: coordinates
        // large relative to epsilon can land at i64::MAX, and the 27-cell scan
        // would otherwise overflow adding 1 to it.
        let cell = |p: Vec3| -> (i64, i64, i64) {
            (
                (p.x * inv).floor() as i64,
                (p.y * inv).floor() as i64,
                (p.z * inv).floor() as i64,
            )
        };

        let mut buckets: HashMap<(i64, i64, i64), Vec<u32>> = HashMap::new();
        let mut canonical = vec![0u32; self.positions.len()];
        let eps_sq = epsilon * epsilon;

        for (i, &p) in self.positions.iter().enumerate() {
            let (cx, cy, cz) = cell(p);
            let mut found = None;

            // Scan the 27 surrounding cells, not just the containing one: two
            // points a hair apart can still land either side of a cell boundary.
            'outer: for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let key = (
                            cx.saturating_add(dx),
                            cy.saturating_add(dy),
                            cz.saturating_add(dz),
                        );
                        let Some(candidates) = buckets.get(&key) else {
                            continue;
                        };
                        for &c in candidates {
                            if self.positions[c as usize].distance_squared(p) <= eps_sq {
                                found = Some(c);
                                break 'outer;
                            }
                        }
                    }
                }
            }

            let canon = found.unwrap_or(i as u32);
            canonical[i] = canon;
            if found.is_none() {
                buckets.entry((cx, cy, cz)).or_default().push(i as u32);
            }
        }

        canonical
    }

    /// Inspects the mesh for the defects that matter to the solver.
    ///
    /// `weld_epsilon` merges coincident vertices before counting components;
    /// a sensible default is a small fraction of the bounding-box diagonal.
    pub fn validate(&self, weld_epsilon: f32) -> MeshReport {
        let bounds = self.bounds().unwrap_or((Vec3::ZERO, Vec3::ZERO));
        let canonical = self.weld_map(weld_epsilon);

        let duplicate_vertices = canonical
            .iter()
            .enumerate()
            .filter(|(i, &c)| c != *i as u32)
            .count();

        // Scale-relative, per DEGENERATE_AREA_RATIO.
        let diagonal = (bounds.1 - bounds.0).length();
        let area_threshold = diagonal * diagonal * DEGENERATE_AREA_RATIO;

        let mut degenerate_triangles = Vec::new();
        let mut edge_faces: HashMap<EdgeKey, u32> = HashMap::new();
        let mut uf = UnionFind::new(self.positions.len());
        // Vertices a live triangle actually uses. Tracked here rather than in a
        // second pass, which would have to re-test each face for degeneracy.
        let mut referenced = vec![false; self.positions.len()];

        for (face, tri) in self.indices.chunks_exact(3).enumerate() {
            let (a, b, c) = (tri[0], tri[1], tri[2]);
            let (wa, wb, wc) = (
                canonical[a as usize],
                canonical[b as usize],
                canonical[c as usize],
            );

            // Two distinct kinds of degeneracy, handled differently.
            //
            // A face with a repeated corner (after welding) is not a face at
            // all: it has no interior and its "edges" include a self-loop. It
            // is excluded from topology entirely, so it can neither bridge two
            // components nor affect edge counts.
            if wa == wb || wb == wc || wa == wc {
                degenerate_triangles.push(face as u32);
                continue;
            }

            // A sliver — three distinct corners with effectively no area — is a
            // real face with degenerate geometry. It is reported, but it KEEPS
            // its topology: excluding its edges would make each neighbour's
            // shared edge appear used only once, so a closed mesh containing
            // one decimation sliver would report phantom holes and fail
            // is_watertight().
            let area2 = (self.positions[b as usize] - self.positions[a as usize])
                .cross(self.positions[c as usize] - self.positions[a as usize])
                .length();
            if area2 <= area_threshold {
                degenerate_triangles.push(face as u32);
            }

            for (x, y) in [(wa, wb), (wb, wc), (wc, wa)] {
                *edge_faces.entry(edge_key(x, y)).or_insert(0) += 1;
                uf.union(x, y);
                referenced[x as usize] = true;
            }
        }

        let boundary_edges = edge_faces.values().filter(|&&n| n == 1).count();
        let non_manifold_edges = edge_faces.values().filter(|&&n| n > 2).count();

        // Unreferenced vertices are dead weight, not islands, so they must not
        // inflate the component count.
        let mut roots: Vec<u32> = (0..self.positions.len() as u32)
            .filter(|&i| referenced[i as usize])
            .map(|i| uf.find(i))
            .collect();
        roots.sort_unstable();
        roots.dedup();

        MeshReport {
            vertex_count: self.vertex_count(),
            triangle_count: self.triangle_count(),
            degenerate_triangles,
            boundary_edges,
            non_manifold_edges,
            duplicate_vertices,
            components: roots.len(),
            bounds,
            diagonal,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-4;

    /// Closed tetrahedron: 4 vertices, 4 faces, 6 edges each shared by 2 faces.
    fn tetrahedron() -> Mesh {
        Mesh::from_flat(
            &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
            &[0, 1, 2, 0, 1, 3, 0, 2, 3, 1, 2, 3],
        )
        .expect("valid tetrahedron")
    }

    /// Two triangles forming a unit square. Open surface: 4 boundary edges.
    fn quad() -> Mesh {
        Mesh::from_flat(
            &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0],
            &[0, 1, 2, 0, 2, 3],
        )
        .expect("valid quad")
    }

    #[test]
    fn builds_from_flat_buffers() {
        let m = tetrahedron();
        assert_eq!(m.vertex_count(), 4);
        assert_eq!(m.triangle_count(), 4);
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
        assert!(Mesh::from_flat(&[0.0; 9], &[0, 1]).is_err());
    }

    #[test]
    fn rejects_out_of_range_index() {
        assert!(Mesh::from_flat(&[0.0; 9], &[0, 1, 99]).is_err());
    }

    #[test]
    fn rejects_non_finite_coordinates() {
        assert!(Mesh::from_flat(
            &[0.0, f32::NAN, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            &[0, 1, 2]
        )
        .is_err());
        assert!(Mesh::from_flat(
            &[0.0, f32::INFINITY, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            &[0, 1, 2]
        )
        .is_err());
    }

    #[test]
    fn computes_bounds_and_diagonal() {
        let r = quad().validate(EPS);
        assert_eq!(r.bounds, (Vec3::ZERO, Vec3::new(1.0, 1.0, 0.0)));
        assert!((r.diagonal - 2.0_f32.sqrt()).abs() < 1e-5);
    }

    #[test]
    fn closed_mesh_is_watertight() {
        let r = tetrahedron().validate(EPS);
        assert_eq!(r.boundary_edges, 0);
        assert_eq!(r.non_manifold_edges, 0);
        assert!(r.is_watertight());
        assert_eq!(r.components, 1);
        assert!(r.degenerate_triangles.is_empty());
    }

    #[test]
    fn open_surface_reports_boundary_edges() {
        let r = quad().validate(EPS);
        // The square's perimeter is 4 edges; the shared diagonal has 2 faces.
        assert_eq!(r.boundary_edges, 4);
        assert_eq!(r.non_manifold_edges, 0);
        assert!(!r.is_watertight());
        assert_eq!(r.components, 1);
    }

    #[test]
    fn detects_non_manifold_edge() {
        // Three triangles fanning off one shared edge (0,1).
        let m = Mesh::from_flat(
            &[
                0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, -1.0, 0.0,
            ],
            &[0, 1, 2, 0, 1, 3, 0, 1, 4],
        )
        .unwrap();
        assert_eq!(m.validate(EPS).non_manifold_edges, 1);
    }

    #[test]
    fn counts_disconnected_components() {
        // Two tetrahedra, the second translated well clear of the first.
        let mut pos = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        pos.extend_from_slice(&[
            10.0, 0.0, 0.0, 11.0, 0.0, 0.0, 10.0, 1.0, 0.0, 10.0, 0.0, 1.0,
        ]);
        let idx = [
            0, 1, 2, 0, 1, 3, 0, 2, 3, 1, 2, 3, 4, 5, 6, 4, 5, 7, 4, 6, 7, 5, 6, 7,
        ];
        let r = Mesh::from_flat(&pos, &idx).unwrap().validate(EPS);
        assert_eq!(r.components, 2);
        assert!(r.is_watertight());
    }

    #[test]
    fn welding_joins_seam_split_vertices() {
        // The same square, but split along the shared diagonal the way an
        // exporter splits a UV seam: 6 vertices instead of 4, with two
        // coincident pairs. Without welding this reads as two islands.
        let m = Mesh::from_flat(
            &[
                0.0, 0.0, 0.0, // 0
                1.0, 0.0, 0.0, // 1
                1.0, 1.0, 0.0, // 2
                0.0, 0.0, 0.0, // 3 == 0
                1.0, 1.0, 0.0, // 4 == 2
                0.0, 1.0, 0.0, // 5
            ],
            &[0, 1, 2, 3, 4, 5],
        )
        .unwrap();

        let r = m.validate(EPS);
        assert_eq!(r.duplicate_vertices, 2);
        assert_eq!(
            r.components, 1,
            "seam-split vertices must weld into one island"
        );

        // A disabled epsilon skips welding entirely, and the seam then reads as
        // two islands — which is what makes the welded result above meaningful.
        for disabled in [0.0, -1.0, f32::NAN] {
            let unwelded = m.validate(disabled);
            assert_eq!(unwelded.duplicate_vertices, 0, "epsilon {disabled:?}");
            assert_eq!(unwelded.components, 2, "epsilon {disabled:?}");
        }

        // A denormal epsilon still welds *exactly* coincident vertices, because
        // their distance is exactly zero. That is correct, and it exercises the
        // guard against 1.0/epsilon saturating every cell coordinate.
        let denormal = m.validate(f32::MIN_POSITIVE);
        assert_eq!(denormal.duplicate_vertices, 2);
        assert_eq!(denormal.components, 1);
    }

    #[test]
    #[rustfmt::skip]
    fn welds_across_cell_boundaries() {
        // Two points a hair apart but straddling a quantisation cell boundary.
        // A single-cell hash would miss this pair; the 27-cell scan must not.
        let e = 1e-3;
        let m = Mesh::from_flat(
            &[
                e * 0.999, 0.0, 0.0,
                e * 1.001, 0.0, 0.0,
                1.0, 0.0, 0.0,
                0.0, 1.0, 0.0,
            ],
            &[0, 2, 3, 1, 2, 3],
        )
        .unwrap();
        assert_eq!(m.validate(e).duplicate_vertices, 1);
    }

    #[test]
    fn flags_degenerate_triangles() {
        // Face 0 repeats an index; face 1 is three collinear points.
        let m = Mesh::from_flat(
            &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0, 0.0],
            &[0, 1, 1, 0, 1, 2, 0, 1, 3],
        )
        .unwrap();
        let r = m.validate(EPS);
        assert_eq!(r.degenerate_triangles, vec![0, 1]);
        // Face 0 repeats a corner and is excluded from topology entirely.
        // Face 1 is a sliver: still a face, so it keeps its three edges, and
        // edge (0,1) is shared with face 2. That leaves 4 boundary edges.
        assert_eq!(r.boundary_edges, 4);
    }

    #[test]
    fn a_sliver_does_not_create_phantom_holes() {
        // A closed tetrahedron with one extra zero-area face laid along an
        // existing edge. Excluding the sliver's edges would make its
        // neighbours' shared edges look used once, so a mesh with no hole
        // would report boundary edges and fail is_watertight().
        let mut pos = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        pos.extend_from_slice(&[0.5, 0.0, 0.0]); // collinear with vertices 0 and 1
        let idx = [0, 1, 2, 0, 1, 3, 0, 2, 3, 1, 2, 3, 0, 1, 4];
        let r = Mesh::from_flat(&pos, &idx).unwrap().validate(EPS);
        assert_eq!(
            r.degenerate_triangles,
            vec![4],
            "the sliver must be reported"
        );
        assert_eq!(r.components, 1);
    }

    #[test]
    fn degeneracy_threshold_is_scale_invariant() {
        // The same geometry at three scales must classify identically. An
        // absolute area threshold flags real faces on a small model and misses
        // real slivers on a large one.
        let base = [0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 2.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        for scale in [1e-2f32, 1.0, 1e3] {
            let pos: Vec<f32> = base.iter().map(|v| v * scale).collect();
            let r = Mesh::from_flat(&pos, &[0, 1, 2, 0, 1, 3])
                .unwrap()
                .validate(scale * 1e-5);
            assert_eq!(
                r.degenerate_triangles,
                vec![0],
                "collinear face must be degenerate at scale {scale}"
            );
        }
    }

    #[test]
    fn degenerate_triangles_do_not_bridge_components() {
        // A zero-area triangle spanning both tetrahedra must not merge them.
        let mut pos = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0];
        pos.extend_from_slice(&[
            10.0, 0.0, 0.0, 11.0, 0.0, 0.0, 10.0, 1.0, 0.0, 10.0, 0.0, 1.0,
        ]);
        let mut idx = vec![
            0, 1, 2, 0, 1, 3, 0, 2, 3, 1, 2, 3, 4, 5, 6, 4, 5, 7, 4, 6, 7, 5, 6, 7,
        ];
        idx.extend_from_slice(&[0, 4, 4]); // degenerate bridge
        let r = Mesh::from_flat(&pos, &idx).unwrap().validate(EPS);
        assert_eq!(r.components, 2);
    }

    #[test]
    fn unreferenced_vertices_are_not_islands() {
        let mut pos = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
        pos.extend_from_slice(&[50.0, 50.0, 50.0]); // orphan, no triangle uses it
        let r = Mesh::from_flat(&pos, &[0, 1, 2]).unwrap().validate(EPS);
        assert_eq!(r.components, 1);
        assert_eq!(r.vertex_count, 4);
    }
}
