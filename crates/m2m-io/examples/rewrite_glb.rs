//! Reads a `.glb` and writes it back out, so the round trip can be measured by
//! an independent reader rather than by our own.

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let input = args.next().ok_or("usage: rewrite_glb <in.glb> <out.glb>")?;
    let output = args.next().ok_or("usage: rewrite_glb <in.glb> <out.glb>")?;

    let document = m2m_io::glb::read(&std::fs::read(&input)?)?;
    let bytes = m2m_io::glb::write(&document)?;
    std::fs::write(&output, &bytes)?;

    let reread = m2m_io::glb::read(&bytes)?;
    let vertices = |d: &m2m_io::glb::Document| -> usize {
        d.primitives.iter().map(|p| p.positions.len() / 3).sum()
    };
    let triangles = |d: &m2m_io::glb::Document| -> usize {
        d.primitives.iter().map(|p| p.indices.len() / 3).sum()
    };
    println!(
        "{} nodes, {} meshes, {} vertices, {} triangles, {} skins, {} clips -> {} bytes",
        reread.nodes.len(),
        reread.mesh_count(),
        vertices(&reread),
        triangles(&reread),
        reread.skins.len(),
        reread.clips.len(),
        bytes.len()
    );
    if reread.nodes.len() != document.nodes.len()
        || vertices(&reread) != vertices(&document)
        || triangles(&reread) != triangles(&document)
        || reread.mesh_count() != document.mesh_count()
        || reread.clips.len() != document.clips.len()
    {
        return Err("the round trip changed the document".into());
    }
    Ok(())
}
