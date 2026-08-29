//! Validation against a real production mesh, not a synthetic fixture.
//!
//! Synthetic tetrahedra cannot exhibit the defects that actually matter —
//! seam-split duplicate vertices, degenerate triangles from decimation, open
//! boundaries at the eyes and mouth. This fixture is the geometry of
//! `legacy/static/test-files/human-small.glb`, exported by
//! `legacy/bench/dump-fixtures.ts`.

#[path = "fixture_support.rs"]
mod fixture_support;

use fixture_support::{load_mesh, load_rig};

#[test]
fn validates_a_real_human_mesh() {
    let mesh = load_mesh(include_bytes!("fixtures/human-small.bin"));

    assert_eq!(mesh.vertex_count(), 8691);
    assert_eq!(mesh.triangle_count(), 13721);

    let diagonal = {
        let (lo, hi) = mesh.bounds().expect("non-empty");
        (hi - lo).length()
    };
    let report = mesh.validate(diagonal * m2m_core::mesh::DEFAULT_WELD_EPSILON_RATIO);

    eprintln!(
        "human-small: {} verts, {} tris, {} dup, {} components, \
         {} boundary edges, {} non-manifold, {} degenerate, diagonal {:.3}",
        report.vertex_count,
        report.triangle_count,
        report.duplicate_vertices,
        report.components,
        report.boundary_edges,
        report.non_manifold_edges,
        report.degenerate_triangles.len(),
        report.diagonal
    );

    // A real character is NOT one island: 61 disconnected pieces — eyes, teeth,
    // tongue, lashes, clothing — plus open boundary and a non-manifold edge.
    // All normal, and all why the solver uses a voxel method rather than
    // anything requiring a watertight manifold.
    assert_eq!(report.components, 61);
    assert_eq!(report.duplicate_vertices, 1698);
    assert_eq!(report.boundary_edges, 26);
    assert_eq!(report.non_manifold_edges, 1);
    assert!(!report.is_watertight());

    // The fixture's extent is 0.812 x 0.764 x 0.154, diagonal 1.126. Pinned
    // tightly: a change means the export path altered the geometry (a dropped
    // world transform, or a dropped mesh), which would silently move the
    // diagonal-derived weld epsilon underneath every other assertion.
    assert!(
        (report.diagonal - 1.126).abs() < 0.01,
        "diagonal moved: {}",
        report.diagonal
    );

    // The real count, not a vacuous bound. This is the number that moves if the
    // degeneracy threshold stops being scale-relative, or if welding over-fuses.
    assert_eq!(report.degenerate_triangles.len(), 0);

    // The fixture's actual extent is 0.406 x 0.764 x 0.154, diagonal 0.879.
    // Pinned tightly: a change here means the export path altered the geometry
    // (a dropped world transform, say), which would silently move the
    // diagonal-derived weld epsilon underneath every other assertion.

    // The real count, not a vacuous bound. This is the number a scale-dependent
    // degeneracy threshold would move.
}

#[test]
fn welding_is_stable_across_the_epsilon_plateau() {
    // The default weld epsilon must sit on a plateau, not a slope: if the
    // component count moved with small epsilon changes, every weight solve
    // would depend on an arbitrary constant. See DEFAULT_WELD_EPSILON_RATIO
    // for the full sweep this pins.
    let mesh = load_mesh(include_bytes!("fixtures/human-small.bin"));
    let (lo, hi) = mesh.bounds().expect("non-empty");
    let diagonal = (hi - lo).length();

    for ratio in [1e-7f32, 1e-6, 1e-5] {
        let r = mesh.validate(diagonal * ratio);
        assert_eq!(r.components, 61, "component count moved at ratio {ratio:e}");
        assert_eq!(
            r.duplicate_vertices, 1698,
            "dup count moved at ratio {ratio:e}"
        );
        assert_eq!(
            r.degenerate_triangles.len(),
            0,
            "over-welded at ratio {ratio:e}"
        );
    }

    // The default must sit inside that band, not at its edge.
    let d = mesh.validate(diagonal * m2m_core::mesh::DEFAULT_WELD_EPSILON_RATIO);
    assert_eq!(d.components, 61);

    // Unwelded, exporter seam splits read as nearly twice as many islands.
    assert_eq!(mesh.validate(0.0).components, 116);

    // Far above the band, welding fuses distinct surfaces and collapses real
    // triangles into slivers: 2890 of 13721 faces at 1e-3. That degenerate
    // count is the sharpest signal that the epsilon is too generous.
    let over = mesh.validate(diagonal * 1e-3);
    assert!(over.components < 20);
    assert!(over.degenerate_triangles.len() > 2000);
}

#[test]
fn validation_is_deterministic() {
    // The weld map walks a HashMap; if iteration order leaked into the result,
    // every downstream golden test would be flaky.
    let mesh = load_mesh(include_bytes!("fixtures/human-small.bin"));
    let a = mesh.validate(1e-4);
    let b = mesh.validate(1e-4);
    assert_eq!(a, b);
}

#[test]
fn voxelises_a_real_non_watertight_mesh() {
    use m2m_core::voxel::{VoxelGrid, DEFAULT_RESOLUTION};

    let mesh = load_mesh(include_bytes!("fixtures/human-small.bin"));
    let grid = VoxelGrid::build(&mesh, DEFAULT_RESOLUTION).expect("grid");
    let s = grid.stats();
    let vs = grid.voxel_size();
    let volume = s.interior as f32 * vs * vs * vs;

    eprintln!(
        "res {DEFAULT_RESOLUTION}: dims {:?}  surface {}  interior {}  volume {volume:.4}",
        grid.dims(),
        s.surface,
        s.interior
    );

    // The mesh has 26 boundary edges and is not watertight, but every hole is
    // far smaller than a voxel here, so conservative rasterisation seals the
    // shell and the body still encloses volume. A leak shows up as interior 0.
    assert!(s.interior > 0, "non-watertight mesh leaked entirely");

    // Interior must dominate surface, or the geodesic field has nothing to
    // propagate through. See DEFAULT_RESOLUTION for the full sweep — this
    // ratio is 0.08 at resolution 32 and 2.69 here.
    assert!(
        s.interior > s.surface * 2,
        "shell-dominated grid: {} interior vs {} surface",
        s.interior,
        s.surface
    );

    // Pinned so a rasterisation change cannot silently move the field the
    // geodesic solver will propagate through.
    assert_eq!(s.surface, 54608);
    assert_eq!(s.interior, 146672);

    // Physical sanity, not just internal consistency. The figure is 0.764
    // units tall, so a 1.75 m human implies 2.29 m/unit and a volume scale of
    // 2.29^3 = 12.0. A 60-70 kg person displaces roughly 60-70 litres.
    let (lo, hi) = mesh.bounds().expect("non-empty");
    let scale = 1.75 / (hi.y - lo.y);
    let litres = volume * scale * scale * scale * 1000.0;
    assert!(
        (40.0..110.0).contains(&litres),
        "voxelised body volume {litres:.1} L is not a plausible human"
    );
}

#[test]
fn geodesic_field_on_a_real_character() {
    use m2m_core::geodesic::GeodesicField;
    use m2m_core::voxel::{VoxelGrid, DEFAULT_RESOLUTION};

    // The template model rig-human.glb was authored for. human-small.glb is a
    // scaled-down test asset — the rig is 2.19x its size — so pairing them puts
    // every bone outside the mesh. See mismatched_rig_reports_every_bone_outside.
    let mesh = load_mesh(include_bytes!("fixtures/template-human-mesh.bin"));
    let bones = load_rig(include_bytes!("fixtures/template-human-rig.bin")).bones;
    assert_eq!(bones.len(), 66);

    let grid = VoxelGrid::build(&mesh, DEFAULT_RESOLUTION).expect("grid");

    let t = std::time::Instant::now();
    let field = GeodesicField::compute(&mesh, &grid, &bones).expect("field");
    let elapsed = t.elapsed();

    let unreachable_bones = field.unreachable_bones();
    let stranded = field.unreachable_vertices();
    eprintln!(
        "geodesic: {} verts x {} bones in {:?}  ({} unreachable bones, {} stranded verts)",
        field.vertex_count(),
        field.bone_count(),
        elapsed,
        unreachable_bones.len(),
        stranded.len()
    );

    // Most bones must reach the mesh. A template rig fitted to its own model
    // should have essentially all of them inside.
    assert!(
        unreachable_bones.len() < bones.len() / 4,
        "{} of {} bones reached nothing",
        unreachable_bones.len(),
        bones.len()
    );

    assert!(
        stranded.is_empty(),
        "template rig should reach every vertex"
    );

    // A loose ceiling, not a benchmark. Measured ~190 ms in release and ~420 ms
    // under the test harness; 10 s catches the plausible regression (an
    // accidental full-grid graph, or losing the parallelism) without being
    // flaky on a loaded machine. Debug builds are far slower, hence the gate.
    if !cfg!(debug_assertions) {
        assert!(
            elapsed.as_secs_f32() < 10.0,
            "geodesic solve took {elapsed:?}, expected well under 10 s"
        );
    }
}

#[test]
fn mismatched_rig_reports_every_bone_outside() {
    use m2m_core::geodesic::GeodesicField;
    use m2m_core::voxel::VoxelGrid;

    // A rig that does not fit its mesh is the single most common rigging
    // mistake, and the solver must say so rather than silently returning a rig
    // with dead limbs. human-small is 2.19x smaller than the template rig, so
    // essentially every bone lands outside it.
    let mesh = load_mesh(include_bytes!("fixtures/human-small.bin"));
    let bones = load_rig(include_bytes!("fixtures/template-human-rig.bin")).bones;
    let grid = VoxelGrid::build(&mesh, 128).expect("grid");
    let field = GeodesicField::compute(&mesh, &grid, &bones).expect("field");

    assert!(
        field.unreachable_bones().len() > bones.len() * 3 / 4,
        "expected most bones reported outside, got {}",
        field.unreachable_bones().len()
    );

    // The complementary half, so this distinguishes "correctly detects a
    // mismatch" from "detects nothing ever". A totally broken solver — empty
    // graph, no seeds, no relaxation — would satisfy the assertion above.
    let template = load_mesh(include_bytes!("fixtures/template-human-mesh.bin"));
    let matched_grid = VoxelGrid::build(&template, 128).expect("grid");
    let matched = GeodesicField::compute(&template, &matched_grid, &bones).expect("field");
    assert!(
        matched.unreachable_bones().is_empty(),
        "the matched rig should reach every bone, got {} unreachable",
        matched.unreachable_bones().len()
    );
}

/// Distance from a point to a line segment.
fn point_to_segment(p: glam::Vec3, a: glam::Vec3, b: glam::Vec3) -> f32 {
    let ab = b - a;
    let len_sq = ab.length_squared();
    let t = if len_sq > 0.0 {
        ((p - a).dot(ab) / len_sq).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

#[test]
fn geodesic_disagrees_with_euclidean_where_it_matters() {
    use m2m_core::geodesic::GeodesicField;
    use m2m_core::voxel::{VoxelGrid, DEFAULT_RESOLUTION};

    // The measurement that justifies replacing the legacy solver. For every
    // vertex, compare the bone Euclidean distance picks (what
    // WeightCalculator.ts:71-80 does) against the bone geodesic distance picks.
    //
    // Measured on the template human, a T-POSE model where limbs are spread
    // apart — the case most favourable to Euclidean. An A-pose model, where
    // arms hang beside the ribcage, is expected to be worse (objective O8).
    let mesh = load_mesh(include_bytes!("fixtures/template-human-mesh.bin"));
    let bones = load_rig(include_bytes!("fixtures/template-human-rig.bin")).bones;
    let grid = VoxelGrid::build(&mesh, DEFAULT_RESOLUTION).expect("grid");
    let field = GeodesicField::compute(&mesh, &grid, &bones).expect("field");

    let mut differ = 0usize;
    let mut compared = 0usize;
    let mut worst_ratio = 0.0f32;

    for v in 0..mesh.vertex_count() {
        // Skip vertices no bone reached. Their row is all infinite, so `min_by`
        // returns the last bone rather than failing, which would count as a
        // disagreement and inflate the headline number with vertices where
        // geodesic distance carried no information at all.
        if !field.vertex_row(v).iter().any(|d| d.is_finite()) {
            continue;
        }
        compared += 1;
        let p = mesh.positions[v];
        let euclid = (0..bones.len())
            .min_by(|&a, &b| {
                point_to_segment(p, bones[a].head, bones[a].tail)
                    .partial_cmp(&point_to_segment(p, bones[b].head, bones[b].tail))
                    .expect("finite distances")
            })
            .expect("at least one bone");
        let geo = (0..bones.len())
            .min_by(|&a, &b| {
                field
                    .distance(v, a)
                    .partial_cmp(&field.distance(v, b))
                    .expect("finite distances")
            })
            .expect("at least one bone");

        if euclid != geo {
            differ += 1;
        }

        let straight = point_to_segment(p, bones[euclid].head, bones[euclid].tail);
        let through = field.distance(v, euclid);
        if straight > 1e-6 && through.is_finite() {
            worst_ratio = worst_ratio.max(through / straight);
        }
    }

    assert!(
        compared > mesh.vertex_count() * 9 / 10,
        "too many stranded vertices to draw a conclusion"
    );
    let pct = 100.0 * differ as f32 / compared as f32;
    eprintln!(
        "euclidean vs geodesic: dominant bone differs on {differ}/{compared} reachable \
         ({pct:.1}%), worst path ratio {worst_ratio:.1}x"
    );

    // Measured 14.6% and 19.4x. Pinned loosely — the point is that the two
    // metrics disagree substantially, and a change that collapsed the
    // difference would mean geodesic distance had stopped being geodesic.
    assert!(
        pct > 8.0,
        "only {pct:.1}% of vertices changed bone; is the field leaking?"
    );
    assert!(
        worst_ratio > 5.0,
        "worst path ratio {worst_ratio:.1}x is too low to be measuring \
         travel through the mesh"
    );
}

#[test]
fn full_pipeline_beats_the_legacy_baseline() {
    use m2m_core::geodesic::GeodesicField;
    use m2m_core::skinning::{assign_weights, SkinningParams, MAX_INFLUENCES};
    use m2m_core::voxel::{VoxelGrid, DEFAULT_RESOLUTION};

    // The A/B that P1-8 exists for, on the same model and rig the legacy
    // baseline used. From bench/baselines/legacy-solver.json, the legacy
    // solver on this template gives:
    //     87% of vertices with a single influence, mean 1.132 influences
    // Those numbers are why it needs a smoothing pass and three per-body-part
    // correctors at all.
    const LEGACY_SINGLE_INFLUENCE_PCT: f32 = 87.0;
    const LEGACY_MEAN_INFLUENCES: f32 = 1.132;

    let mesh = load_mesh(include_bytes!("fixtures/template-human-mesh.bin"));
    // The same bone mask template_ab uses. Without it this test compares
    // against the same legacy figures with a different bone set — letting
    // weight land on the root and the 13 leaf bones the legacy solver never
    // touches — so the two human A/Bs could disagree.
    let rig = load_rig(include_bytes!("fixtures/template-human-rig.bin"));
    let bones = rig.bones;
    let allowed = rig.weightable;

    let t = std::time::Instant::now();
    let grid = VoxelGrid::build(&mesh, DEFAULT_RESOLUTION).expect("grid");
    let field = GeodesicField::compute(&mesh, &grid, &bones).expect("field");
    let weights = assign_weights(
        &field,
        &mesh.positions,
        &bones,
        &allowed,
        SkinningParams::default(),
    );
    let elapsed = t.elapsed();

    // Count influences above a MEANINGFUL threshold, not merely non-zero.
    //
    // Counting anything above zero flatters the result: the falloff leaves a
    // fourth influence on almost every vertex, but its median value is 0.001 —
    // a tenth of a percent, invisible in deformation. Measured on this mesh,
    // mean influences reads 3.999 at threshold 0 and 2.767 at 1%. The second is
    // the number that describes how the mesh actually deforms, so it is the one
    // asserted on, with the raw figure reported alongside for comparison
    // against the legacy baseline, which used a 1e-6 threshold.
    const MEANINGFUL: f32 = 0.01;

    let n = weights.vertex_count();
    let mut single = 0usize;
    let mut total_meaningful = 0usize;
    let mut total_raw = 0usize;
    for v in 0..n {
        let meaningful = weights
            .influences(v)
            .filter(|(_, w)| *w > MEANINGFUL)
            .count();
        if meaningful == 1 {
            single += 1;
        }
        total_meaningful += meaningful;
        total_raw += weights.influences(v).count();
    }
    let single_pct = 100.0 * single as f32 / n as f32;
    let mean = total_meaningful as f32 / n as f32;
    let mean_raw = total_raw as f32 / n as f32;

    eprintln!(
        "full pipeline: {n} verts, {} bones, {elapsed:?}\n  \
         single-influence {single_pct:.1}% at >1% weight (legacy {LEGACY_SINGLE_INFLUENCE_PCT}%)\n  \
         mean influences {mean:.3} at >1% weight, {mean_raw:.3} raw \
         (legacy {LEGACY_MEAN_INFLUENCES} raw)\n  \
         fallback vertices {}",
        bones.len(),
        weights.fallback_vertices.len()
    );

    // Invariants 1, 2, 3 and 5 from memory/test.md §3 on real geometry rather
    // than a synthetic bar. Invariant 4 needs a mask (unit tests cover it),
    // 6 and 7 are covered in the unit tests, 8 is not covered anywhere yet.
    assert_eq!(weights.first_unnormalised(1e-5), None);
    assert!(weights.weights.iter().all(|w| w.is_finite() && *w >= 0.0));
    for &b in &weights.indices {
        assert!((b as usize) < bones.len());
    }
    for v in 0..n {
        assert!(weights.influences(v).count() <= MAX_INFLUENCES);
    }

    // The actual improvement. Smooth deformation needs 2-4 influences near
    // joints; the legacy solver averages 1.13.
    assert!(
        mean > LEGACY_MEAN_INFLUENCES * 1.5,
        "mean influences {mean:.3} is not a clear improvement on {LEGACY_MEAN_INFLUENCES}"
    );
    assert!(
        single_pct < LEGACY_SINGLE_INFLUENCE_PCT / 2.0,
        "single-influence {single_pct:.1}% is not a clear improvement on \
         {LEGACY_SINGLE_INFLUENCE_PCT}%"
    );

    // Budget from memory/test.md §6 is 3 s for a 50k-vertex mesh; this is 7.4k.
    if !cfg!(debug_assertions) {
        assert!(elapsed.as_secs_f32() < 5.0, "full solve took {elapsed:?}");
    }
}
