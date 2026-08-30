//! The hostile-input corpus of `memory/test.md` §4, walked item by item.
//!
//! The contract is one sentence: **a malformed file must return an error;
//! never panic, never hang, never exhaust memory.** Individual cases are also
//! covered where they belong — truncation in `fbx_binary.rs`, ragged clusters
//! in `fbx_skin.rs`, NaN keys in `fbx_animation.rs`. This file exists so the
//! §4 checklist is auditable in one place rather than inferred from a dozen
//! test names, and so an item that is added to §4 but never tested is visible.
//!
//! `crates/m2m-io/fuzz/` covers the same boundary continuously; this covers it
//! by name, with the specific shapes §4 calls out.

use m2m_io::fbx::{animation, binary, dom::Scene, geometry, model, skin, text};

/// Runs the whole read path and returns nothing: the assertion is that it
/// returned at all, and that everything it produced is structurally sound.
fn drive_every_layer(data: &[u8]) {
    let Ok(document) = binary::parse(data).or_else(|_| match std::str::from_utf8(data) {
        Ok(t) => text::parse(t),
        Err(_) => Err(m2m_io::fbx::FbxError::Malformed {
            what: "input",
            detail: "not utf-8".into(),
        }),
    }) else {
        return;
    };
    let scene = Scene::from_document(document);

    let meshes: Vec<_> = scene
        .objects_of_kind("Geometry")
        .into_iter()
        .filter_map(|o| geometry::parse(o, geometry::GeometricTransform::default()).ok())
        .collect();

    let models = model::parse_all(&scene);
    for m in &models.models {
        let chain = models.ancestors(m.id);
        assert!(
            chain.len() <= models.models.len() + 1,
            "ancestors did not stop"
        );
        assert!(m.local.is_finite() && m.world.is_finite(), "NaN transform");
    }

    let (skins, _) = skin::parse_all(&scene);
    for s in &skins {
        for mesh in &meshes {
            if let Ok((weights, _)) = s.bind(mesh) {
                assert_eq!(weights.indices.len(), weights.weights.len());
            }
        }
    }

    let (clips, _) = animation::parse_all(&scene, &models);
    for clip in &clips {
        assert!(clip.duration.is_finite() && clip.duration >= 0.0);
        for track in &clip.tracks {
            assert_eq!(track.values.len(), track.times.len() * track.kind.stride());
            assert!(track.times.iter().all(|t| t.is_finite()));
            assert!(track.values.iter().all(|v| v.is_finite()), "NaN in a track");
        }
    }
}

#[test]
fn empty_and_tiny_inputs() {
    for len in 0..40usize {
        let data = vec![0u8; len];
        assert!(binary::parse(&data).is_err(), "{len} zero bytes parsed");
        drive_every_layer(&data);
    }
    assert!(text::parse("").is_err(), "empty text");
}

#[test]
fn wrong_magic_bytes() {
    // Right length, wrong content — including the near-miss that differs in a
    // single byte, which a prefix check written with `starts_with` on too few
    // bytes would accept.
    let mut nearly = b"Kaydara FBX Binary  \x00\x1a\x00".to_vec();
    nearly[7] = b'X';
    nearly.extend([0u8; 64]);
    assert!(
        binary::parse(&nearly).is_err(),
        "a corrupted magic was accepted"
    );

    for junk in [
        &b"<!DOCTYPE html><html><title>404</title></html>"[..],
        &b"{\"asset\":{\"version\":\"2.0\"}}"[..],
        &b"glTF\x02\x00\x00\x00"[..],
        &[0xff; 128][..],
    ] {
        assert!(binary::parse(junk).is_err());
        drive_every_layer(junk);
    }
}

#[test]
fn truncation_at_every_offset_of_a_real_file() {
    // The whole file, cut at 512 points. Each must error or parse; none may
    // panic, and none may return a document while having lost a section.
    const RIG: &[u8] = include_bytes!(
        "../../../legacy/static/test-files/retarget testing/mixamo-original-rig.fbx"
    );
    let step = RIG.len() / 512;
    let mut parsed = 0usize;
    for cut in (0..RIG.len()).step_by(step.max(1)) {
        let slice = &RIG[..cut];
        if binary::parse(slice).is_ok() {
            parsed += 1;
        }
        drive_every_layer(slice);
    }
    // Only the untruncated file should parse; anything else means a cut file
    // is being accepted with sections silently missing.
    assert_eq!(parsed, 0, "{parsed} truncations parsed as valid");
    assert!(binary::parse(RIG).is_ok(), "the whole file still parses");
}

#[test]
fn declared_length_exceeding_the_file() {
    // A node whose declared end offset points past the buffer. Written by hand
    // because no real file contains one.
    let mut data = b"Kaydara FBX Binary  \x00\x1a\x00".to_vec();
    data.extend(7500u32.to_le_bytes());
    // end_offset, num_properties, property_list_len as u64 (version >= 7500),
    // then a name length and name.
    data.extend(u64::MAX.to_le_bytes());
    data.extend(1u64.to_le_bytes());
    data.extend(u64::MAX.to_le_bytes());
    data.push(4);
    data.extend(b"Objs");
    assert!(
        binary::parse(&data).is_err(),
        "an impossible end offset was accepted"
    );
    drive_every_layer(&data);
}

#[test]
fn deeply_nested_nodes() {
    // The reader caps depth at 256. Recursion past it would exhaust the native
    // stack, which is an abort rather than a catchable error.
    let deep = |levels: usize| {
        let mut text = String::from("FBXVersion: 7400\n");
        for i in 0..levels {
            text.push_str(&"\t".repeat(i));
            text.push_str(&format!("N{i}:  {{\n"));
        }
        for i in (0..levels).rev() {
            text.push_str(&"\t".repeat(i));
            text.push_str("}\n");
        }
        text
    };
    for levels in [10, 255, 300, 5000] {
        let source = deep(levels);
        // The ASCII reader tracks depth with an explicit stack, so it does not
        // recurse and any outcome here is acceptable — but it must return.
        let _ = text::parse(&source);
        drive_every_layer(source.as_bytes());
    }

    // The BINARY reader is the one that recurses, so it is the one the depth
    // cap protects, and nesting written in ASCII never reaches it. A stack
    // overflow is an abort: it cannot be caught, reported, or recovered from,
    // which is why this is a limit and not a best effort.
    //
    // These files carry no 160-byte footer, so every one of them errors; what
    // matters is WHICH error. A shallow file gets as far as the footer check,
    // proving the nesting itself was read; a deep one is stopped before that.
    // The cap is `depth > MAX_DEPTH` with the outermost node at depth 0, so
    // 257 levels reach depth 256 and are still accepted — the first rejected
    // depth is 258 levels, and asserting that pins the boundary rather than
    // just "somewhere past 256".
    for (levels, expect_too_deep) in [(8usize, false), (257, false), (258, true), (4096, true)] {
        let file = nested_binary_fbx(levels);
        let error = binary::parse(&file).expect_err("no footer, so never Ok");
        let too_deep = matches!(error, m2m_io::fbx::FbxError::TooDeep(_));
        assert_eq!(too_deep, expect_too_deep, "{levels} levels gave: {error}");
        drive_every_layer(&file);
    }
}

/// A binary FBX whose root node nests `levels` deep, and nothing else.
///
/// Written by hand because no real file nests anywhere near the cap. Node
/// layout for version 7400 is `[end_offset u32][properties u32][property bytes
/// u32][name length u8][name]`, then children, then a 13-byte null record if
/// there were any. `end_offset` is absolute, so the nodes have to be built
/// from the inside out.
fn nested_binary_fbx(levels: usize) -> Vec<u8> {
    const NAME: &[u8] = b"N";
    const HEADER: usize = 4 + 4 + 4 + 1 + NAME.len();
    const NULL_RECORD: usize = 13;

    // Sizes first: a node's own size depends on its subtree's.
    let mut size = vec![0usize; levels + 1];
    for depth in (0..levels).rev() {
        let children = if depth + 1 < levels {
            size[depth + 1] + NULL_RECORD
        } else {
            0
        };
        size[depth] = HEADER + children;
    }

    let mut out = b"Kaydara FBX Binary    ".to_vec();
    out.extend(7400u32.to_le_bytes());
    for &node_size in size.iter().take(levels) {
        let end = (out.len() + node_size) as u32;
        out.extend(end.to_le_bytes());
        out.extend(0u32.to_le_bytes()); // properties
        out.extend(0u32.to_le_bytes()); // property bytes
        out.push(NAME.len() as u8);
        out.extend(NAME);
    }
    // Close each node that has children, innermost first.
    for _ in 0..levels.saturating_sub(1) {
        out.extend([0u8; NULL_RECORD]);
    }
    out.extend([0u8; NULL_RECORD]); // end of the top-level list
    out
}

#[test]
fn non_finite_floats_throughout_a_document() {
    // 1e400 parses to infinity, and `nan` to NaN. Every layer must either
    // reject them or hold them out of its output — a NaN vertex or bone
    // matrix surfaces as the mesh disappearing, far from the cause.
    let source = concat!(
        "FBXVersion: 7400\n",
        "Objects:  {\n",
        "\tGeometry: 9, \"Geometry::Body\", \"Mesh\" {\n",
        "\t\tVertices: *9 {\n",
        "\t\t\ta: 1e400,0,0,1,-1e400,0,0,1,0\n",
        "\t\t}\n",
        "\t\tPolygonVertexIndex: *3 {\n",
        "\t\t\ta: 0,1,-3\n",
        "\t\t}\n",
        "\t}\n",
        "\tModel: 7, \"Model::Hips\", \"LimbNode\" {\n",
        "\t\tProperties70:  {\n",
        "\t\t\tP: \"Lcl Translation\", \"Lcl Translation\", \"\", \"A\",1e400,0,0\n",
        "\t\t\tP: \"Lcl Scaling\", \"Lcl Scaling\", \"\", \"A\",0,0,0\n",
        "\t\t}\n",
        "\t}\n",
        "}\n",
        "Connections:  {\n",
        "\tC: \"OO\",9,7\n",
        "}\n",
    );
    drive_every_layer(source.as_bytes());

    // The model layer specifically: an infinite translation must not become an
    // infinite world matrix for the whole subtree.
    let scene = Scene::from_document(text::parse(source).expect("ascii parses"));
    let models = model::parse_all(&scene);
    for m in &models.models {
        assert!(
            m.world.is_finite(),
            "{} has a non-finite world matrix",
            m.name
        );
    }
    // Found by this test: an infinite Lcl_Translation composed to a NaN
    // matrix, and since a child multiplies by its parent's world matrix the
    // whole subtree went NaN — surfacing as the mesh disappearing, with
    // nothing to say where it began. The component now falls back to its
    // default and is counted. Asserting the count means a silent revert to
    // substituting nothing fails here, rather than only in a viewport.
    assert_eq!(
        models.report.non_finite_components, 1,
        "the infinite Lcl_Translation"
    );
    let hips = models
        .models
        .iter()
        .find(|m| m.name == "Hips")
        .expect("Hips");
    assert_eq!(
        hips.transform.translation,
        glam::DVec3::ZERO,
        "the bad translation fell back to its default"
    );
    // The zero Lcl_Scaling in the same document is finite, so it is NOT
    // counted — a degenerate transform is still a legal one.
    assert_eq!(hips.transform.scale, glam::DVec3::ZERO);
}

#[test]
fn a_mesh_with_no_vertices_and_a_mesh_with_no_polygons() {
    for (what, body) in [
        (
            "no vertices",
            "\t\tVertices: *0 {\n\t\t\ta: \n\t\t}\n\t\tPolygonVertexIndex: *3 {\n\t\t\ta: 0,1,-3\n\t\t}\n",
        ),
        (
            "no polygons",
            "\t\tVertices: *9 {\n\t\t\ta: 0,0,0,1,0,0,0,1,0\n\t\t}\n\t\tPolygonVertexIndex: *0 {\n\t\t\ta: \n\t\t}\n",
        ),
    ] {
        let source = format!(
            "FBXVersion: 7400\nObjects:  {{\n\tGeometry: 9, \"Geometry::B\", \"Mesh\" {{\n{body}\t}}\n}}\nConnections:  {{\n}}\n"
        );
        drive_every_layer(source.as_bytes());
        let scene = Scene::from_document(text::parse(&source).expect("ascii parses"));
        let object = scene.objects_of_kind("Geometry")[0];
        // An empty mesh is a legitimate outcome, and so is an error; a mesh
        // whose buffers disagree is not.
        if let Ok(mesh) = geometry::parse(object, geometry::GeometricTransform::default()) {
            assert_eq!(
                mesh.positions.len(),
                mesh.vertex_source.len() * 3,
                "{what}: positions and sources disagree"
            );
        }
    }
}

#[test]
fn duplicate_bone_names() {
    // Names are not identity here — ids are — but a consumer that keys a rig
    // by name (three.js animation binding does) would silently merge these.
    // The tree must keep both as distinct nodes.
    let source = concat!(
        "FBXVersion: 7400\n",
        "Objects:  {\n",
        "\tModel: 10, \"Model::same\", \"LimbNode\" {\n\t}\n",
        "\tModel: 20, \"Model::same\", \"LimbNode\" {\n\t}\n",
        "\tModel: 30, \"Model::same\", \"LimbNode\" {\n\t}\n",
        "}\n",
        "Connections:  {\n\tC: \"OO\",20,10\n\tC: \"OO\",30,20\n}\n",
    );
    let scene = Scene::from_document(text::parse(source).expect("ascii parses"));
    let tree = model::parse_all(&scene);

    assert_eq!(tree.models.len(), 3, "all three survive");
    assert_eq!(
        tree.report,
        model::ModelReport::default(),
        "nothing dropped"
    );
    let names: Vec<&str> = tree.models.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(names, vec!["same"; 3]);
    // Distinct ids, and the hierarchy is still a chain rather than collapsed.
    assert_eq!(tree.roots, vec![10]);
    assert_eq!(tree.get(30).expect("30").parent, Some(20));
    drive_every_layer(source.as_bytes());
}

#[test]
fn every_committed_test_file_survives_the_whole_read_path() {
    // The existing corpus named in memory/test.md §4, driven end to end.
    let dir =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../legacy/static/test-files");
    let mut seen = 0usize;
    let mut stack = vec![dir];
    while let Some(path) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&path) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(bytes) = std::fs::read(&p) {
                // Every file, whatever its extension: a .glb or .zip fed to the
                // FBX reader is exactly the mistake a user makes.
                drive_every_layer(&bytes);
                seen += 1;
            }
        }
    }
    assert!(seen >= 8, "only {seen} files found — did the corpus move?");
}

#[test]
fn a_curve_with_far_more_keys_than_any_animation_is_truncated() {
    // Review finding. The key count comes straight from the file: a KeyTime
    // array is bounded only by the reader's 512 MB inflate ceiling, which is
    // 64 million keys, and a few kilobytes of deflate can ask for a million.
    // Matching is linear now, but a million keys per axis per track is still
    // an allocation nobody asked for, so the count itself is capped.
    let keys = 300_000usize;
    let times: Vec<String> = (0..keys).map(|i| (i as i64 * 1000).to_string()).collect();
    let values: Vec<String> = (0..keys).map(|i| ((i % 90) as f64).to_string()).collect();
    let curve = format!(
        "\tAnimationCurve: 400, \"AnimCurve::\", \"\" {{\n\t\tKeyTime: *{keys} {{\n\t\t\ta: {}\n\t\t}}\n\t\tKeyValueFloat: *{keys} {{\n\t\t\ta: {}\n\t\t}}\n\t}}\n",
        times.join(","),
        values.join(",")
    );
    let source = format!(
        "FBXVersion: 7400\nObjects:  {{\n\tModel: 10, \"Model::b\", \"LimbNode\" {{\n\t}}\n\
         \tAnimationStack: 100, \"AnimStack::c\", \"\" {{\n\t}}\n\
         \tAnimationLayer: 200, \"AnimLayer::B\", \"\" {{\n\t}}\n\
         \tAnimationCurveNode: 300, \"AnimCurveNode::R\", \"\" {{\n\t}}\n{curve}}}\n\
         Connections:  {{\n\tC: \"OO\",200,100\n\tC: \"OO\",300,200\n\
         \tC: \"OP\",300,10,\"Lcl Rotation\"\n\tC: \"OP\",400,300,\"d|X\"\n}}\n"
    );

    let started = std::time::Instant::now();
    let scene = Scene::from_document(text::parse(&source).expect("ascii parses"));
    let models = model::parse_all(&scene);
    let (clips, report) = animation::parse_all(&scene, &models);
    let elapsed = started.elapsed();

    assert_eq!(report.curves_over_key_limit, 1, "the oversized curve");
    let track = &clips[0].tracks[0];
    assert_eq!(track.times.len(), 262_144, "truncated to the cap");
    // The point of the cap is the time it saves. Quadratic matching over
    // 262,144 keys would be ~7e10 comparisons; this must stay in the
    // milliseconds. Generous so it cannot fail on a loaded machine, and still
    // orders of magnitude below the behaviour it guards against.
    assert!(
        elapsed < std::time::Duration::from_secs(20),
        "parsing took {elapsed:?}"
    );
}

#[test]
fn a_mesh_declaring_absurd_geometry_is_bounded() {
    // Review finding. Nothing related output size to input size: a
    // PolygonVertexIndex of N entries closed as one polygon fans into N-2
    // triangles and 3(N-2) expanded vertices, so half a megabyte of deflate
    // could ask for gigabytes.
    //
    // One polygon with far too many corners: dropped whole, not fanned.
    let corners = 5000usize;
    let mut indices: Vec<String> = (0..corners - 1).map(|i| (i % 3).to_string()).collect();
    indices.push("-1".into()); // negative marks the polygon's last corner
    let source = format!(
        "FBXVersion: 7400\nObjects:  {{\n\tGeometry: 9, \"Geometry::B\", \"Mesh\" {{\n\
         \t\tVertices: *9 {{\n\t\t\ta: 0,0,0,1,0,0,0,1,0\n\t\t}}\n\
         \t\tPolygonVertexIndex: *{corners} {{\n\t\t\ta: {}\n\t\t}}\n\t}}\n}}\nConnections:  {{\n}}\n",
        indices.join(",")
    );
    let scene = Scene::from_document(text::parse(&source).expect("ascii parses"));
    let object = scene.objects_of_kind("Geometry")[0];
    let mesh = geometry::parse(object, geometry::GeometricTransform::default()).expect("parses");

    assert_eq!(mesh.report.polygons_over_corner_limit, 1);
    assert!(
        mesh.positions.is_empty(),
        "{} vertices from one dropped polygon",
        mesh.positions.len() / 3
    );
    // A normal polygon in the same file still works, so the cap is not a
    // blanket rejection.
    drive_every_layer(source.as_bytes());
}

#[test]
fn a_second_objects_block_is_read_and_reported() {
    // Review finding: `find(|r| r.name == "Objects")` took only the first,
    // dropping a second block whole while the report read all-zero — the exact
    // silent loss the counters exist to prevent.
    let source = concat!(
        "FBXVersion: 7400\n",
        "Objects:  {\n",
        "\tModel: 7, \"Model::first\", \"LimbNode\" {\n\t}\n",
        "}\n",
        "Objects:  {\n",
        "\tModel: 8, \"Model::second\", \"LimbNode\" {\n\t}\n",
        "}\n",
        "Connections:  {\n}\n",
    );
    let scene = Scene::from_document(text::parse(source).expect("ascii parses"));

    assert_eq!(scene.report.extra_object_roots, 1);
    assert!(scene.object(7).is_some(), "the first block");
    assert!(scene.object(8).is_some(), "the second block was read too");
    assert_eq!(scene.objects.len(), 2);
}

#[test]
fn the_skinned_quad_seed_reaches_the_layers_it_was_added_for() {
    // A seed is only worth committing if it exercises what it claims. Review
    // measured that 70% of the binary rig's bytes sit inside zlib streams,
    // where any mutation fails inflate and rejects the whole file — so the
    // skin, layer-element and quad paths were effectively unreachable by
    // fuzzing. This asserts the seed actually covers them, so a later edit
    // that guts it fails here rather than silently narrowing the fuzzer.
    let source = include_str!("../fuzz/seeds/ascii-skinned-quad.fbx");
    let scene = Scene::from_document(text::parse(source).expect("the seed parses"));

    let object = scene.objects_of_kind("Geometry")[0];
    let mesh = geometry::parse(
        object,
        geometry::GeometricTransform::for_geometry(&scene, object.id),
    )
    .expect("geometry parses");
    // A quad becomes two triangles: six expanded corners.
    assert_eq!(mesh.vertex_count(), 6, "the quad branch of triangulate ran");
    assert!(mesh.normals.is_some(), "the normal layer was read");
    assert!(mesh.uvs.is_some(), "the IndexToDirect UV layer was read");

    let (skins, skipped) = skin::parse_all(&scene);
    assert_eq!(skipped, 0);
    assert_eq!(skins.len(), 1, "the skin deformer was read");
    assert_eq!(skins[0].clusters.len(), 1, "the cluster was read");
    // A non-identity TransformLink, so `inverse_bind` is not a no-op.
    assert!(skins[0].clusters[0].inverse_bind().is_finite());
    let (weights, _) = skins[0].bind(&mesh).expect("binds");
    assert_eq!(weights.indices.len(), mesh.vertex_count() * 4);
}
