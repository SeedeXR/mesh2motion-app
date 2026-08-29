//! Geodesic distance from each bone, measured through the mesh interior.
//!
//! Step 3-4 of the pipeline (`docs/algorithms/geodesic-voxel-binding.md`), and
//! the reason this project is replacing the legacy solver at all.
//!
//! The legacy weighting measures Euclidean distance from a vertex to a bone
//! (`legacy/src/lib/solvers/WeightCalculator.ts:71-80`), which travels straight
//! through empty space: a hand resting beside a hip is "near" the hip bone even
//! though no flesh connects them. Distance measured through the voxel interior
//! cannot make that jump — the path has to run up the arm, through the
//! shoulder, and down the torso. That single change is what removes the need
//! for the three per-body-part weight correctors the legacy solver carries.
//!
//! # Resolution floor — the limit of the claim above
//!
//! Two surfaces closer together than roughly **1.5 voxels** land in adjacent
//! voxels and the path leaks between them, restoring the Euclidean shortcut.
//! Measured on two disconnected boxes at resolution 32 (voxel 0.094):
//!
//! | gap | gap in voxels | result |
//! |---|---|---|
//! | 0.05 | 0.53 | leaks |
//! | 0.10 | 1.07 | leaks |
//! | 0.15 | 1.60 | separated |
//! | 0.20+ | 2.13+ | separated |
//!
//! This is inherent to voxel methods, not a defect: a grid cannot resolve a gap
//! it cannot represent. What it means in practice, at
//! [`crate::voxel::DEFAULT_RESOLUTION`] on a 1.75 m human, is a voxel of about
//! 7 mm and a floor of roughly **1 cm**. An A-pose arm hangs 2-5 cm from the
//! ribcage, so it is resolved comfortably; an arm actually touching the body is
//! not, and arguably should not be, since the surfaces genuinely meet there.
//!
//! Raise the resolution when a model has deliberately narrow clearances.
//!
//! # Memory
//!
//! A distance field per bone over the whole grid would be
//! `bones * voxels * 4` bytes — 66 bones over a 256-resolution grid is about
//! 900 MB, well past the budget in `memory/test.md`. Two things avoid that:
//!
//! 1. Only non-exterior voxels participate. On the reference character that is
//!    201k of 3.4M, a 17x reduction, and the exterior is the part that would
//!    dominate.
//! 2. Only distances **at vertices** are kept. Each bone's voxel field is
//!    dropped as soon as it has been sampled, so the retained result is
//!    `vertices * bones * 4` bytes — 1.9 MB for the reference character. Peak
//!    is about twice that during the transpose, plus one `solid_voxels * 4`
//!    scratch field per active worker.

use crate::mesh::Mesh;
use crate::voxel::{VoxelGrid, VoxelState};
use glam::{IVec3, Vec3};
use rayon::prelude::*;
use std::collections::BinaryHeap;

/// A bone as the solver sees it: a segment between two joints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoneSegment {
    /// Joint the bone rotates about.
    pub head: Vec3,
    /// Far end of the bone.
    pub tail: Vec3,
}

/// Sentinel for "no path from this bone to this vertex".
pub const UNREACHABLE: f32 = f32::INFINITY;

/// The 26 neighbours of a voxel, with their Euclidean step lengths.
///
/// 26-connected rather than 6: a 6-connected grid can only travel along axes,
/// so a diagonal path is overestimated by up to 41% and the resulting weights
/// visibly favour the axes. 26-connectivity brings the worst-case anisotropy
/// down to roughly 8%, which the falloff in P1-5 then smooths.
fn neighbours() -> [(IVec3, f32); 26] {
    let mut out = [(IVec3::ZERO, 0.0); 26];
    let mut n = 0;
    let mut i = -1;
    while i <= 1 {
        let mut j = -1;
        while j <= 1 {
            let mut k = -1;
            while k <= 1 {
                if !(i == 0 && j == 0 && k == 0) {
                    let d = IVec3::new(i, j, k);
                    let len = ((i * i + j * j + k * k) as f32).sqrt();
                    out[n] = (d, len);
                    n += 1;
                }
                k += 1;
            }
            j += 1;
        }
        i += 1;
    }
    out
}

/// A compact graph over the voxels a path may travel through.
///
/// Exterior voxels are excluded, which is what confines distance to the inside
/// of the mesh. Surface voxels are **included**: mesh vertices sit on the
/// surface, so excluding them would leave every vertex unreachable.
struct SolidGraph {
    /// Voxel coordinate for each compact node.
    coords: Vec<IVec3>,
    /// Grid index -> compact node, or `u32::MAX` for exterior.
    lookup: Vec<u32>,
}

impl SolidGraph {
    const NONE: u32 = u32::MAX;

    fn build(grid: &VoxelGrid) -> Self {
        let [dx, dy, dz] = grid.dims();
        let mut coords = Vec::new();
        let mut lookup = vec![Self::NONE; grid.len()];

        for z in 0..dz as i32 {
            for y in 0..dy as i32 {
                for x in 0..dx as i32 {
                    let c = IVec3::new(x, y, z);
                    let Some(idx) = grid.index(c) else { continue };
                    if grid.state(c) != Some(VoxelState::Exterior) {
                        lookup[idx] = coords.len() as u32;
                        coords.push(c);
                    }
                }
            }
        }
        Self { coords, lookup }
    }

    fn node_at(&self, grid: &VoxelGrid, c: IVec3) -> Option<u32> {
        let idx = grid.index(c)?;
        let n = self.lookup[idx];
        (n != Self::NONE).then_some(n)
    }

    fn len(&self) -> usize {
        self.coords.len()
    }
}

/// Heap entry ordered by ascending distance.
///
/// `BinaryHeap` is a max-heap, so the comparison is deliberately reversed.
struct Visit(f32, u32);

impl Ord for Visit {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Distances pushed here are always finite and non-NaN by construction:
        // seeds start at 0 and every edge weight is a positive step length.
        //
        // The node index breaks ties so that `cmp` returns Equal exactly when
        // PartialEq does. Without it the Ord/Eq contract is violated, which is
        // harmless for BinaryHeap but silently wrong for any future use in a
        // BTreeSet or binary_search.
        other
            .0
            .partial_cmp(&self.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| other.1.cmp(&self.1))
    }
}

impl PartialOrd for Visit {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialEq for Visit {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == std::cmp::Ordering::Equal
    }
}

impl Eq for Visit {}

/// Geodesic distance from every bone to every vertex.
#[derive(Debug, Clone)]
pub struct GeodesicField {
    vertex_count: usize,
    bone_count: usize,
    /// Row-major `[vertex * bone_count + bone]`.
    distances: Vec<f32>,
}

impl GeodesicField {
    /// Computes the field for every bone, in parallel.
    ///
    /// Returns `None` if the mesh has no vertices or the bone list is empty.
    pub fn compute(mesh: &Mesh, grid: &VoxelGrid, bones: &[BoneSegment]) -> Option<Self> {
        if mesh.positions.is_empty() || bones.is_empty() {
            return None;
        }

        let graph = SolidGraph::build(grid);
        let steps = neighbours();

        // Each vertex's node, resolved once. A vertex whose voxel is exterior
        // (possible for geometry right on the grid edge) has no node and stays
        // unreachable from every bone.
        let vertex_nodes: Vec<Option<u32>> = mesh
            .positions
            .iter()
            .map(|&p| graph.node_at(grid, grid.coord_of(p)))
            .collect();

        // One column per bone, computed independently — the parallel axis.
        let columns: Vec<Vec<f32>> = bones
            .par_iter()
            .map(|bone| {
                let field = dijkstra(grid, &graph, &steps, bone);
                vertex_nodes
                    .iter()
                    .map(|n| n.map_or(UNREACHABLE, |n| field[n as usize]))
                    .collect()
            })
            .collect();

        // Transpose into vertex-major order: P1-5 reads all bones for one
        // vertex at a time, so this is the cache-friendly layout for it.
        let bone_count = bones.len();
        let vertex_count = mesh.positions.len();
        let mut distances = vec![UNREACHABLE; vertex_count * bone_count];
        for (b, column) in columns.iter().enumerate() {
            for (v, &d) in column.iter().enumerate() {
                distances[v * bone_count + b] = d;
            }
        }

        Some(Self {
            vertex_count,
            bone_count,
            distances,
        })
    }

    /// Number of vertices covered.
    pub fn vertex_count(&self) -> usize {
        self.vertex_count
    }

    /// Number of bones covered.
    pub fn bone_count(&self) -> usize {
        self.bone_count
    }

    /// Distance from `bone` to `vertex`, or [`UNREACHABLE`].
    pub fn distance(&self, vertex: usize, bone: usize) -> f32 {
        self.distances[vertex * self.bone_count + bone]
    }

    /// All bone distances for one vertex.
    pub fn vertex_row(&self, vertex: usize) -> &[f32] {
        let start = vertex * self.bone_count;
        &self.distances[start..start + self.bone_count]
    }

    /// Bones that reached no vertex at all.
    ///
    /// This is the signal that a bone sits outside the mesh — the single most
    /// common rigging mistake, and something the UI must surface rather than
    /// silently producing a rig with a dead limb.
    pub fn unreachable_bones(&self) -> Vec<usize> {
        (0..self.bone_count)
            .filter(|&b| (0..self.vertex_count).all(|v| !self.distance(v, b).is_finite()))
            .collect()
    }

    /// Vertices no bone could reach.
    ///
    /// Non-empty means an island the voxel grid did not connect to anything —
    /// eyes or teeth modelled as separate closed shells, for example. P1-5 must
    /// fall back to nearest-bone for these rather than leave them unweighted.
    pub fn unreachable_vertices(&self) -> Vec<usize> {
        (0..self.vertex_count)
            .filter(|&v| !self.vertex_row(v).iter().any(|d| d.is_finite()))
            .collect()
    }
}

/// Dijkstra from the voxels a bone passes through.
fn dijkstra(
    grid: &VoxelGrid,
    graph: &SolidGraph,
    steps: &[(IVec3, f32); 26],
    bone: &BoneSegment,
) -> Vec<f32> {
    let mut dist = vec![UNREACHABLE; graph.len()];
    let mut heap = BinaryHeap::new();

    for node in seed_nodes(grid, graph, bone) {
        if dist[node as usize] != 0.0 {
            dist[node as usize] = 0.0;
            heap.push(Visit(0.0, node));
        }
    }

    let scale = grid.voxel_size();
    while let Some(Visit(d, node)) = heap.pop() {
        // Stale entry: a shorter path to this node was already settled.
        if d > dist[node as usize] {
            continue;
        }
        let c = graph.coords[node as usize];
        for &(offset, len) in steps.iter() {
            let Some(next) = graph.node_at(grid, c + offset) else {
                continue;
            };
            let candidate = d + len * scale;
            if candidate < dist[next as usize] {
                dist[next as usize] = candidate;
                heap.push(Visit(candidate, next));
            }
        }
    }

    dist
}

/// Voxels along the bone segment that lie inside the mesh.
///
/// Sampled at half-voxel spacing, which cannot skip a voxel. A bone entirely
/// outside the mesh yields no seeds, and every distance from it stays
/// [`UNREACHABLE`] — see [`GeodesicField::unreachable_bones`].
fn seed_nodes(grid: &VoxelGrid, graph: &SolidGraph, bone: &BoneSegment) -> Vec<u32> {
    let span = bone.tail - bone.head;
    let length = span.length();
    let samples = if length > 0.0 {
        ((length / (grid.voxel_size() * 0.5)).ceil() as usize).max(1)
    } else {
        1
    };

    let mut out = Vec::new();
    let mut last = IVec3::splat(i32::MIN);
    for i in 0..=samples {
        let t = i as f32 / samples as f32;
        let c = grid.coord_of(bone.head + span * t);
        if c == last {
            continue;
        }
        last = c;
        if let Some(n) = graph.node_at(grid, c) {
            out.push(n);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Axis-aligned box as 12 triangles, appended to existing buffers.
    fn push_box(positions: &mut Vec<f32>, indices: &mut Vec<u32>, lo: Vec3, hi: Vec3) {
        let base = (positions.len() / 3) as u32;
        for corner in [
            Vec3::new(lo.x, lo.y, lo.z),
            Vec3::new(hi.x, lo.y, lo.z),
            Vec3::new(hi.x, hi.y, lo.z),
            Vec3::new(lo.x, hi.y, lo.z),
            Vec3::new(lo.x, lo.y, hi.z),
            Vec3::new(hi.x, lo.y, hi.z),
            Vec3::new(hi.x, hi.y, hi.z),
            Vec3::new(lo.x, hi.y, hi.z),
        ] {
            positions.extend_from_slice(&[corner.x, corner.y, corner.z]);
        }
        for f in [
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 1, 5],
            [0, 5, 4],
            [3, 7, 6],
            [3, 6, 2],
            [0, 4, 7],
            [0, 7, 3],
            [1, 2, 6],
            [1, 6, 5],
        ] {
            indices.extend(f.iter().map(|i| base + i));
        }
    }

    fn single_box(lo: Vec3, hi: Vec3) -> Mesh {
        let (mut p, mut i) = (Vec::new(), Vec::new());
        push_box(&mut p, &mut i, lo, hi);
        Mesh::from_flat(&p, &i).expect("valid box")
    }

    /// Two prongs joined by a base — geometry where Euclidean and geodesic
    /// distance disagree sharply, like an arm hanging beside a torso.
    ///
    /// ```text
    ///   |    |        prongs at x 0..1 and 2..3, y 1..4
    ///   |    |        gap between them is empty space
    ///   +----+        base at y 0..1 joins them
    /// ```
    fn u_shape() -> Mesh {
        let (mut p, mut i) = (Vec::new(), Vec::new());
        push_box(
            &mut p,
            &mut i,
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(3.0, 1.0, 1.0),
        );
        push_box(
            &mut p,
            &mut i,
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 4.0, 1.0),
        );
        push_box(
            &mut p,
            &mut i,
            Vec3::new(2.0, 1.0, 0.0),
            Vec3::new(3.0, 4.0, 1.0),
        );
        Mesh::from_flat(&p, &i).expect("valid U")
    }

    #[test]
    fn distance_grows_with_separation() {
        let mesh = single_box(Vec3::ZERO, Vec3::new(1.0, 4.0, 1.0));
        let grid = VoxelGrid::build(&mesh, 32).expect("grid");
        let bone = BoneSegment {
            head: Vec3::new(0.5, 0.2, 0.5),
            tail: Vec3::new(0.5, 0.5, 0.5),
        };
        let field = GeodesicField::compute(&mesh, &grid, &[bone]).expect("field");

        // Vertices at y=0 are near the bone; vertices at y=4 are far.
        let near: Vec<f32> = (0..mesh.vertex_count())
            .filter(|&v| mesh.positions[v].y < 0.5)
            .map(|v| field.distance(v, 0))
            .collect();
        let far: Vec<f32> = (0..mesh.vertex_count())
            .filter(|&v| mesh.positions[v].y > 3.5)
            .map(|v| field.distance(v, 0))
            .collect();

        assert!(!near.is_empty() && !far.is_empty());
        let max_near = near.iter().cloned().fold(0.0f32, f32::max);
        let min_far = far.iter().cloned().fold(f32::INFINITY, f32::min);
        assert!(
            min_far > max_near * 2.0,
            "far {min_far} should greatly exceed near {max_near}"
        );
    }

    #[test]
    fn geodesic_defeats_the_euclidean_shortcut() {
        // The property the whole solver rests on. A bone in the left prong is
        // Euclidean-close to the right prong across the gap, but geodesically
        // it must travel down, across the base, and back up.
        //
        // This is the arm-beside-a-ribcage case that forces the legacy solver
        // to carry ArmWeightCorrector and ExtremityWeightCorrector.
        let mesh = u_shape();
        let grid = VoxelGrid::build(&mesh, 48).expect("grid");
        let bone = BoneSegment {
            head: Vec3::new(0.5, 2.5, 0.5),
            tail: Vec3::new(0.5, 3.5, 0.5),
        };
        let field = GeodesicField::compute(&mesh, &grid, &[bone]).expect("field");

        // The nearest point of the opposite prong, straight across the gap.
        let probe = Vec3::new(2.0, 3.0, 0.5);
        let target = (0..mesh.vertex_count())
            .min_by(|&a, &b| {
                let da = mesh.positions[a].distance(probe);
                let db = mesh.positions[b].distance(probe);
                da.partial_cmp(&db).unwrap()
            })
            .expect("a vertex");

        let euclidean = mesh.positions[target].distance(Vec3::new(0.5, 3.0, 0.5));
        let geodesic = field.distance(target, 0);
        assert!(
            geodesic.is_finite(),
            "opposite prong must still be reachable"
        );

        // Straight across is ~1.5 units; around the U is ~2 down + 2 across +
        // 2 up. Requiring 2.5x proves the path is not cutting through air.
        assert!(
            geodesic > euclidean * 2.5,
            "geodesic {geodesic:.2} vs euclidean {euclidean:.2}: \
             distance is leaking across the gap"
        );
    }

    #[test]
    fn narrow_gaps_leak_below_the_resolution_floor() {
        // The documented limit of the module's central claim. Two surfaces
        // closer than ~1.5 voxels land in adjacent voxels and the path leaks,
        // restoring the Euclidean shortcut. This is inherent to voxel methods —
        // a grid cannot resolve a gap it cannot represent — but it must be
        // pinned so the threshold cannot drift without anyone noticing.
        let separated = |gap: f32| -> bool {
            let (mut p, mut i) = (Vec::new(), Vec::new());
            push_box(&mut p, &mut i, Vec3::ZERO, Vec3::new(1.0, 3.0, 1.0));
            push_box(
                &mut p,
                &mut i,
                Vec3::new(1.0 + gap, 0.0, 0.0),
                Vec3::new(2.0 + gap, 3.0, 1.0),
            );
            let mesh = Mesh::from_flat(&p, &i).unwrap();
            let grid = VoxelGrid::build(&mesh, 32).unwrap();
            let bone = BoneSegment {
                head: Vec3::new(0.5, 1.4, 0.5),
                tail: Vec3::new(0.5, 1.6, 0.5),
            };
            let field = GeodesicField::compute(&mesh, &grid, &[bone]).unwrap();
            let far = (0..mesh.vertex_count())
                .find(|&v| mesh.positions[v].x > 1.0 + gap + 0.5)
                .expect("a vertex on the far box");
            !field.distance(far, 0).is_finite()
        };

        // Voxel size here is 3.0/32 = 0.094.
        assert!(!separated(0.05), "0.53 voxels: expected to leak");
        assert!(!separated(0.10), "1.07 voxels: expected to leak");
        assert!(separated(0.15), "1.60 voxels: expected to separate");
        assert!(separated(0.30), "3.20 voxels: expected to separate");
    }

    #[test]
    fn reports_a_bone_outside_the_mesh() {
        // The most common rigging mistake, and something the UI has to surface
        // rather than silently shipping a rig with a dead limb.
        let mesh = single_box(Vec3::ZERO, Vec3::ONE);
        let grid = VoxelGrid::build(&mesh, 24).expect("grid");
        let inside = BoneSegment {
            head: Vec3::new(0.4, 0.4, 0.5),
            tail: Vec3::new(0.6, 0.6, 0.5),
        };
        let outside = BoneSegment {
            head: Vec3::new(9.0, 9.0, 9.0),
            tail: Vec3::new(9.5, 9.5, 9.5),
        };
        let field = GeodesicField::compute(&mesh, &grid, &[inside, outside]).expect("field");

        assert_eq!(field.unreachable_bones(), vec![1]);
        assert!(field.distance(0, 0).is_finite());
        assert!(!field.distance(0, 1).is_finite());
    }

    #[test]
    fn disconnected_island_is_reported_not_silently_zero() {
        // A separate closed shell far away: no voxel path connects it, so every
        // one of its vertices is unreachable. P1-5 must fall back to
        // nearest-bone for these rather than leave them unweighted.
        let (mut p, mut i) = (Vec::new(), Vec::new());
        push_box(&mut p, &mut i, Vec3::ZERO, Vec3::ONE);
        push_box(&mut p, &mut i, Vec3::splat(5.0), Vec3::splat(6.0));
        let mesh = Mesh::from_flat(&p, &i).unwrap();

        let grid = VoxelGrid::build(&mesh, 48).expect("grid");
        let bone = BoneSegment {
            head: Vec3::new(0.4, 0.5, 0.5),
            tail: Vec3::new(0.6, 0.5, 0.5),
        };
        let field = GeodesicField::compute(&mesh, &grid, &[bone]).expect("field");

        let stranded = field.unreachable_vertices();
        assert_eq!(stranded.len(), 8, "the far cube's 8 corners are stranded");
        assert!(stranded.iter().all(|&v| mesh.positions[v].x > 4.0));
    }

    #[test]
    fn rejects_empty_input() {
        let mesh = single_box(Vec3::ZERO, Vec3::ONE);
        let grid = VoxelGrid::build(&mesh, 16).unwrap();
        assert!(GeodesicField::compute(&mesh, &grid, &[]).is_none());
    }

    #[test]
    fn is_deterministic() {
        // rayon runs bones concurrently; if any shared state leaked, repeated
        // runs would differ and every downstream golden test would be flaky.
        let mesh = u_shape();
        let grid = VoxelGrid::build(&mesh, 32).unwrap();
        let bones: Vec<BoneSegment> = (0..8)
            .map(|i| {
                let y = 0.5 + i as f32 * 0.4;
                BoneSegment {
                    head: Vec3::new(0.5, y, 0.5),
                    tail: Vec3::new(0.5, y + 0.3, 0.5),
                }
            })
            .collect();
        let a = GeodesicField::compute(&mesh, &grid, &bones).unwrap();
        let b = GeodesicField::compute(&mesh, &grid, &bones).unwrap();
        assert_eq!(a.distances, b.distances);
    }
}
