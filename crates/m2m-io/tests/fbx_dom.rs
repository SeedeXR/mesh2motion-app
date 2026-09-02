//! The FBX document model: objects, connections, flattened properties.
//!
//! Two things are checked here that the reader tests cannot: that a real rig
//! resolves into the objects and relationships the pipeline needs, and that
//! **both readers produce the same model** — asserted directly by building the
//! same document through each path and comparing, rather than by testing each
//! side separately and hoping.

use m2m_io::fbx::binary::{self, FbxDocument, FbxNode, FbxProperty};
use m2m_io::fbx::dom::{Scene, SceneReport, TypedProperty};
use m2m_io::fbx::text;

const MIXAMO: &[u8] =
    include_bytes!("../../../assets/test-files/retarget testing/mixamo-original-rig.fbx");

fn mixamo_scene() -> Scene {
    Scene::from_document(binary::parse(MIXAMO).expect("parses"))
}

#[test]
fn resolves_a_real_rig_into_objects() {
    let scene = mixamo_scene();
    assert_eq!(scene.version, 7700);
    assert_eq!(scene.objects.len(), 642);

    // The exact set of object kinds, not a spot check: a missing kind is how a
    // reshaping bug hides.
    let mut kinds: Vec<(String, usize)> = {
        let mut counts = std::collections::BTreeMap::new();
        for o in scene.objects.values() {
            *counts.entry(o.kind.clone()).or_insert(0usize) += 1;
        }
        counts.into_iter().collect()
    };
    kinds.sort();
    assert_eq!(
        kinds,
        vec![
            ("AnimationCurve".into(), 315),
            ("AnimationCurveNode".into(), 54),
            ("AnimationLayer".into(), 2),
            ("AnimationStack".into(), 2),
            ("Deformer".into(), 131),
            ("Geometry".into(), 2),
            ("Material".into(), 2),
            ("Model".into(), 67),
            ("NodeAttribute".into(), 65),
            ("Pose".into(), 2),
        ]
    );

    // A skin is one Skin deformer per mesh plus one Cluster per influencing
    // bone — the shape the weight import in P2-4 will walk.
    let mut subclasses: Vec<(String, usize)> = {
        let mut counts = std::collections::BTreeMap::new();
        for d in scene.objects_of_kind("Deformer") {
            *counts.entry(d.subclass.clone()).or_insert(0usize) += 1;
        }
        counts.into_iter().collect()
    };
    subclasses.sort();
    assert_eq!(
        subclasses,
        vec![("Cluster".into(), 129), ("Skin".into(), 2)]
    );
}

#[test]
fn flattens_properties70_with_real_values() {
    let scene = mixamo_scene();
    let hips = scene
        .objects_of_kind("Model")
        .into_iter()
        .find(|m| m.name == "mixamorig:Hips")
        .expect("a Mixamo rig has a Hips bone");

    assert_eq!(hips.subclass, "LimbNode");

    // `Lcl Translation` is rewritten to `Lcl_Translation` so the name is a
    // usable identifier, matching the legacy convention.
    let translation = hips.property("Lcl_Translation").expect("Lcl_Translation");
    assert_eq!(translation.type_name, "Lcl_Translation");
    let [x, y, z] = translation.as_vec3().expect("three components");

    // Physical sanity: Mixamo exports centimetres, and a hip sits about a
    // metre up on a human figure. A wrong unit or a mis-parsed property would
    // land orders of magnitude away.
    assert!(
        (80.0..130.0).contains(&y),
        "hip height {y} cm is not plausible for a human figure"
    );
    assert!(
        x.abs() < 1.0,
        "hips should sit on the centre line, got x={x}"
    );
    assert!(z.abs() < 20.0, "hip depth {z} is implausible");

    let scaling = hips.property("Lcl_Scaling").expect("Lcl_Scaling");
    let [sx, sy, sz] = scaling.as_vec3().expect("three components");
    for s in [sx, sy, sz] {
        assert!((s - 1.0).abs() < 1e-6, "unscaled rig expected, got {s}");
    }
}

#[test]
fn builds_the_connection_graph() {
    let scene = mixamo_scene();

    // A Geometry hangs off exactly one Model, which is how the mesh finds its
    // node — the relationship the mesh import depends on.
    let geometry = scene.objects_of_kind("Geometry");
    assert_eq!(geometry.len(), 2);
    for g in &geometry {
        let parents = scene.parents_of(g.id, Some("Model"));
        assert_eq!(
            parents.len(),
            1,
            "geometry {} has parents {parents:?}",
            g.id
        );
    }

    // Every Cluster connects to exactly one bone Model. A cluster without one
    // is a weight set with nothing to weight.
    let clusters: Vec<_> = scene
        .objects_of_kind("Deformer")
        .into_iter()
        .filter(|d| d.subclass == "Cluster")
        .collect();
    assert_eq!(clusters.len(), 129);
    let with_bone = clusters
        .iter()
        .filter(|c| scene.children_of(c.id, Some("Model")).len() == 1)
        .count();
    assert_eq!(with_bone, 129, "every cluster must name exactly one bone");
}

#[test]
fn object_iteration_is_deterministic() {
    // HashMap order varies per run, and a rig built in a different bone order
    // is a different rig. objects_of_kind sorts by id for exactly this reason.
    let scene = mixamo_scene();
    let first: Vec<i64> = scene
        .objects_of_kind("Model")
        .iter()
        .map(|o| o.id)
        .collect();
    for _ in 0..5 {
        let again: Vec<i64> = scene
            .objects_of_kind("Model")
            .iter()
            .map(|o| o.id)
            .collect();
        assert_eq!(first, again);
    }
    assert!(first.windows(2).all(|w| w[0] < w[1]), "sorted ascending");
}

/// Renders a property value by MEANING, not by variant.
///
/// `{:?}` on the raw variant is what made an earlier version of this test
/// vacuous: ASCII yields `I64(104)` where binary yields `F64(104.0)` for the
/// identical source line, so a full comparison failed and the test had been
/// weakened to compare row counts instead. Reading through the accessors is
/// the contract that lets the widths differ, so it is what the digest uses.
fn render(value: &FbxProperty) -> String {
    if let Some(s) = value.as_str() {
        return format!("s:{s}");
    }
    if let Some(v) = value.as_f64_vec() {
        return format!("a:{v:?}");
    }
    match value.as_f64() {
        Some(f) => format!("n:{f}"),
        None => format!("?:{value:?}"),
    }
}

/// The semantic content of a scene, for comparing across readers.
///
/// Deliberately excludes the raw node: the two formats legitimately differ
/// there (binary knows an `i32` is an `i32`; ASCII cannot), and the claim being
/// tested is that the *model* matches, not that the bytes do.
fn digest(scene: &Scene) -> Vec<String> {
    let mut rows: Vec<String> = scene
        .objects
        .values()
        .map(|o| {
            let mut props: Vec<String> = o
                .properties
                .iter()
                .map(|(k, v)| {
                    let values: Vec<String> = v.values.iter().map(render).collect();
                    format!("{k}={}:[{}]", v.type_name, values.join(","))
                })
                .collect();
            props.sort();
            format!(
                "{} {} {:?} {:?} [{}] parents={:?} children={:?}",
                o.id,
                o.kind,
                o.name,
                o.subclass,
                props.join(","),
                scene.parents_of(o.id, None),
                scene.children_of(o.id, None),
            )
        })
        .collect();
    rows.sort();
    rows
}

#[test]
fn both_readers_produce_the_same_model() {
    // The claim the two readers were built around, asserted directly.
    //
    // The same document is expressed twice: once as ASCII text, once as the
    // node tree a binary file would decode to. If the DOM layer depended on a
    // format quirk — a leading `*N`, a name encoded the other way round, a
    // property width — these two would differ.
    let ascii = text::parse(concat!(
        "FBXVersion: 7400\n",
        "Objects:  {\n",
        "\tModel: 7, \"Model::Hips\", \"LimbNode\" {\n",
        "\t\tProperties70:  {\n",
        "\t\t\tP: \"Lcl Translation\", \"Lcl Translation\", \"\", \"A\",0,104,1\n",
        "\t\t\tP: \"InheritType\", \"enum\", \"\", \"\",1\n",
        "\t\t}\n",
        "\t}\n",
        "\tGeometry: 9, \"Geometry::Body\", \"Mesh\" {\n",
        "\t\tVertices: *3 {\n",
        "\t\t\ta: 1,2,3\n",
        "\t\t}\n",
        "\t}\n",
        "}\n",
        "Connections:  {\n",
        "\tC: \"OO\",9,7\n",
        "}\n",
    ))
    .expect("ascii parses");

    // The equivalent binary tree: the name is encoded the other way round, and
    // integers keep their declared width.
    let node = |name: &str, props: Vec<FbxProperty>, children: Vec<FbxNode>| FbxNode {
        name: name.into(),
        properties: props,
        children,
        empty_scope: false,
    };
    let p = |name: &str, ty: &str, flags: &str, values: Vec<FbxProperty>| {
        let mut props = vec![
            FbxProperty::Str(name.into()),
            FbxProperty::Str(ty.into()),
            FbxProperty::Str(String::new()),
            FbxProperty::Str(flags.into()),
        ];
        props.extend(values);
        node("P", props, vec![])
    };
    let bin = FbxDocument {
        version: 7400,
        roots: vec![
            node(
                "Objects",
                vec![],
                vec![
                    node(
                        "Model",
                        vec![
                            FbxProperty::I64(7),
                            FbxProperty::Str("Hips\u{0}\u{1}Model".into()),
                            FbxProperty::Str("LimbNode".into()),
                        ],
                        vec![node(
                            "Properties70",
                            vec![],
                            vec![
                                p(
                                    "Lcl Translation",
                                    "Lcl Translation",
                                    "A",
                                    vec![
                                        FbxProperty::F64(0.0),
                                        FbxProperty::F64(104.0),
                                        FbxProperty::F64(1.0),
                                    ],
                                ),
                                p("InheritType", "enum", "", vec![FbxProperty::I32(1)]),
                            ],
                        )],
                    ),
                    node(
                        "Geometry",
                        vec![
                            FbxProperty::I64(9),
                            FbxProperty::Str("Body\u{0}\u{1}Geometry".into()),
                            FbxProperty::Str("Mesh".into()),
                        ],
                        vec![node(
                            "Vertices",
                            vec![FbxProperty::F64Array(vec![1.0, 2.0, 3.0])],
                            vec![],
                        )],
                    ),
                ],
            ),
            node(
                "Connections",
                vec![],
                vec![node(
                    "C",
                    vec![
                        FbxProperty::Str("OO".into()),
                        FbxProperty::I64(9),
                        FbxProperty::I64(7),
                    ],
                    vec![],
                )],
            ),
        ],
    };

    let from_ascii = Scene::from_document(ascii);
    let from_binary = Scene::from_document(bin);

    // Identity and relationships must match exactly.
    let names = |s: &Scene| {
        let mut v: Vec<(i64, String, String, String)> = s
            .objects
            .values()
            .map(|o| (o.id, o.kind.clone(), o.name.clone(), o.subclass.clone()))
            .collect();
        v.sort();
        v
    };
    assert_eq!(names(&from_ascii), names(&from_binary), "object identity");
    assert_eq!(
        from_ascii.parents_of(9, None),
        from_binary.parents_of(9, None)
    );
    assert_eq!(
        from_ascii.children_of(7, None),
        from_binary.children_of(7, None)
    );

    // Property values must agree through the accessors, which is the contract
    // that lets the widths differ.
    {
        let a = from_ascii.object(7).expect("Model 7 in the ascii model");
        let b = from_binary.object(7).expect("Model 7 in the binary model");
        let mut a_keys: Vec<&String> = a.properties.keys().collect();
        let mut b_keys: Vec<&String> = b.properties.keys().collect();
        a_keys.sort();
        b_keys.sort();
        assert_eq!(a_keys, b_keys, "property names");

        assert_eq!(
            a.property("Lcl_Translation")
                .and_then(TypedProperty::as_vec3),
            Some([0.0, 104.0, 1.0])
        );
        assert_eq!(
            b.property("Lcl_Translation")
                .and_then(TypedProperty::as_vec3),
            Some([0.0, 104.0, 1.0])
        );
        assert_eq!(
            a.property("InheritType").and_then(TypedProperty::as_i64),
            Some(1)
        );
        assert_eq!(
            b.property("InheritType").and_then(TypedProperty::as_i64),
            Some(1)
        );
    }

    // The whole digest, not its length. Comparing `.len()` was the tautology
    // this test existed to avoid: 2 == 2 passes however far the two models
    // diverge in names, values, kinds or connection direction.
    assert_eq!(digest(&from_ascii), digest(&from_binary));
}

#[test]
fn object_to_property_links_resolve_to_a_property() {
    // 215 of the 666 connections in the reference rig are object-to-property,
    // and the animation path walks them back to the property they target. That
    // lookup silently returned None for every `Lcl` channel while the link name
    // kept a space and the flattened key used an underscore.
    let scene = mixamo_scene();

    let mut targeted: std::collections::BTreeSet<String> = Default::default();
    let mut resolved = 0usize;
    let mut unresolved: Vec<String> = Vec::new();

    for (id, links) in &scene.links {
        for link in &links.parents {
            let Some(name) = link.property.as_deref() else {
                continue;
            };
            targeted.insert(name.to_string());
            match scene.object(link.id).and_then(|o| o.property(name)) {
                Some(_) => resolved += 1,
                None => {
                    if unresolved.len() < 5 {
                        unresolved.push(format!("{id} -> {} .{name}", link.id));
                    }
                }
            }
        }
    }

    // The exact set of targeted property names, so a normalisation change shows
    // up here rather than as a silent miss downstream.
    assert_eq!(
        targeted.iter().map(String::as_str).collect::<Vec<_>>(),
        ["Lcl_Rotation", "Lcl_Translation", "d|X", "d|Y", "d|Z"]
    );
    assert!(
        resolved > 0,
        "no OP link resolved to a property; unresolved: {unresolved:?}"
    );
}

#[test]
fn a_document_without_objects_yields_an_empty_scene() {
    // Matching the legacy, which checks `'Connections' in fbxTree` before
    // reading it. Whether an empty scene is acceptable is the caller's call.
    let doc = FbxDocument {
        version: 7400,
        roots: vec![],
    };
    let scene = Scene::from_document(doc);
    assert!(scene.objects.is_empty());
    assert!(scene.links.is_empty());
    assert_eq!(scene.objects_of_kind("Model").len(), 0);
    assert_eq!(scene.children_of(1, None), Vec::<i64>::new());
}

#[test]
fn objects_the_scene_cannot_key_are_counted_rather_than_asserted_away() {
    // Found by `cargo-fuzz` (P2-8) against `fbx_pipeline`, on a mutation of an
    // ASCII seed that put control bytes inside an `Objects` block.
    //
    // Both conditions used to be `debug_assert!`s. That made them panics on
    // untrusted file content in every debug build — including plain
    // `cargo test` — while the release test suite compiled them out and passed
    // straight over. An assertion is for an invariant this code controls; what
    // a file contains is not one.
    //
    // These assertions are on the counters, not on "it did not panic", so they
    // are meaningful in release too.
    let no_id = text::parse(concat!(
        "FBXVersion: 7400\n",
        "Objects:  {\n",
        // Two children whose first attribute is not a number, so neither can
        // be keyed or connected to.
        "\tModel: \"notanumber\", \"Model::x\", \"LimbNode\" {\n",
        "\t}\n",
        "\tGeometry: , \"Geometry::y\", \"Mesh\" {\n",
        "\t}\n",
        // ...and one that is fine, so the block is not wholly rejected.
        "\tModel: 7, \"Model::good\", \"LimbNode\" {\n",
        "\t}\n",
        "}\n",
    ))
    .expect("ascii parses");
    let scene = Scene::from_document(no_id);
    assert_eq!(
        scene.report.objects_without_id, 2,
        "the two unkeyable nodes"
    );
    assert_eq!(scene.objects.len(), 1, "the good one survives");
    assert!(scene.object(7).is_some());

    // A second object claiming an id already taken. Flat id keying cannot hold
    // both, and which one wins is arbitrary — so it has to be reported.
    let duplicate = text::parse(concat!(
        "FBXVersion: 7400\n",
        "Objects:  {\n",
        "\tModel: 7, \"Model::first\", \"LimbNode\" {\n",
        "\t}\n",
        "\tGeometry: 7, \"Geometry::second\", \"Mesh\" {\n",
        "\t}\n",
        "}\n",
    ))
    .expect("ascii parses");
    let scene = Scene::from_document(duplicate);
    assert_eq!(scene.report.duplicate_object_ids, 1);
    assert_eq!(scene.objects.len(), 1, "only one can be keyed at id 7");

    // And a clean file reports nothing, so the counters stay honest.
    let clean = Scene::from_document(
        binary::parse(include_bytes!(
            "../../../assets/test-files/retarget testing/mixamo-original-rig.fbx"
        ))
        .expect("parses"),
    );
    assert_eq!(clean.report, SceneReport::default(), "the reference rig");
}

/// The frame rate the file's tick times are meant to be read at. Blender is
/// what actually caught this — omitting TimeMode made it read this rig's
/// 148-frame clip as 123.5, the same 76,960 keys played 20% slow, with the
/// curve count, key count and driven paths all still matching. Blender is not
/// in CI, so the fact it caught is asserted here too.
#[test]
fn the_reference_rig_carries_its_frame_rate() {
    let scene = Scene::from_document(binary::parse(MIXAMO).expect("parses"));
    assert_eq!(
        scene.time_mode,
        Some(6),
        "TimeMode 6 is 30fps, verified by writing it and reading Blender's frame range"
    );
}

/// A file with no GlobalSettings has no frame rate to report, rather than
/// silently claiming a default one — the caller decides what to assume.
#[test]
fn a_file_without_global_settings_reports_no_frame_rate() {
    let scene = Scene::from_document(binary::FbxDocument {
        version: 7700,
        roots: Vec::new(),
    });
    assert_eq!(scene.time_mode, None);
}
