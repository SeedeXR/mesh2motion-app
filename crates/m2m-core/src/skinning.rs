//! Skinning weights from the geodesic distance field.
//!
//! Steps 5-7 of the pipeline (`docs/algorithms/geodesic-voxel-binding.md`):
//! turn per-bone geodesic distances into at most four normalised influences
//! per vertex.
//!
//! # Why this replaces four legacy passes
//!
//! The legacy solver assigns each vertex to exactly one bone and then repairs
//! the result with `ExtremityWeightCorrector`, `ArmWeightCorrector`,
//! `HeadWeightCorrector` and `WeightSmoother`. Measured on the shipping
//! templates, 68% of its vertices end up with a single influence
//! (`bench/baselines/legacy-solver.json`), which is why the seams need
//! smoothing at all. Blending the nearest few bones by geodesic falloff
//! produces smooth boundaries directly, so there is nothing left for those
//! passes to fix.

use crate::geodesic::GeodesicField;
use glam::Vec3;

/// Maximum bones influencing a single vertex.
///
/// Four is the GPU skinning limit that glTF, FBX and every real-time engine
/// assume; exceeding it silently truncates downstream.
pub const MAX_INFLUENCES: usize = 4;

/// Default sharpness of the distance falloff.
///
/// Higher concentrates weight on the nearest bone and stiffens joints; lower
/// spreads influence further and softens them. 2.0 is the usual inverse-square
/// choice and is the starting point for the artist-facing control in P3.
pub const DEFAULT_FALLOFF: f32 = 2.0;

/// Per-vertex skinning weights: bone indices and their normalised influences.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SkinWeights {
    /// Bone index per influence, `MAX_INFLUENCES` per vertex.
    pub indices: Vec<u16>,
    /// Influence weight per bone, `MAX_INFLUENCES` per vertex, summing to 1.0.
    pub weights: Vec<f32>,
    /// Vertices no bone reached geodesically, resolved by Euclidean fallback.
    ///
    /// Non-empty is normal — a real character has islands like eyes and teeth
    /// (61 components on the reference mesh). It is reported so the UI can show
    /// which regions were guessed rather than solved.
    pub fallback_vertices: Vec<u32>,
}

impl SkinWeights {
    /// Allocates zeroed weights for `vertex_count` vertices.
    pub fn zeroed(vertex_count: usize) -> Self {
        Self {
            indices: vec![0; vertex_count * MAX_INFLUENCES],
            weights: vec![0.0; vertex_count * MAX_INFLUENCES],
            fallback_vertices: Vec::new(),
        }
    }

    /// Number of vertices these weights cover.
    pub fn vertex_count(&self) -> usize {
        self.weights.len() / MAX_INFLUENCES
    }

    /// Returns the index of the first vertex whose weights are not a valid
    /// normalised set, or `None` if every vertex is.
    ///
    /// Invariants 1 and 2 from `memory/test.md` §3. The `is_finite` check is
    /// not incidental: `(NaN - 1.0).abs() > tolerance` is **false**, so a
    /// sum-only test reports a NaN vertex as correctly normalised — and this is
    /// the assertion the whole solver's tests lean on.
    pub fn first_unnormalised(&self, tolerance: f32) -> Option<usize> {
        self.weights.chunks_exact(MAX_INFLUENCES).position(|w| {
            !w.iter().all(|x| x.is_finite() && *x >= 0.0)
                || (w.iter().sum::<f32>() - 1.0).abs() > tolerance
        })
    }

    /// Influences of one vertex as `(bone, weight)` pairs, zero weights omitted.
    pub fn influences(&self, vertex: usize) -> impl Iterator<Item = (u16, f32)> + '_ {
        let start = vertex * MAX_INFLUENCES;
        (start..start + MAX_INFLUENCES)
            .filter(move |&i| self.weights[i] > 0.0)
            .map(move |i| (self.indices[i], self.weights[i]))
    }
}

/// Smallest and largest usable falloff exponent.
///
/// Zero would make every candidate weigh the same (`0f32.powf(0.0) == 1.0`),
/// collapsing the falloff to uniform blending; negative turns a zero base into
/// infinity and then NaN. This is a slider in the UI, so the bounds are
/// enforced here rather than trusted.
pub const FALLOFF_RANGE: std::ops::RangeInclusive<f32> = 0.25..=8.0;

/// Tunables for weight assignment.
#[derive(Debug, Clone, Copy)]
pub struct SkinningParams {
    falloff: f32,
}

impl SkinningParams {
    /// Builds parameters, clamping `falloff` into [`FALLOFF_RANGE`].
    ///
    /// A non-finite value falls back to [`DEFAULT_FALLOFF`].
    pub fn new(falloff: f32) -> Self {
        let falloff = if falloff.is_finite() {
            falloff.clamp(*FALLOFF_RANGE.start(), *FALLOFF_RANGE.end())
        } else {
            DEFAULT_FALLOFF
        };
        Self { falloff }
    }

    /// The validated falloff exponent.
    pub fn falloff(&self) -> f32 {
        self.falloff
    }
}

impl Default for SkinningParams {
    fn default() -> Self {
        Self::new(DEFAULT_FALLOFF)
    }
}

/// Which bones may receive weight.
///
/// `m2m-core` deliberately knows nothing about bone names or hierarchy, so the
/// caller decides. This is how the legacy invariant is preserved — the root
/// bone carries global transform only, and leaf bones exist to orient their
/// parent, so neither may hold weight
/// (`legacy/src/lib/solvers/WeightCalculator.ts` `initialize_caches`). Encoding
/// that here would put a naming convention in the geometry layer.
pub type BoneMask<'a> = &'a [bool];

/// Assigns weights from geodesic distance.
///
/// `positions` and `bone_segments` are used only for the Euclidean fallback on
/// vertices the field could not reach. `allowed` must have one entry per bone;
/// pass all-true to weight every bone.
///
/// # Panics
///
/// Panics if the slice lengths disagree with the field, or if no bone is
/// allowed. All are caller bugs rather than bad data, and every one of them
/// would otherwise surface as an unnormalised vertex much later.
pub fn assign_weights(
    field: &GeodesicField,
    positions: &[Vec3],
    bone_segments: &[crate::geodesic::BoneSegment],
    allowed: BoneMask<'_>,
    params: SkinningParams,
) -> SkinWeights {
    assert_eq!(
        allowed.len(),
        field.bone_count(),
        "bone mask length must match the field's bone count"
    );
    assert_eq!(
        bone_segments.len(),
        field.bone_count(),
        "bone segment count must match the field's bone count"
    );
    assert!(
        positions.len() >= field.vertex_count(),
        "positions must cover every vertex in the field"
    );
    // Without this the Euclidean fallback has nothing to choose and leaves the
    // vertex at all-zero weights — silently breaking invariant 1, on a path
    // only taken for meshes that happen to have unreachable vertices.
    assert!(
        allowed.iter().any(|&a| a),
        "at least one bone must be allowed to receive weight"
    );

    let mut out = SkinWeights::zeroed(field.vertex_count());
    // Scratch reused across vertices: (bone, distance) for the best few.
    let mut best: Vec<(u16, f32)> = Vec::with_capacity(MAX_INFLUENCES + 1);

    // The index addresses four different structures — the field's rows, the
    // positions, and both output slices — so iterating one of them would not
    // remove the indexing, only hide which is being iterated.
    #[allow(clippy::needless_range_loop)]
    for v in 0..field.vertex_count() {
        best.clear();
        collect_nearest(field.vertex_row(v), allowed, &mut best);

        if best.is_empty() {
            // No bone reached this vertex through the mesh — an island the
            // voxel grid never connected. Falling back to the nearest bone by
            // straight-line distance is a guess, but leaving the vertex
            // unweighted would detach it from the rig entirely.
            out.fallback_vertices.push(v as u32);
            let bone = nearest_euclidean(positions[v], bone_segments, allowed)
                .expect("at least one bone is allowed, checked above");
            out.indices[v * MAX_INFLUENCES] = bone;
            out.weights[v * MAX_INFLUENCES] = 1.0;
            continue;
        }

        write_falloff(&best, params.falloff(), &mut out, v);
    }

    out
}

/// Fills `best` with up to `MAX_INFLUENCES + 1` nearest allowed bones, ascending.
///
/// One extra beyond the limit: the surplus entry becomes the cutoff distance at
/// which influence reaches zero, which is what makes the falloff continuous
/// instead of stepping at the fourth bone.
fn collect_nearest(row: &[f32], allowed: BoneMask<'_>, best: &mut Vec<(u16, f32)>) {
    for (bone, &d) in row.iter().enumerate() {
        if !allowed[bone] || !d.is_finite() {
            continue;
        }
        let entry = (bone as u16, d);
        // partition_point over (distance, bone) so an exact tie resolves by
        // ascending bone index. Tie-breaking by insertion order instead would
        // let a later bone evict an earlier one at the truncation boundary,
        // which breaks invariant 8: a mirrored mesh with exact ties between
        // mirrored bone pairs would not produce mirrored weights.
        let at = best.partition_point(|&(b, existing)| (existing, b) < (d, bone as u16));
        if at < MAX_INFLUENCES + 1 {
            best.insert(at, entry);
            best.truncate(MAX_INFLUENCES + 1);
        }
    }
}

/// Converts sorted `(bone, distance)` pairs into normalised weights.
///
/// Does nothing for an empty slice.
fn write_falloff(best: &[(u16, f32)], falloff: f32, out: &mut SkinWeights, vertex: usize) {
    let base = vertex * MAX_INFLUENCES;
    let Some(&(nearest_bone, nearest_d)) = best.first() else {
        return;
    };

    // A vertex sitting exactly on a bone belongs entirely to it. Without this
    // the reciprocal below is infinite and normalisation produces NaN.
    // NaN cannot reach here: `collect_nearest` drops non-finite distances.
    if nearest_d <= 0.0 {
        out.indices[base] = nearest_bone;
        out.weights[base] = 1.0;
        return;
    }

    let used = best.len().min(MAX_INFLUENCES);

    // Work in units of the nearest distance rather than raw mesh units.
    //
    // This is what keeps the reciprocal safe: every ratio is >= 1, so `1/u` is
    // in (0, 1] and neither the division nor `powf` can overflow. Raw distances
    // could not promise that — `1.0 / d` is infinite for any d below ~2.9e-39,
    // which is reachable on a small mesh, and the result was NaN after
    // normalisation. It also makes invariant 7 (scale invariance) exact rather
    // than approximate, since a uniform scale cancels in the ratio.
    let inv_cutoff = if best.len() > MAX_INFLUENCES {
        // Modified Shepard: weight reaches exactly zero at the surplus
        // (k+1)-th distance, so a bone entering or leaving the top four does
        // not step the result.
        nearest_d / best[MAX_INFLUENCES].1
    } else {
        // No surplus bone means nothing to fade toward, so the cutoff is
        // effectively infinite and this degrades to inverse-distance weighting.
        // That is the continuous limit of the branch above as the (k+1)-th
        // distance grows, not a separate rule.
        0.0
    };

    let mut total = 0.0f32;
    for (slot, &(bone, d)) in best.iter().take(used).enumerate() {
        let inv_u = nearest_d / d;
        let w = (inv_u - inv_cutoff).max(0.0).powf(falloff);
        out.indices[base + slot] = bone;
        out.weights[base + slot] = w;
        total += w;
    }

    if total > 0.0 && total.is_finite() {
        for slot in 0..used {
            out.weights[base + slot] /= total;
        }
    } else {
        // Everything underflowed at a high falloff exponent. Give it all to the
        // nearest bone, and clear the other slots — leaving them would strand
        // whatever the loop wrote while the vertex looked repaired.
        out.indices[base] = nearest_bone;
        out.weights[base] = 1.0;
        for slot in 1..MAX_INFLUENCES {
            out.indices[base + slot] = 0;
            out.weights[base + slot] = 0.0;
        }
    }
}

/// Nearest allowed bone by straight-line distance to its segment.
fn nearest_euclidean(
    p: Vec3,
    bones: &[crate::geodesic::BoneSegment],
    allowed: BoneMask<'_>,
) -> Option<u16> {
    let mut best: Option<(u16, f32)> = None;
    for (i, bone) in bones.iter().enumerate() {
        if !allowed[i] {
            continue;
        }
        let ab = bone.tail - bone.head;
        let len_sq = ab.length_squared();
        let t = if len_sq > 0.0 {
            ((p - bone.head).dot(ab) / len_sq).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let d = p.distance(bone.head + ab * t);
        if best.is_none_or(|(_, bd)| d < bd) {
            best = Some((i as u16, d));
        }
    }
    best.map(|(i, _)| i)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geodesic::{BoneSegment, GeodesicField};
    use crate::mesh::Mesh;
    use crate::voxel::VoxelGrid;

    fn push_box(p: &mut Vec<f32>, i: &mut Vec<u32>, lo: Vec3, hi: Vec3) {
        let base = (p.len() / 3) as u32;
        for c in [
            Vec3::new(lo.x, lo.y, lo.z),
            Vec3::new(hi.x, lo.y, lo.z),
            Vec3::new(hi.x, hi.y, lo.z),
            Vec3::new(lo.x, hi.y, lo.z),
            Vec3::new(lo.x, lo.y, hi.z),
            Vec3::new(hi.x, lo.y, hi.z),
            Vec3::new(hi.x, hi.y, hi.z),
            Vec3::new(lo.x, hi.y, hi.z),
        ] {
            p.extend_from_slice(&[c.x, c.y, c.z]);
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
            i.extend(f.iter().map(|k| base + k));
        }
    }

    fn bar() -> Mesh {
        #[rustfmt::skip]
        let p = [
            0.0f32, 0.0, 0.0,  1.0, 0.0, 0.0,  1.0, 4.0, 0.0,  0.0, 4.0, 0.0,
            0.0, 0.0, 1.0,  1.0, 0.0, 1.0,  1.0, 4.0, 1.0,  0.0, 4.0, 1.0,
        ];
        #[rustfmt::skip]
        let i = [
            0,2,1, 0,3,2, 4,5,6, 4,6,7, 0,1,5, 0,5,4,
            3,7,6, 3,6,2, 0,4,7, 0,7,3, 1,2,6, 1,6,5,
        ];
        Mesh::from_flat(&p, &i).unwrap()
    }

    /// A chain of bones running up the bar, like a spine.
    fn chain(n: usize) -> Vec<BoneSegment> {
        (0..n)
            .map(|i| {
                let step = 4.0 / n as f32;
                BoneSegment {
                    head: Vec3::new(0.5, i as f32 * step, 0.5),
                    tail: Vec3::new(0.5, (i + 1) as f32 * step, 0.5),
                }
            })
            .collect()
    }

    fn solve(mesh: &Mesh, bones: &[BoneSegment], allowed: &[bool]) -> SkinWeights {
        let grid = VoxelGrid::build(mesh, 32).expect("grid");
        let field = GeodesicField::compute(mesh, &grid, bones).expect("field");
        assign_weights(
            &field,
            &mesh.positions,
            bones,
            allowed,
            SkinningParams::default(),
        )
    }

    #[test]
    fn weights_satisfy_every_invariant() {
        // Invariants 1, 2, 3 and 5 from memory/test.md §3. Invariant 4 is
        // covered by masked_bones_never_receive_weight, 6 by is_deterministic,
        // 7 by is_scale_invariant. Invariant 8 (mirror symmetry) is not yet
        // covered anywhere — see todo P1-7.
        let mesh = bar();
        let bones = chain(6);
        let allowed = vec![true; bones.len()];
        let w = solve(&mesh, &bones, &allowed);

        assert_eq!(w.vertex_count(), mesh.vertex_count());
        assert_eq!(w.first_unnormalised(1e-5), None, "must sum to 1.0");

        for (v, chunk) in w.weights.chunks_exact(MAX_INFLUENCES).enumerate() {
            for &x in chunk {
                assert!(x.is_finite(), "vertex {v} has a non-finite weight");
                assert!(x >= 0.0, "vertex {v} has a negative weight {x}");
            }
            assert!(
                chunk.iter().filter(|x| **x > 0.0).count() <= MAX_INFLUENCES,
                "vertex {v} exceeds the influence limit"
            );
        }
        for &b in &w.indices {
            assert!((b as usize) < bones.len(), "bone index {b} out of range");
        }
    }

    #[test]
    fn masked_bones_never_receive_weight() {
        // How the legacy invariant is preserved: the root bone carries only the
        // global transform and leaf bones only orient their parent, so neither
        // may hold weight. Here bone 0 stands in for the root.
        let mesh = bar();
        let bones = chain(6);
        let mut allowed = vec![true; bones.len()];
        allowed[0] = false;
        allowed[5] = false;

        let w = solve(&mesh, &bones, &allowed);
        assert_eq!(w.first_unnormalised(1e-5), None);

        for v in 0..w.vertex_count() {
            for (bone, weight) in w.influences(v) {
                assert!(
                    bone != 0 && bone != 5,
                    "vertex {v} got weight {weight} on masked bone {bone}"
                );
            }
        }
    }

    #[test]
    fn blends_rather_than_assigning_one_bone() {
        // The whole point. The legacy solver leaves 68% of vertices with a
        // single influence, which is why it needs a smoothing pass; blending
        // the nearest bones by falloff should produce far more shared vertices.
        let mesh = bar();
        let bones = chain(8);
        let allowed = vec![true; bones.len()];
        let w = solve(&mesh, &bones, &allowed);

        let total: usize = (0..w.vertex_count()).map(|v| w.influences(v).count()).sum();
        let mean = total as f32 / w.vertex_count() as f32;
        assert!(
            mean > 2.0,
            "mean influences {mean:.2} is barely better than rigid assignment"
        );
    }

    #[test]
    fn a_vertex_on_a_bone_belongs_entirely_to_it() {
        // Guards the reciprocal in the falloff: distance zero would otherwise
        // produce an infinite weight and then NaN after normalisation.
        let mesh = bar();
        let bones = chain(4);
        let allowed = vec![true; bones.len()];
        let grid = VoxelGrid::build(&mesh, 32).unwrap();
        let field = GeodesicField::compute(&mesh, &grid, &bones).unwrap();

        // Force an exact hit by weighting a synthetic zero-distance row.
        let mut w = SkinWeights::zeroed(1);
        write_falloff(&[(2, 0.0), (1, 0.5), (3, 0.9)], 2.0, &mut w, 0);
        assert_eq!(w.weights[0], 1.0);
        assert_eq!(w.indices[0], 2);
        assert_eq!(w.first_unnormalised(1e-6), None);

        // And the real solve stays finite everywhere.
        let real = assign_weights(
            &field,
            &mesh.positions,
            &bones,
            &allowed,
            SkinningParams::default(),
        );
        assert!(real.weights.iter().all(|x| x.is_finite()));
    }

    #[test]
    fn stranded_vertices_fall_back_instead_of_going_unweighted() {
        // A separate island the voxel grid never connects. Eyes and teeth are
        // real instances of this, so it must produce a usable weight and be
        // reported, not silently left at zero.
        let mut p: Vec<f32> = bar()
            .positions
            .iter()
            .flat_map(|v| [v.x, v.y, v.z])
            .collect();
        let mut i = bar().indices.clone();
        let base = (p.len() / 3) as u32;
        for c in [
            Vec3::new(9.0, 9.0, 9.0),
            Vec3::new(10.0, 9.0, 9.0),
            Vec3::new(10.0, 10.0, 9.0),
            Vec3::new(9.0, 10.0, 9.0),
            Vec3::new(9.0, 9.0, 10.0),
            Vec3::new(10.0, 9.0, 10.0),
            Vec3::new(10.0, 10.0, 10.0),
            Vec3::new(9.0, 10.0, 10.0),
        ] {
            p.extend_from_slice(&[c.x, c.y, c.z]);
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
            i.extend(f.iter().map(|k| base + k));
        }
        let mesh = Mesh::from_flat(&p, &i).unwrap();
        let bones = chain(4);
        let allowed = vec![true; bones.len()];

        let w = solve(&mesh, &bones, &allowed);
        assert_eq!(w.fallback_vertices.len(), 8, "the island's 8 corners");
        assert_eq!(
            w.first_unnormalised(1e-5),
            None,
            "fallbacks still normalise"
        );
        for &v in &w.fallback_vertices {
            assert_eq!(w.influences(v as usize).count(), 1);
        }
    }

    #[test]
    fn is_scale_invariant() {
        // Invariant 7. Bone assignment must be exactly scale-invariant; weight
        // values converge only as the grid refines, because the geodesic path
        // is a chain of discrete voxel steps. See
        // scale_invariance_converges_with_resolution in tests/invariants.rs for
        // the measured convergence this tolerance comes from.
        let solve_at = |scale: f32| {
            let base = bar();
            let p: Vec<f32> = base
                .positions
                .iter()
                .flat_map(|v| [v.x * scale, v.y * scale, v.z * scale])
                .collect();
            let mesh = Mesh::from_flat(&p, &base.indices).unwrap();
            let bones: Vec<BoneSegment> = chain(6)
                .into_iter()
                .map(|b| BoneSegment {
                    head: b.head * scale,
                    tail: b.tail * scale,
                })
                .collect();
            let allowed = vec![true; bones.len()];
            solve(&mesh, &bones, &allowed)
        };

        let a = solve_at(1.0);
        let b = solve_at(10.0);
        assert_eq!(
            a.indices, b.indices,
            "bone choice must be exactly scale-invariant"
        );
        for (x, y) in a.weights.iter().zip(b.weights.iter()) {
            assert!((x - y).abs() < 0.1, "weights diverged: {x} vs {y}");
        }
    }

    /// Mirrors a mesh through the X axis, keeping vertex indices aligned so
    /// vertex `i` in the result is the mirror of vertex `i` in the input.
    fn mirror_mesh(m: &Mesh) -> Mesh {
        let p: Vec<f32> = m.positions.iter().flat_map(|v| [-v.x, v.y, v.z]).collect();
        // Reverse winding so the mirrored mesh is not inside out. The
        // voxeliser is occupancy-based and does not read winding, but a mesh
        // that only works because nothing looks at its normals is a trap for
        // whoever adds a normal-dependent step later.
        let i: Vec<u32> = m
            .indices
            .chunks_exact(3)
            .flat_map(|t| [t[0], t[2], t[1]])
            .collect();
        Mesh::from_flat(&p, &i).expect("mirrored mesh is valid")
    }

    fn mirror_bones(b: &[BoneSegment]) -> Vec<BoneSegment> {
        b.iter()
            .map(|s| BoneSegment {
                head: Vec3::new(-s.head.x, s.head.y, s.head.z),
                tail: Vec3::new(-s.tail.x, s.tail.y, s.tail.z),
            })
            .collect()
    }

    #[test]
    fn mirroring_everything_leaves_weights_unchanged() {
        // Invariant 8, first half: the solver must have no preferred handedness.
        // Mirroring the mesh and the skeleton together maps vertex i to vertex i
        // and bone j to bone j, so the weights should come back bit-identical.
        // Anything that leaked an axis bias — a tie-break on coordinate order, a
        // scan direction in the voxeliser — shows up here.
        let mesh = bar();
        let bones = chain(6);
        let allowed = vec![true; bones.len()];

        let original = solve(&mesh, &bones, &allowed);
        let mirrored = solve(&mirror_mesh(&mesh), &mirror_bones(&bones), &allowed);

        assert_eq!(
            original.indices, mirrored.indices,
            "bone choice differs under mirroring"
        );
        for (a, b) in original.weights.iter().zip(mirrored.weights.iter()) {
            assert!(
                (a - b).abs() < 1e-6,
                "weights differ under mirroring: {a} vs {b}"
            );
        }
    }

    /// Worst weight difference between mirrored vertex pairs, and the number of
    /// pairs whose *bone sets* disagree, at a given resolution.
    fn symmetry_error(res: u32) -> (f32, usize) {
        // A cross: a central column with one arm on each side.
        let (mut p, mut i) = (Vec::new(), Vec::new());
        push_box(
            &mut p,
            &mut i,
            Vec3::new(-0.5, 0.0, 0.0),
            Vec3::new(0.5, 4.0, 1.0),
        );
        push_box(
            &mut p,
            &mut i,
            Vec3::new(-3.0, 2.0, 0.0),
            Vec3::new(-0.5, 3.0, 1.0),
        );
        push_box(
            &mut p,
            &mut i,
            Vec3::new(0.5, 2.0, 0.0),
            Vec3::new(3.0, 3.0, 1.0),
        );
        let mesh = Mesh::from_flat(&p, &i).expect("valid cross");

        // Bone 0 spine, then mirrored arm pairs: (1,2) inner, (3,4) outer.
        let bones = vec![
            BoneSegment {
                head: Vec3::new(0.0, 1.0, 0.5),
                tail: Vec3::new(0.0, 3.0, 0.5),
            },
            BoneSegment {
                head: Vec3::new(-0.5, 2.5, 0.5),
                tail: Vec3::new(-1.75, 2.5, 0.5),
            },
            BoneSegment {
                head: Vec3::new(0.5, 2.5, 0.5),
                tail: Vec3::new(1.75, 2.5, 0.5),
            },
            BoneSegment {
                head: Vec3::new(-1.75, 2.5, 0.5),
                tail: Vec3::new(-3.0, 2.5, 0.5),
            },
            BoneSegment {
                head: Vec3::new(1.75, 2.5, 0.5),
                tail: Vec3::new(3.0, 2.5, 0.5),
            },
        ];
        let partner = [0usize, 2, 1, 4, 3];
        let allowed = vec![true; bones.len()];

        let grid = VoxelGrid::build(&mesh, res).expect("grid");
        let field = GeodesicField::compute(&mesh, &grid, &bones).expect("field");
        let w = assign_weights(
            &field,
            &mesh.positions,
            &bones,
            &allowed,
            SkinningParams::default(),
        );

        let mut worst = 0.0f32;
        let mut mismatches = 0usize;
        let mut checked = 0usize;
        for v in 0..mesh.vertex_count() {
            let here = mesh.positions[v];
            if here.x.abs() < 1e-6 {
                continue; // on the mirror plane, self-paired
            }
            let target = Vec3::new(-here.x, here.y, here.z);
            let Some(m) =
                (0..mesh.vertex_count()).find(|&u| mesh.positions[u].distance(target) < 1e-5)
            else {
                continue;
            };

            let mut mine: Vec<(usize, f32)> = w
                .influences(v)
                .map(|(b, x)| (partner[b as usize], x))
                .collect();
            let mut theirs: Vec<(usize, f32)> =
                w.influences(m).map(|(b, x)| (b as usize, x)).collect();
            mine.sort_by_key(|&(b, _)| b);
            theirs.sort_by_key(|&(b, _)| b);
            checked += 1;

            if mine.len() != theirs.len() || mine.iter().zip(&theirs).any(|(a, b)| a.0 != b.0) {
                mismatches += 1;
                continue;
            }
            for ((_, a), (_, b)) in mine.iter().zip(theirs.iter()) {
                worst = worst.max((a - b).abs());
            }
        }
        assert!(
            checked >= 8,
            "only {checked} mirrored pairs found; test is too weak"
        );
        (worst, mismatches)
    }

    #[test]
    fn a_symmetric_rig_assigns_mirrored_bones_exactly() {
        // Invariant 8, the half that must hold exactly. On a left/right
        // symmetric body with a symmetric rig, a vertex and its mirror must be
        // influenced by the *same set of bones*, mirrored. A human rig is
        // symmetric, so a violation here would weight one arm from different
        // bones than the other on identical geometry.
        for res in [24u32, 48, 96] {
            let (_, mismatches) = symmetry_error(res);
            assert_eq!(mismatches, 0, "bone sets disagree at resolution {res}");
        }
    }

    #[test]
    fn symmetry_error_converges_with_resolution() {
        // Invariant 8, the half that holds only approximately — and the test
        // that proves why. Weight *values* are not exactly mirrored, because
        // the voxel grid is not aligned to the symmetry plane: a vertex and its
        // mirror sit at different offsets within their voxels.
        //
        // The distinction that matters is bias versus discretisation. A
        // systematic handedness bias would not shrink with resolution;
        // discretisation error falls linearly with voxel size. Measured:
        //
        //   res  24  voxel 0.2500  worst delta 0.0182
        //   res  48  voxel 0.1250  worst delta 0.0080
        //   res  96  voxel 0.0625  worst delta 0.0037
        //   res 192  voxel 0.0312  worst delta 0.0018
        //   res 384  voxel 0.0156  worst delta 0.0009
        //
        // Halving each time: first-order convergence, so it vanishes as the
        // grid refines. At DEFAULT_RESOLUTION this is well under 0.2% of a
        // weight, and the 0.01 bound below has 2.7x margin over the measured
        // 0.0037. Asserting convergence rather than a fixed tolerance is what
        // makes this a guard against bias rather than against a magic number.
        //
        // These figures depend on zero-seeding the Dijkstra front; seeding with
        // sub-voxel distances made every one of them ~2.5x worse. See the
        // comment in geodesic.rs::dijkstra.
        let coarse = symmetry_error(24).0;
        let medium = symmetry_error(48).0;
        let fine = symmetry_error(96).0;

        assert!(
            coarse > 0.0,
            "no error at all suggests the test is not measuring"
        );
        assert!(
            medium < coarse * 0.75,
            "error did not fall with resolution: {coarse:.5} -> {medium:.5}"
        );
        assert!(
            fine < medium * 0.75,
            "error did not fall with resolution: {medium:.5} -> {fine:.5}"
        );
        assert!(
            fine < 0.01,
            "error {fine:.5} at resolution 96 is larger than discretisation explains"
        );
    }

    #[test]
    fn is_deterministic() {
        // Invariant 6. rayon reductions over floats are order-dependent, so a
        // leak here would make every golden test flaky.
        let mesh = bar();
        let bones = chain(6);
        let allowed = vec![true; bones.len()];
        assert_eq!(
            solve(&mesh, &bones, &allowed),
            solve(&mesh, &bones, &allowed)
        );
    }

    #[test]
    fn hostile_falloff_values_cannot_produce_nan() {
        // The falloff is an artist-facing slider, so it will reach these.
        // Before validation: 0.0 collapsed the falloff to uniform blending
        // (0f32.powf(0.0) == 1.0, including at the cutoff), a negative exponent
        // turned a zero base into infinity, and NaN propagated to every weight.
        let mesh = bar();
        let bones = chain(6);
        let allowed = vec![true; bones.len()];
        let grid = VoxelGrid::build(&mesh, 32).unwrap();
        let field = GeodesicField::compute(&mesh, &grid, &bones).unwrap();

        for hostile in [0.0f32, -3.0, f32::NAN, f32::INFINITY, 1e30, -0.0] {
            let params = SkinningParams::new(hostile);
            assert!(
                FALLOFF_RANGE.contains(&params.falloff()),
                "falloff {hostile} was not clamped, got {}",
                params.falloff()
            );
            let w = assign_weights(&field, &mesh.positions, &bones, &allowed, params);
            assert_eq!(
                w.first_unnormalised(1e-5),
                None,
                "falloff {hostile} produced invalid weights"
            );
        }
    }

    #[test]
    fn survives_a_very_small_mesh() {
        // Invariant 7 downward. Working in raw mesh units, `1.0 / d` overflows
        // to infinity below ~2.9e-39 and every weight became NaN. Distances are
        // now expressed as ratios to the nearest, which cannot overflow.
        let base = bar();
        for scale in [1e-6f32, 1e-3, 1.0] {
            let p: Vec<f32> = base
                .positions
                .iter()
                .flat_map(|v| [v.x * scale, v.y * scale, v.z * scale])
                .collect();
            let mesh = Mesh::from_flat(&p, &base.indices).unwrap();
            let bones: Vec<BoneSegment> = chain(6)
                .into_iter()
                .map(|b| BoneSegment {
                    head: b.head * scale,
                    tail: b.tail * scale,
                })
                .collect();
            let allowed = vec![true; bones.len()];
            let w = solve(&mesh, &bones, &allowed);
            assert_eq!(
                w.first_unnormalised(1e-4),
                None,
                "scale {scale} produced invalid weights"
            );
        }
    }

    #[test]
    #[should_panic(expected = "at least one bone must be allowed")]
    fn refuses_a_fully_masked_rig() {
        // Otherwise the Euclidean fallback has nothing to pick and leaves the
        // vertex at all-zero weights, silently breaking invariant 1.
        let mesh = bar();
        let bones = chain(4);
        let grid = VoxelGrid::build(&mesh, 32).unwrap();
        let field = GeodesicField::compute(&mesh, &grid, &bones).unwrap();
        let none = vec![false; bones.len()];
        let _ = assign_weights(
            &field,
            &mesh.positions,
            &bones,
            &none,
            SkinningParams::default(),
        );
    }

    #[test]
    fn empty_candidate_list_is_a_no_op() {
        // write_falloff is reachable directly; an empty slice used to index
        // best[used - 1] and underflow.
        let mut w = SkinWeights::zeroed(1);
        write_falloff(&[], 2.0, &mut w, 0);
        assert!(w.weights.iter().all(|x| *x == 0.0));
    }

    #[test]
    fn first_unnormalised_detects_nan() {
        // The regression guard for the guard. (NaN - 1.0).abs() > tol is false,
        // so a sum-only check reported a NaN vertex as correctly normalised —
        // and every other test in this module leans on this function.
        let mut w = SkinWeights::zeroed(1);
        w.weights[0] = f32::NAN;
        assert_eq!(w.first_unnormalised(1e-5), Some(0));

        let mut neg = SkinWeights::zeroed(1);
        neg.weights[0] = 2.0;
        neg.weights[1] = -1.0;
        assert_eq!(
            neg.first_unnormalised(1e-5),
            Some(0),
            "sums to 1 but negative"
        );
    }

    #[test]
    fn zeroed_has_correct_shape() {
        let w = SkinWeights::zeroed(10);
        assert_eq!(w.vertex_count(), 10);
        assert_eq!(w.indices.len(), 10 * MAX_INFLUENCES);
    }

    #[test]
    fn detects_unnormalised_vertex() {
        let mut w = SkinWeights::zeroed(2);
        w.weights[0] = 1.0;
        assert_eq!(w.first_unnormalised(1e-5), Some(1));
    }
}
