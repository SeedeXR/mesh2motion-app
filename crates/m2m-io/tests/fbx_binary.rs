//! FBX binary parsing against a real Mixamo export, and against hostile input.
//!
//! `memory/test.md` §4: parsers are a trust boundary. A malformed file must
//! return an error — never panic, hang, or exhaust memory.

use m2m_io::fbx::binary::{parse, FbxProperty};
use m2m_io::fbx::FbxError;

/// A real Mixamo FBX 7.7 export, 2.1 MB.
const MIXAMO: &[u8] =
    include_bytes!("../../../legacy/static/test-files/retarget testing/mixamo-original-rig.fbx");

#[test]
fn parses_a_real_mixamo_export() {
    let doc = parse(MIXAMO).expect("real Mixamo FBX must parse");

    assert_eq!(doc.version, 7700, "FBX 7.7");
    assert_eq!(doc.node_count(), 6099);

    // The top-level sections an FBX file is required to have, in file order.
    let roots: Vec<&str> = doc.roots.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(
        roots,
        [
            "FBXHeaderExtension",
            "FileId",
            "CreationTime",
            "Creator",
            "GlobalSettings",
            "Documents",
            "References",
            "Definitions",
            "Objects",
            "Connections",
            "Takes",
        ]
    );

    // What a rigged, animated character is made of. These counts are what the
    // DOM layer in P2-3 will consume, so a change here changes that.
    let objects = doc.root("Objects").expect("Objects section");
    let count = |name: &str| objects.children_named(name).count();
    assert_eq!(count("Model"), 67, "bones plus the mesh node");
    assert_eq!(count("Deformer"), 131, "skin plus per-bone clusters");
    assert_eq!(count("Geometry"), 2);
    assert_eq!(count("AnimationCurve"), 315);
    assert_eq!(count("AnimationCurveNode"), 54);
    assert_eq!(count("Pose"), 2);

    assert_eq!(
        doc.root("Connections").expect("Connections").children.len(),
        666
    );
}

#[test]
fn decodes_geometry_arrays() {
    // Proves the property decoding is real and not merely structural: the
    // vertex array is a compressed f64 array, so this exercises the inflate
    // path and the array reader together.
    let doc = parse(MIXAMO).unwrap();
    let geometry = doc
        .root("Objects")
        .and_then(|o| o.child("Geometry"))
        .expect("a Geometry node");

    let vertices = geometry.child("Vertices").expect("Vertices");
    let Some(FbxProperty::F64Array(values)) = vertices.properties.first() else {
        panic!(
            "Vertices should hold an f64 array, got {:?}",
            vertices.properties.first()
        );
    };
    assert_eq!(
        values.len(),
        42_696,
        "divisible by 3; coordinates come in triples"
    );
    assert!(
        values.iter().all(|v| v.is_finite()),
        "a real mesh has no non-finite coordinates"
    );

    // Sanity against physical scale: Mixamo exports centimetres, so a humanoid
    // spans roughly 100-200 units, not 0.001 or 1e6.
    let extent = values
        .chunks_exact(3)
        .map(|c| c[1])
        .fold(f64::NEG_INFINITY, f64::max)
        - values
            .chunks_exact(3)
            .map(|c| c[1])
            .fold(f64::INFINITY, f64::min);
    assert!(
        (50.0..500.0).contains(&extent),
        "height {extent} is not a plausible centimetre-scale character"
    );

    let indices = geometry
        .child("PolygonVertexIndex")
        .expect("PolygonVertexIndex");
    assert!(matches!(
        indices.properties.first(),
        Some(FbxProperty::I32Array(_))
    ));
}

#[test]
fn rejects_files_that_are_not_fbx() {
    assert!(matches!(parse(b""), Err(FbxError::BadMagic)));
    assert!(matches!(
        parse(b"not an fbx file at all"),
        Err(FbxError::BadMagic)
    ));
    // Right length, wrong content.
    assert!(matches!(parse(&[0u8; 64]), Err(FbxError::BadMagic)));
}

#[test]
fn rejects_an_unsupported_version() {
    let mut data = MIXAMO[..27].to_vec();
    data[23..27].copy_from_slice(&6000u32.to_le_bytes());
    assert!(matches!(
        parse(&data),
        Err(FbxError::UnsupportedVersion(6000))
    ));
}

#[test]
fn every_truncation_errors_rather_than_panicking() {
    // The single most likely corruption in the wild: a partial download, or a
    // file copied while still being written.
    for cut in [23, 27, 28, 40, 100, 1_000, 10_000, 100_000, 1_000_000] {
        let result = parse(&MIXAMO[..cut]);
        assert!(
            result.is_err(),
            "truncating to {cut} bytes should fail, not succeed"
        );
    }
}

#[test]
fn a_barely_truncated_file_does_not_silently_lose_sections() {
    // The bug this guards is the worst kind for an animation tool: quiet,
    // partial success. Cutting 578 bytes from the 2.1 MB file — 0.03% — used to
    // return Ok with 10 roots and 6089 nodes, having discarded the whole
    // `Takes` section, i.e. every animation stack. The end-of-content test is a
    // heuristic on offsets, so a cut inside the last root just stops the loop
    // early and everything looks fine.
    let cut = &MIXAMO[..MIXAMO.len() - 578];
    match parse(cut) {
        Err(FbxError::MissingFooter) => {}
        Err(other) => panic!("expected MissingFooter, got {other}"),
        Ok(doc) => panic!(
            "a truncated file parsed with {} roots and {} nodes",
            doc.roots.len(),
            doc.node_count()
        ),
    }

    // Sweep the last few KB: every cut must be rejected, not just the one that
    // happened to be found.
    for back in [1usize, 2, 16, 17, 160, 200, 578, 1_000, 5_000] {
        assert!(
            parse(&MIXAMO[..MIXAMO.len() - back]).is_err(),
            "cutting {back} bytes from the end should be rejected"
        );
    }
}

#[test]
fn every_animation_section_survives_a_full_parse() {
    // The complement of the test above: prove the sections a truncation would
    // drop are actually present when the file is whole, so that test is
    // asserting the loss of something real.
    let doc = parse(MIXAMO).unwrap();
    let takes = doc.root("Takes").expect("Takes section");
    assert!(!takes.children.is_empty());
    let objects = doc.root("Objects").unwrap();
    assert_eq!(objects.children_named("AnimationStack").count(), 2);
    assert_eq!(objects.children_named("AnimationLayer").count(), 2);
}

#[test]
fn corrupted_bytes_never_panic() {
    // A cheap stand-in for the fuzzing in test.md §5, which arrives with P2-8.
    // Flips bytes across the whole file and requires that every outcome is
    // either a clean parse or a clean error.
    //
    // Deterministic: a random seed here would mean a failure nobody can
    // reproduce from the test name alone.
    let mut state = 0x5eed_1234_u64;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };

    let mut errors = 0usize;
    let mut parsed = 0usize;
    for _ in 0..600 {
        let mut data = MIXAMO.to_vec();
        // Corrupt a handful of bytes, biased toward the header and node tree
        // where structure lives rather than the bulk vertex data.
        for _ in 0..4 {
            let pos = (next() as usize) % data.len().min(200_000);
            data[pos] ^= (next() as u8) | 1;
        }
        match parse(&data) {
            Ok(doc) => {
                parsed += 1;
                // A corruption that still parses must still yield something
                // coherent — the footer check means the file was whole, so a
                // document with no roots or an absurd node count would mean the
                // parser had invented structure rather than read it.
                assert!(!doc.roots.is_empty());
                assert!(
                    doc.node_count() <= 20_000,
                    "corrupted file yielded {} nodes; the real file has 6099",
                    doc.node_count()
                );
            }
            Err(_) => errors += 1,
        }
    }

    // The real assertion is that the loop completed at all: any panic aborts
    // the test. `parsed + errors == 600` would be true by construction, so it
    // is not asserted.
    assert!(
        errors > 30,
        "only {errors} of 600 corruptions were detected; is the corruption landing anywhere meaningful?"
    );
    // `parsed` is reported, not asserted on. All 600 being rejected is a
    // legitimate and better outcome — it became the actual result once
    // mid-list null records started erroring instead of truncating the child
    // list. Requiring the accept path to be exercised would penalise the
    // parser for getting stricter.
    eprintln!("{parsed} corruptions still parsed, {errors} were rejected");
}

#[test]
fn deep_nesting_errors_instead_of_overflowing_the_stack() {
    // A stack overflow aborts the process rather than unwinding, so it cannot
    // be caught and would take the whole app down. This has to be a depth
    // limit, not a caught panic.
    //
    // Builds a chain of nodes each declaring the next as its child. FBX 7500+
    // header: u64 end offset, u64 property count, u64 property-list bytes,
    // u8 name length, then the name.
    const DEPTH: usize = 2_000;
    const HEADER: usize = 8 + 8 + 8 + 1 + 1; // name is one byte, "n"
    const NULL_RECORD: usize = 8 + 8 + 8 + 1;

    let mut data = Vec::new();
    data.extend_from_slice(b"Kaydara FBX Binary  \x00\x1a\x00");
    data.extend_from_slice(&7700u32.to_le_bytes());

    // Innermost node ends where the whole chain ends.
    let base = data.len();
    for i in 0..DEPTH {
        let remaining = DEPTH - i;
        let end = base + remaining * HEADER + NULL_RECORD;
        data.extend_from_slice(&(end as u64).to_le_bytes());
        data.extend_from_slice(&0u64.to_le_bytes()); // no properties
        data.extend_from_slice(&0u64.to_le_bytes()); // no property bytes
        data.push(1);
        data.push(b'n');
    }
    data.extend_from_slice(&[0u8; NULL_RECORD]);
    data.extend_from_slice(&[0u8; 200]); // footer padding

    match parse(&data) {
        Err(FbxError::TooDeep(_)) => {}
        Err(other) => panic!("expected a depth limit, got {other}"),
        Ok(_) => panic!("2000-deep nesting should not parse"),
    }
}

#[test]
fn an_implausible_array_length_is_rejected_before_allocating() {
    // A file claiming four billion elements must fail on the declared size, not
    // by attempting the reservation.
    let mut data = Vec::new();
    data.extend_from_slice(b"Kaydara FBX Binary  \x00\x1a\x00");
    data.extend_from_slice(&7700u32.to_le_bytes());

    let body_start = data.len();
    let mut node = Vec::new();
    node.extend_from_slice(&1u64.to_le_bytes()); // one property
    node.extend_from_slice(&0u64.to_le_bytes());
    node.push(1);
    node.push(b'n');
    node.push(b'd'); // f64 array
    node.extend_from_slice(&u32::MAX.to_le_bytes()); // element count
    node.extend_from_slice(&0u32.to_le_bytes()); // uncompressed
    node.extend_from_slice(&0u32.to_le_bytes());

    let end = body_start + 8 + node.len();
    data.extend_from_slice(&(end as u64).to_le_bytes());
    data.extend_from_slice(&node);
    data.extend_from_slice(&[0u8; 200]);

    // Specifically ImplausibleLength, not Truncated: the point is that the
    // declared size is rejected BEFORE Vec::with_capacity, and accepting either
    // variant would let the test pass with the capacity check deleted.
    match parse(&data) {
        Err(FbxError::ImplausibleLength { .. }) => {}
        Err(other) => panic!("expected ImplausibleLength, got {other}"),
        Ok(_) => panic!("a u32::MAX element array should not parse"),
    }
}
