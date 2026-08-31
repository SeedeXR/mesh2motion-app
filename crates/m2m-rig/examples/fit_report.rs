//! Prints what fitting a template onto a model actually produces, so the
//! numbers a test asserts can be seen rather than guessed at.

use glam::{Mat4, Quat, Vec3};
use m2m_core::mesh::Mesh;
use m2m_rig::fit::{fit_uniform, Landmarks, RestPose};

fn world(document: &m2m_io::glb::Document) -> Vec<Mat4> {
    let local: Vec<Mat4> = document
        .nodes
        .iter()
        .map(|n| {
            Mat4::from_scale_rotation_translation(
                Vec3::from(n.transform.scale),
                Quat::from_array(n.transform.rotation),
                Vec3::from(n.transform.translation),
            )
        })
        .collect();
    let mut out = vec![Mat4::IDENTITY; document.nodes.len()];
    for (i, slot) in out.iter_mut().enumerate() {
        let mut chain = vec![i];
        let mut cursor = i;
        while let Some(p) = document.nodes[cursor].parent {
            chain.push(p);
            cursor = p;
        }
        let mut m = Mat4::IDENTITY;
        for &node in chain.iter().rev() {
            m *= local[node];
        }
        *slot = m;
    }
    out
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let model = args
        .next()
        .ok_or("usage: fit_report <model.glb> <rig.glb>")?;
    let rig = args
        .next()
        .ok_or("usage: fit_report <model.glb> <rig.glb> <template.json>")?;
    let manifest = args
        .next()
        .ok_or("usage: fit_report <model.glb> <rig.glb> <template.json>")?;

    let doc = m2m_io::glb::read(&std::fs::read(&model)?)?;
    let w = world(&doc);
    let mut mesh = Mesh::default();
    for p in &doc.primitives {
        let t = p.node.map_or(Mat4::IDENTITY, |n| w[n]);
        for c in p.positions.chunks_exact(3) {
            mesh.positions
                .push(t.transform_point3(Vec3::new(c[0], c[1], c[2])));
        }
    }
    let landmarks = Landmarks::of(&mesh).ok_or("no vertices")?;

    let rdoc = m2m_io::glb::read(&std::fs::read(&rig)?)?;
    let rw = world(&rdoc);
    let skin = rdoc.skins.first().ok_or("no skin")?;
    let rest = RestPose {
        bones: skin
            .joints
            .iter()
            .map(|&j| rdoc.nodes[j].name.clone())
            .collect(),
        positions: skin
            .joints
            .iter()
            .map(|&j| rw[j].transform_point3(Vec3::ZERO))
            .collect(),
    };
    let (rmin, rmax) = rest.bounds().ok_or("no bones")?;
    // The spine comes from the template, not a hardcoded list: a fox's spine
    // bones are not a human's, and this tool has to work on every species.
    let template: m2m_rig::template::Template =
        serde_json::from_str(&std::fs::read_to_string(&manifest)?)?;
    let spine: Vec<String> = template
        .of_kind(m2m_rig::template::ChainKind::Spine)
        .flat_map(|c| c.bones.clone())
        .collect();
    let fitted = fit_uniform(&rest, &landmarks, &spine).ok_or("cannot fit")?;

    println!("mesh   min {:?}  max {:?}", landmarks.min, landmarks.max);
    println!(
        "mesh   ground {:.4}  symmetry_x {:.4}  symmetry_error {:.5}",
        landmarks.ground,
        landmarks.symmetry_x,
        landmarks.symmetry_error(&mesh)
    );
    println!("rest   min {rmin:?}  max {rmax:?}");
    println!(
        "fit    scale {:.4}  offset {:?}",
        fitted.scale, fitted.offset
    );

    // What refinement does to each spine joint.
    let axis = m2m_rig::fit::body_axis(&rest, &spine).expect("axis");
    println!("axis   {axis:?}");
    let mut refined = fitted.clone();
    m2m_rig::fit::refine_spine(&mut refined, &mesh, &landmarks, &spine, axis);
    for bone in spine.iter().map(String::as_str) {
        let (a, b) = (
            fitted.position_of(bone).unwrap(),
            refined.position_of(bone).unwrap(),
        );
        println!(
            "  {bone:14} before ({:+.3},{:+.3},{:+.3})  after ({:+.3},{:+.3},{:+.3})  moved {:.4}",
            a.x,
            a.y,
            a.z,
            b.x,
            b.y,
            b.z,
            a.distance(b)
        );
    }

    // Where the lower spine lands, and what the mesh looks like at that height.
    for bone in spine.iter().map(String::as_str) {
        let Some(at) = fitted.position_of(bone) else {
            continue;
        };
        let band: Vec<&Vec3> = mesh
            .positions
            .iter()
            .filter(|v| (v.y - at.y).abs() < landmarks.extent().y * 0.02)
            .collect();
        if band.is_empty() {
            println!("  {bone:9} at {at:?}  -- no mesh at this height");
            continue;
        }
        let zmin = band.iter().map(|v| v.z).fold(f32::INFINITY, f32::min);
        let zmax = band.iter().map(|v| v.z).fold(f32::NEG_INFINITY, f32::max);
        let xmin = band.iter().map(|v| v.x).fold(f32::INFINITY, f32::min);
        let xmax = band.iter().map(|v| v.x).fold(f32::NEG_INFINITY, f32::max);
        let inside = at.z > zmin && at.z < zmax && at.x > xmin && at.x < xmax;
        println!(
            "  {bone:9} at ({:+.3},{:+.3},{:+.3})  slice x[{xmin:+.3},{xmax:+.3}] z[{zmin:+.3},{zmax:+.3}]  {}",
            at.x, at.y, at.z,
            if inside { "in slice" } else { "OUTSIDE slice" }
        );
    }
    Ok(())
}
