//! Grafting a skeleton and skin onto an already-authored glTF **without
//! rebuilding it**, so the mesh keeps its materials, textures, UVs and normals.
//!
//! [`super::write`] builds a fresh file from the parts the rig pipeline models,
//! and so necessarily drops shading — the reader never read it, the writer
//! cannot invent it. Export does not need a fresh file: the user's mesh is
//! already a valid glTF, and all it lacks is the rig. So this takes the original
//! bytes, ADDS joint/weight attributes to its primitives, a skin, a skeleton of
//! nodes and (optionally) one animation, and leaves everything else — every
//! material, texture, image, UV and normal — byte-for-byte untouched.
//!
//! The JSON is edited as a [`serde_json::Value`] rather than through the typed
//! `gltf_json` structs: the edits are all "append to an array" or "set a field",
//! which are one line each on a `Value` and a paragraph each on the typed API,
//! and the reader already round-trips glTF this way (`without_ignorable_extensions`).

use serde_json::{json, Value};

use super::{GlbError, Path};

/// One bone to graft: its name, its parent within the bone list, its local rest
/// transform, and its inverse bind matrix. The pipeline composes these exactly
/// as it does for the rebuilt export, so the two exports bind identically.
pub struct GraftBone {
    /// Bone name, written onto the node.
    pub name: String,
    /// Parent bone index, or `None` for a root.
    pub parent: Option<usize>,
    /// Local translation, in the parent's rotated frame.
    pub translation: [f32; 3],
    /// Local rest rotation, xyzw.
    pub rotation: [f32; 4],
    /// Column-major inverse bind matrix.
    pub inverse_bind: [f32; 16],
}

/// The skin for one primitive: four joint indices and four weights per vertex,
/// in the primitive's own vertex order. Indices are into the bone list.
pub struct GraftPrimitiveSkin {
    /// Four joint indices per vertex.
    pub joints: Vec<u16>,
    /// Four weights per vertex.
    pub weights: Vec<f32>,
}

/// One animation channel to graft: which bone, which property, and the keys.
pub struct GraftChannel {
    /// Bone index the channel drives.
    pub bone: usize,
    /// Which property — rotation or translation (scale is not produced).
    pub path: Path,
    /// Key times, seconds.
    pub times: Vec<f32>,
    /// Key values, flattened (xyzw per key for rotation, xyz for translation).
    pub values: Vec<f32>,
}

/// One animation to graft.
pub struct GraftAnimation {
    /// The clip's name.
    pub name: String,
    /// Its channels.
    pub channels: Vec<GraftChannel>,
}

// glTF accessor component types and the primitive mode we skin.
const F32: u64 = 5126;
const U16: u64 = 5123;
const TRIANGLES: u64 = 4;

/// Grafts `bones`, a skin and an optional `animation` onto `model`.
///
/// `per_primitive` gives the skin for each **triangle** primitive of the model,
/// in the order the reader visits them (meshes in index order, primitives in
/// index order) — the same order the bind's merged mesh was built in, so the
/// weights line up. A mismatch between the vertex counts here and in the file is
/// an error, never a silent misalignment.
///
/// # Errors
///
/// [`GlbError`] when the model is not a glTF this can parse, or the per-primitive
/// vertex counts do not match the file's.
pub fn graft_skin(
    model: &[u8],
    bones: &[GraftBone],
    per_primitive: &[GraftPrimitiveSkin],
    animation: Option<&GraftAnimation>,
) -> Result<Vec<u8>, GlbError> {
    let (mut root, mut bin) = split(model)?;
    let obj = root
        .as_object_mut()
        .ok_or_else(|| GlbError::Serialize("glTF root is not an object".into()))?;

    ensure_array(obj, "bufferViews");
    ensure_array(obj, "accessors");
    ensure_array(obj, "nodes");
    ensure_array(obj, "skins");

    // Attach JOINTS_0/WEIGHTS_0 to each triangle primitive, in read order.
    let mut primitive_paths: Vec<(usize, usize)> = Vec::new();
    for (mesh_index, mesh) in obj
        .get("meshes")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        for (prim_index, primitive) in mesh
            .get("primitives")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .enumerate()
        {
            let mode = primitive
                .get("mode")
                .and_then(Value::as_u64)
                .unwrap_or(TRIANGLES);
            if mode == TRIANGLES {
                primitive_paths.push((mesh_index, prim_index));
            }
        }
    }
    if primitive_paths.len() != per_primitive.len() {
        return Err(GlbError::Serialize(format!(
            "grafting {} skins onto {} triangle primitives",
            per_primitive.len(),
            primitive_paths.len()
        )));
    }

    for (&(mesh_index, prim_index), skin) in primitive_paths.iter().zip(per_primitive) {
        let vertices = expected_vertices(obj, mesh_index, prim_index)?;
        if skin.weights.len() != vertices * 4 || skin.joints.len() != vertices * 4 {
            return Err(GlbError::Serialize(format!(
                "primitive {mesh_index}/{prim_index} has {vertices} vertices but the skin has {}",
                skin.weights.len() / 4
            )));
        }
        let joints = push_accessor(
            obj,
            &mut bin,
            &u16_bytes(&skin.joints),
            vertices,
            U16,
            "VEC4",
        );
        let weights = push_accessor(
            obj,
            &mut bin,
            &f32_bytes(&skin.weights),
            vertices,
            F32,
            "VEC4",
        );
        let primitive = &mut obj["meshes"][mesh_index]["primitives"][prim_index];
        let attributes = primitive
            .get_mut("attributes")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| GlbError::Serialize("primitive has no attributes".into()))?;
        attributes.insert("JOINTS_0".into(), json!(joints));
        attributes.insert("WEIGHTS_0".into(), json!(weights));
    }

    // The inverse bind matrices, one accessor for all bones.
    let ibm: Vec<f32> = bones.iter().flat_map(|b| b.inverse_bind).collect();
    let ibm_accessor = push_accessor(obj, &mut bin, &f32_bytes(&ibm), bones.len(), F32, "MAT4");

    // A node per bone. glTF stores children, so parents are inverted here.
    let node_base = obj["nodes"].as_array().map_or(0, Vec::len);
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); bones.len()];
    for (bone, b) in bones.iter().enumerate() {
        if let Some(parent) = b.parent {
            if let Some(slot) = children.get_mut(parent) {
                slot.push(node_base + bone);
            }
        }
    }
    for (bone, b) in bones.iter().enumerate() {
        let mut node = json!({
            "name": b.name,
            "translation": b.translation,
            "rotation": b.rotation,
        });
        if !children[bone].is_empty() {
            node["children"] = json!(children[bone]);
        }
        obj["nodes"].as_array_mut().unwrap().push(node);
    }

    // The skin, and the scene roots for its bones.
    let joints: Vec<usize> = (0..bones.len()).map(|b| node_base + b).collect();
    let root_bone = bones.iter().position(|b| b.parent.is_none()).unwrap_or(0);
    obj["skins"].as_array_mut().unwrap().push(json!({
        "joints": joints,
        "inverseBindMatrices": ibm_accessor,
        "skeleton": node_base + root_bone,
    }));
    let skin_index = obj["skins"].as_array().unwrap().len() - 1;

    // Every node that draws a mesh is now skinned by that skin.
    if let Some(nodes) = obj["nodes"].as_array_mut() {
        for node in nodes.iter_mut() {
            if node.get("mesh").is_some() {
                node.as_object_mut()
                    .unwrap()
                    .insert("skin".into(), json!(skin_index));
            }
        }
    }

    // The skeleton's roots join the scene so the armature is actually present.
    add_scene_roots(obj, bones, node_base);

    if let Some(animation) = animation {
        graft_animation(obj, &mut bin, animation, node_base)?;
    }

    // The buffer now runs to the end of everything appended.
    set_buffer_length(obj, bin.len());

    Ok(assemble_glb(
        &serde_json::to_vec(&root).map_err(|e| GlbError::Serialize(e.to_string()))?,
        &bin,
    ))
}

/// Splits a `.glb` into its JSON (as a mutable `Value`) and a copy of its binary
/// chunk's data. A plain `.gltf` has no binary chunk and starts empty.
fn split(bytes: &[u8]) -> Result<(Value, Vec<u8>), GlbError> {
    if !bytes.starts_with(b"glTF") {
        let root = serde_json::from_slice(bytes).map_err(|e| GlbError::Serialize(e.to_string()))?;
        return Ok((root, Vec::new()));
    }
    let json_len = u32::from_le_bytes(
        bytes
            .get(12..16)
            .ok_or(GlbError::UnreadableJson)?
            .try_into()
            .unwrap(),
    ) as usize;
    let json = bytes
        .get(20..20 + json_len)
        .ok_or(GlbError::UnreadableJson)?;
    let root = serde_json::from_slice(json).map_err(|e| GlbError::Serialize(e.to_string()))?;
    // The binary chunk begins after the JSON chunk: an 8-byte header then data.
    let bin = match bytes.get(20 + json_len..) {
        Some(chunk) if chunk.len() >= 8 => {
            let len = u32::from_le_bytes(chunk[0..4].try_into().unwrap()) as usize;
            chunk.get(8..8 + len).unwrap_or(&[]).to_vec()
        }
        _ => Vec::new(),
    };
    Ok((root, bin))
}

/// Assembles a `.glb` from a JSON chunk and binary data, padding both chunks to
/// four bytes as glTF requires (JSON with spaces, BIN with zeros).
fn assemble_glb(json: &[u8], bin: &[u8]) -> Vec<u8> {
    let mut json = json.to_vec();
    while json.len() % 4 != 0 {
        json.push(b' ');
    }
    let mut bin = bin.to_vec();
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let has_bin = !bin.is_empty();
    let total = 12 + 8 + json.len() + if has_bin { 8 + bin.len() } else { 0 };
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"glTF");
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes()); // "JSON"
    out.extend_from_slice(&json);
    if has_bin {
        out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
        out.extend_from_slice(&0x004E_4942u32.to_le_bytes()); // "BIN\0"
        out.extend_from_slice(&bin);
    }
    out
}

/// Appends `data` to the binary buffer as a new buffer view, then an accessor
/// over the whole view, and returns the accessor's index. Pads the buffer to
/// four bytes first, so every accessor offset is component-aligned.
fn push_accessor(
    obj: &mut serde_json::Map<String, Value>,
    bin: &mut Vec<u8>,
    data: &[u8],
    count: usize,
    component_type: u64,
    kind: &str,
) -> usize {
    while bin.len() % 4 != 0 {
        bin.push(0);
    }
    let offset = bin.len();
    bin.extend_from_slice(data);
    let views = obj["bufferViews"].as_array_mut().unwrap();
    let view = views.len();
    views.push(json!({ "buffer": 0, "byteOffset": offset, "byteLength": data.len() }));
    let accessors = obj["accessors"].as_array_mut().unwrap();
    let index = accessors.len();
    accessors.push(json!({
        "bufferView": view,
        "byteOffset": 0,
        "componentType": component_type,
        "count": count,
        "type": kind,
    }));
    index
}

/// The vertex count declared by a primitive's POSITION accessor.
fn expected_vertices(
    obj: &serde_json::Map<String, Value>,
    mesh: usize,
    prim: usize,
) -> Result<usize, GlbError> {
    let position = obj
        .get("meshes")
        .and_then(|m| m.get(mesh))
        .and_then(|m| m.get("primitives"))
        .and_then(|p| p.get(prim))
        .and_then(|p| p.get("attributes"))
        .and_then(|a| a.get("POSITION"))
        .and_then(Value::as_u64)
        .ok_or_else(|| GlbError::Serialize("primitive has no POSITION".into()))?;
    obj["accessors"][position as usize]
        .get("count")
        .and_then(Value::as_u64)
        .map(|c| c as usize)
        .ok_or_else(|| GlbError::Serialize("POSITION accessor has no count".into()))
}

/// Adds each root bone's node to the file's scene, so the armature is present.
fn add_scene_roots(
    obj: &mut serde_json::Map<String, Value>,
    bones: &[GraftBone],
    node_base: usize,
) {
    let roots: Vec<usize> = bones
        .iter()
        .enumerate()
        .filter(|(_, b)| b.parent.is_none())
        .map(|(bone, _)| node_base + bone)
        .collect();
    let scene = obj.get("scene").and_then(Value::as_u64).unwrap_or(0) as usize;
    if let Some(nodes) = obj
        .get_mut("scenes")
        .and_then(|s| s.get_mut(scene))
        .and_then(|s| s.get_mut("nodes"))
        .and_then(Value::as_array_mut)
    {
        for root in roots {
            nodes.push(json!(root));
        }
    }
}

/// Grafts one animation: an accessor pair per channel (times, values) and a
/// sampler + channel that target the grafted bone nodes.
fn graft_animation(
    obj: &mut serde_json::Map<String, Value>,
    bin: &mut Vec<u8>,
    animation: &GraftAnimation,
    node_base: usize,
) -> Result<(), GlbError> {
    ensure_array(obj, "animations");
    let mut samplers = Vec::new();
    let mut channels = Vec::new();
    for channel in &animation.channels {
        let stride = match channel.path {
            Path::Rotation => 4,
            Path::Translation | Path::Scale => 3,
            Path::MorphWeights => continue,
        };
        if channel.times.is_empty() || channel.values.len() != channel.times.len() * stride {
            continue;
        }
        let (lo, hi) = min_max(&channel.times);
        let input = push_accessor(
            obj,
            bin,
            &f32_bytes(&channel.times),
            channel.times.len(),
            F32,
            "SCALAR",
        );
        // The time accessor must carry min/max for a sampler input.
        obj["accessors"][input]["min"] = json!([lo]);
        obj["accessors"][input]["max"] = json!([hi]);
        let kind = if stride == 4 { "VEC4" } else { "VEC3" };
        let output = push_accessor(
            obj,
            bin,
            &f32_bytes(&channel.values),
            channel.times.len(),
            F32,
            kind,
        );
        let sampler = samplers.len();
        samplers.push(json!({ "input": input, "output": output, "interpolation": "LINEAR" }));
        channels.push(json!({
            "sampler": sampler,
            "target": { "node": node_base + channel.bone, "path": path_name(channel.path) },
        }));
    }
    if !channels.is_empty() {
        obj["animations"].as_array_mut().unwrap().push(json!({
            "name": animation.name,
            "samplers": samplers,
            "channels": channels,
        }));
    }
    Ok(())
}

fn path_name(path: Path) -> &'static str {
    match path {
        Path::Rotation => "rotation",
        Path::Translation => "translation",
        Path::Scale => "scale",
        Path::MorphWeights => "weights",
    }
}

fn ensure_array(obj: &mut serde_json::Map<String, Value>, key: &str) {
    if !obj.get(key).is_some_and(Value::is_array) {
        obj.insert(key.into(), json!([]));
    }
}

fn set_buffer_length(obj: &mut serde_json::Map<String, Value>, len: usize) {
    match obj.get_mut("buffers").and_then(Value::as_array_mut) {
        Some(buffers) if !buffers.is_empty() => {
            buffers[0]["byteLength"] = json!(len);
        }
        _ => {
            obj.insert("buffers".into(), json!([{ "byteLength": len }]));
        }
    }
}

fn min_max(values: &[f32]) -> (f32, f32) {
    values
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &v| {
            (lo.min(v), hi.max(v))
        })
}

fn f32_bytes(values: &[f32]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}

fn u16_bytes(values: &[u16]) -> Vec<u8> {
    values.iter().flat_map(|v| v.to_le_bytes()).collect()
}
