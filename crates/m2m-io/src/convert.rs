//! FBX into the glTF document model.
//!
//! # Why this direction, and why glTF is the wire format
//!
//! `memory/architecture.md` §4 says bulk data crosses the IPC boundary as raw
//! bytes with a small JSON header, never as JSON numbers. That description is
//! glTF: a JSON header and a binary chunk. So rather than invent a private wire
//! format and a hand-written decoder on the other side, the app sends a `.glb`
//! and the frontend hands it to a loader that already exists.
//!
//! Which leaves one gap, filled here: an FBX has to reach [`glb::Document`]
//! before it can be written as one.
//!
//! # What this carries, and what it does not
//!
//! Carried: the node hierarchy with local transforms, triangulated meshes,
//! skins with their joint lists and inverse bind matrices, and the unit scale.
//!
//! Not carried yet: **animation**, normals and UVs. Animation is the real
//! omission — [`crate::fbx::animation`] already reads it and
//! [`glb::Clip`] already holds it, so it is work rather than a question. It is
//! left out because the viewport needs geometry before it needs playback, and
//! shipping half a converter with the gap named beats shipping a whole one
//! guessed at. Normals and UVs have nowhere to go: [`glb::Primitive`] has no
//! field for them.

use crate::fbx::dom::Scene;
use crate::fbx::geometry::{self, GeometricTransform};
use crate::fbx::{model, skin, FbxError};
use crate::glb;
use std::collections::HashMap;

/// Centimetres per metre. FBX measures in the unit `UnitScaleFactor` declares,
/// counted in centimetres; glTF is defined in metres.
const CM_PER_M: f64 = 100.0;

/// Converts a parsed FBX scene into a glTF document.
///
/// # Errors
///
/// Propagates the geometry and skin readers' errors, and reports a cluster
/// whose bone is not a model in this scene as [`FbxError::Malformed`] — see the
/// note at that call site on why such a cluster cannot simply be dropped.
pub fn fbx_to_gltf(scene: &Scene) -> Result<glb::Document, FbxError> {
    let models = model::parse_all(scene);
    let (skins, _) = skin::parse_all(scene);

    // Parents before children, which is the order glTF node indices must be
    // resolvable in and the order `Node::parent` is written against.
    let mut order: Vec<i64> = Vec::new();
    let mut queue: Vec<i64> = models.roots.clone();
    while let Some(id) = queue.pop() {
        let Some(node) = models.get(id) else { continue };
        order.push(id);
        queue.extend(node.children.iter().rev().copied());
    }
    let index: HashMap<i64, usize> = order.iter().enumerate().map(|(i, &id)| (id, i)).collect();

    // The whole scene is scaled by putting the factor on the ROOT nodes, so
    // everything below inherits it and nothing else in this function has to
    // think about units. Inverse bind matrices are deliberately left unscaled:
    // a joint's world becomes `S · L` and its matrix stays `L⁻¹ · M`, so the
    // product is `S · M` — the bind pose, scaled, which is the intent. Scaling
    // both would cancel out and leave the mesh at its original size.
    let scale = scene.unit_scale.unwrap_or(1.0) / CM_PER_M;

    let mut nodes: Vec<glb::Node> = Vec::with_capacity(order.len());
    for &id in &order {
        let source = models.get(id).expect("built from the same order");
        let (mut node_scale, rotation, mut translation) =
            source.local.to_scale_rotation_translation();
        if source.parent.is_none() {
            node_scale *= scale;
            translation *= scale;
        }
        nodes.push(glb::Node {
            name: source.name.clone(),
            parent: source.parent.and_then(|p| index.get(&p).copied()),
            transform: glb::Trs {
                translation: translation.as_vec3().to_array(),
                rotation: rotation.as_quat().to_array(),
                scale: node_scale.as_vec3().to_array(),
            },
            skin: None,
        });
    }

    let mut primitives: Vec<glb::Primitive> = Vec::new();
    let mut out_skins: Vec<glb::Skin> = Vec::new();

    for (node_index, &id) in order.iter().enumerate() {
        for geometry_id in scene.children_of(id, Some("Geometry")) {
            let Some(object) = scene.object(geometry_id) else {
                continue;
            };
            let mesh =
                geometry::parse(object, GeometricTransform::for_geometry(scene, geometry_id))?;

            let mut joints = Vec::new();
            let mut weights = Vec::new();
            if let Some(source) = skins.iter().find(|s| s.geometry_id == geometry_id) {
                let (bound, _) = source.bind(&mesh)?;

                // `bind` numbers bones by their position in `clusters`, so the
                // joint list has to be every cluster in that order. A missing
                // bone cannot be dropped the way a writer drops it: dropping
                // one shifts every joint index after it, and the mesh would
                // deform to the wrong bones rather than to none.
                let mut joint_nodes = Vec::with_capacity(source.clusters.len());
                let mut inverse_bind_matrices = Vec::with_capacity(source.clusters.len());
                for cluster in &source.clusters {
                    let Some(&joint) = index.get(&cluster.bone_id) else {
                        return Err(FbxError::Malformed {
                            what: "skin cluster",
                            detail: format!(
                                "bone {} is not a model in this scene",
                                cluster.bone_id
                            ),
                        });
                    };
                    joint_nodes.push(joint);
                    let ibm = cluster.transform_link.inverse() * cluster.transform;
                    inverse_bind_matrices.push(ibm.as_mat4().to_cols_array());
                }

                joints = bound.indices;
                weights = bound.weights;
                nodes[node_index].skin = Some(out_skins.len());
                out_skins.push(glb::Skin {
                    joints: joint_nodes,
                    inverse_bind_matrices,
                });
            }

            // Weld the per-corner expansion back to one vertex per FBX source.
            // `geometry::parse` expands every polygon corner into its own
            // vertex so that per-corner normals and UVs have somewhere to live;
            // we carry neither, so all corners of a source vertex are identical
            // in what we keep — position and skin weights — and merging them is
            // lossless. On the reference rig this is 62,520 corners down to
            // 10,514 vertices, which is what crosses the bulk channel and sits
            // in the GPU.
            let welded = weld_by_source(&mesh, &joints, &weights);
            primitives.push(glb::Primitive {
                mesh: primitives.len(),
                node: Some(node_index),
                positions: welded.positions,
                // The FBX reader does not yet carry shading through, so an
                // FBX-sourced model still exports without it (see the export docs).
                normals: Vec::new(),
                uvs: Vec::new(),
                material: None,
                indices: welded.indices,
                joints: welded.joints,
                weights: welded.weights,
                colors: Vec::new(),
            });
        }
    }

    Ok(glb::Document {
        nodes,
        primitives,
        materials: Vec::new(),
        skins: out_skins,
        clips: Vec::new(),
        report: glb::GlbReport::default(),
    })
}

/// A mesh collapsed from per-corner vertices to one per FBX source vertex.
struct Welded {
    positions: Vec<f32>,
    indices: Vec<u32>,
    joints: Vec<u16>,
    weights: Vec<f32>,
}

/// Collapses the per-corner expansion, keyed on the FBX source vertex.
///
/// Every corner of a source vertex shares its position (the expansion only
/// duplicated per-corner normals and UVs, which are not carried) and its skin
/// weights (`Skin::bind` fills every corner of a source from the same
/// `per_source` entry), so keeping the first corner of each source and
/// remapping the triangle indices is exact.
///
/// `joints` and `weights` are `MAX_INFLUENCES` per corner and may be empty for
/// an unskinned mesh, in which case they stay empty.
fn weld_by_source(mesh: &geometry::MeshGeometry, joints: &[u16], weights: &[f32]) -> Welded {
    use m2m_core::skinning::MAX_INFLUENCES;

    // Source vertex id -> new compact index, in first-seen order so the result
    // is deterministic.
    let mut new_index = vec![u32::MAX; mesh.source_vertex_count];
    let mut positions = Vec::new();
    let mut out_joints = Vec::new();
    let mut out_weights = Vec::new();
    let skinned = !joints.is_empty();

    for corner in 0..mesh.vertex_source.len() {
        let source = mesh.vertex_source[corner] as usize;
        if new_index[source] != u32::MAX {
            continue;
        }
        new_index[source] = positions.len() as u32 / 3;
        positions.extend_from_slice(&mesh.positions[corner * 3..corner * 3 + 3]);
        if skinned {
            let base = corner * MAX_INFLUENCES;
            out_joints.extend_from_slice(&joints[base..base + MAX_INFLUENCES]);
            out_weights.extend_from_slice(&weights[base..base + MAX_INFLUENCES]);
        }
    }

    let indices = mesh
        .indices
        .iter()
        .map(|&corner| new_index[mesh.vertex_source[corner as usize] as usize])
        .collect();

    Welded {
        positions,
        indices,
        joints: out_joints,
        weights: out_weights,
    }
}
