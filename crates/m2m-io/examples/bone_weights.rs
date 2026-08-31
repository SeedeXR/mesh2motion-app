//! Reports how much of the mesh each bone actually deforms.
//!
//! A rig can contain bones that drive other bones rather than the mesh — IK
//! targets, pole targets. They appear in the skin's joint list like any other,
//! and the only way to tell from the file is that nothing is weighted to them.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: bone_weights <file.glb>")?;
    let document = m2m_io::glb::read(&std::fs::read(&path)?)?;
    let skin = document.skins.first().ok_or("no skin")?;

    let mut total = vec![0.0f64; skin.joints.len()];
    let mut vertices = vec![0usize; skin.joints.len()];
    for primitive in &document.primitives {
        for (joints, weights) in primitive
            .joints
            .chunks_exact(4)
            .zip(primitive.weights.chunks_exact(4))
        {
            for (&joint, &weight) in joints.iter().zip(weights) {
                let index = joint as usize;
                if weight > 0.0 && index < total.len() {
                    total[index] += f64::from(weight);
                    vertices[index] += 1;
                }
            }
        }
    }

    let mut unused: Vec<&str> = Vec::new();
    for (index, &joint) in skin.joints.iter().enumerate() {
        if vertices[index] == 0 {
            unused.push(&document.nodes[joint].name);
        }
    }
    println!(
        "{} joints, {} deform the mesh, {} deform nothing",
        skin.joints.len(),
        skin.joints.len() - unused.len(),
        unused.len()
    );
    if !unused.is_empty() {
        println!("  deform nothing: {unused:?}");
    }
    Ok(())
}
