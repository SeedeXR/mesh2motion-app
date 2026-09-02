//! Retargets Mixamo `.fbx` animation clips onto the m2m human skeleton and
//! merges them into the human animation library.
//!
//! The shipped libraries are pre-authored: the app retargets a library clip
//! onto a fitted mesh, it does not import raw Mixamo files. This is the offline
//! step that turns a Mixamo download into a library clip. For each input FBX it
//! reads the `mixamorig` skeleton and its animation (our own FBX reader, which
//! `convert::fbx_to_gltf` does not yet carry animation through, so the clip is
//! attached here), maps `mixamorig` to the human template bones with the same
//! `map_bones_best` + Mixamo `KnownRig` the app's retargeter uses, moves the
//! motion across, and appends it — named after the file — to the target
//! library's existing clips.
//!
//!     cargo run -p m2m-rig --release --example human_clips -- <library.glb> <out.glb> <clip.fbx>...
//!
//! The clips are rotation-only (Mixamo root translation is centimetre-scale and
//! an in-place clip is what a retargetable library wants); existing clips in the
//! library are kept verbatim.

use std::collections::HashMap;

use glam::{Mat4, Quat, Vec3};
use m2m_io::fbx::{animation, binary, dom::Scene, model, text};
use m2m_io::{convert, glb};
use m2m_rig::automap::{map_bones_best, KnownRig, Skeleton};
use m2m_rig::retarget::{retarget, Clip, RestRotations, RotationTrack};

/// A rig's skeleton, rest pose and clips, ready to retarget from or into.
struct Rig {
    document: glb::Document,
    nodes: Vec<usize>,
    skeleton: Skeleton,
    rotations: RestRotations,
}

/// Builds a `Rig` from a glb document, treating `joints` (node indices) as the
/// bones. A skinned library passes its skin's joints; an animation-only FBX,
/// which has no skin, passes every node.
fn rig_of(document: glb::Document, joints: Vec<usize>) -> Result<Rig, Box<dyn std::error::Error>> {
    if joints.is_empty() {
        return Err("no bones".into());
    }
    let world = world_transforms(&document);
    let slots: HashMap<usize, usize> = joints
        .iter()
        .enumerate()
        .map(|(slot, &node)| (node, slot))
        .collect();
    Ok(Rig {
        skeleton: Skeleton {
            names: joints
                .iter()
                .map(|&j| document.nodes[j].name.clone())
                .collect(),
            parents: joints
                .iter()
                .map(|&j| {
                    document.nodes[j]
                        .parent
                        .and_then(|p| slots.get(&p).copied())
                })
                .collect(),
            positions: joints
                .iter()
                .map(|&j| world[j].transform_point3(Vec3::ZERO))
                .collect(),
        },
        rotations: RestRotations {
            local: joints
                .iter()
                .map(|&j| Quat::from_array(document.nodes[j].transform.rotation))
                .collect(),
        },
        nodes: joints,
        document,
    })
}

/// The joints of a document's first skin, or every node when it has no skin.
fn joints_of(document: &glb::Document) -> Vec<usize> {
    match document.skins.first() {
        Some(skin) => skin.joints.clone(),
        None => (0..document.nodes.len()).collect(),
    }
}

fn world_transforms(document: &glb::Document) -> Vec<Mat4> {
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
    world
}

/// Reads an FBX file into a glb document with its skeleton and one clip.
fn read_fbx(path: &str) -> Result<glb::Document, Box<dyn std::error::Error>> {
    let bytes = std::fs::read(path)?;
    let doc = match binary::parse(&bytes) {
        Ok(d) => d,
        Err(_) => {
            let source = std::str::from_utf8(&bytes)?;
            text::parse(source)?
        }
    };
    let scene = Scene::from_document(doc);
    let models = model::parse_all(&scene);
    let mut document = convert::fbx_to_gltf(&scene)?;
    let (clips, _) = animation::parse_all(&scene, &models);

    let node_of_name: HashMap<String, usize> = document
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.name.clone(), i))
        .collect();

    let mut out_clips = Vec::new();
    for clip in &clips {
        let mut channels = Vec::new();
        for track in &clip.tracks {
            let Some(m) = models.get(track.model) else {
                continue;
            };
            let Some(&node) = node_of_name.get(&m.name) else {
                continue;
            };
            let path = match track.kind {
                animation::TrackKind::Quaternion => glb::Path::Rotation,
                animation::TrackKind::Position => glb::Path::Translation,
                animation::TrackKind::Scale => continue,
            };
            channels.push(glb::Channel {
                node,
                path,
                times: track.times.clone(),
                values: track.values.clone(),
            });
        }
        out_clips.push(glb::Clip {
            name: clip.name.clone(),
            duration: clip.duration as f32,
            channels,
        });
    }
    document.clips = out_clips;
    Ok(document)
}

/// The clip name for an FBX path: its file stem (Mixamo names every stack
/// `mixamo.com`, which is useless in a library).
fn clip_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "clip".to_owned())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let library = args
        .next()
        .ok_or("usage: human_clips <library.glb> <out.glb> <clip.fbx>...")?;
    let out_path = args.next().ok_or("need an output path")?;
    let fbx_paths: Vec<String> = args.collect();
    if fbx_paths.is_empty() {
        return Err("need at least one .fbx".into());
    }

    let target_doc = glb::read(&std::fs::read(&library)?)?;
    let target = rig_of(target_doc.clone(), joints_of(&target_doc))?;
    let known: Vec<KnownRig> = ["mixamo.json", "rigify.json"]
        .iter()
        .map(|file| {
            let path = concat!(env!("CARGO_MANIFEST_DIR"), "/known-rigs/").to_owned() + file;
            serde_json::from_str(&std::fs::read_to_string(&path).expect("reads")).expect("parses")
        })
        .collect();

    let mut out = target.document.clone();
    let before = out.clips.len();

    for fbx in &fbx_paths {
        let source_doc = read_fbx(fbx)?;
        let source = rig_of(source_doc.clone(), joints_of(&source_doc))?;
        let name = clip_name(fbx);

        let (target_to_source, strategy) =
            map_bones_best(&target.skeleton, &source.skeleton, &known, 0.5);
        let source_to_target: HashMap<usize, usize> =
            target_to_source.iter().map(|(&t, &s)| (s, t)).collect();
        let node_to_bone: HashMap<usize, usize> = source
            .nodes
            .iter()
            .enumerate()
            .map(|(bone, &node)| (node, bone))
            .collect();

        for clip in &source.document.clips {
            // Rotation only: the Mixamo root translation is in centimetres while
            // the skeleton is in metres, and an in-place clip is what a library
            // wants anyway (the app retargets it onto a fitted mesh). Keeping the
            // translation flings the root ~270 units off. Same choice as the
            // other shipped libraries.
            let mut rotations = Vec::new();
            for channel in &clip.channels {
                let Some(&bone) = node_to_bone.get(&channel.node) else {
                    continue;
                };
                if channel.path == glb::Path::Rotation {
                    rotations.push(RotationTrack {
                        bone,
                        times: channel.times.clone(),
                        rotations: channel
                            .values
                            .chunks_exact(4)
                            .map(|q| Quat::from_xyzw(q[0], q[1], q[2], q[3]))
                            .collect(),
                    });
                }
            }

            let (moved, report) = retarget(
                &source.skeleton,
                &source.rotations,
                &target.skeleton,
                &target.rotations,
                &source_to_target,
                &Clip {
                    name: name.clone(),
                    tracks: rotations,
                },
            );

            let mut channels = Vec::new();
            for track in &moved.tracks {
                channels.push(glb::Channel {
                    node: target.nodes[track.bone],
                    path: glb::Path::Rotation,
                    times: track.times.clone(),
                    values: track
                        .rotations
                        .iter()
                        .flat_map(|q| [q.x, q.y, q.z, q.w])
                        .collect(),
                });
            }
            println!(
                "  {name}: {} channels via {strategy:?} ({} unmapped, {} malformed)",
                channels.len(),
                report.unmapped_tracks,
                report.malformed_tracks
            );
            out.clips.push(glb::Clip {
                name: name.clone(),
                duration: clip.duration,
                channels,
            });
        }
    }

    std::fs::write(&out_path, glb::write(&out)?)?;
    println!(
        "wrote {out_path}: {} clips ({before} kept + {} added)",
        out.clips.len(),
        out.clips.len() - before
    );
    Ok(())
}
