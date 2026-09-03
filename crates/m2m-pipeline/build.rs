//! Embeds every rig `.glb` a template can name, as `OUT_DIR/skeletons.rs`.
//!
//! All nine total ~158 KB, so carrying them in the binary costs less than the
//! machinery to find them on disk would. Globbed rather than listed, matching
//! `m2m-rig`'s manifests: adding a creature stays a matter of adding files. They
//! live under `assets/rigs/` at the repo root.
fn main() {
    use std::{fs, path::PathBuf};

    // From crates/m2m-pipeline, the repo root is two levels up.
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets this"))
        .join("../../assets/rigs");
    println!("cargo:rerun-if-changed={}", dir.display());

    let mut rigs: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "glb"))
        .collect();
    rigs.sort();

    let mut source = String::from("static SKELETONS: &[(&str, &[u8])] = &[\n");
    for path in rigs {
        let name = path
            .file_name()
            .expect("a .glb has a name")
            .to_string_lossy()
            .into_owned();
        source.push_str(&format!(
            "    ({name:?}, include_bytes!({:?})),\n",
            path.display().to_string()
        ));
    }
    source.push_str("];\n");

    let out =
        PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets this")).join("skeletons.rs");
    fs::write(&out, source).unwrap_or_else(|e| panic!("cannot write {}: {e}", out.display()));
}
