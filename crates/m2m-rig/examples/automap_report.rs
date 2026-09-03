//! Reports how well structural matching agrees with a hand-authored table.
//!
//! Exists to answer whether a known-rig table earns its place: if structure
//! already reproduces the table, the table is duplication.

use glam::{Mat4, Quat, Vec3};
use m2m_rig::automap::{map_bones, Skeleton};

fn skeleton_of(path: &str) -> Result<Skeleton, Box<dyn std::error::Error>> {
    let document = m2m_io::glb::read(&std::fs::read(path)?)?;
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
    let mut world = vec![Mat4::IDENTITY; document.nodes.len()];
    for (index, slot) in world.iter_mut().enumerate() {
        let mut chain = vec![index];
        let mut cursor = index;
        while let Some(parent) = document.nodes[cursor].parent {
            chain.push(parent);
            cursor = parent;
        }
        let mut matrix = Mat4::IDENTITY;
        for &node in chain.iter().rev() {
            matrix *= local[node];
        }
        *slot = matrix;
    }
    let skin = document.skins.first().ok_or("no skin")?;
    let slots: std::collections::HashMap<usize, usize> = skin
        .joints
        .iter()
        .enumerate()
        .map(|(slot, &node)| (node, slot))
        .collect();
    Ok(Skeleton {
        names: skin
            .joints
            .iter()
            .map(|&j| document.nodes[j].name.clone())
            .collect(),
        parents: skin
            .joints
            .iter()
            .map(|&j| {
                document.nodes[j]
                    .parent
                    .and_then(|p| slots.get(&p).copied())
            })
            .collect(),
        positions: skin
            .joints
            .iter()
            .map(|&j| world[j].transform_point3(Vec3::ZERO))
            .collect(),
    })
}

/// Names compare ignoring case and any punctuation, so `mixamorig:Hips` and
/// `mixamorigHips` are the same bone.
fn normalise(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let ours = skeleton_of(
        &args
            .next()
            .ok_or("usage: <ours.glb> <theirs.glb> <table.json>")?,
    )?;
    let theirs = skeleton_of(&args.next().ok_or("need theirs.glb")?)?;
    let table: std::collections::HashMap<String, String> = serde_json::from_str(
        &std::fs::read_to_string(args.next().ok_or("need table.json")?)?,
    )?;

    let mapping = map_bones(&ours, &theirs);
    let (mut agree, mut disagree, mut absent) = (0, 0, 0);
    let mut examples = Vec::new();
    for (from, expected) in &table {
        let Some(index) = ours.names.iter().position(|n| n == from) else {
            absent += 1;
            continue;
        };
        match mapping.get(&index) {
            None => absent += 1,
            Some(&to) => {
                if normalise(&theirs.names[to]) == normalise(expected) {
                    agree += 1;
                } else {
                    disagree += 1;
                    if examples.len() < 8 {
                        examples.push(format!(
                            "{from} -> {} (table: {expected})",
                            theirs.names[to]
                        ));
                    }
                }
            }
        }
    }
    println!("structure agrees with the table on {agree}, differs on {disagree}, absent {absent}");
    for example in examples {
        println!("   {example}");
    }
    Ok(())
}
