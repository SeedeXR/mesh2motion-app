//! The binary FBX writer, checked by round trip.
//!
//! `parse(encode(parse(bytes)))` must equal `parse(bytes)`. Comparing the
//! documents rather than the bytes is deliberate: the container has legitimate
//! freedoms — whether an array is deflated, how the footer is padded — that
//! change the bytes without changing what the file means. A byte comparison
//! would fail on choices that are ours to make, and would still not prove the
//! result is readable.
//!
//! This needs no reference implementation, which is why it is the first half of
//! the writer to build.
//!
//! # What a round trip cannot prove
//!
//! It proves our writer and our reader agree. It does **not** prove the output
//! is valid FBX. Four of the eight mutations run against the encoder survive
//! this file, every one of them a conformance detail our reader does not
//! check:
//!
//! - the null record that ends a child list — `end_offset` is authoritative, so
//!   a file without one is self-consistent to us;
//! - the null record that ends the top-level list — never read, because the
//!   footer heuristic stops the loop first;
//! - an array's declared byte length when it is uncompressed — ignored;
//! - the footer's 16-byte alignment padding — only the last 16 bytes are checked.
//!
//! The encoder writes all four as the format requires. The point is that
//! nothing here would notice if it stopped. Catching that needs a different
//! reader, which is what the legacy-loader gate is for — see P2-6 in
//! `memory/todo.md`.

use m2m_io::fbx::{binary, encode};

const RIG: &[u8] =
    include_bytes!("../../../legacy/static/test-files/retarget testing/mixamo-original-rig.fbx");

#[test]
fn the_reference_rig_survives_a_round_trip() {
    let original = binary::parse(RIG).expect("the rig parses");
    let bytes = encode::encode(&original).expect("encodes");
    let reparsed = binary::parse(&bytes).expect("our own output parses");

    // Not a length or a count: the whole document, every node, property and
    // value, compared by value.
    assert_eq!(reparsed, original, "the round trip changed the document");

    // And the document is substantial, so the comparison is not vacuous.
    assert_eq!(original.version, 7700);
    assert_eq!(original.roots.len(), 11);
    let objects = original.root("Objects").expect("Objects");
    assert!(
        objects.children.len() > 600,
        "only {} objects — is this the right file?",
        objects.children.len()
    );
}

#[test]
fn every_property_type_round_trips_including_the_awkward_ones() {
    use binary::{FbxDocument, FbxNode, FbxProperty};

    // One of each variant, with values chosen to catch a wrong width or a
    // sign error rather than merely to exist.
    let properties = vec![
        FbxProperty::Bool(true),
        FbxProperty::Bool(false),
        FbxProperty::I16(i16::MIN),
        FbxProperty::I32(i32::MIN),
        FbxProperty::I64(i64::MIN),
        FbxProperty::F32(-0.0),
        FbxProperty::F64(f64::MAX),
        // Non-ASCII, but NOT an embedded NUL — see the test below for why.
        FbxProperty::Str("mixamorig:Hips \u{e9}".into()),
        FbxProperty::Raw(vec![0, 255, 128, 1]),
        FbxProperty::BoolArray(vec![true, false, true]),
        FbxProperty::I32Array(vec![i32::MIN, 0, i32::MAX]),
        FbxProperty::I64Array(vec![i64::MIN, 0, i64::MAX]),
        FbxProperty::F32Array(vec![f32::MIN, 0.0, f32::MAX]),
        FbxProperty::F64Array(vec![f64::MIN, 0.0, f64::MAX]),
        // Empty arrays are a real shape: 40 of the reference rig's skin
        // clusters carry no indices.
        FbxProperty::I32Array(vec![]),
        FbxProperty::Str(String::new()),
    ];

    let document = FbxDocument {
        version: 7400,
        roots: vec![FbxNode {
            name: "Objects".into(),
            properties: properties.clone(),
            children: vec![FbxNode {
                name: "Nested".into(),
                properties: vec![FbxProperty::I32(7)],
                children: vec![FbxNode {
                    name: "Deeper".into(),
                    properties: vec![],
                    children: vec![],
                }],
            }],
        }],
    };

    let bytes = encode::encode(&document).expect("encodes");
    let reparsed = binary::parse(&bytes).expect("parses");
    assert_eq!(reparsed, document);

    // -0.0 == 0.0 under PartialEq, so the equality above would not notice the
    // sign being dropped. Check the bit pattern.
    let f32s: Vec<f32> = reparsed.roots[0]
        .properties
        .iter()
        .filter_map(|p| match p {
            FbxProperty::F32(v) => Some(*v),
            _ => None,
        })
        .collect();
    assert_eq!(f32s[0].to_bits(), (-0.0f32).to_bits(), "sign of zero lost");
}

#[test]
fn a_document_with_64_bit_offsets_round_trips() {
    use binary::{FbxDocument, FbxNode, FbxProperty};

    // Version 7500 and later widen every node offset from 32 to 64 bits, and
    // the reference rig is 7400 — so without this the wide path is written by
    // the encoder and never read back.
    let document = FbxDocument {
        version: 7700,
        roots: vec![FbxNode {
            name: "Objects".into(),
            properties: vec![FbxProperty::I64(1 << 40)],
            children: vec![FbxNode {
                name: "Child".into(),
                properties: vec![FbxProperty::F64(1.5)],
                children: vec![],
            }],
        }],
    };
    let bytes = encode::encode(&document).expect("encodes");
    assert_eq!(binary::parse(&bytes).expect("parses"), document);

    // The two widths must produce genuinely different bytes, or this test is
    // only re-running the narrow path under another version number.
    let narrow = encode::encode(&FbxDocument {
        version: 7400,
        ..document.clone()
    })
    .expect("encodes");
    assert_ne!(
        bytes.len(),
        narrow.len(),
        "the offset width made no difference"
    );
}

#[test]
fn a_name_too_long_for_its_length_byte_is_refused() {
    use binary::{FbxDocument, FbxNode};

    // A node name's length is stored in one byte, so 256 cannot be written.
    // Truncating it would produce a file that parses into a different document.
    let document = FbxDocument {
        version: 7400,
        roots: vec![FbxNode {
            name: "n".repeat(256),
            properties: vec![],
            children: vec![],
        }],
    };
    assert!(encode::encode(&document).is_err());

    // 255 is the largest that fits, and must still work.
    let ok = FbxDocument {
        version: 7400,
        roots: vec![FbxNode {
            name: "n".repeat(255),
            properties: vec![],
            children: vec![],
        }],
    };
    let bytes = encode::encode(&ok).expect("255 is writable");
    assert_eq!(binary::parse(&bytes).expect("parses"), ok);
}

#[test]
fn a_name_keeps_its_class_suffix_through_the_document() {
    // Binary FBX encodes an object's name as `Name\0\x01Class`. The reader used
    // to stop at the first NUL, so the document held `Hips` and the encoder
    // wrote a name with no class — and our round trip passed anyway, because
    // both parses truncated identically. The loss was only in the bytes.
    //
    // Blender is what found it: `elem_split_name_class` raises
    // `ValueError: not enough values to unpack (expected 2, got 1)` on a name
    // without the separator, so the file would not open at all. Verified on
    // Blender 5.2 against the encoder's output before and after this fix.
    //
    // three.js truncates the same way we did and never noticed, which is why a
    // second reader was not enough and a genuinely different one was.
    use binary::{FbxDocument, FbxNode, FbxProperty};

    let document = FbxDocument {
        version: 7700,
        roots: vec![FbxNode {
            name: "Objects".into(),
            properties: vec![FbxProperty::Str("Hips\u{0}\u{1}Model".into())],
            children: vec![],
        }],
    };
    let reparsed = binary::parse(&encode::encode(&document).expect("encodes")).expect("parses");

    let FbxProperty::Str(name) = &reparsed.roots[0].properties[0] else {
        panic!("expected a string property");
    };
    assert_eq!(name, "Hips\u{0}\u{1}Model", "the class suffix must survive");
    assert_eq!(reparsed, document, "the document round-trips exactly now");

    // And the reference rig's own names carry theirs, so this is not a
    // property of hand-built documents only.
    let rig = binary::parse(RIG).expect("the rig parses");
    let objects = rig.root("Objects").expect("Objects");
    let with_class = objects
        .children
        .iter()
        .flat_map(|c| c.properties.iter())
        .filter_map(|p| match p {
            FbxProperty::Str(s) => Some(s),
            _ => None,
        })
        .filter(|s| s.contains('\u{0}'))
        .count();
    assert!(
        with_class > 500,
        "only {with_class} names carry a class separator — is the reader truncating again?"
    );
}
