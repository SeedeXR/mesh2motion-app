//! Re-encode an FBX rig through our own semantic types.
//!
//! Reads an FBX with the full reader stack — DOM, models, skins, geometry,
//! animation — rebuilds a [`build::Scene`] from those layers, and encodes it
//! back to FBX bytes. This is the O9 acceptance operation (`memory/
//! project_context.md`): an existing rig must survive import and export
//! untouched — skeleton, bone names, hierarchy, bind matrices, skin weights and
//! the animation. Unlike a byte-level re-encode it goes all the way through the
//! semantic layers, so anything those layers drop shows up in the output.
//!
//! Shared by the `rebuild_rig` example and the cross-engine bridge tests, so
//! "our encoder's output" is one code path both check.

use crate::fbx::binary::FbxProperty;
use crate::fbx::{binary, build, dom::Scene, encode, model, skin, FbxError};
use std::collections::HashMap;

fn malformed(what: &'static str) -> FbxError {
    FbxError::Malformed {
        what,
        detail: "missing or wrong-typed array".to_string(),
    }
}

/// Reads `bytes` as an FBX rig and re-encodes it through our own types.
///
/// # Errors
///
/// [`FbxError`] if the input does not parse or a required array is missing.
pub fn reencode(bytes: &[u8]) -> Result<Vec<u8>, FbxError> {
    let scene = Scene::from_document(binary::parse(bytes)?);
    let models = model::parse_all(&scene);
    let (skins, _skins_without_geometry) = skin::parse_all(&scene);

    // Bones, parents before children — `build::Scene` requires that ordering so
    // a single forward pass emits connections and a cycle cannot be expressed.
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

    // Meshes, taken RAW rather than through `geometry::parse` (which triangulates
    // and expands per corner — right for a solver, wrong for a writer: O9 says a
    // quad mesh comes back a quad mesh).
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
            .ok_or_else(|| malformed("geometry Vertices"))?;
        let raw_polygons = object
            .node
            .child("PolygonVertexIndex")
            .and_then(|n| n.properties.first())
            .and_then(FbxProperty::as_i64_vec)
            .ok_or_else(|| malformed("geometry PolygonVertexIndex"))?;
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

    // Skins. A cluster whose bone is not in the skeleton is dropped, not guessed.
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
        skin_meshes.push(mesh);
        cluster_store.push(clusters);
    }
    let built_skins: Vec<build::Skin> = skin_meshes
        .iter()
        .zip(&cluster_store)
        .map(|(&mesh, clusters)| build::Skin { mesh, clusters })
        .collect();

    // Animation, passed through RAW — the reader's quaternion tracks are lossy to
    // convert back, so the original Euler curves are carried through exactly as
    // the polygons are.
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
    for stack in scene.objects_of_kind("AnimationStack") {
        let mut channels: Vec<RawChannel> = Vec::new();
        let mut duration = 0i64;
        let mut layer_name = String::from("Layer0");
        for layer in scene.children_of(stack.id, Some("AnimationLayer")) {
            if let Some(object) = scene.object(layer) {
                layer_name = object.name.clone();
            }
            for node_id in scene.children_of(layer, Some("AnimationCurveNode")) {
                let Some(curve_node) = scene.object(node_id) else {
                    continue;
                };
                let Some(target) = scene
                    .links
                    .get(&node_id)
                    .and_then(|l| l.parents.iter().find(|p| p.property.is_some()).cloned())
                else {
                    continue;
                };
                let Some(&bone) = bone_index.get(&target.id) else {
                    continue;
                };
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

    // The source's own frame rate (6 = 30fps); without it Blender reads the
    // tick times at the wrong rate.
    let time_mode = scene.time_mode.unwrap_or(6);

    let document = build::build(&build::Scene {
        meshes: &meshes,
        bones: &bones,
        skins: &built_skins,
        clips: &clips,
        time_mode,
    });
    encode::encode(&document)
}
