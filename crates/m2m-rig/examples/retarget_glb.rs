//! Retargets a clip from one rigged `.glb` onto another and writes the result.
//!
//! The whole pipeline in one place: read both rigs, map their bones, move the
//! animation across, write it out. Exists so the result can be checked by
//! Blender and assimp rather than only by our own reader.
//!
//! Usage: retarget_glb <source.glb> <target.glb> <out.glb>

use std::collections::HashMap;

use glam::{Mat4, Quat, Vec3};
use m2m_rig::automap::{map_bones_best, KnownRig, Skeleton};
use m2m_rig::retarget::{
    height_scale, retarget, retarget_translations, Clip, RestRotations, RestTranslations,
    RotationTrack, TranslationTrack,
};

/// A rig's skeleton, rest pose and clips, read from a `.glb`.
struct Rig {
    document: m2m_io::glb::Document,
    /// Node index of each bone, so tracks can be pointed back at nodes.
    nodes: Vec<usize>,
    skeleton: Skeleton,
    rotations: RestRotations,
    translations: RestTranslations,
}

fn read(path: &str) -> Result<Rig, Box<dyn std::error::Error>> {
    let document = m2m_io::glb::read(&std::fs::read(path)?)?;
    let skin = document.skins.first().ok_or("no skin")?;
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

    let slots: HashMap<usize, usize> = skin
        .joints
        .iter()
        .enumerate()
        .map(|(slot, &node)| (node, slot))
        .collect();
    Ok(Rig {
        nodes: skin.joints.clone(),
        skeleton: Skeleton {
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
        },
        rotations: RestRotations {
            local: skin
                .joints
                .iter()
                .map(|&j| Quat::from_array(document.nodes[j].transform.rotation))
                .collect(),
        },
        translations: RestTranslations {
            local: skin
                .joints
                .iter()
                .map(|&j| Vec3::from(document.nodes[j].transform.translation))
                .collect(),
        },
        document,
    })
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let source_path = args.next().ok_or("usage: retarget_glb <src> <dst> <out>")?;
    let target_path = args.next().ok_or("need a target")?;
    let out_path = args.next().ok_or("need an output path")?;

    let source = read(&source_path)?;
    let target = read(&target_path)?;

    let known: Vec<KnownRig> = ["mixamo.json", "rigify.json"]
        .iter()
        .map(|file| {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/known-rigs/").to_owned() + file;
            serde_json::from_str(&std::fs::read_to_string(&path).expect("reads")).expect("parses")
        })
        .collect();

    // The mapping runs target-to-source here: we are filling in the target's
    // bones, so we need "for each of my bones, whose motion drives it".
    let (target_to_source, strategy) =
        map_bones_best(&target.skeleton, &source.skeleton, &known, 0.5);
    let source_to_target: HashMap<usize, usize> =
        target_to_source.iter().map(|(&t, &s)| (s, t)).collect();
    println!("mapped {} bones via {strategy:?}", source_to_target.len());

    let scale = height_scale(&source.skeleton, &target.skeleton);
    let node_to_bone: HashMap<usize, usize> = source
        .nodes
        .iter()
        .enumerate()
        .map(|(bone, &node)| (node, bone))
        .collect();

    let mut out = target.document.clone();
    out.clips.clear();
    for clip in &source.document.clips {
        let mut rotations = Vec::new();
        let mut translations = Vec::new();
        for channel in &clip.channels {
            let Some(&bone) = node_to_bone.get(&channel.node) else {
                continue;
            };
            match channel.path {
                m2m_io::glb::Path::Rotation => rotations.push(RotationTrack {
                    bone,
                    times: channel.times.clone(),
                    rotations: channel
                        .values
                        .chunks_exact(4)
                        .map(|q| Quat::from_xyzw(q[0], q[1], q[2], q[3]))
                        .collect(),
                }),
                m2m_io::glb::Path::Translation => translations.push(TranslationTrack {
                    bone,
                    times: channel.times.clone(),
                    translations: channel
                        .values
                        .chunks_exact(3)
                        .map(|v| Vec3::new(v[0], v[1], v[2]))
                        .collect(),
                }),
                _ => {}
            }
        }

        let (moved, report) = retarget(
            &source.skeleton,
            &source.rotations,
            &target.skeleton,
            &target.rotations,
            &source_to_target,
            &Clip {
                name: clip.name.clone(),
                tracks: rotations,
            },
        );
        let moved_translations = retarget_translations(
            &source.translations,
            &target.translations,
            &source_to_target,
            &translations,
            scale,
        );

        let mut channels = Vec::new();
        for track in &moved.tracks {
            channels.push(m2m_io::glb::Channel {
                node: target.nodes[track.bone],
                path: m2m_io::glb::Path::Rotation,
                times: track.times.clone(),
                values: track
                    .rotations
                    .iter()
                    .flat_map(|q| [q.x, q.y, q.z, q.w])
                    .collect(),
            });
        }
        for track in &moved_translations {
            channels.push(m2m_io::glb::Channel {
                node: target.nodes[track.bone],
                path: m2m_io::glb::Path::Translation,
                times: track.times.clone(),
                values: track
                    .translations
                    .iter()
                    .flat_map(|v| [v.x, v.y, v.z])
                    .collect(),
            });
        }
        println!(
            "  {}: {} channels, {} unmapped, {} malformed",
            clip.name,
            channels.len(),
            report.unmapped_tracks,
            report.malformed_tracks
        );
        out.clips.push(m2m_io::glb::Clip {
            name: clip.name.clone(),
            duration: clip.duration,
            channels,
        });
    }

    std::fs::write(&out_path, m2m_io::glb::write(&out)?)?;
    println!("wrote {out_path}");
    Ok(())
}
