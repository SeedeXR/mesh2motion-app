//! Writes `parse -> encode` of an FBX file, for a different reader to check.
//!
//! Our own round-trip test proves the writer and the reader agree; it cannot
//! prove the output is valid FBX. Four conformance details survive it because
//! our reader does not check them. This exists so `legacy/bench/` can hand the
//! bytes to three.js's loader, which does.
//!
//!     cargo run -p m2m-io --release --example encode_roundtrip -- <in.fbx> <out.fbx>

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("usage: encode_roundtrip <in.fbx> <out.fbx>");
        std::process::exit(2);
    };

    let bytes = std::fs::read(&input)?;
    let document = m2m_io::fbx::binary::parse(&bytes)?;
    let encoded = m2m_io::fbx::encode::encode(&document)?;

    // Confirm our own reader still agrees before handing it to another one, so
    // a failure downstream is unambiguously about conformance.
    let reparsed = m2m_io::fbx::binary::parse(&encoded)?;
    if reparsed != document {
        return Err("the document changed across our own round trip".into());
    }

    std::fs::write(&output, &encoded)?;
    println!(
        "{} bytes in, {} bytes out, {} roots",
        bytes.len(),
        encoded.len(),
        document.roots.len()
    );
    Ok(())
}
