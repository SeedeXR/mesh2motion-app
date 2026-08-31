//! Prints what `import::inspect` makes of a file.
//!
//!     cargo run -p m2m-io --example inspect -- <file>...

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for path in std::env::args().skip(1) {
        let bytes = std::fs::read(&path)?;
        match m2m_io::import::inspect(&bytes) {
            Ok(i) => println!(
                "{path}\n  {:?} meshes={} skinned={} bones={} clips={} over_influence={} rigged={}\n  first bones: {:?}\n  first clips: {:?}",
                i.format,
                i.meshes,
                i.skinned_meshes,
                i.bones.len(),
                i.clips.len(),
                i.over_influence_limit,
                i.already_rigged(),
                &i.bones[..i.bones.len().min(4)],
                &i.clips[..i.clips.len().min(3)],
            ),
            Err(e) => println!("{path}\n  ERROR {e}"),
        }
    }
    Ok(())
}
