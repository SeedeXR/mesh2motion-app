//! Parses every FBX in `references/`, when it is present.
//!
//! Those files are Mixamo exports and are gitignored — royalty-free to use but
//! not CC0, and this repo licenses its art as CC0 (see `.gitignore`). So this
//! runs locally and skips in CI rather than failing there.
//!
//! Deliberately a breadth check, not a correctness one: the committed
//! `mixamo-original-rig.fbx` in `fbx_binary.rs` carries the assertions. This
//! only answers "does the parser survive files it has never seen".

use m2m_io::fbx::binary::parse;

#[test]
fn parses_every_reference_export_when_available() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../references/human_based_fbx_mixamo_animations");

    let Ok(entries) = std::fs::read_dir(&dir) else {
        eprintln!("references/ not present (expected in CI) — skipping");
        return;
    };

    let mut parsed = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("fbx") {
            continue;
        }
        let data = std::fs::read(&path).expect("readable file");
        let doc =
            parse(&data).unwrap_or_else(|e| panic!("{} failed to parse: {e}", path.display()));

        // Every Mixamo export shares the same top-level shape.
        assert_eq!(doc.version, 7700);
        assert_eq!(doc.roots.len(), 11);
        assert!(doc.root("Objects").is_some());
        assert!(doc.root("Takes").is_some());
        parsed += 1;
    }

    if parsed == 0 {
        eprintln!("references/ present but held no .fbx files — skipping");
    } else {
        eprintln!("parsed {parsed} reference exports");
    }
}
