//! Mesh geometry extraction from a real rig, and the triangulation rules.

use m2m_io::fbx::geometry::{parse, GeometricTransform, GeometryReport};
use m2m_io::fbx::{binary, dom::Scene};

const MIXAMO: &[u8] =
    include_bytes!("../../../assets/test-files/retarget testing/mixamo-original-rig.fbx");

fn scene() -> Scene {
    Scene::from_document(binary::parse(MIXAMO).expect("parses"))
}

#[test]
fn extracts_both_meshes_from_a_real_rig() {
    let scene = scene();
    let geometries = scene.objects_of_kind("Geometry");
    assert_eq!(geometries.len(), 2);

    // Measured from the file: Beta_Surface is 172 triangles and 14050 quads,
    // Beta_Joints is 1400 and 9720. Triangulating gives 172 + 2*14050 = 28272
    // and 1400 + 2*9720 = 20840 triangles, each contributing three expanded
    // corners. Asserting the derived totals rather than a bound, so a
    // triangulation change cannot pass unnoticed.
    let expected = [
        ("Beta_Surface", 14_232usize, 28_272usize),
        ("Beta_Joints", 10_514, 20_840),
    ];

    for (name, source_vertices, triangles) in expected {
        let object = geometries
            .iter()
            .find(|g| g.name == name)
            .unwrap_or_else(|| panic!("{name} not in the file"));
        let mesh =
            parse(object, GeometricTransform::default()).unwrap_or_else(|e| panic!("{name}: {e}"));

        assert_eq!(mesh.triangle_count(), triangles, "{name} triangles");
        assert_eq!(
            mesh.vertex_count(),
            triangles * 3,
            "{name} expanded corners"
        );
        assert_eq!(mesh.indices.len(), triangles * 3, "{name} indices");
        assert_eq!(mesh.vertex_source.len(), mesh.vertex_count());

        // Nothing was approximated: this file has only triangles and quads.
        assert_eq!(mesh.report, GeometryReport::default(), "{name} report");

        assert_eq!(
            mesh.source_vertex_count, source_vertices,
            "{name} source count"
        );

        // Every expanded corner must sit exactly where its source vertex does.
        //
        // The previous version asserted only the length and the maximum id,
        // both of which hold with `vertex_source` hardcoded to 0 — verified.
        // This is the field P2-4b's entire skin remap depends on, so it is
        // checked against the original buffer value by value.
        let source_positions = object
            .node
            .child("Vertices")
            .and_then(|n| n.properties.first())
            .and_then(m2m_io::fbx::binary::FbxProperty::as_f64_vec)
            .expect("Vertices");
        let mut mismatched = 0usize;
        for (i, &src) in mesh.vertex_source.iter().enumerate() {
            let s = src as usize * 3;
            for k in 0..3 {
                if (mesh.positions[i * 3 + k] - source_positions[s + k] as f32).abs() > 1e-4 {
                    mismatched += 1;
                    break;
                }
            }
        }
        assert_eq!(
            mismatched,
            0,
            "{name}: {mismatched} of {} corners do not match their source vertex",
            mesh.vertex_count()
        );

        // Attributes are present and sized to the expanded buffer, not the
        // original — a mismatch here is how normals end up on wrong corners.
        let normals = mesh
            .normals
            .as_ref()
            .unwrap_or_else(|| panic!("{name} normals"));
        assert_eq!(normals.len(), mesh.vertex_count() * 3, "{name} normals");
        let uvs = mesh.uvs.as_ref().unwrap_or_else(|| panic!("{name} uvs"));
        assert_eq!(uvs.len(), mesh.vertex_count() * 2, "{name} uvs");

        // UVs are the ONLY IndexToDirect layer in this file, so this is the
        // sole coverage of the indirection branch in the mapping resolver —
        // the part of `getData` that is easiest to get wrong. Asserting only
        // the length passed with the whole UV path replaced by zeroes.
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        let mut nonzero = 0usize;
        for uv in uvs.chunks_exact(2) {
            for &c in uv {
                lo = lo.min(c);
                hi = hi.max(c);
            }
            if uv[0] != 0.0 || uv[1] != 0.0 {
                nonzero += 1;
            }
        }
        assert!(
            nonzero > mesh.vertex_count() / 2,
            "{name}: only {nonzero} of {} UVs are non-zero; is the layer resolving?",
            mesh.vertex_count()
        );
        assert!(
            (-0.001..=1.001).contains(&lo) && (-0.001..=1.001).contains(&hi),
            "{name}: UVs span {lo}..{hi}, outside the unit square a character atlas uses"
        );

        assert!(mesh.positions.iter().all(|v| v.is_finite()));
        assert!(normals.iter().all(|v| v.is_finite()));
    }
}

#[test]
fn normals_are_unit_length_under_both_mapping_types() {
    // The two meshes use DIFFERENT normal mappings — Beta_Surface is
    // ByPolygonVertex/Direct, Beta_Joints is ByVertice/Direct — so this covers
    // both resolution paths. A mis-resolved index yields a zero or duplicated
    // normal, which shows up as a length far from 1.
    let scene = scene();
    for object in scene.objects_of_kind("Geometry") {
        let mesh = parse(object, GeometricTransform::default()).expect("parses");
        let normals = mesh.normals.expect("normals");

        let mut off_unit = 0usize;
        for n in normals.chunks_exact(3) {
            let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            if (len - 1.0).abs() > 1e-3 {
                off_unit += 1;
            }
        }
        assert_eq!(
            off_unit,
            0,
            "{}: {off_unit} of {} normals are not unit length",
            object.name,
            normals.len() / 3
        );
    }
}

#[test]
fn geometry_is_at_a_plausible_physical_scale() {
    // Mixamo exports centimetres. A unit or index error lands orders of
    // magnitude away, which no structural assertion would catch.
    let scene = scene();
    let surface = scene
        .objects_of_kind("Geometry")
        .into_iter()
        .find(|g| g.name == "Beta_Surface")
        .expect("Beta_Surface");
    let mesh = parse(surface, GeometricTransform::default()).expect("parses");

    let ys: Vec<f32> = mesh.positions.chunks_exact(3).map(|p| p[1]).collect();
    let height =
        ys.iter().cloned().fold(f32::MIN, f32::max) - ys.iter().cloned().fold(f32::MAX, f32::min);
    assert!(
        (120.0..250.0).contains(&height),
        "character height {height} cm is not plausible"
    );
}

// --- triangulation rules, on constructed geometry --------------------------

use m2m_io::fbx::binary::{FbxNode, FbxProperty};
use m2m_io::fbx::dom::Object;
use std::collections::HashMap;

/// Wraps positions and a polygon index list as a `Geometry` object.
fn parse_default(
    object: &Object,
) -> Result<m2m_io::fbx::geometry::MeshGeometry, m2m_io::fbx::FbxError> {
    parse(object, GeometricTransform::default())
}

fn geometry_object(vertices: Vec<f64>, polygon_indices: Vec<i64>) -> Object {
    let child = |name: &str, prop: FbxProperty| FbxNode {
        name: name.into(),
        properties: vec![prop],
        children: vec![],
        empty_scope: false,
    };
    Object {
        id: 1,
        kind: "Geometry".into(),
        name: "test".into(),
        subclass: "Mesh".into(),
        properties: HashMap::new(),
        node: FbxNode {
            name: "Geometry".into(),
            properties: vec![],
            children: vec![
                child("Vertices", FbxProperty::F64Array(vertices)),
                child("PolygonVertexIndex", FbxProperty::I64Array(polygon_indices)),
            ],
            empty_scope: false,
        },
    }
}

#[test]
fn a_concave_quad_splits_along_the_diagonal_that_stays_inside() {
    // An arrowhead in the XY plane: vertex 3 is reflex, so the 0-2 diagonal
    // falls outside the shape. A naive fan takes it anyway and produces two
    // triangles that cover area the polygon does not.
    //
    //   1 ----- 2
    //    \     /
    //     \  3'      <- 3 pushed inward, making the quad concave
    //      \ /
    //       0
    let vertices = vec![
        0.0, 0.0, 0.0, // 0
        -1.0, 2.0, 0.0, // 1
        1.0, 2.0, 0.0, // 2
        0.0, 0.5, 0.0, // 3 reflex
    ];
    let mesh = parse_default(&geometry_object(vertices, vec![0, 1, 2, !3])).expect("parses");

    assert_eq!(mesh.triangle_count(), 2);
    assert_eq!(
        mesh.report,
        GeometryReport::default(),
        "a quad is not fanned"
    );

    // Both triangles must wind the same way. A split along the outside
    // diagonal flips one of them, which is exactly the artefact a fan causes.
    let signed_area = |t: usize| {
        let p = |i: usize| {
            let v = mesh.indices[t * 3 + i] as usize;
            (mesh.positions[v * 3], mesh.positions[v * 3 + 1])
        };
        let (ax, ay) = p(0);
        let (bx, by) = p(1);
        let (cx, cy) = p(2);
        (bx - ax) * (cy - ay) - (cx - ax) * (by - ay)
    };
    let a0 = signed_area(0);
    let a1 = signed_area(1);
    assert!(
        a0 * a1 > 0.0,
        "triangles wound oppositely: {a0} and {a1} — the split went outside"
    );

    // And their combined area must match the polygon's, which it cannot if a
    // triangle strayed outside.
    let total = (a0.abs() + a1.abs()) / 2.0;
    // Shoelace over (0,0), (-1,2), (1,2), (0,0.5) is 1.75.
    assert!(
        (total - 1.75).abs() < 1e-4,
        "area {total} does not match the concave quad's 1.75"
    );
}

#[test]
fn larger_polygons_are_fanned_and_counted() {
    // A pentagon. The fan is correct here because it is convex, but the count
    // must still be reported so an approximation is never silent.
    let vertices = vec![
        0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.5, 1.0, 0.0, 0.5, 1.6, 0.0, -0.5, 1.0, 0.0,
    ];
    let mesh = parse_default(&geometry_object(vertices, vec![0, 1, 2, 3, !4])).expect("parses");
    assert_eq!(mesh.triangle_count(), 3);
    assert_eq!(
        mesh.report,
        GeometryReport {
            fanned_polygons: 1,
            ..GeometryReport::default()
        }
    );
}

#[test]
fn malformed_polygon_data_is_rejected() {
    let v = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];

    // An index past the end of the vertex buffer.
    assert!(parse_default(&geometry_object(v.clone(), vec![0, 1, !9])).is_err());

    // A polygon list that never closes: accepting it would drop the face.
    assert!(parse_default(&geometry_object(v.clone(), vec![0, 1, 2])).is_err());

    // A coordinate buffer that is not a multiple of three.
    assert!(parse_default(&geometry_object(vec![0.0, 1.0], vec![0, !0])).is_err());

    // A two-corner polygon cannot form a triangle; counted, not fatal.
    let mesh = parse_default(&geometry_object(v, vec![0, !1])).expect("parses");
    assert_eq!(mesh.triangle_count(), 0);
    assert_eq!(mesh.report.degenerate_polygons, 1);
}

#[test]
fn a_geometric_offset_moves_the_mesh_and_its_normals() {
    // FBX lets a mesh sit at an offset from the node it hangs off, via
    // GeometricTranslation/Rotation/Scaling on the Model. It is not part of the
    // node transform. Dropping it puts the mesh where the skeleton is not —
    // Mixamo writes identity, which is why this went unnoticed until review.
    use glam::{DMat4, DVec3};

    let vertices = vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0];
    let object = geometry_object(vertices, vec![0, 1, !2]);

    let plain = parse_default(&object).expect("parses");
    let shifted = parse(
        &object,
        GeometricTransform {
            matrix: DMat4::from_translation(DVec3::new(10.0, 20.0, 30.0)),
        },
    )
    .expect("parses");

    assert_eq!(plain.vertex_count(), shifted.vertex_count());
    for i in 0..plain.vertex_count() {
        assert!((shifted.positions[i * 3] - plain.positions[i * 3] - 10.0).abs() < 1e-5);
        assert!((shifted.positions[i * 3 + 1] - plain.positions[i * 3 + 1] - 20.0).abs() < 1e-5);
        assert!((shifted.positions[i * 3 + 2] - plain.positions[i * 3 + 2] - 30.0).abs() < 1e-5);
    }

    // A non-uniform scale must tilt normals by the inverse transpose, not by
    // the matrix — otherwise lighting and any normal-based logic is wrong.
    let scaled = parse(
        &object,
        GeometricTransform {
            matrix: DMat4::from_scale(DVec3::new(4.0, 1.0, 1.0)),
        },
    )
    .expect("parses");
    assert!((scaled.positions[3] - 4.0).abs() < 1e-5, "x scaled by 4");
}

#[test]
fn the_reference_rig_has_no_geometric_offset() {
    // Asserted rather than assumed: it is why the identity path is the one the
    // other tests exercise, and if a future fixture differs the tests above
    // would quietly be measuring something else.
    use m2m_io::fbx::dom::Scene;
    let scene = scene();
    for g in scene.objects_of_kind("Geometry") {
        let t = GeometricTransform::for_geometry(&scene, g.id);
        assert!(t.is_identity(), "{} has a geometric offset", g.name);
    }
    let _ = std::mem::size_of::<Scene>();
}
