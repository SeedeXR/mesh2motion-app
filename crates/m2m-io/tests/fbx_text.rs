//! FBX ASCII parsing.
//!
//! The cases here are ported from `legacy/src/lib/io/fbx/TextParser.test.ts`,
//! which is a set of regression tests for real bugs the legacy parser fixed
//! over upstream three.js. They come across because the bugs would come across
//! with the code otherwise.

use m2m_io::fbx::binary::{FbxNode, FbxProperty};
use m2m_io::fbx::text::{ascii_version, is_ascii_fbx, parse};
use m2m_io::fbx::FbxError;

/// A minimal but structurally realistic ASCII document, from the legacy test.
///
/// Deliberately exercises the shapes that used to desynchronise the parser: no
/// leading comment block, document-level properties outside any node, a node
/// header carrying attributes before the brace, and a property value that
/// contains braces.
const ASCII_FBX: &str = concat!(
    "FBXHeaderExtension:  {\n",
    "\tFBXHeaderVersion: 1003\n",
    "\tFBXVersion: 7400\n",
    "\tCreationTimeStamp:  {\n",
    "\t\tVersion: 1000\n",
    "\t\tYear: 2026\n",
    "\t}\n",
    "\tCreator: \"FBX SDK/FBX Plugins version 2020.0\"\n",
    "\tSceneInfo: \"SceneInfo::GlobalInfo\", \"UserData\" {\n",
    "\t\tType: \"UserData\"\n",
    "\t\tProperties70:  {\n",
    "\t\t\tP: \"DocumentUrl\", \"KString\", \"Url\", \"\", \"D:\\Art\\{Project}\\char.fbx\"\n",
    "\t\t}\n",
    "\t}\n",
    "}\n",
    "CreationTime: \"2026-08-03 10:15:00:000\"\n",
    "Creator: \"FBX SDK/FBX Plugins version 2020.0\"\n",
    "\n",
    "; Object definitions\n",
    ";------------------------------------------------------------------\n",
    "\n",
    "GlobalSettings:  {\n",
    "\tVersion: 1000\n",
    "\tProperties70:  {\n",
    "\t\tP: \"UpAxis\", \"int\", \"Integer\", \"\",1\n",
    "\t\tP: \"UnitScaleFactor\", \"double\", \"Number\", \"\",1\n",
    "\t}\n",
    "}\n",
    "Objects:  {\n",
    "\tGeometry: 140234, \"Geometry::\", \"Mesh\" {\n",
    "\t\tVertices: *9 {\n",
    "\t\t\ta: 0,0,0,1,0,0,0,1,0\n",
    "\t\t}\n",
    "\t\tPolygonVertexIndex: *3 {\n",
    "\t\t\ta: 0,1,-3\n",
    "\t\t}\n",
    "\t}\n",
    "}\n",
    "Connections:  {\n",
    "\tC: \"OO\",140234,0\n",
    "}\n",
);

fn root_names(doc: &m2m_io::fbx::binary::FbxDocument) -> Vec<&str> {
    doc.roots.iter().map(|r| r.name.as_str()).collect()
}

fn f64_array(node: &FbxNode) -> &[f64] {
    match node.properties.iter().find_map(|p| match p {
        FbxProperty::F64Array(v) => Some(v),
        _ => None,
    }) {
        Some(v) => v,
        None => panic!("{} has no f64 array: {:?}", node.name, node.properties),
    }
}

#[test]
fn detects_ascii_documents() {
    // Regression: the upstream heuristic sampled a fixed byte offset against the
    // binary magic and rejected any file without the usual leading comment.
    assert!(is_ascii_fbx(ASCII_FBX), "no leading comment block");
    assert!(is_ascii_fbx(&format!(
        "; FBX 7.4.0 project file\n\n{ASCII_FBX}"
    )));

    assert!(!is_ascii_fbx("Kaydara FBX Binary  \0\x1a\0"));
    assert!(!is_ascii_fbx(
        "<!DOCTYPE html>\n<html><title>404</title></html>"
    ));
    assert!(!is_ascii_fbx("{\"asset\":{\"version\":\"2.0\"}}"));
}

#[test]
fn reads_the_version_regardless_of_spacing() {
    assert_eq!(ascii_version(ASCII_FBX), Some(7400));
    assert_eq!(ascii_version("\tFBXVersion:7300"), Some(7300));
    assert_eq!(ascii_version("\tFBXVersion:   7500  "), Some(7500));
    assert_eq!(ascii_version("no version here"), None);
}

#[test]
fn a_brace_inside_a_value_is_not_a_block_delimiter() {
    // The bug this guards: the unanchored pattern read the `P:` line holding
    // `D:\Art\{Project}\char.fbx` as a node beginning, so the indent drifted and
    // every node after it was silently discarded — a quiet partial success.
    let doc = parse(ASCII_FBX).expect("parses");

    // Every top-level section must survive, in order.
    assert_eq!(
        root_names(&doc),
        [
            "FBXHeaderExtension",
            "CreationTime",
            "Creator",
            "GlobalSettings",
            "Objects",
            "Connections",
        ]
    );

    // And the offending value itself must be intact.
    let p = doc
        .root("FBXHeaderExtension")
        .and_then(|h| h.child("SceneInfo"))
        .and_then(|s| s.child("Properties70"))
        .and_then(|p| p.child("P"))
        .expect("the P node");
    let joined = format!("{:?}", p.properties);
    assert!(joined.contains("{Project}"), "got {joined}");
}

#[test]
fn document_level_properties_land_on_the_root() {
    // Regression: these lines sit outside any block, and upstream dereferenced
    // the absent node — "Cannot read properties of undefined (reading 'name')".
    let doc = parse(ASCII_FBX).unwrap();

    let creation = doc.root("CreationTime").expect("CreationTime");
    assert_eq!(
        creation.properties,
        vec![FbxProperty::Str("2026-08-03 10:15:00:000".into())]
    );
    assert!(doc.root("Creator").is_some());
}

#[test]
fn parses_the_nodes_the_dom_layer_depends_on() {
    let doc = parse(ASCII_FBX).unwrap();

    // Node attributes come from the header line before the brace.
    let geometry = doc
        .root("Objects")
        .and_then(|o| o.child("Geometry"))
        .expect("Geometry");
    assert_eq!(
        geometry.properties,
        vec![
            FbxProperty::I64(140234),
            FbxProperty::Str("Geometry::".into()),
            FbxProperty::Str("Mesh".into()),
        ]
    );

    // `a:` arrays are hoisted onto their parent, matching the binary layout.
    // The FULL property vector is asserted, not just "contains an array": the
    // `*9` count used to appear as a leading Str, so the ASCII tree differed
    // from the binary one at properties[0] while a find_map-style check passed.
    let vertices = geometry.child("Vertices").expect("Vertices");
    assert_eq!(
        vertices.properties,
        vec![FbxProperty::F64Array(vec![
            0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0, 0.0
        ])]
    );
    assert_eq!(
        f64_array(
            geometry
                .child("PolygonVertexIndex")
                .expect("PolygonVertexIndex")
        ),
        [0.0, 1.0, -3.0]
    );

    // Properties70 entries stay as raw `P` nodes — flattening them is P2-3's
    // job — but their tokenisation is this parser's job, so it is asserted.
    // The legacy test checked the flattened `.value`; the equivalent here is
    // the property vector the flattening will read.
    let props = doc
        .root("GlobalSettings")
        .and_then(|g| g.child("Properties70"))
        .expect("Properties70");
    let p: Vec<&FbxNode> = props.children_named("P").collect();
    assert_eq!(p.len(), 2);
    assert_eq!(
        p[0].properties,
        vec![
            FbxProperty::Str("UpAxis".into()),
            FbxProperty::Str("int".into()),
            FbxProperty::Str("Integer".into()),
            FbxProperty::Str("".into()),
            FbxProperty::I64(1),
        ]
    );
    assert_eq!(
        p[1].properties.first(),
        Some(&FbxProperty::Str("UnitScaleFactor".into()))
    );

    // Legacy asserted the parsed connection was [[140234, 0]]; the equivalent
    // here is the C node's property vector, which is what P2-3 will collect.
    let connections = doc.root("Connections").expect("Connections");
    let c: Vec<&FbxNode> = connections.children_named("C").collect();
    assert_eq!(c.len(), 1);
    assert_eq!(
        c[0].properties,
        vec![
            FbxProperty::Str("OO".into()),
            FbxProperty::I64(140234),
            FbxProperty::I64(0),
        ]
    );
}

#[test]
fn arrays_split_across_lines_are_rejoined() {
    let doc = parse(concat!(
        "FBXVersion: 7400\n",
        "Objects:  {\n",
        "\tGeometry: 1, \"Geometry::\", \"Mesh\" {\n",
        "\t\tVertices: *6 {\n",
        "\t\t\ta: 1,2,3,\n",
        "4,5,6\n",
        "\t\t}\n",
        "\t}\n",
        "}\n",
    ))
    .unwrap();

    let vertices = doc
        .root("Objects")
        .and_then(|o| o.child("Geometry"))
        .and_then(|g| g.child("Vertices"))
        .expect("Vertices");
    assert_eq!(f64_array(vertices), [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]);
}

#[test]
fn a_stray_closing_brace_does_not_corrupt_the_tree() {
    // An extra `}` empties the node stack while nodes remain open. The legacy
    // parser skips it rather than failing, and its test asserts that everything
    // after the stray brace survives — which is the property that matters.
    let doc = parse(concat!(
        "FBXVersion: 7400\n",
        "Objects:  {\n",
        "\tGeometry: 1, \"Geometry::\", \"Mesh\" {\n",
        "\t\tVersion: 100\n",
        "\t}\n",
        "}\n",
        "}\n",
        "Connections:  {\n",
        "\tC: \"OO\",1,0\n",
        "}\n",
    ))
    .expect("a stray brace must not fail the parse");

    let connections = doc.root("Connections").expect("Connections survived");
    assert_eq!(connections.children_named("C").count(), 1);
    assert!(doc.root("Objects").is_some());
}

#[test]
fn rejects_input_that_is_not_ascii_fbx() {
    assert!(matches!(parse("<!DOCTYPE html>"), Err(FbxError::BadMagic)));
    assert!(matches!(parse(""), Err(FbxError::BadMagic)));
    assert!(matches!(
        parse("Kaydara FBX Binary  \0\x1a\0"),
        Err(FbxError::BadMagic)
    ));
}

#[test]
fn deep_nesting_is_bounded() {
    // Indentation must actually increase with depth. Capping it made the
    // resync close blocks back each line, so the stack never grew and the
    // limit never fired — the test was measuring nothing.
    let mut text = String::from("FBXVersion: 7400\n");
    for depth in 0..300 {
        text.push_str(&"\t".repeat(depth));
        text.push_str("N:  {\n");
    }
    assert!(matches!(parse(&text), Err(FbxError::TooDeep(_))));
}

#[test]
fn a_missing_closing_brace_does_not_swallow_later_sections() {
    // The quiet partial success this parser is most prone to. Without indent
    // tracking, `Connections` became a descendant of `Objects` and
    // `doc.root("Connections")` returned None with no error anywhere.
    let doc = parse(concat!(
        "FBXVersion: 7400\n",
        "Objects:  {\n",
        "\tGeometry: 1, \"Geometry::\", \"Mesh\" {\n",
        "\t\tVersion: 100\n",
        // the closing brace for Geometry is missing
        "}\n",
        "Connections:  {\n",
        "\tC: \"OO\",1,0\n",
        "}\n",
    ))
    .expect("recoverable");

    assert!(
        doc.root("Connections").is_some(),
        "Connections was swallowed; roots are {:?}",
        doc.roots.iter().map(|r| &r.name).collect::<Vec<_>>()
    );
    assert!(doc.root("Objects").is_some());
}

#[test]
fn a_comma_inside_a_quoted_value_does_not_split_it() {
    // Object names contain commas. Splitting naively shifted every positional
    // index, so the DOM layer reading properties[2] as the attribute type got
    // a fragment of the name.
    let doc = parse(concat!(
        "FBXVersion: 7400\n",
        "Objects:  {\n",
        "\tModel: 1, \"Model::Bob, Jr\", \"Mesh\" {\n",
        "\t\tVersion: 100\n",
        "\t}\n",
        "}\n",
    ))
    .unwrap();

    let model = doc.root("Objects").and_then(|o| o.child("Model")).unwrap();
    assert_eq!(
        model.properties,
        vec![
            FbxProperty::I64(1),
            FbxProperty::Str("Model::Bob, Jr".into()),
            FbxProperty::Str("Mesh".into()),
        ]
    );
}

#[test]
fn a_short_array_is_rejected_rather_than_silently_truncated() {
    // `*9` declaring nine values with three present used to parse "fine",
    // handing the DOM layer a third of a mesh with nothing reported.
    let short = parse(concat!(
        "FBXVersion: 7400\n",
        "Objects:  {\n",
        "\tGeometry: 1, \"Geometry::\", \"Mesh\" {\n",
        "\t\tVertices: *9 {\n",
        "\t\t\ta: 1,2,3\n",
        "\t\t}\n",
        "\t}\n",
        "}\n",
    ));
    assert!(
        matches!(short, Err(FbxError::Malformed { .. })),
        "got {short:?}"
    );

    // And a non-numeric element is an error, not something to skip.
    let junk = parse(concat!(
        "FBXVersion: 7400\n",
        "Objects:  {\n",
        "\tGeometry: 1, \"Geometry::\", \"Mesh\" {\n",
        "\t\tVertices: *3 {\n",
        "\t\t\ta: 1,oops,3\n",
        "\t\t}\n",
        "\t}\n",
        "}\n",
    ));
    assert!(
        matches!(junk, Err(FbxError::Malformed { .. })),
        "got {junk:?}"
    );
}

#[test]
fn non_ascii_in_the_header_does_not_panic() {
    // is_ascii_fbx used to slice `&text[..1024]`, which panics when the cut
    // lands inside a multibyte character — and exporters put non-ASCII creator
    // names and paths in the header.
    let padding = "é".repeat(600); // pushes a char across the 1024-byte boundary
    let text = format!("; {padding}\nFBXVersion: 7400\nObjects:  {{\n}}\n");
    assert!(
        !is_ascii_fbx(&text),
        "sniff window ends before the header here"
    );
    // The important part is that neither call panics.
    let _ = parse(&text);

    let early = format!("FBXVersion: 7400\n; {padding}\nObjects:  {{\n}}\n");
    assert!(is_ascii_fbx(&early));
    assert!(parse(&early).is_ok());
}

#[test]
fn embedded_content_payloads_are_kept() {
    // `Content: ,` puts its base64 on the next line. Not handling it dropped
    // embedded textures silently: the node came out empty and the payload was
    // discarded as an unclassifiable line.
    let doc = parse(concat!(
        "FBXVersion: 7400\n",
        "Objects:  {\n",
        "\tVideo: 1, \"Video::tex\", \"Clip\" {\n",
        "\t\tContent: ,\n",
        "\t\t \"/9j/4RDaRXhpZgAATU0A\"\n",
        "\t}\n",
        "}\n",
    ))
    .unwrap();

    let content = doc
        .root("Objects")
        .and_then(|o| o.child("Video"))
        .and_then(|v| v.child("Content"))
        .expect("Content node");
    assert_eq!(
        content.properties,
        vec![FbxProperty::Str("/9j/4RDaRXhpZgAATU0A".into())]
    );
}

#[test]
fn a_truncated_document_keeps_what_it_had() {
    // The failure mode that matters for a parser is quiet partial success. An
    // ASCII file cut mid-document has no footer to check against, so the honest
    // behaviour is to return the nodes it did contain — but it must not lose
    // completed sections that appeared BEFORE the cut.
    let full = parse(ASCII_FBX).unwrap();
    assert_eq!(full.roots.len(), 6);

    let cut_at = ASCII_FBX.find("Connections").expect("has Connections");
    let truncated = parse(&ASCII_FBX[..cut_at]).expect("parses what it has");

    // Everything before the cut is still there, and nothing was invented.
    assert_eq!(
        root_names(&truncated),
        [
            "FBXHeaderExtension",
            "CreationTime",
            "Creator",
            "GlobalSettings",
            "Objects",
        ]
    );
    let geometry = truncated
        .root("Objects")
        .and_then(|o| o.child("Geometry"))
        .expect("Geometry survived the truncation");
    assert_eq!(f64_array(geometry.child("Vertices").unwrap()).len(), 9);
}

#[test]
fn every_prefix_of_a_real_document_parses_without_panicking() {
    // Cheap structural fuzz: every truncation point must be handled, and the
    // root count must never exceed the complete document's.
    for cut in 0..ASCII_FBX.len() {
        if !ASCII_FBX.is_char_boundary(cut) {
            continue;
        }
        if let Ok(doc) = parse(&ASCII_FBX[..cut]) {
            assert!(
                doc.roots.len() <= 6,
                "prefix of {cut} bytes produced {} roots; the whole file has 6",
                doc.roots.len()
            );
        }
    }
}

#[test]
fn numeric_accessors_bridge_the_two_formats() {
    // ASCII carries no type codes, so `Version: 100` is I64 here and I32 from
    // binary for the identical source line, and an untyped `a:` array is
    // F64Array where binary would say I32Array. Rather than guess a width,
    // consumers ask by meaning — this is the contract P2-3 relies on.
    use m2m_io::fbx::binary::split_object_name;

    let doc = parse(ASCII_FBX).unwrap();
    let geometry = doc
        .root("Objects")
        .and_then(|o| o.child("Geometry"))
        .unwrap();

    assert_eq!(geometry.properties[0].as_i64(), Some(140234));
    assert_eq!(geometry.properties[0].as_f64(), Some(140234.0));
    assert_eq!(geometry.properties[2].as_str(), Some("Mesh"));

    // The index array reads as integers even though ASCII stored it as floats.
    let indices = geometry.child("PolygonVertexIndex").unwrap();
    assert_eq!(indices.properties[0].as_i64_vec(), Some(vec![0, 1, -3]));

    // A genuinely fractional array must not silently truncate to integers.
    assert_eq!(
        FbxProperty::F64Array(vec![1.5, 2.0]).as_i64_vec(),
        None,
        "1.5 is not an integer"
    );

    // Object names are encoded oppositely by the two formats.
    assert_eq!(split_object_name("Geometry::Bob"), ("Bob", "Geometry"));
    assert_eq!(
        split_object_name("Bob\u{0}\u{1}Geometry"),
        ("Bob", "Geometry")
    );
    assert_eq!(split_object_name("plain"), ("plain", ""));
}
