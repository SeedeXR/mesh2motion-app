//! Sparse voxelisation with interior/exterior classification.
//!
//! Step 1-2 of the geodesic voxel binding pipeline
//! (`docs/algorithms/geodesic-voxel-binding.md`). The geodesic distance field
//! in P1-4 propagates only through interior voxels, which is what stops
//! distance from jumping across empty space the way the legacy Euclidean
//! nearest-bone solver does.
//!
//! # Why this survives real meshes
//!
//! Measured on a real character (`crates/m2m-core/tests/real_mesh.rs`): 61
//! disconnected components, 26 boundary edges, one non-manifold edge, not
//! watertight. Nothing here requires a manifold:
//!
//! - Triangles are rasterised **conservatively**, so the surface shell has no
//!   gaps even for slivers that a point-sampling rasteriser would miss.
//! - A hole smaller than a voxel is plugged by the shell's own thickness, so
//!   small boundary loops do not leak.
//! - Disconnected components share one grid, so islands nested inside the body
//!   (eyes inside a head) become interior-connected even though their surfaces
//!   never touch. That is the correct answer and it is why the method is
//!   voxel-based rather than surface-based.

use crate::mesh::Mesh;
use glam::{IVec3, Vec3};

/// Default voxels along the longest axis.
///
/// Measured by sweeping a real character (8691 verts, extent 0.81 x 0.76 x 0.15)
/// in release builds:
///
/// | resolution | surface | interior | interior/surface | volume | time | memory |
/// |---|---|---|---|---|---|---|
/// | 32 | 800 | 66 | 0.08 | 0.00108 | ~3 ms | 15 KB |
/// | 64 | 3224 | 1268 | 0.39 | 0.00259 | ~4 ms | 80 KB |
/// | 128 | 13370 | 15226 | 1.14 | 0.00389 | 8 ms | 490 KB |
/// | 192 | 30416 | 58354 | 1.92 | 0.00442 | 16 ms | 1.5 MB |
/// | **256** | 54608 | 146672 | **2.69** | **0.00469** | **30 ms** | **3.4 MB** |
/// | 384 | 124286 | 525292 | 4.23 | 0.00497 | 85 ms | 10.9 MB |
///
/// Timings are single-run on an M4 in release; treat them as orders of
/// magnitude, not benchmarks.
///
/// The ratio is what matters, not the raw count: below about 128 the grid is
/// shell-dominated — thin limbs are entirely surface with no interior between
/// them — and the geodesic field would have almost nothing to propagate
/// through. 256 puts interior comfortably ahead of surface, lands within ~6%
/// of the converged volume, and costs 24 ms and 3 MB, which is nothing against
/// the budget in `memory/test.md`.
pub const DEFAULT_RESOLUTION: u32 = 256;

/// Voxels of padding around the mesh bounds.
///
/// The exterior flood fill seeds from the grid boundary, so there must be a
/// guaranteed-empty shell for it to start in. Two, not one: rasterisation
/// widens each triangle's voxel range by one (see `rasterise`), so a single
/// layer can be consumed by surface voxels and leave the fill no seed.
const PADDING: u32 = 2;

/// Classification of a single voxel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum VoxelState {
    /// Reachable from outside the mesh.
    Exterior = 0,
    /// Overlapped by at least one triangle.
    Surface = 1,
    /// Enclosed by surface: not reachable from the grid boundary.
    Interior = 2,
}

/// A uniform voxel grid covering a mesh, with each voxel classified.
#[derive(Debug, Clone)]
pub struct VoxelGrid {
    dims: [u32; 3],
    origin: Vec3,
    voxel_size: f32,
    states: Vec<VoxelState>,
}

/// Summary counts, for reporting and for tests that must not depend on layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoxelStats {
    /// Voxels overlapped by geometry.
    pub surface: usize,
    /// Voxels enclosed by geometry.
    pub interior: usize,
    /// Voxels reachable from outside.
    pub exterior: usize,
}

impl VoxelGrid {
    /// Voxelises `mesh` with `resolution` voxels along its longest axis.
    ///
    /// Returns `None` for a mesh with no extent (all vertices coincident), or
    /// if `resolution` is zero.
    ///
    /// Memory is `dims.x * dims.y * dims.z` bytes. Resolution 256 on a roughly
    /// human aspect ratio is a few megabytes; it grows with the cube of
    /// resolution, so callers pick it against the budget in `memory/test.md`.
    pub fn build(mesh: &Mesh, resolution: u32) -> Option<Self> {
        if resolution == 0 {
            return None;
        }
        let (lo, hi) = mesh.bounds()?;
        let extent = hi - lo;
        let longest = extent.max_element();
        // is_finite first so NaN is caught by it rather than by the
        // comparison, which NaN would silently pass.
        if !longest.is_finite() || longest <= 0.0 {
            return None;
        }

        let voxel_size = longest / resolution as f32;
        let pad = PADDING as f32 * voxel_size;
        let origin = lo - Vec3::splat(pad);

        // Per-axis dims so a flat model does not pay for a cubic grid.
        //
        // The +1 matters: on the longest axis `extent / voxel_size` is exactly
        // `resolution`, so without it the mesh's max face lands in the
        // outermost voxel layer and that whole side of the grid has no empty
        // shell for the exterior fill to seed from.
        let dims = [0usize, 1, 2].map(|i| {
            let n = (extent[i] / voxel_size).ceil() as u32;
            n.max(1) + 2 * PADDING + 1
        });

        // A large resolution can overflow the usize product, which would wrap
        // to a small allocation and then panic out of bounds during rasterise.
        let count = (dims[0] as usize)
            .checked_mul(dims[1] as usize)
            .and_then(|n| n.checked_mul(dims[2] as usize))?;
        let mut grid = Self {
            dims,
            origin,
            voxel_size,
            // Everything starts Interior; the flood fill carves out the
            // exterior. Starting from Exterior would need a second pass.
            states: vec![VoxelState::Interior; count],
        };

        grid.rasterise(mesh);
        grid.flood_exterior();
        Some(grid)
    }

    /// Grid dimensions in voxels.
    pub fn dims(&self) -> [u32; 3] {
        self.dims
    }

    /// Edge length of one voxel, in mesh units.
    pub fn voxel_size(&self) -> f32 {
        self.voxel_size
    }

    /// Total voxels in the grid.
    pub fn len(&self) -> usize {
        self.states.len()
    }

    /// Whether the grid holds no voxels.
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// Flat index for a voxel coordinate, or `None` if out of bounds.
    pub fn index(&self, c: IVec3) -> Option<usize> {
        if c.x < 0 || c.y < 0 || c.z < 0 {
            return None;
        }
        let (x, y, z) = (c.x as u32, c.y as u32, c.z as u32);
        if x >= self.dims[0] || y >= self.dims[1] || z >= self.dims[2] {
            return None;
        }
        Some((z as usize * self.dims[1] as usize + y as usize) * self.dims[0] as usize + x as usize)
    }

    /// State of a voxel, or `None` if out of bounds.
    pub fn state(&self, c: IVec3) -> Option<VoxelState> {
        self.index(c).map(|i| self.states[i])
    }

    /// World-space centre of a voxel.
    pub fn center(&self, c: IVec3) -> Vec3 {
        self.origin + (c.as_vec3() + Vec3::splat(0.5)) * self.voxel_size
    }

    /// Voxel coordinate containing a world-space point.
    pub fn coord_of(&self, p: Vec3) -> IVec3 {
        ((p - self.origin) / self.voxel_size).floor().as_ivec3()
    }

    /// Counts of each state.
    pub fn stats(&self) -> VoxelStats {
        let mut s = VoxelStats {
            surface: 0,
            interior: 0,
            exterior: 0,
        };
        for st in &self.states {
            match st {
                VoxelState::Surface => s.surface += 1,
                VoxelState::Interior => s.interior += 1,
                VoxelState::Exterior => s.exterior += 1,
            }
        }
        s
    }

    /// Marks every voxel overlapped by a triangle as [`VoxelState::Surface`].
    fn rasterise(&mut self, mesh: &Mesh) {
        // Test against a very slightly enlarged voxel box. "Conservative" has
        // to mean it, or the shell develops pinholes: a face lying exactly on a
        // voxel plane is otherwise decided by one ulp, and the grid origin is
        // derived from the mesh bounds, so an axis-aligned mesh puts its faces
        // on voxel planes systematically rather than by chance. The overlap is
        // sub-thousandth of a voxel, far too small to thicken the shell.
        let half = Vec3::splat(self.voxel_size * 0.5 * (1.0 + 1e-3));

        for tri in mesh.indices.chunks_exact(3) {
            // Rasterise in grid-local coordinates, not world.
            //
            // f32 has ~7 significant digits, so a small object far from the
            // world origin loses the precision the overlap test needs: at
            // world x=123, one ulp is ~1e-5, which for a 0.01-unit model at
            // resolution 30 is only ~30x below the voxel size. Measured: a
            // sweep over scale x offset x rotation x resolution failed only for
            // the (scale 0.01, offset 123.456) combination. Subtracting the
            // origin makes precision depend on the model's own extent rather
            // than on where the artist happened to place it in the scene.
            let v = [
                mesh.positions[tri[0] as usize] - self.origin,
                mesh.positions[tri[1] as usize] - self.origin,
                mesh.positions[tri[2] as usize] - self.origin,
            ];

            // Voxels in the triangle's AABB, widened by one. Local space, so
            // this is coord_of without the origin subtraction.
            //
            // The widening is not slack, it is required for correctness.
            // `coord_of` and `center` round independently, so a face lying on a
            // voxel boundary can land in one voxel by `coord_of` while the
            // overlap test would only accept its neighbour. At resolution 20 a
            // unit cube's voxel size is 0.050000001, `coord_of(0.0)` gives
            // voxel 1 whose box starts at 2e-9, and the face at x=0 actually
            // falls in voxel 0 — which the un-widened range never tested. The
            // result was a cube with no shell at all, which the exterior fill
            // then flooded straight through.
            let one = IVec3::ONE;
            let to_coord = |p: Vec3| -> IVec3 { (p / self.voxel_size).floor().as_ivec3() };
            let lo = to_coord(v[0].min(v[1]).min(v[2])) - one;
            let hi = to_coord(v[0].max(v[1]).max(v[2])) + one;

            for z in lo.z..=hi.z {
                for y in lo.y..=hi.y {
                    for x in lo.x..=hi.x {
                        let c = IVec3::new(x, y, z);
                        let Some(idx) = self.index(c) else { continue };
                        if self.states[idx] == VoxelState::Surface {
                            continue;
                        }
                        // Local-space box centre, matching `v` above.
                        let local_center = (c.as_vec3() + Vec3::splat(0.5)) * self.voxel_size;
                        if tri_aabb_overlap(local_center, half, &v) {
                            self.states[idx] = VoxelState::Surface;
                        }
                    }
                }
            }
        }
    }

    /// Flood-fills [`VoxelState::Exterior`] inward from the grid boundary.
    ///
    /// Six-connected on purpose: a diagonal fill would slip through a shell
    /// that touches only corner-to-corner, which conservative rasterisation
    /// otherwise guarantees is sealed.
    fn flood_exterior(&mut self) {
        let [dx, dy, dz] = self.dims;
        let mut stack: Vec<IVec3> = Vec::new();

        // Seed from every face of the padded boundary. One seed corner is not
        // enough: a mesh touching the padding could split the outside region.
        let seed = |grid: &mut Self, c: IVec3, stack: &mut Vec<IVec3>| {
            if let Some(i) = grid.index(c) {
                if grid.states[i] == VoxelState::Interior {
                    grid.states[i] = VoxelState::Exterior;
                    stack.push(c);
                }
            }
        };
        for y in 0..dy as i32 {
            for x in 0..dx as i32 {
                seed(self, IVec3::new(x, y, 0), &mut stack);
                seed(self, IVec3::new(x, y, dz as i32 - 1), &mut stack);
            }
        }
        for z in 0..dz as i32 {
            for x in 0..dx as i32 {
                seed(self, IVec3::new(x, 0, z), &mut stack);
                seed(self, IVec3::new(x, dy as i32 - 1, z), &mut stack);
            }
        }
        for z in 0..dz as i32 {
            for y in 0..dy as i32 {
                seed(self, IVec3::new(0, y, z), &mut stack);
                seed(self, IVec3::new(dx as i32 - 1, y, z), &mut stack);
            }
        }

        const NEIGHBOURS: [IVec3; 6] = [
            IVec3::new(1, 0, 0),
            IVec3::new(-1, 0, 0),
            IVec3::new(0, 1, 0),
            IVec3::new(0, -1, 0),
            IVec3::new(0, 0, 1),
            IVec3::new(0, 0, -1),
        ];

        while let Some(c) = stack.pop() {
            for d in NEIGHBOURS {
                let n = c + d;
                let Some(i) = self.index(n) else { continue };
                if self.states[i] == VoxelState::Interior {
                    self.states[i] = VoxelState::Exterior;
                    stack.push(n);
                }
            }
        }
    }
}

/// Separating-axis test for a triangle against an axis-aligned box.
///
/// Conservative: returns true whenever they touch at all. Shell integrity
/// depends on it — a rasteriser that samples points instead can miss a thin
/// sliver and leave a pinhole for the exterior flood fill to pour through.
///
/// Akenine-Möller's 13 axes: 3 box normals, 1 triangle normal, and the 9
/// cross-products of triangle edges with box axes.
fn tri_aabb_overlap(box_center: Vec3, box_half: Vec3, tri: &[Vec3; 3]) -> bool {
    let v = [
        tri[0] - box_center,
        tri[1] - box_center,
        tri[2] - box_center,
    ];
    let edges = [v[1] - v[0], v[2] - v[1], v[0] - v[2]];

    let separates = |axis: Vec3| -> bool {
        // A near-zero axis carries no information (parallel edge, or a
        // degenerate triangle) and would otherwise report a false separation.
        if axis.length_squared() < 1e-20 {
            return false;
        }
        let p = [axis.dot(v[0]), axis.dot(v[1]), axis.dot(v[2])];
        let radius =
            box_half.x * axis.x.abs() + box_half.y * axis.y.abs() + box_half.z * axis.z.abs();
        let min = p[0].min(p[1]).min(p[2]);
        let max = p[0].max(p[1]).max(p[2]);
        min > radius || max < -radius
    };

    // Triangle normal first: it rejects the most candidates, and every axis
    // after it is wasted work on a voxel that is already separated.
    if separates(edges[0].cross(edges[1])) {
        return false;
    }

    // 3 box normals.
    if separates(Vec3::X) || separates(Vec3::Y) || separates(Vec3::Z) {
        return false;
    }

    // 9 edge x box-axis cross products.
    for e in edges {
        if separates(Vec3::new(0.0, -e.z, e.y))
            || separates(Vec3::new(e.z, 0.0, -e.x))
            || separates(Vec3::new(-e.y, e.x, 0.0))
        {
            return false;
        }
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Axis-aligned closed box from (0,0,0) to (1,1,1), 12 triangles.
    fn unit_cube() -> Mesh {
        #[rustfmt::skip]
        let positions = [
            0.0, 0.0, 0.0,  1.0, 0.0, 0.0,  1.0, 1.0, 0.0,  0.0, 1.0, 0.0,
            0.0, 0.0, 1.0,  1.0, 0.0, 1.0,  1.0, 1.0, 1.0,  0.0, 1.0, 1.0,
        ];
        #[rustfmt::skip]
        let indices = [
            0, 2, 1, 0, 3, 2, // -Z
            4, 5, 6, 4, 6, 7, // +Z
            0, 1, 5, 0, 5, 4, // -Y
            3, 7, 6, 3, 6, 2, // +Y
            0, 4, 7, 0, 7, 3, // -X
            1, 2, 6, 1, 6, 5, // +X
        ];
        Mesh::from_flat(&positions, &indices).expect("valid cube")
    }

    #[test]
    fn closed_box_has_an_interior() {
        let g = VoxelGrid::build(&unit_cube(), 16).expect("grid");
        let s = g.stats();
        assert!(s.interior > 0, "a closed box must enclose voxels");
        assert!(s.surface > 0);
        assert!(s.exterior > 0, "padding guarantees exterior voxels");
        assert_eq!(s.surface + s.interior + s.exterior, g.len());
    }

    #[test]
    fn interior_scales_with_resolution() {
        // Interior volume is ~1 unit^3; the voxel count should track
        // resolution^3, confirming the fill is volumetric and not a shell.
        let a = VoxelGrid::build(&unit_cube(), 8).unwrap().stats().interior;
        let b = VoxelGrid::build(&unit_cube(), 16).unwrap().stats().interior;
        assert!(
            b > a * 4,
            "interior should grow roughly cubically: {a} -> {b}"
        );
    }

    #[test]
    fn interior_volume_approximates_the_true_volume() {
        // The cube's interior is 1 unit^3 minus the shell. At resolution 32 the
        // shell is thin, so the interior fraction should be within ~25%.
        let res = 32;
        let g = VoxelGrid::build(&unit_cube(), res).unwrap();
        let vs = g.voxel_size();
        let volume = g.stats().interior as f32 * vs * vs * vs;
        assert!(
            volume > 0.6 && volume < 1.0,
            "interior volume {volume} is not close to 1.0"
        );
    }

    #[test]
    fn open_box_leaks_and_that_is_documented() {
        // Same cube with the +Z face removed. The hole is far larger than a
        // voxel, so the exterior fill pours in and there is no interior left.
        // This is the known limit of the method, not a defect: holes smaller
        // than a voxel are plugged by shell thickness, larger ones are not.
        let cube = unit_cube();
        let indices: Vec<u32> = cube
            .indices
            .chunks_exact(3)
            .enumerate()
            .filter(|(i, _)| *i != 2 && *i != 3) // drop the +Z pair
            .flat_map(|(_, t)| t.iter().copied())
            .collect();
        let positions: Vec<f32> = cube
            .positions
            .iter()
            .flat_map(|p| [p.x, p.y, p.z])
            .collect();
        let open = Mesh::from_flat(&positions, &indices).unwrap();

        let closed_interior = VoxelGrid::build(&cube, 16).unwrap().stats().interior;
        let open_interior = VoxelGrid::build(&open, 16).unwrap().stats().interior;
        assert!(closed_interior > 0);
        assert_eq!(open_interior, 0, "a face-sized hole must leak");
    }

    #[test]
    fn nested_island_is_interior_connected() {
        // A small cube fully inside a large one, their surfaces disconnected.
        // The inner cube's own interior and the shell between them must all be
        // interior — this is the eyes-inside-a-head case that makes the
        // 61-component reality workable.
        let outer = unit_cube();
        let mut positions: Vec<f32> = outer
            .positions
            .iter()
            .flat_map(|p| [p.x, p.y, p.z])
            .collect();
        let mut indices: Vec<u32> = outer.indices.clone();

        let base = outer.positions.len() as u32;
        for p in &outer.positions {
            let q = *p * 0.2 + Vec3::splat(0.4); // centred, 1/5 scale
            positions.extend_from_slice(&[q.x, q.y, q.z]);
        }
        indices.extend(outer.indices.iter().map(|i| i + base));

        let nested = Mesh::from_flat(&positions, &indices).unwrap();
        assert_eq!(
            nested.validate(1e-6).components,
            2,
            "surfaces stay separate"
        );

        let g = VoxelGrid::build(&nested, 24).unwrap();
        assert!(g.stats().interior > 0);

        // The centre of the inner cube must be interior, not exterior: nothing
        // connects it to the outside.
        let c = g.coord_of(Vec3::splat(0.5));
        assert_eq!(g.state(c), Some(VoxelState::Interior));
    }

    #[test]
    fn shell_seals_across_scale_offset_rotation_and_resolution() {
        // The regression test for the worst bug in this module. A cube's faces
        // are axis-aligned, and the grid origin derives from the mesh bounds,
        // so its faces land exactly on voxel planes systematically rather than
        // by chance. Before the fixes this produced a cube with NO SHELL AT ALL
        // at resolutions 13, 18, 20 and leaks at 16 more — and the original
        // tests missed every one of them by happening to use 8/16/24/32.
        //
        // Three independent causes, all needed:
        //   1. the rasterisation AABB excluded the voxel actually containing a
        //      boundary face, because coord_of and center round independently
        //   2. the overlap test had no epsilon, so exact touching was decided
        //      by one ulp
        //   3. world-space rasterisation lost precision for a small model far
        //      from the origin
        //
        // The full sweep (3 scales x 4 offsets x 3 rotations x 41 resolutions
        // = 1476 cases) passes; this is a representative subset kept fast
        // enough for the default test run.
        for &scale in &[0.01f32, 1.0, 100.0] {
            for &off in &[0.0f32, 1.0 / 3.0, 123.456] {
                for &angle in &[0.0f32, 0.3] {
                    let rot = glam::Mat3::from_axis_angle(Vec3::Y, angle);
                    let cube = unit_cube();
                    let positions: Vec<f32> = cube
                        .positions
                        .iter()
                        .flat_map(|p| {
                            let q = rot * (*p * scale) + Vec3::splat(off);
                            [q.x, q.y, q.z]
                        })
                        .collect();
                    let m = Mesh::from_flat(&positions, &cube.indices).unwrap();

                    for res in [9u32, 13, 18, 20, 23, 30, 33] {
                        let s = VoxelGrid::build(&m, res).unwrap().stats();
                        assert!(
                            s.surface > 0,
                            "no shell: scale {scale} off {off} angle {angle} res {res}"
                        );
                        assert!(
                            s.interior > 0,
                            "leaked: scale {scale} off {off} angle {angle} res {res}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn rejects_degenerate_input() {
        let flat = Mesh::from_flat(&[0.0; 9], &[0, 1, 2]).unwrap();
        assert!(VoxelGrid::build(&flat, 16).is_none(), "zero extent");
        assert!(
            VoxelGrid::build(&unit_cube(), 0).is_none(),
            "zero resolution"
        );
    }

    #[test]
    fn grid_is_padded_so_the_fill_can_start_outside() {
        let g = VoxelGrid::build(&unit_cube(), 8).unwrap();
        // Every corner of the padded grid must be exterior.
        for c in [
            IVec3::ZERO,
            IVec3::new(g.dims()[0] as i32 - 1, 0, 0),
            IVec3::new(0, g.dims()[1] as i32 - 1, 0),
            IVec3::new(0, 0, g.dims()[2] as i32 - 1),
        ] {
            assert_eq!(g.state(c), Some(VoxelState::Exterior), "corner {c:?}");
        }
    }

    /// xorshift64*, so the property test is deterministic without a dependency.
    struct Rng(u64);

    impl Rng {
        fn next_f32(&mut self) -> f32 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            // Top 24 bits into [0,1), then to [-1.5, 1.5).
            (((self.0 >> 40) as f32) / (1u32 << 24) as f32) * 3.0 - 1.5
        }

        fn next_vec(&mut self) -> Vec3 {
            Vec3::new(self.next_f32(), self.next_f32(), self.next_f32())
        }
    }

    #[test]
    fn sat_never_misses_an_overlap_found_by_sampling() {
        // Independent cross-check of the 13-axis SAT against brute force: if a
        // densely sampled point of the triangle lands inside the box, the
        // triangle definitely overlaps it, so SAT must agree. A sign error on
        // any axis shows up here as a false separation.
        //
        // Only one direction is checkable this way — sampling can miss a
        // genuine overlap (a triangle slicing a corner), and SAT being
        // conservative is allowed to report those. So this asserts no false
        // NEGATIVES, which is the direction that would puncture the shell.
        let mut rng = Rng(0x2026_0829_1234_5678);
        let half = Vec3::splat(0.5);
        let mut agreed = 0usize;

        for _ in 0..3000 {
            let tri = [rng.next_vec(), rng.next_vec(), rng.next_vec()];
            let sat = tri_aabb_overlap(Vec3::ZERO, half, &tri);

            // Barycentric sampling of the triangle interior.
            let mut sampled_hit = false;
            const N: u32 = 24;
            'sample: for i in 0..=N {
                for j in 0..=(N - i) {
                    let a = i as f32 / N as f32;
                    let b = j as f32 / N as f32;
                    let p = tri[0] * a + tri[1] * b + tri[2] * (1.0 - a - b);
                    if p.x.abs() <= half.x && p.y.abs() <= half.y && p.z.abs() <= half.z {
                        sampled_hit = true;
                        break 'sample;
                    }
                }
            }

            if sampled_hit {
                assert!(sat, "SAT missed an overlap sampling found: {tri:?}");
                agreed += 1;
            }
        }

        // Guard against the test passing vacuously on non-overlapping cases.
        assert!(agreed > 300, "only {agreed} overlapping cases generated");
    }

    #[test]
    fn sat_rejects_clearly_separated_triangles() {
        // The other direction, where it can be asserted safely: a triangle
        // wholly beyond the box on one axis must always separate.
        let mut rng = Rng(0xfeed_face_dead_beef);
        let half = Vec3::splat(0.5);

        for _ in 0..2000 {
            let offset = Vec3::new(10.0, 0.0, 0.0);
            let tri = [
                rng.next_vec() + offset,
                rng.next_vec() + offset,
                rng.next_vec() + offset,
            ];
            assert!(
                !tri_aabb_overlap(Vec3::ZERO, half, &tri),
                "SAT reported overlap for a distant triangle: {tri:?}"
            );
        }
    }

    #[test]
    fn tri_aabb_overlap_basics() {
        let h = Vec3::splat(0.5);
        let inside = [
            Vec3::new(-0.2, -0.2, 0.0),
            Vec3::new(0.2, -0.2, 0.0),
            Vec3::new(0.0, 0.2, 0.0),
        ];
        assert!(tri_aabb_overlap(Vec3::ZERO, h, &inside));

        let far = inside.map(|p| p + Vec3::new(10.0, 0.0, 0.0));
        assert!(!tri_aabb_overlap(Vec3::ZERO, h, &far));

        // A large triangle whose vertices are all outside but which still
        // slices through the box — the case point sampling misses.
        let slicing = [
            Vec3::new(-5.0, 0.0, 0.0),
            Vec3::new(5.0, 0.0, 0.0),
            Vec3::new(0.0, 5.0, 0.1),
        ];
        assert!(tri_aabb_overlap(Vec3::ZERO, h, &slicing));
    }
}
