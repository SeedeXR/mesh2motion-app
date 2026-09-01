//! Embeds every template manifest in `templates/`.
//!
//! Globbed rather than listed, because `lib.rs`'s design rule says adding a
//! creature must never require a change in this crate. A hand-written list of
//! `include_str!` calls would break that on the first new species.

use std::{env, fs, path::PathBuf};

fn main() {
    let dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("cargo sets this")).join("templates");
    println!("cargo:rerun-if-changed={}", dir.display());

    let mut manifests: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    // Sorted so the shipped order is the same on every machine and in every
    // build; `read_dir` order is the filesystem's, not anything stable.
    manifests.sort();

    let mut source = String::from("static MANIFESTS: &[(&str, &str)] = &[\n");
    for path in manifests {
        let name = path
            .file_stem()
            .expect("a .json has a stem")
            .to_string_lossy()
            .into_owned();
        source.push_str(&format!(
            "    ({name:?}, include_str!({:?})),\n",
            path.display().to_string()
        ));
    }
    source.push_str("];\n");

    let out = PathBuf::from(env::var("OUT_DIR").expect("cargo sets this")).join("manifests.rs");
    fs::write(&out, source).unwrap_or_else(|e| panic!("cannot write {}: {e}", out.display()));
}
