//! Reads an FBX rig through our own types and writes it back out.
//!
//! This is the acceptance test for O9 (`memory/project_context.md`): an
//! existing rig must survive import and export untouched — skeleton, bone
//! names, hierarchy, bind matrices and skin weights. Unlike
//! `encode_roundtrip`, which re-encodes the document the reader produced, this
//! goes all the way through the semantic layers and rebuilds from them, so
//! anything those layers drop is visible in the output.
//!
//!     cargo run -p m2m-io --release --example rebuild_rig -- <in.fbx> <out.fbx>

use m2m_io::fbx::binary::FbxProperty;
use m2m_io::fbx::{binary, build, dom::Scene, encode, model, skin};
use std::collections::HashMap;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let (Some(input), Some(output)) = (args.next(), args.next()) else {
        eprintln!("usage: rebuild_rig <in.fbx> <out.fbx>");
        std::process::exit(2);
    };

    let bytes = std::fs::read(&input)?;
    let scene = Scene::from_document(binary::parse(&bytes)?);
    let models = model::parse_all(&scene);
    let (skins, skins_without_geometry) = skin::parse_all(&scene);

    // Bones, parents before children. `build::Scene` requires that ordering so
    // a single forward pass can emit connections — and so a cycle cannot be
    // expressed at all.
    let mut order: Vec<i64> = Vec::new();
    let mut queue: Vec<i64> = models.roots.clone();
    while let Some(id) = queue.pop() {
        let Some(node) = models.get(id) else { continue };
        if node.is_bone() {
            order.push(id);
        }
        queue.extend(node.children.iter().rev().copied());
    }
    let bone_index: HashMap<i64, usize> =
        order.iter().enumerate().map(|(i, &id)| (id, i)).collect();

    let bones: Vec<build::Bone> = order
        .iter()
        .map(|&id| {
            let m = models.get(id).expect("in order");
            build::Bone {
                name: &m.name,
                parent: m.parent.and_then(|p| bone_index.get(&p).copied()),
                translation: m.transform.translation.to_array(),
                rotation: m.transform.rotation.to_array(),
                scale: m.transform.scale.to_array(),
                pre_rotation: m.transform.pre_rotation.to_array(),
            }
        })
        .collect();

    // Meshes, taken RAW rather than through `geometry::parse`.
    //
    // That layer triangulates and expands per corner, which is right for a
    // solver and wrong for a writer: rebuilding from the triangulated form
    // turns the reference rig's 11,120 and 14,222 polygons into 20,840 and
    // 28,272, splitting every quad. O9 says a model comes back as it went in,
    // and an artist notices a quad mesh returned as triangles.
    let mut mesh_positions: Vec<Vec<f32>> = Vec::new();
    let mut mesh_polygons: Vec<Vec<i32>> = Vec::new();
    let mut mesh_names: Vec<String> = Vec::new();
    let mut geometry_index: HashMap<i64, usize> = HashMap::new();

    for object in scene.objects_of_kind("Geometry") {
        let raw_vertices = object
            .node
            .child("Vertices")
            .and_then(|n| n.properties.first())
            .and_then(FbxProperty::as_f64_vec)
            .ok_or("geometry has no Vertices array")?;
        let raw_polygons = object
            .node
            .child("PolygonVertexIndex")
            .and_then(|n| n.properties.first())
            .and_then(FbxProperty::as_i64_vec)
            .ok_or("geometry has no PolygonVertexIndex array")?;
        geometry_index.insert(object.id, mesh_names.len());
        mesh_names.push(object.name.clone());
        mesh_positions.push(raw_vertices.iter().map(|&v| v as f32).collect());
        mesh_polygons.push(raw_polygons.iter().map(|&v| v as i32).collect());
    }

    let meshes: Vec<build::Mesh> = (0..mesh_names.len())
        .map(|i| build::Mesh {
            name: &mesh_names[i],
            positions: &mesh_positions[i],
            faces: build::Faces::Polygons(&mesh_polygons[i]),
        })
        .collect();

    // Skins. A cluster whose bone is not in the skeleton is dropped rather
    // than guessed at, and counted so the drop is never silent.
    let mut dropped_clusters = 0usize;
    let mut cluster_store: Vec<Vec<build::Cluster>> = Vec::new();
    let mut skin_meshes: Vec<usize> = Vec::new();
    for s in &skins {
        let Some(&mesh) = geometry_index.get(&s.geometry_id) else {
            continue;
        };
        let clusters: Vec<build::Cluster> = s
            .clusters
            .iter()
            .filter_map(|c| {
                let bone = bone_index.get(&c.bone_id).copied()?;
                Some(build::Cluster {
                    bone,
                    indices: &c.indices,
                    weights: &c.weights,
                    transform: c.transform.to_cols_array(),
                    transform_link: c.transform_link.to_cols_array(),
                })
            })
            .collect();
        dropped_clusters += s.clusters.len() - clusters.len();
        skin_meshes.push(mesh);
        cluster_store.push(clusters);
    }
    let built_skins: Vec<build::Skin> = skin_meshes
        .iter()
        .zip(&cluster_store)
        .map(|(&mesh, clusters)| build::Skin { mesh, clusters })
        .collect();

    // Animation, passed through RAW.
    //
    // `animation::parse_all` gives quaternion tracks with times in seconds,
    // which is what a player and a retargeter want. FBX stores Euler curves in
    // ticks, and converting back is lossy and ambiguous — many Euler triples
    // give one quaternion, and the reader also merges the three axes onto a
    // single time array and can insert sub-keys. O9 says the animation comes
    // back as it went in, so the original curves are carried through, exactly
    // as the polygons are.
    struct RawCurve {
        axis: String,
        times: Vec<i64>,
        values: Vec<f32>,
        default: f64,
    }
    struct RawChannel {
        bone: usize,
        property: String,
        kind: String,
        curves: Vec<RawCurve>,
    }
    struct RawClip {
        name: String,
        duration: i64,
        layer: String,
        channels: Vec<RawChannel>,
    }

    let mut raw_clips: Vec<RawClip> = Vec::new();
    let mut channels_without_bone = 0usize;
    for stack in scene.objects_of_kind("AnimationStack") {
        let mut channels: Vec<RawChannel> = Vec::new();
        let mut duration = 0i64;
        // One layer per stack, which is what Mixamo exports and what the
        // builder writes. A stack with several layers is flattened into one
        // here, keeping the last layer's name — enough for the round trip,
        // not enough for a file that actually blends layers.
        let mut layer_name = String::from("Layer0");
        for layer in scene.children_of(stack.id, Some("AnimationLayer")) {
            if let Some(object) = scene.object(layer) {
                layer_name = object.name.clone();
            }
            for node_id in scene.children_of(layer, Some("AnimationCurveNode")) {
                let Some(curve_node) = scene.object(node_id) else {
                    continue;
                };
                // The property connection names both the bone and the property.
                let Some(target) = scene
                    .links
                    .get(&node_id)
                    .and_then(|l| l.parents.iter().find(|p| p.property.is_some()).cloned())
                else {
                    continue;
                };
                let Some(&bone) = bone_index.get(&target.id) else {
                    channels_without_bone += 1;
                    continue;
                };
                // Written back with a space, as FBX stores it — the DOM
                // normalises `Lcl Rotation` to `Lcl_Rotation` for lookup.
                let property = target
                    .property
                    .clone()
                    .unwrap_or_default()
                    .replace('_', " ");

                let mut curves: Vec<RawCurve> = Vec::new();
                for curve_id in scene.children_of(node_id, Some("AnimationCurve")) {
                    let Some(curve) = scene.object(curve_id) else {
                        continue;
                    };
                    let Some(axis) = scene
                        .links
                        .get(&curve_id)
                        .and_then(|l| l.parents.first())
                        .and_then(|p| p.property.clone())
                    else {
                        continue;
                    };
                    let times = curve
                        .node
                        .child("KeyTime")
                        .and_then(|n| n.properties.first())
                        .and_then(FbxProperty::as_i64_vec)
                        .unwrap_or_default();
                    let values: Vec<f32> = curve
                        .node
                        .child("KeyValueFloat")
                        .and_then(|n| n.properties.first())
                        .and_then(FbxProperty::as_f64_vec)
                        .unwrap_or_default()
                        .iter()
                        .map(|&v| v as f32)
                        .collect();
                    let default = curve
                        .node
                        .child("Default")
                        .and_then(|n| n.properties.first())
                        .and_then(FbxProperty::as_f64)
                        .unwrap_or(0.0);
                    duration = duration.max(times.last().copied().unwrap_or(0));
                    curves.push(RawCurve {
                        axis,
                        times,
                        values,
                        default,
                    });
                }
                if curves.is_empty() {
                    continue;
                }
                channels.push(RawChannel {
                    bone,
                    property,
                    kind: curve_node.name.clone(),
                    curves,
                });
            }
        }
        if channels.is_empty() {
            continue;
        }
        raw_clips.push(RawClip {
            name: stack.name.clone(),
            duration,
            layer: layer_name,
            channels,
        });
    }

    let curve_views: Vec<Vec<Vec<build::Curve>>> = raw_clips
        .iter()
        .map(|clip| {
            clip.channels
                .iter()
                .map(|channel| {
                    channel
                        .curves
                        .iter()
                        .map(|c| build::Curve {
                            axis: &c.axis,
                            times: &c.times,
                            values: &c.values,
                            default: c.default,
                        })
                        .collect()
                })
                .collect()
        })
        .collect();
    let channel_views: Vec<Vec<build::Channel>> = raw_clips
        .iter()
        .zip(&curve_views)
        .map(|(clip, curves)| {
            clip.channels
                .iter()
                .zip(curves)
                .map(|(channel, curves)| build::Channel {
                    bone: channel.bone,
                    property: &channel.property,
                    kind: &channel.kind,
                    curves,
                })
                .collect()
        })
        .collect();
    let clips: Vec<build::Clip> = raw_clips
        .iter()
        .zip(&channel_views)
        .map(|(clip, channels)| build::Clip {
            name: &clip.name,
            duration: clip.duration,
            layer: &clip.layer,
            channels,
        })
        .collect();

    // The source's own frame rate. 6 is 30fps; without it Blender reads this
    // rig's 148-frame clip as 123.5, the same keys played 20% slow.
    let time_mode = scene.time_mode.unwrap_or(6);

    let document = build::build(&build::Scene {
        meshes: &meshes,
        bones: &bones,
        skins: &built_skins,
        clips: &clips,
        time_mode,
    });
    let encoded = encode::encode(&document)?;
    std::fs::write(&output, &encoded)?;

    println!(
        "{} bones, {} meshes, {} skins, {} clusters ({} dropped, {} skins without geometry), \
         {} clips / {} channels / {} curves ({} channels without a bone), {} bytes",
        bones.len(),
        meshes.len(),
        built_skins.len(),
        cluster_store.iter().map(Vec::len).sum::<usize>(),
        dropped_clusters,
        skins_without_geometry,
        clips.len(),
        clips.iter().map(|c| c.channels.len()).sum::<usize>(),
        clips
            .iter()
            .flat_map(|c| c.channels.iter())
            .map(|c| c.curves.len())
            .sum::<usize>(),
        channels_without_bone,
        encoded.len()
    );
    Ok(())
}
