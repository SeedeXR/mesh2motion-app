//! Reads a `.glb` and prints what the reader found, as JSON.
//!
//! Exists so the differential gate can compare these numbers against Blender's
//! for the same file, the same way `rebuild_rig` is compared for FBX.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: read_glb <file.glb>")?;
    let bytes = std::fs::read(&path)?;
    let document = m2m_io::glb::read(&bytes)?;

    let vertices: usize = document
        .primitives
        .iter()
        .map(|p| p.positions.len() / 3)
        .sum();
    let triangles: usize = document
        .primitives
        .iter()
        .map(|p| p.indices.len() / 3)
        .sum();
    let bones = document.skins.first().map(|s| s.joints.len()).unwrap_or(0);
    let mut bone_names: Vec<&str> = Vec::new();
    if let Some(skin) = document.skins.first() {
        for &joint in &skin.joints {
            bone_names.push(document.nodes[joint].name.as_str());
        }
    }
    // A vertex counts as weighted if any of its four influences is non-zero.
    let weighted: usize = document
        .primitives
        .iter()
        .map(|p| {
            p.weights
                .chunks_exact(4)
                .filter(|w| w.iter().any(|&x| x > 0.0))
                .count()
        })
        .sum();

    let clips: Vec<String> = document
        .clips
        .iter()
        .map(|c| {
            let keys: usize = c.channels.iter().map(|ch| ch.times.len()).sum();
            // Blender splits each channel into one F-curve per component, so
            // its key total is the stride-weighted one. Emitted to make the
            // two directly comparable.
            let component_keys: usize = c
                .channels
                .iter()
                .map(|ch| ch.times.len() * ch.path.stride().unwrap_or(1))
                .sum();
            format!(
                "{}:channels={},keys={},component_keys={},duration={:.4}",
                c.name,
                c.channels.len(),
                keys,
                component_keys,
                c.duration
            )
        })
        .collect();

    println!("{{");
    println!("  \"nodes\": {},", document.nodes.len());
    println!("  \"primitives\": {},", document.primitives.len());
    println!("  \"meshes\": {},", document.mesh_count());
    println!("  \"vertices\": {vertices},");
    println!("  \"triangles\": {triangles},");
    println!("  \"skins\": {},", document.skins.len());
    println!("  \"bones\": {bones},");
    println!("  \"weighted_vertices\": {weighted},");
    println!("  \"clips\": {:?},", clips);
    println!("  \"bone_names\": {:?},", bone_names);
    println!("  \"report\": \"{:?}\"", document.report);
    println!("}}");
    Ok(())
}
