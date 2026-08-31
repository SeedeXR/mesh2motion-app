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
            empty_scope: false,
            children: vec![FbxNode {
                name: "Nested".into(),
                properties: vec![FbxProperty::I32(7)],
                children: vec![FbxNode {
                    name: "Deeper".into(),
                    properties: vec![],
                    children: vec![],
                    empty_scope: false,
                }],
                empty_scope: false,
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
                empty_scope: false,
            }],
            empty_scope: false,
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
            empty_scope: false,
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
            empty_scope: false,
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
            empty_scope: false,
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

/// A node's empty-scope declaration survives a round trip, and `AnimationLayer`
/// keeps the one it needs.
///
/// **This is the bug a third reader found and neither of the first two could.**
/// A childless node may either declare a nested list that holds only the
/// terminating null record, or declare none at all. Our reader represents both
/// as `children: []`, so no test written against it can tell them apart — and
/// re-encoding used to write "none" for both.
///
/// The consequence: assimp reads an `AnimationLayer` written without the empty
/// list as no layer at all, so its stack has no layers and the whole file loads
/// with **zero animations** — mesh and 129 bones perfect, all 76,960 keyframes
/// silently gone. Blender and three.js read it either way.
///
/// Writing one for every node is equally wrong: the reference rig has 5,144
/// childless nodes with no list and exactly 3 with an empty one (`References`
/// and its two `AnimationLayer`s), and always-write breaks the three.js loader
/// on materials.
///
/// assimp is not in CI, so the byte-level property it needs is asserted here.
#[test]
fn empty_scopes_survive_a_round_trip() {
    let original = binary::parse(RIG).expect("the rig parses");
    let bytes = encode::encode(&original).expect("encodes");

    let before = scope_census(RIG);
    let after = scope_census(&bytes);
    assert_eq!(
        after, before,
        "the round trip changed which nodes declare an empty scope"
    );
    // Pinned so a change to either side of the round trip has to be deliberate.
    assert_eq!(
        before,
        (3, 5144, 952),
        "the reference rig's structure is the baseline for this test"
    );
}

/// Counts, over a whole binary FBX: childless nodes declaring an empty scope,
/// childless nodes declaring none, and nodes with children.
///
/// Walks the raw bytes because this distinction is invisible in the parsed
/// document — which is the entire point.
fn scope_census(bytes: &[u8]) -> (usize, usize, usize) {
    let version = u32::from_le_bytes(bytes[23..27].try_into().expect("version"));
    let wide = version >= 7500;
    let word = if wide { 8 } else { 4 };

    fn read_word(bytes: &[u8], at: usize, wide: bool) -> usize {
        if wide {
            u64::from_le_bytes(bytes[at..at + 8].try_into().expect("u64")) as usize
        } else {
            u32::from_le_bytes(bytes[at..at + 4].try_into().expect("u32")) as usize
        }
    }

    fn walk(
        bytes: &[u8],
        mut at: usize,
        end: usize,
        wide: bool,
        word: usize,
        census: &mut (usize, usize, usize),
    ) {
        let null_len = word * 3 + 1;
        while at + null_len <= end {
            let end_offset = read_word(bytes, at, wide);
            if end_offset == 0 {
                return;
            }
            assert!(end_offset <= end, "a node ends past its parent");
            let property_bytes = read_word(bytes, at + word * 2, wide);
            let name_len = bytes[at + word * 3] as usize;
            let after_properties = at + word * 3 + 1 + name_len + property_bytes;
            assert!(
                after_properties <= end_offset,
                "properties overrun the node"
            );

            let body = end_offset - after_properties;
            if body == 0 {
                census.1 += 1;
            } else if body == null_len
                && bytes[after_properties..end_offset].iter().all(|&b| b == 0)
            {
                census.0 += 1;
            } else {
                census.2 += 1;
                walk(
                    bytes,
                    after_properties,
                    end_offset - null_len,
                    wide,
                    word,
                    census,
                );
            }
            at = end_offset;
        }
    }

    let mut census = (0, 0, 0);
    walk(bytes, 27, bytes.len(), wide, word, &mut census);
    census
}
