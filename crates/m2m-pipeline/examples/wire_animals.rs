//! Wires an animals-3d `output/glb/<animal>.glb` (mesh + native skeleton + all
//! clips) into the app's two-file form, using the app's OWN glTF reader/writer —
//! no Blender. Emits a skeleton-only `rig-<animal>.glb` (embedded as the template
//! rig) and a `<animal>-animations.glb` clips library (the mesh stays in the
//! source glb, which is copied separately as the character sample).
//!
//! Run: `cargo run -p m2m-pipeline --example wire_animals -- \
//!         <input.glb> <rig-out.glb> <library-out.glb>`
//!
//! `*_source_*` clips (raw un-authored passthroughs) are dropped.

use m2m_io::glb;

fn main() {
    // Optional trailing `<old_prefix> <new_prefix>` renames clip labels (e.g.
    // cat2_ -> cat_); the animation data is untouched.
    let args: Vec<String> = std::env::args().collect();
    let (input, rig_out, lib_out, rename): (&str, &str, &str, Option<(&str, &str)>) = match args
        .as_slice()
    {
        [_, i, r, l] => (i, r, l, None),
        [_, i, r, l, from, to] => (i, r, l, Some((from, to))),
        _ => {
            eprintln!("usage: wire_animals <input.glb> <rig-out.glb> <library-out.glb> [old_prefix new_prefix]");
            std::process::exit(2);
        }
    };

    let bytes = std::fs::read(input).unwrap_or_else(|e| panic!("read {input}: {e}"));
    let doc = glb::read(&bytes).unwrap_or_else(|e| panic!("parse {input}: {e}"));
    let skin = doc.skins.first().expect("the source glb has a skin");
    let clip_names: Vec<&str> = doc.clips.iter().map(|c| c.name.as_str()).collect();
    println!(
        "read {input}: nodes={} primitives={} skins={} joints={} clips={:?}",
        doc.nodes.len(),
        doc.primitives.len(),
        doc.skins.len(),
        skin.joints.len(),
        clip_names
    );

    // Skeleton-only: keep nodes + skins, drop the mesh geometry and materials.
    let skeleton = |clips: Vec<glb::Clip>| glb::Document {
        nodes: doc.nodes.clone(),
        primitives: Vec::new(),
        materials: Vec::new(),
        skins: doc.skins.clone(),
        clips,
        report: glb::GlbReport::default(),
    };

    let rig = skeleton(Vec::new());
    let rig_bytes = glb::write(&rig).expect("write rig");
    std::fs::write(rig_out, &rig_bytes).unwrap_or_else(|e| panic!("write {rig_out}: {e}"));

    // Library: the same skeleton plus the authored clips (drop *_source_* raws).
    let kept: Vec<glb::Clip> = doc
        .clips
        .iter()
        .filter(|c| !c.name.contains("_source_"))
        .cloned()
        .map(|mut c| {
            if let Some((from, to)) = rename {
                if let Some(rest) = c.name.strip_prefix(from) {
                    c.name = format!("{to}{rest}");
                }
            }
            c
        })
        .collect();
    let dropped: Vec<&str> = doc
        .clips
        .iter()
        .filter(|c| c.name.contains("_source_"))
        .map(|c| c.name.as_str())
        .collect();
    let kept_names: Vec<&str> = kept.iter().map(|c| c.name.as_str()).collect();
    let library = skeleton(kept.clone());
    let lib_bytes = glb::write(&library).expect("write library");
    std::fs::write(lib_out, &lib_bytes).unwrap_or_else(|e| panic!("write {lib_out}: {e}"));

    // Verify both re-read, and the library kept exactly the authored clips.
    let rig_back = glb::read(&rig_bytes).expect("re-read rig");
    let lib_back = glb::read(&lib_bytes).expect("re-read library");
    assert!(!rig_back.skins.is_empty(), "rig lost its skin");
    assert_eq!(rig_back.primitives.len(), 0, "rig kept mesh");
    assert_eq!(lib_back.clips.len(), kept.len(), "library lost clips");
    assert_eq!(
        rig_back.skins[0].joints.len(),
        skin.joints.len(),
        "rig lost joints"
    );

    println!(
        "wrote {rig_out} ({} KB, {} joints) and {lib_out} ({} KB, {} clips)",
        rig_bytes.len() / 1024,
        rig_back.skins[0].joints.len(),
        lib_bytes.len() / 1024,
        lib_back.clips.len()
    );
    println!("kept clips: {kept_names:?}");
    println!("dropped *_source_*: {dropped:?}");
}
