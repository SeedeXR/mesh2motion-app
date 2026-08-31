//! Building an FBX document from geometry.
//!
//! These assert the structure our own reader can see. They cannot tell whether
//! the file is one an importer will accept — that is
//! `legacy/bench/fbx-conformance.ts`, which runs Blender. Both matter: this
//! catches a wrong count fast, Blender catches a wrong format at all.

use m2m_io::fbx::binary::{self, FbxProperty};
use m2m_io::fbx::{build, dom::Scene, encode, geometry};

/// A unit square: 4 vertices, 2 triangles, with the diagonal shared.
fn square() -> (Vec<f32>, Vec<u32>) {
    (
        vec![0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0],
        vec![0, 1, 2, 0, 2, 3],
    )
}

fn build_square() -> binary::FbxDocument {
    let (positions, triangles) = square();
    build::build(&build::Scene {
        meshes: &[build::Mesh {
            name: "Square",
            positions: &positions,
            triangles: &triangles,
        }],
    })
}

#[test]
fn a_built_document_survives_encoding_and_reparsing() {
    let document = build_square();
    let bytes = encode::encode(&document).expect("encodes");
    let reparsed = binary::parse(&bytes).expect("our reader accepts it");
    assert_eq!(reparsed, document, "the round trip changed the document");
}

#[test]
fn the_last_corner_of_every_polygon_is_negated() {
    // FBX has no per-face vertex count: a polygon's final corner is written
    // bitwise-negated, and that is the only thing separating one face from the
    // next. Get it wrong and the whole mesh becomes one polygon, or none.
    let document = build_square();
    let objects = document.root("Objects").expect("Objects");
    let geometry = objects
        .children
        .iter()
        .find(|c| c.name == "Geometry")
        .expect("Geometry");
    let FbxProperty::I32Array(indices) = geometry
        .child("PolygonVertexIndex")
        .and_then(|n| n.properties.first())
        .expect("PolygonVertexIndex")
    else {
        panic!("expected an i32 array");
    };

    // Two triangles: 0,1,~2, 0,2,~3.
    assert_eq!(indices, &vec![0, 1, !2, 0, 2, !3]);
    // Stated as a property rather than a literal, so a change of winding or
    // vertex order does not make this test wrong for the right reason.
    for (i, &index) in indices.iter().enumerate() {
        let is_last = i % 3 == 2;
        assert_eq!(
            index < 0,
            is_last,
            "corner {i}: only the third corner of each triangle may be negated"
        );
    }
}

#[test]
fn every_object_name_carries_its_class() {
    // Binary FBX pairs a name with its class as `Name\0\x01Class`. Blender
    // splits on that separator and raises "not enough values to unpack"
    // without it — a file written bare does not open at all.
    let document = build_square();
    let objects = document.root("Objects").expect("Objects");
    assert!(!objects.children.is_empty());

    for object in &objects.children {
        let FbxProperty::Str(name) = &object.properties[1] else {
            panic!("{} has no name property", object.name);
        };
        let (before, after) = name
            .split_once('\u{0}')
            .unwrap_or_else(|| panic!("{} name {name:?} has no separator", object.name));
        assert_eq!(after, format!("\u{1}{}", object.name), "{name:?}");
        assert!(!before.is_empty(), "{name:?} has an empty name");
    }
}

#[test]
fn the_declared_counts_match_the_objects_written() {
    // Importers size their tables from Definitions. A count that disagrees
    // with the Objects block is a file describing itself wrongly, and nothing
    // in our reader would notice.
    let (positions, triangles) = square();
    let meshes: Vec<build::Mesh> = (0..3)
        .map(|i| build::Mesh {
            name: ["A", "B", "C"][i],
            positions: &positions,
            triangles: &triangles,
        })
        .collect();
    let document = build::build(&build::Scene { meshes: &meshes });

    let definitions = document.root("Definitions").expect("Definitions");
    let declared = |kind: &str| -> i32 {
        definitions
            .children
            .iter()
            .filter(|c| c.name == "ObjectType")
            .find(|c| matches!(&c.properties[0], FbxProperty::Str(s) if s == kind))
            .and_then(|c| c.child("Count"))
            .and_then(|c| c.properties.first())
            .and_then(|p| match p {
                FbxProperty::I32(v) => Some(*v),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no Count for {kind}"))
    };

    let objects = document.root("Objects").expect("Objects");
    let actual = |kind: &str| objects.children.iter().filter(|c| c.name == kind).count() as i32;

    assert_eq!(declared("Geometry"), actual("Geometry"), "Geometry count");
    assert_eq!(declared("Model"), actual("Model"), "Model count");
    assert_eq!(declared("Geometry"), 3, "three meshes were asked for");
}

#[test]
fn our_own_reader_reads_the_geometry_back() {
    // The strongest thing this file can assert without another reader: the
    // geometry layer, which has its own tests against real files, agrees about
    // what was written.
    let document = build_square();
    let bytes = encode::encode(&document).expect("encodes");
    let scene = Scene::from_document(binary::parse(&bytes).expect("parses"));
    assert_eq!(scene.report, m2m_io::fbx::dom::SceneReport::default());

    let object = scene.objects_of_kind("Geometry")[0];
    assert_eq!(object.name, "Square");
    let mesh = geometry::parse(object, geometry::GeometricTransform::default()).expect("geometry");

    // Two triangles expand to six corners, from four source vertices.
    assert_eq!(mesh.vertex_count(), 6);
    assert_eq!(mesh.source_vertex_count, 4);
    assert_eq!(mesh.report, geometry::GeometryReport::default());
    // And the positions are the square's, not something transposed: corner 0
    // is the origin and corner 1 is (1,0,0).
    assert_eq!(&mesh.positions[..6], &[0.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

    let models = m2m_io::fbx::model::parse_all(&scene);
    assert_eq!(models.models.len(), 1);
    assert_eq!(models.models[0].name, "Square");

    // The two connections, asserted explicitly.
    //
    // Measured: dropping either one is invisible to everything else in this
    // file — a Model with no parent is simply a root, and a Geometry with no
    // Model still parses. Blender catches both (the mesh vanishes), but Blender
    // does not run in CI, so without these two assertions those mutations
    // would ship.
    let geometry_id = object.id;
    let model_id = models.models[0].id;
    assert_eq!(
        scene.parents_of(geometry_id, Some("Model")),
        vec![model_id],
        "the geometry must hang off its model"
    );
    let model_parents: Vec<i64> = scene
        .links
        .get(&model_id)
        .map(|l| l.parents.iter().map(|p| p.id).collect())
        .unwrap_or_default();
    assert_eq!(
        model_parents,
        vec![0],
        "the model must hang off the scene root, id 0"
    );
}
