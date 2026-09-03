//! Reads an FBX rig through our own types and writes it back out.
//!
//! This is the acceptance test for O9 (`memory/project_context.md`): an existing
//! rig must survive import and export untouched — skeleton, bone names,
//! hierarchy, bind matrices, skin weights and animation. It goes all the way
//! through the semantic layers and rebuilds from them (unlike `encode_roundtrip`,
//! which re-encodes the reader's document), so anything those layers drop is
//! visible in the output. The round-trip itself lives in `fbx::roundtrip` so the
//! cross-engine bridge tests exercise the same code path.
//!
//!     cargo run -p m2m-io --release --example rebuild_rig -- <in.fbx> <out.fbx>

use m2m_io::fbx::roundtrip;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("usage: rebuild_rig <in.fbx> <out.fbx>");
        std::process::exit(2);
    };

    let encoded = roundtrip::reencode(&std::fs::read(&input)?)?;
    std::fs::write(&output, &encoded)?;
    println!("{} -> {}, {} bytes", input, output, encoded.len());
    Ok(())
}
