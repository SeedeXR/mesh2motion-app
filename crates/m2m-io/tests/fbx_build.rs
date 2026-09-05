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
            normals: &[],
            uvs: &[],
            material: None,
            faces: build::Faces::Triangles(&triangles),
        }],
        bones: &[],
        skins: &[],
        materials: &[],
        clips: &[],
        time_mode: 6,
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
            normals: &[],
            uvs: &[],
            material: None,
            faces: build::Faces::Triangles(&triangles),
        })
        .collect();
    let document = build::build(&build::Scene {
        meshes: &meshes,
        bones: &[],
        skins: &[],
        materials: &[],
        clips: &[],
        time_mode: 6,
    });

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

/// Two bones, a parent and its child, with the child weighted onto a square.
fn rigged_square() -> (Vec<f32>, Vec<u32>, [f64; 16]) {
    let (positions, triangles) = square();
    let identity = [
        1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
    ];
    (positions, triangles, identity)
}

fn build_rigged() -> binary::FbxDocument {
    let (positions, triangles, identity) = rigged_square();
    let bones = [
        build::Bone {
            name: "root",
            parent: None,
            translation: [0.0, 0.0, 0.0],
            rotation: [0.0; 3],
            scale: [1.0; 3],
            pre_rotation: [0.0; 3],
        },
        build::Bone {
            name: "child",
            parent: Some(0),
            translation: [0.0, 10.0, 0.0],
            rotation: [0.0; 3],
            scale: [1.0; 3],
            pre_rotation: [90.0, 0.0, 0.0],
        },
    ];
    let indices = [0u32, 1, 2, 3];
    let weights = [1.0f64, 1.0, 0.5, 0.5];
    let clusters = [
        build::Cluster {
            bone: 1,
            indices: &indices,
            weights: &weights,
            transform: identity,
            transform_link: identity,
        },
        // A bone bound to the skin but influencing nothing: 40 of the reference
        // rig's 129 clusters are exactly this.
        build::Cluster {
            bone: 0,
            indices: &[],
            weights: &[],
            transform: identity,
            transform_link: identity,
        },
    ];
    build::build(&build::Scene {
        meshes: &[build::Mesh {
            name: "Square",
            positions: &positions,
            normals: &[],
            uvs: &[],
            material: None,
            faces: build::Faces::Triangles(&triangles),
        }],
        bones: &bones,
        skins: &[build::Skin {
            mesh: 0,
            clusters: &clusters,
        }],
        materials: &[],
        clips: &[],
        time_mode: 6,
    })
}

#[test]
fn a_rigged_document_survives_encoding_and_our_own_layers() {
    let document = build_rigged();
    let bytes = encode::encode(&document).expect("encodes");
    let reparsed = binary::parse(&bytes).expect("parses");
    assert_eq!(reparsed, document);

    let scene = Scene::from_document(reparsed);
    assert_eq!(scene.report, m2m_io::fbx::dom::SceneReport::default());

    // The skeleton comes back as a two-bone chain, in order.
    let models = m2m_io::fbx::model::parse_all(&scene);
    let bones: Vec<&str> = models
        .models
        .iter()
        .filter(|m| m.is_bone())
        .map(|m| m.name.as_str())
        .collect();
    assert_eq!(bones.len(), 2, "bones: {bones:?}");
    let child = models
        .models
        .iter()
        .find(|m| m.name == "child")
        .expect("child");
    let root = models
        .models
        .iter()
        .find(|m| m.name == "root")
        .expect("root");
    assert_eq!(child.parent, Some(root.id), "the chain must survive");
    // PreRotation is what places a Mixamo joint; 440 of 522 models in the
    // corpus carry one, so a writer that drops it moves every bone.
    assert_eq!(child.transform.pre_rotation.x, 90.0);
    assert_eq!(child.transform.translation.y, 10.0);

    // And the skin comes back with both clusters, including the empty one.
    let (skins, skipped) = m2m_io::fbx::skin::parse_all(&scene);
    assert_eq!(skipped, 0);
    assert_eq!(skins.len(), 1);
    assert_eq!(skins[0].clusters.len(), 2, "the empty cluster is kept");
    assert_eq!(skins[0].report, m2m_io::fbx::skin::SkinReport::default());

    let weighted: Vec<&m2m_io::fbx::skin::Cluster> = skins[0]
        .clusters
        .iter()
        .filter(|c| !c.indices.is_empty())
        .collect();
    assert_eq!(weighted.len(), 1);
    assert_eq!(weighted[0].indices, vec![0, 1, 2, 3]);
    assert_eq!(weighted[0].weights, vec![1.0, 1.0, 0.5, 0.5]);
}

#[test]
fn every_bone_gets_the_attribute_that_makes_it_a_joint() {
    // A Model alone is an empty transform. The NodeAttribute with
    // TypeFlags "Skeleton" is what makes an importer build an armature —
    // measured: without it Blender imports the bones as empties and produces
    // no armature at all.
    let document = build_rigged();
    let objects = document.root("Objects").expect("Objects");

    let attributes: Vec<&binary::FbxNode> = objects
        .children
        .iter()
        .filter(|c| c.name == "NodeAttribute")
        .collect();
    assert_eq!(attributes.len(), 2, "one per bone");
    for attribute in &attributes {
        let flags = attribute
            .child("TypeFlags")
            .and_then(|n| n.properties.first())
            .expect("TypeFlags");
        assert!(
            matches!(flags, FbxProperty::Str(s) if s == "Skeleton"),
            "TypeFlags must be Skeleton, got {flags:?}"
        );
    }

    // And each is connected to its Model, or it marks nothing.
    let scene = Scene::from_document(document);
    for attribute in scene.objects_of_kind("NodeAttribute") {
        let parents = scene.parents_of(attribute.id, Some("Model"));
        assert_eq!(parents.len(), 1, "{} is unattached", attribute.name);
    }
}

#[test]
fn the_bone_drives_the_cluster_and_not_the_other_way_round() {
    // The direction that surprises everyone reading FBX connections: the bone's
    // Model is the CHILD of the Cluster. Reversing it produces a file our
    // reader still parses — `skin::parse_all` looks for a Model child of the
    // cluster — but with no bone found, so every cluster loses its bone.
    let document = build_rigged();
    let scene = Scene::from_document(document);

    let (skins, _) = m2m_io::fbx::skin::parse_all(&scene);
    assert_eq!(skins[0].report.clusters_without_bone, 0);
    for cluster in &skins[0].clusters {
        let bone = scene.object(cluster.bone_id).expect("the bone exists");
        assert_eq!(bone.kind, "Model");
    }

    // The skin itself hangs off the geometry, or it deforms nothing.
    let geometry = scene.objects_of_kind("Geometry")[0];
    assert_eq!(skins[0].geometry_id, geometry.id);
}

#[test]
fn a_cluster_with_no_influence_writes_no_arrays() {
    // 40 of the reference rig's 129 clusters carry no Indexes node at all,
    // every one a finger bone. Writing an empty array instead of omitting the
    // node would make those bones look weighted-with-nothing rather than
    // simply unused.
    let document = build_rigged();
    let objects = document.root("Objects").expect("Objects");
    let clusters: Vec<&binary::FbxNode> = objects
        .children
        .iter()
        .filter(|c| {
            c.name == "Deformer"
                && matches!(&c.properties[2], FbxProperty::Str(s) if s == "Cluster")
        })
        .collect();
    assert_eq!(clusters.len(), 2);

    let empty = clusters
        .iter()
        .filter(|c| c.child("Indexes").is_none())
        .count();
    assert_eq!(empty, 1, "the influence-less cluster omits its arrays");

    // The one that does have influence carries both arrays, at equal length.
    let full = clusters
        .iter()
        .find(|c| c.child("Indexes").is_some())
        .expect("a weighted cluster");
    let len = |name: &str| match full.child(name).and_then(|n| n.properties.first()) {
        Some(FbxProperty::I32Array(a)) => a.len(),
        Some(FbxProperty::F64Array(a)) => a.len(),
        other => panic!("{name}: {other:?}"),
    };
    assert_eq!(len("Indexes"), len("Weights"));
    // Both bind matrices are always written, empty cluster or not — they place
    // the bone, and a cluster without them binds to the identity.
    for cluster in &clusters {
        assert!(cluster.child("Transform").is_some(), "Transform missing");
        assert!(
            cluster.child("TransformLink").is_some(),
            "TransformLink missing"
        );
    }
}

/// A one-bone rig with one clip: two keys of translation on X.
///
/// Times are FBX ticks. 46186158000 ticks is one second, so these two keys are
/// frame 0 and frame 30 of a 30fps clip.
const TICK: i64 = 46_186_158_000;

fn build_animated(time_mode: i32) -> binary::FbxDocument {
    let (positions, triangles) = square();
    build::build(&build::Scene {
        meshes: &[build::Mesh {
            name: "Square",
            positions: &positions,
            normals: &[],
            uvs: &[],
            material: None,
            faces: build::Faces::Triangles(&triangles),
        }],
        bones: &[build::Bone {
            name: "Root",
            parent: None,
            translation: [0.0, 0.0, 0.0],
            rotation: [0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            pre_rotation: [0.0, 0.0, 0.0],
        }],
        skins: &[],
        materials: &[],
        clips: &[build::Clip {
            name: "Walk",
            duration: TICK,
            layer: "Layer0",
            channels: &[build::Channel {
                bone: 0,
                property: "Lcl Translation",
                kind: "T",
                curves: &[build::Curve {
                    axis: "d|X",
                    times: &[0, TICK],
                    values: &[0.0, 7.0],
                    default: 0.0,
                }],
            }],
        }],
        time_mode,
    })
}

/// The frame rate is written as asked. This is the one Blender caught: with no
/// TimeMode, it read the reference rig's 148-frame clip as 123.5 — the same
/// keys at 25fps instead of 30 — while the curve count, key count and driven
/// paths all still matched. Blender is not in CI, so assert it here.
#[test]
fn the_frame_rate_is_written_into_global_settings() {
    for time_mode in [6, 11] {
        let document = build_animated(time_mode);
        let written = document
            .root("GlobalSettings")
            .and_then(|gs| gs.child("Properties70"))
            .expect("GlobalSettings/Properties70")
            .children_named("P")
            .find_map(|p| match (p.properties.first(), p.properties.get(4)) {
                (Some(FbxProperty::Str(n)), Some(FbxProperty::I32(v))) if n == "TimeMode" => {
                    Some(*v)
                }
                _ => None,
            });
        assert_eq!(
            written,
            Some(time_mode),
            "TimeMode {time_mode} was not written"
        );
    }
}

/// A built clip reads back through the animation parser with its name, its
/// keys and its times intact. Not an independent reader — that is the Blender
/// gate — but it catches a wrong id, a missing connection or a dropped curve
/// in CI, where Blender does not run.
#[test]
fn a_built_clip_reads_back_as_a_track() {
    let document = build_animated(6);
    let bytes = encode::encode(&document).expect("encodes");
    let scene = Scene::from_document(binary::parse(&bytes).expect("reparses"));
    let models = m2m_io::fbx::model::parse_all(&scene);
    let (clips, report) = m2m_io::fbx::animation::parse_all(&scene, &models);

    assert_eq!(clips.len(), 1, "expected one clip, got {clips:?}");
    let clip = &clips[0];
    assert_eq!(clip.name, "Walk");
    assert_eq!(clip.tracks.len(), 1, "the channel did not reach a Model");
    let track = &clip.tracks[0];
    assert_eq!(track.times, vec![0.0, 1.0], "key times, in seconds");
    // Only X was given a curve; Y and Z fall back to the curve node's default.
    assert_eq!(track.values, vec![0.0, 0.0, 0.0, 7.0, 0.0, 0.0]);
    assert_eq!(clip.duration, 1.0);
    assert_eq!(report.curve_nodes_without_model, 0);
    assert_eq!(report.unattached_curves, 0);
}

/// The layer name is the source's, not a hardcoded one: Blender names the
/// imported action `<Armature>|<stack>|<layer>`, so a wrong layer name renames
/// every action in the file.
#[test]
fn the_layer_keeps_the_name_it_was_given() {
    let document = build_animated(6);
    let objects = document.root("Objects").expect("Objects");
    let layers: Vec<&str> = objects
        .children_named("AnimationLayer")
        .filter_map(|n| n.properties.get(1).and_then(FbxProperty::as_str))
        .collect();
    assert_eq!(layers, vec!["Layer0\u{0}\u{1}AnimLayer"]);
}

/// The animation layer declares an empty scope rather than no scope at all.
///
/// A childless node can either declare a nested list holding only its
/// terminating null record, or declare none. Our reader represents both as
/// `children: []`, so this is invisible to it — but **assimp reads an
/// `AnimationLayer` written without the empty list as no layer at all**, and
/// the file then loads with zero animations: mesh and bones perfect, every
/// keyframe gone. Blender and three.js accept either form, which is why this
/// needed a third reader to find. The reference rig writes the empty list for
/// its two layers and for `References`, and for nothing else.
#[test]
fn the_animation_layer_declares_an_empty_scope() {
    let document = build_animated(6);
    let layer = document
        .root("Objects")
        .expect("Objects")
        .children_named("AnimationLayer")
        .next()
        .expect("an AnimationLayer");
    assert!(layer.children.is_empty(), "the layer has no child nodes");
    assert!(
        layer.empty_scope,
        "without this assimp reads the file with no animation at all"
    );
}
