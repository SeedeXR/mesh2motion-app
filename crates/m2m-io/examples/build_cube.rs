//! Writes a cube built by `fbx::build`, for an independent reader to check.
//!
//!     cargo run -p m2m-io --release --example build_cube -- <out.fbx>

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let Some(output) = std::env::args().nth(1) else {
        eprintln!("usage: build_cube <out.fbx>");
        std::process::exit(2);
    };

    // A unit cube: 8 corners, 12 triangles. Small enough to verify by eye in
    // an importer, and it exercises shared vertices between faces.
    let positions: [f32; 24] = [
        0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 0.0, 1.0, 0.0, //
        0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0, 0.0, 1.0, 1.0,
    ];
    let triangles: [u32; 36] = [
        0, 2, 1, 0, 3, 2, // back
        4, 5, 6, 4, 6, 7, // front
        0, 1, 5, 0, 5, 4, // bottom
        2, 3, 7, 2, 7, 6, // top
        0, 4, 7, 0, 7, 3, // left
        1, 2, 6, 1, 6, 5, // right
    ];

    let mesh = m2m_io::fbx::build::Mesh {
        name: "Cube",
        positions: &positions,
        faces: m2m_io::fbx::build::Faces::Triangles(&triangles),
    };
    let document = m2m_io::fbx::build::build(&m2m_io::fbx::build::Scene {
        meshes: &[mesh],
        bones: &[],
        skins: &[],
    });
    let bytes = m2m_io::fbx::encode::encode(&document)?;
    std::fs::write(&output, &bytes)?;
    println!("{} bytes, {} roots", bytes.len(), document.roots.len());
    Ok(())
}
