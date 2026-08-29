//! Validation against a real production mesh, not a synthetic fixture.
//!
//! Synthetic tetrahedra cannot exhibit the defects that actually matter —
//! seam-split duplicate vertices, degenerate triangles from decimation, open
//! boundaries at the eyes and mouth. This fixture is the geometry of
//! `legacy/static/test-files/human-small.glb`, exported by
//! `legacy/bench/dump-fixtures.ts`.

use m2m_core::mesh::Mesh;

/// `[u32 vertexCount][u32 indexCount][f32 positions...][u32 indices...]`, LE.
fn load_fixture(bytes: &[u8]) -> Mesh {
    assert!(bytes.len() >= 8, "fixture truncated");
    let vertex_count = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    let index_count = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;

    let pos_bytes = 8 + vertex_count * 3 * 4;
    let idx_bytes = pos_bytes + index_count * 4;
    assert_eq!(
        bytes.len(),
        idx_bytes,
        "fixture size does not match its header"
    );

    let positions: Vec<f32> = bytes[8..pos_bytes]
        .chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect();
    let indices: Vec<u32> = bytes[pos_bytes..idx_bytes]
        .chunks_exact(4)
        .map(|c| u32::from_le_bytes(c.try_into().unwrap()))
        .collect();

    Mesh::from_flat(&positions, &indices).expect("fixture is a valid mesh")
}

#[test]
fn validates_a_real_human_mesh() {
    let mesh = load_fixture(include_bytes!("fixtures/human-small.bin"));

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
    let mesh = load_fixture(include_bytes!("fixtures/human-small.bin"));
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
    let mesh = load_fixture(include_bytes!("fixtures/human-small.bin"));
    let a = mesh.validate(1e-4);
    let b = mesh.validate(1e-4);
    assert_eq!(a, b);
}
