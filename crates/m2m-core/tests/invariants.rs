//! Randomised property tests for the solver invariants in `memory/test.md` §3.
//!
//! The fixed fixtures in the unit tests pin specific known-hard cases; these
//! sweep the continuous parameter space around them. The two are complementary:
//! a grid of hand-picked values cannot hit the corner where a scale happens to
//! make the voxel size exactly representable, and a random search cannot be
//! relied on to revisit a case that once failed.

use glam::{Mat3, Vec3};
use m2m_core::geodesic::{BoneSegment, GeodesicField};
use m2m_core::mesh::Mesh;
use m2m_core::skinning::{assign_weights, SkinningParams, MAX_INFLUENCES};
use m2m_core::voxel::VoxelGrid;
use proptest::prelude::*;

/// Appends an axis-aligned box, transformed by `place`.
fn push_box(
    positions: &mut Vec<f32>,
    indices: &mut Vec<u32>,
    lo: Vec3,
    hi: Vec3,
    place: &dyn Fn(Vec3) -> Vec3,
) {
    let base = (positions.len() / 3) as u32;
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
        let p = place(c);
        positions.extend_from_slice(&[p.x, p.y, p.z]);
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
        indices.extend(f.iter().map(|k| base + k));
    }
}

/// Shapes the generators build.
///
/// A single convex box exercises none of the geometry the solver actually
/// struggles with, so the topology is varied too: the concave case is where
/// Euclidean and geodesic distance disagree, and the detached case is the only
/// one that reaches the nearest-bone fallback.
#[derive(Debug, Clone, Copy)]
enum Shape {
    /// One box. The easy case.
    Box,
    /// A U: two prongs joined at the base, with air between them.
    Concave,
    /// A box plus a detached island the voxel grid cannot connect.
    Detached,
}

/// A shape with a bone chain through it, transformed into an arbitrary pose.
fn scene_shaped(
    shape: Shape,
    scale: f32,
    offset: Vec3,
    angle: f32,
    bone_count: usize,
) -> (Mesh, Vec<BoneSegment>) {
    let rot = Mat3::from_axis_angle(Vec3::new(1.0, 2.0, 3.0).normalize(), angle);
    let place = move |v: Vec3| rot * (v * scale) + offset;

    let (mut positions, mut indices) = (Vec::new(), Vec::new());
    match shape {
        Shape::Box => push_box(
            &mut positions,
            &mut indices,
            Vec3::ZERO,
            Vec3::new(1.0, 4.0, 1.0),
            &place,
        ),
        Shape::Concave => {
            push_box(
                &mut positions,
                &mut indices,
                Vec3::ZERO,
                Vec3::new(3.0, 1.0, 1.0),
                &place,
            );
            push_box(
                &mut positions,
                &mut indices,
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(1.0, 4.0, 1.0),
                &place,
            );
            push_box(
                &mut positions,
                &mut indices,
                Vec3::new(2.0, 1.0, 0.0),
                Vec3::new(3.0, 4.0, 1.0),
                &place,
            );
        }
        Shape::Detached => {
            push_box(
                &mut positions,
                &mut indices,
                Vec3::ZERO,
                Vec3::new(1.0, 4.0, 1.0),
                &place,
            );
            push_box(
                &mut positions,
                &mut indices,
                Vec3::new(6.0, 0.0, 0.0),
                Vec3::new(7.0, 1.0, 1.0),
                &place,
            );
        }
    }

    let bones = (0..bone_count)
        .map(|i| {
            let step = 4.0 / bone_count as f32;
            BoneSegment {
                head: place(Vec3::new(0.5, i as f32 * step, 0.5)),
                tail: place(Vec3::new(0.5, (i + 1) as f32 * step, 0.5)),
            }
        })
        .collect();

    (
        Mesh::from_flat(&positions, &indices).expect("generated mesh is valid"),
        bones,
    )
}

/// The single-box scene, for tests that need a fixed topology.
fn scene(scale: f32, offset: Vec3, angle: f32, bone_count: usize) -> (Mesh, Vec<BoneSegment>) {
    scene_shaped(Shape::Box, scale, offset, angle, bone_count)
}

proptest! {
    // Each case runs a full voxelise + geodesic + weight solve, so the count is
    // kept modest and resolutions small enough to stay quick in debug builds.
    #![proptest_config(ProptestConfig::with_cases(48))]

    /// Invariants 1, 2, 3 and 5 over an arbitrary pose, rig size and falloff.
    #[test]
    fn weights_are_always_valid(
        scale in 0.01f32..100.0,
        ox in -50.0f32..50.0,
        oy in -50.0f32..50.0,
        oz in -50.0f32..50.0,
        angle in 0.0f32..std::f32::consts::TAU,
        bone_count in 2usize..10,
        resolution in 16u32..40,
        falloff in 0.25f32..8.0,
        mask_first in any::<bool>(),
        shape_pick in 0usize..3,
    ) {
        let shape = [Shape::Box, Shape::Concave, Shape::Detached][shape_pick];
        let (mesh, bones) =
            scene_shaped(shape, scale, Vec3::new(ox, oy, oz), angle, bone_count);
        let grid = VoxelGrid::build(&mesh, resolution).expect("grid");
        let field = GeodesicField::compute(&mesh, &grid, &bones).expect("field");

        // Exercise invariant 4 too: masking the first bone stands in for the
        // root, which must never receive weight.
        let mut allowed = vec![true; bones.len()];
        if mask_first {
            allowed[0] = false;
        }

        let w = assign_weights(
            &field,
            &mesh.positions,
            &bones,
            &allowed,
            SkinningParams::new(falloff),
        );

        // Invariants 1 and 2: normalised, finite, non-negative.
        prop_assert_eq!(w.first_unnormalised(1e-4), None);

        for v in 0..w.vertex_count() {
            // Invariant 3. Note `influences` iterates a fixed-size window, so
            // counting it can never exceed the limit — that assertion would be
            // unfalsifiable. What can actually go wrong is a weight written
            // past the fourth slot, so the raw buffer is checked instead.
            let slots = &w.weights[v * MAX_INFLUENCES..(v + 1) * MAX_INFLUENCES];
            prop_assert_eq!(slots.len(), MAX_INFLUENCES);
            prop_assert!(w.influences(v).count() <= slots.len());
            for (bone, weight) in w.influences(v) {
                // Invariant 5: every index addresses a real bone.
                prop_assert!((bone as usize) < bones.len());
                // Invariant 4: a masked bone never receives weight.
                prop_assert!(
                    allowed[bone as usize],
                    "masked bone {} got weight {}", bone, weight
                );
            }
        }
    }

    /// Invariant 7, the half that holds exactly: a uniform scale must never
    /// change which bones influence a vertex.
    ///
    /// Weight *values* only converge — see
    /// `scale_invariance_converges_with_resolution` below for why, and for the
    /// bound this test's tolerance comes from. Measured over 480 random poses:
    /// zero bone-assignment mismatches at every resolution tried.
    #[test]
    fn scaling_the_scene_does_not_change_bone_assignment(
        base in 0.05f32..5.0,
        factor in 2.0f32..50.0,
        angle in 0.0f32..std::f32::consts::TAU,
        bone_count in 2usize..8,
    ) {
        let solve = |scale: f32| {
            let (mesh, bones) = scene(scale, Vec3::ZERO, angle, bone_count);
            let grid = VoxelGrid::build(&mesh, 24).expect("grid");
            let field = GeodesicField::compute(&mesh, &grid, &bones).expect("field");
            let allowed = vec![true; bones.len()];
            assign_weights(
                &field,
                &mesh.positions,
                &bones,
                &allowed,
                SkinningParams::default(),
            )
        };

        let small = solve(base);
        let large = solve(base * factor);

        prop_assert_eq!(&small.indices, &large.indices);

        // Resolution 24 is coarse; the measured maximum weight delta there is
        // 0.090 over 120 random poses. 0.15 leaves margin without being so
        // loose that a real regression slips through — the convergence test
        // below is what actually pins the behaviour.
        for (a, b) in small.weights.iter().zip(large.weights.iter()) {
            prop_assert!(
                (a - b).abs() < 0.15,
                "weights diverged under scaling far beyond discretisation: {} vs {}", a, b
            );
        }
    }

    /// Invariant 6: identical input yields bit-identical output.
    #[test]
    fn solving_twice_gives_the_same_answer(
        scale in 0.1f32..20.0,
        angle in 0.0f32..std::f32::consts::TAU,
        bone_count in 2usize..8,
    ) {
        let solve = || {
            let (mesh, bones) = scene(scale, Vec3::ZERO, angle, bone_count);
            let grid = VoxelGrid::build(&mesh, 24).expect("grid");
            let field = GeodesicField::compute(&mesh, &grid, &bones).expect("field");
            let allowed = vec![true; bones.len()];
            assign_weights(
                &field,
                &mesh.positions,
                &bones,
                &allowed,
                SkinningParams::default(),
            )
        };
        prop_assert_eq!(solve(), solve());
    }
}

/// Median worst-per-case weight difference between the same scene at two
/// scales, plus the number of cases whose bone assignment differed.
///
/// The **median**, not the max: over a handful of cases the maximum is
/// dominated by whichever single pose happens to straddle a voxel boundary, and
/// does not fall monotonically even though the underlying error does. A first
/// attempt at this test used the max over 5 cases and was not convergent.
fn scale_error(res: u32) -> (f32, usize) {
    let mut per_case: Vec<f32> = Vec::new();
    let mut mismatches = 0usize;

    // Deterministic sweep, so the numbers in the test comment stay stable.
    for i in 0..24 {
        let base = 0.05 + (i as f32) * 0.11;
        let factor = 3.0 + (i as f32) * 2.3;
        let angle = (i as f32) * 0.37;
        let bone_count = 2 + (i as usize % 5);

        let solve = |scale: f32| {
            let (mesh, bones) = scene(scale, Vec3::ZERO, angle, bone_count);
            let grid = VoxelGrid::build(&mesh, res).expect("grid");
            let field = GeodesicField::compute(&mesh, &grid, &bones).expect("field");
            let allowed = vec![true; bones.len()];
            assign_weights(
                &field,
                &mesh.positions,
                &bones,
                &allowed,
                SkinningParams::default(),
            )
        };
        let a = solve(base);
        let b = solve(base * factor);
        if a.indices != b.indices {
            mismatches += 1;
            continue;
        }
        let worst = a
            .weights
            .iter()
            .zip(b.weights.iter())
            .map(|(x, y)| (x - y).abs())
            .fold(0.0f32, f32::max);
        per_case.push(worst);
    }

    per_case.sort_by(|a, b| a.partial_cmp(b).expect("finite"));
    // If every case mismatched there is no median. Returning 0.0 lets the
    // caller's mismatch assertion report the real failure; indexing an empty
    // vec here would surface it as an out-of-bounds panic in a helper instead.
    let median = per_case.get(per_case.len() / 2).copied().unwrap_or(0.0);
    (median, mismatches)
}

#[test]
fn scale_invariance_converges_with_resolution() {
    // Invariant 7, and the test that says what kind of invariance this is.
    //
    // Bone assignment is exactly scale-invariant. Weight *values* are not,
    // because the geodesic path is a chain of discrete voxel steps: the same
    // mesh at two scales can route through one more or one fewer step. The
    // distinction that matters is a systematic scale bias, which would not
    // shrink, against discretisation, which falls linearly with voxel size.
    //
    // Measured on the deterministic 24-case sweep below, median and max of the
    // worst per-case delta:
    //
    //   res  24   p50 0.0340   max 0.0800
    //   res  48   p50 0.0187   max 0.0420
    //   res  96   p50 0.0098   max 0.0165
    //   res 192   p50 0.0048   max 0.0104
    //
    // Roughly halving each doubling, with 0 bone-assignment mismatches at every
    // resolution. Extrapolated to DEFAULT_RESOLUTION that is well under 1%.
    let coarse = scale_error(24);
    let medium = scale_error(48);
    let fine = scale_error(96);

    assert_eq!(
        coarse.1, 0,
        "bone assignment must be exactly scale-invariant"
    );
    assert_eq!(medium.1, 0);
    assert_eq!(fine.1, 0);

    assert!(
        coarse.0 > 0.0,
        "no error at all suggests the test is not measuring"
    );
    // Measured ratios are 0.55 and 0.53, so 0.8 leaves margin without being so
    // loose that a stalled convergence would pass.
    assert!(
        medium.0 < coarse.0 * 0.8,
        "error did not fall with resolution: {:.5} -> {:.5}",
        coarse.0,
        medium.0
    );
    assert!(
        fine.0 < medium.0 * 0.8,
        "error did not fall with resolution: {:.5} -> {:.5}",
        medium.0,
        fine.0
    );
}
