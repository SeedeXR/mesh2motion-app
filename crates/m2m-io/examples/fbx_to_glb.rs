//! Converts an FBX to a `.glb`.
//!
//!     cargo run -p m2m-io --release --example fbx_to_glb -- <in.fbx> <out.glb>

use m2m_io::fbx::{binary, dom::Scene};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("usage: fbx_to_glb <in.fbx> <out.glb>");
        std::process::exit(2);
    };

    let scene = Scene::from_document(binary::parse(&std::fs::read(&input)?)?);
    println!("unit scale: {:?}", scene.unit_scale);

    let document = m2m_io::convert::fbx_to_gltf(&scene)?;
    let heights: Vec<f32> = document
        .primitives
        .iter()
        .map(|p| {
            let ys: Vec<f32> = p.positions.chunks_exact(3).map(|v| v[1]).collect();
            ys.iter().copied().fold(f32::MIN, f32::max)
                - ys.iter().copied().fold(f32::MAX, f32::min)
        })
        .collect();
    println!(
        "nodes={} primitives={} skins={} joints={:?} mesh heights (file units)={:?}",
        document.nodes.len(),
        document.primitives.len(),
        document.skins.len(),
        document
            .skins
            .iter()
            .map(|s| s.joints.len())
            .collect::<Vec<_>>(),
        heights,
    );
    println!(
        "root scale: {:?}",
        document
            .nodes
            .iter()
            .filter(|n| n.parent.is_none())
            .map(|n| n.transform.scale)
            .collect::<Vec<_>>()
    );

    let bytes = m2m_io::glb::write(&document)?;
    std::fs::write(&output, &bytes)?;
    println!("wrote {} bytes to {output}", bytes.len());
    Ok(())
}
