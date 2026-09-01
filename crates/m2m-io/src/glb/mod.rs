//! Reading glTF 2.0, in its self-contained binary form (`.glb`).
//!
//! # Why only self-contained files
//!
//! A `.gltf` file may point its buffers and images at arbitrary URIs — relative
//! file paths, absolute paths, `http://`, `data:`. Honouring those turns
//! "open this model" into "let this file choose what to read off the disk and
//! the network". This reader resolves **only** the BIN chunk embedded in the
//! `.glb` container, and reports any external reference as
//! [`GlbError::ExternalBuffer`] rather than fetching it.
//!
//! That is not a limitation for this app: the templates in `static/rigs` and
//! everything the exporter writes are self-contained `.glb`.
//!
//! # Trust boundary
//!
//! Same rule as the FBX readers: malformed input returns an error, never a
//! panic, hang, or OOM. See [`crate::fbx`].

mod write;
pub use write::write;

use std::collections::HashMap;

/// What went wrong reading a `.glb`.
#[derive(Debug, thiserror::Error)]
pub enum GlbError {
    /// The container or its JSON did not parse, or failed glTF validation.
    #[error("not a valid glb: {0}")]
    Invalid(#[from] gltf::Error),
    /// The file has no BIN chunk, so its accessors resolve to nothing.
    #[error("glb has no binary chunk")]
    NoBinaryChunk,
    /// A buffer points somewhere outside the file. Deliberately not followed.
    #[error("buffer {index} refers to external data, which is not read")]
    ExternalBuffer {
        /// Which buffer.
        index: usize,
    },
    /// The assembled document could not be serialized to JSON.
    ///
    /// In practice a count that does not fit where glTF puts it; this writer
    /// builds the structure itself, so a malformed one is a bug here rather
    /// than something a caller can provoke with data.
    #[error("could not serialize glb json: {0}")]
    Serialize(String),
    /// The GLB container header is malformed.
    ///
    /// Checked here rather than left to the `gltf` crate because that crate
    /// computes `header.length - 12` on a length read straight from the file
    /// (`gltf-1.4.1/src/binary.rs:252`). A declared length below 12 underflows:
    /// harmless in release, a panic in any debug build — and `cargo test` is a
    /// debug build. Found by fuzzing.
    #[error("malformed glb header: {reason}")]
    MalformedHeader {
        /// What was wrong with it.
        reason: &'static str,
    },
    /// An index in the JSON points past the end of the array it indexes.
    ///
    /// Checked here because the `gltf` crate's own validation dereferences some
    /// of these before it validates them — `root.accessors[index]` in
    /// `gltf-json-1.4.1/src/mesh.rs:151`, for one — so an out-of-range index
    /// panics the validator instead of being reported by it. That is a panic on
    /// file content in release, not just debug. Found by fuzzing.
    #[error("{owner} index {index} is out of range: only {len} {target} exist")]
    IndexOutOfRange {
        /// What held the index.
        owner: &'static str,
        /// What it indexes.
        target: &'static str,
        /// The offending value.
        index: usize,
        /// How many exist.
        len: usize,
    },
    /// A mesh primitive without positions cannot be drawn or skinned.
    #[error("mesh primitive {primitive} of mesh {mesh} has no POSITION attribute")]
    NoPositions {
        /// Which mesh.
        mesh: usize,
        /// Which primitive within it.
        primitive: usize,
    },
}

/// One drawable primitive, triangulated.
#[derive(Debug, Clone, PartialEq)]
pub struct Primitive {
    /// Which glTF mesh this belongs to.
    ///
    /// A glTF mesh holds one primitive per material, and importers usually
    /// merge them into a single object — Blender does. So the number of
    /// primitives is not the number of meshes, and comparing the two is a
    /// mistake: `human-jay.glb` is one mesh of 22 primitives.
    pub mesh: usize,
    /// The node that carries this primitive's mesh, or `None` when the mesh is
    /// not instanced by any node.
    pub node: Option<usize>,
    /// `x, y, z` per vertex, in the mesh's own space.
    pub positions: Vec<f32>,
    /// Triangle corners into `positions`. Always a multiple of three.
    pub indices: Vec<u32>,
    /// Four joint indices per vertex, empty when unskinned.
    ///
    /// These index the *skin's joint list*, not the node list — resolve them
    /// through [`Skin::joints`].
    pub joints: Vec<u16>,
    /// Four weights per vertex, matching `joints`.
    pub weights: Vec<f32>,
}

/// A node's local transform, kept as TRS rather than a matrix so a bone's
/// rotation survives the round trip unmultiplied.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Trs {
    /// Local translation.
    pub translation: [f32; 3],
    /// Local rotation, xyzw.
    pub rotation: [f32; 4],
    /// Local scale.
    pub scale: [f32; 3],
}

/// One node of the scene graph.
#[derive(Debug, Clone, PartialEq)]
pub struct Node {
    /// Its name, empty when the file gives none.
    pub name: String,
    /// Index of its parent, or `None` for a root.
    pub parent: Option<usize>,
    /// Local transform.
    pub transform: Trs,
    /// The skin that deforms this node's mesh, if it has one.
    ///
    /// glTF puts the skin on the node, not the mesh, so a round trip that
    /// dropped this would write an armature and a mesh with no connection
    /// between them — the mesh would import unweighted.
    pub skin: Option<usize>,
}

/// A skin: the joints that deform a mesh, and their bind pose.
#[derive(Debug, Clone, PartialEq)]
pub struct Skin {
    /// Node indices of the joints, in the order the primitives' joint indices
    /// refer to them.
    pub joints: Vec<usize>,
    /// One 4x4 column-major inverse bind matrix per joint. Empty when the file
    /// omits them, which glTF allows and which means "all identity".
    pub inverse_bind_matrices: Vec<[f32; 16]>,
}

/// Which property of a node a channel animates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Path {
    /// Three floats per key.
    Translation,
    /// Four floats per key, xyzw.
    Rotation,
    /// Three floats per key.
    Scale,
    /// Morph target weights; `stride` is the target count, so it varies.
    MorphWeights,
}

impl Path {
    /// Floats per key, or `None` for morph weights, whose stride depends on the
    /// mesh rather than the path.
    pub fn stride(self) -> Option<usize> {
        match self {
            Self::Translation | Self::Scale => Some(3),
            Self::Rotation => Some(4),
            Self::MorphWeights => None,
        }
    }
}

/// One animated property of one node.
#[derive(Debug, Clone, PartialEq)]
pub struct Channel {
    /// The node this drives.
    pub node: usize,
    /// Which property.
    pub path: Path,
    /// Key times in seconds, ascending.
    pub times: Vec<f32>,
    /// Key values, `times.len()` keys' worth.
    pub values: Vec<f32>,
}

/// One animation.
#[derive(Debug, Clone, PartialEq)]
pub struct Clip {
    /// Its name, empty when the file gives none.
    pub name: String,
    /// Largest key time across its channels, in seconds.
    pub duration: f32,
    /// Its channels.
    pub channels: Vec<Channel>,
}

/// What reading had to skip, rather than fail on.
///
/// Same principle as the FBX reader's reports: a file that is unusual is not a
/// file that is broken, and the caller decides which of these matter.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GlbReport {
    /// Primitives that were not triangles (points, lines, strips, fans).
    pub non_triangle_primitives: usize,
    /// Channels whose sampler had no input or output accessor.
    pub channels_without_data: usize,
    /// Channels animating morph weights, which this does not expand.
    pub morph_channels: usize,
    /// Primitives with JOINTS_0 but no WEIGHTS_0, or the reverse.
    pub half_skinned_primitives: usize,
    /// Primitives declaring JOINTS_1 or beyond, whose influences past the
    /// fourth are not read.
    ///
    /// glTF stores bone influences in sets of four; a vertex needing more than
    /// four uses JOINTS_1/WEIGHTS_1 as well. This reader takes set 0 only, so
    /// those extra influences are dropped. Counted rather than silent, because
    /// dropping an influence changes how the mesh deforms and O9 requires an
    /// existing rig to survive import intact (`memory/todo.md` P3-0).
    pub primitives_over_influence_limit: usize,
    /// Triangles dropped for naming a vertex the primitive does not have.
    ///
    /// glTF validation checks that an accessor fits inside its buffer, not that
    /// the *values* in an index accessor are vertices that exist — so a file can
    /// pass validation and still say "draw vertex 49233" of a 995-vertex mesh.
    /// Found by fuzzing. Dropped rather than errored, so one bad triangle does
    /// not make a mesh unopenable, and counted so a caller can tell.
    pub out_of_range_triangles: usize,
    /// Trailing indices that did not complete a triangle.
    pub incomplete_triangles: usize,
    /// Non-finite floats replaced with zero.
    ///
    /// A file can hold NaN or infinity anywhere it holds a float. They are
    /// replaced at the boundary rather than carried, for two reasons. A NaN
    /// vertex makes a mesh vanish with nothing to say where it started — the
    /// FBX reader learned that in session 019 — and JSON has no way to write
    /// one, so `serde_json` emits `null` and the exported file cannot be read
    /// back, by us or anyone. Found by fuzzing the round trip.
    pub non_finite_values: usize,
    /// Accessors skipped for declaring a type glTF does not allow there.
    ///
    /// The `gltf` crate reads an accessor into a fixed Rust type and
    /// `debug_assert!`s that the sizes agree
    /// (`gltf-1.4.1/src/accessor/util.rs:371`), so a file declaring POSITION as
    /// anything but VEC3/f32 panics it in a debug build — and `cargo test` is a
    /// debug build. Checking the declared type first turns that into a skip.
    /// Found by fuzzing.
    pub invalid_accessors: usize,
}

/// A `.glb` read into the parts this app cares about.
#[derive(Debug, Clone, PartialEq)]
pub struct Document {
    /// Scene nodes, indexed as the file indexes them.
    pub nodes: Vec<Node>,
    /// Triangulated primitives.
    pub primitives: Vec<Primitive>,
    /// Skins.
    pub skins: Vec<Skin>,
    /// Animations.
    pub clips: Vec<Clip>,
    /// What was skipped.
    pub report: GlbReport,
}

impl Document {
    /// Joints of a skin that no vertex is weighted to.
    ///
    /// A skin's joint list is not the same as the set of bones that deform the
    /// mesh, and the difference is not a defect. Three kinds of bone show up
    /// here in real files:
    ///
    /// - **Roots and leaf tips.** Our own `rig-human` has three: `root` and the
    ///   two `thumb_04_leaf` bones. They mark structure and orientation.
    /// - **Controls.** An `african buffalo.glb` supplied as reference carries
    ///   `PoleTarget.L/R` and `PoleTargetBack.L/R`, which drive an IK solve and
    ///   sit *outside* the body on purpose — a pole target belongs in front of
    ///   the knee, not inside it.
    /// - Bones left over from an authoring rig that the exporter kept.
    ///
    /// Callers need this because anything that asks "is this bone inside the
    /// mesh" or "should this bone be exported" gets the wrong answer for them.
    /// Returns indices into `skin.joints`.
    pub fn non_deforming_joints(&self, index: usize) -> Vec<usize> {
        let Some(skin) = self.skins.get(index) else {
            return Vec::new();
        };
        let mut deforms = vec![false; skin.joints.len()];
        for primitive in &self.primitives {
            // Only the primitives THIS skin deforms. A joint index means
            // nothing outside its own skin's joint list, so counting every
            // primitive marks a joint as deforming because some other mesh's
            // unrelated skin happened to use the same slot. On the converted
            // reference rig that is the difference between 57 and 39.
            let owner = primitive
                .node
                .and_then(|n| self.nodes.get(n))
                .and_then(|n| n.skin);
            if owner != Some(index) {
                continue;
            }
            for (joints, weights) in primitive
                .joints
                .chunks_exact(4)
                .zip(primitive.weights.chunks_exact(4))
            {
                for (&joint, &weight) in joints.iter().zip(weights) {
                    if weight > 0.0 {
                        if let Some(slot) = deforms.get_mut(joint as usize) {
                            *slot = true;
                        }
                    }
                }
            }
        }
        deforms
            .iter()
            .enumerate()
            .filter(|(_, &d)| !d)
            .map(|(i, _)| i)
            .collect()
    }

    /// World transform of every node, composed down the hierarchy.
    ///
    /// glTF does not require a node to appear after its parent, so this
    /// resolves each chain on demand and memoises it rather than assuming an
    /// order. Anything asking where a bone or a mesh actually *is* needs this;
    /// it was written three times in tests and examples before it was written
    /// once here.
    pub fn world_transforms(&self) -> Vec<glam::Mat4> {
        fn resolve(
            index: usize,
            nodes: &[Node],
            local: &[glam::Mat4],
            world: &mut [glam::Mat4],
            done: &mut [bool],
        ) -> glam::Mat4 {
            if done[index] {
                return world[index];
            }
            // Marked before recursing: a file whose parent links form a cycle
            // would otherwise recurse until the stack overflows, which aborts
            // rather than unwinds. A cycle resolves to the identity instead.
            done[index] = true;
            world[index] = match nodes[index].parent {
                Some(parent) if parent != index => {
                    resolve(parent, nodes, local, world, done) * local[index]
                }
                _ => local[index],
            };
            world[index]
        }

        let local: Vec<glam::Mat4> = self
            .nodes
            .iter()
            .map(|n| {
                glam::Mat4::from_scale_rotation_translation(
                    glam::Vec3::from(n.transform.scale),
                    glam::Quat::from_array(n.transform.rotation),
                    glam::Vec3::from(n.transform.translation),
                )
            })
            .collect();

        let mut world = vec![glam::Mat4::IDENTITY; self.nodes.len()];
        let mut done = vec![false; self.nodes.len()];
        for index in 0..self.nodes.len() {
            resolve(index, &self.nodes, &local, &mut world, &mut done);
        }
        world
    }

    /// How many distinct glTF meshes the primitives came from.
    pub fn mesh_count(&self) -> usize {
        let mut seen: Vec<usize> = self.primitives.iter().map(|p| p.mesh).collect();
        seen.sort_unstable();
        seen.dedup();
        seen.len()
    }
}

/// Reads a self-contained `.glb`.
///
/// # Errors
///
/// [`GlbError`] for anything malformed, and for any buffer that points outside
/// the file — see the module docs on why those are refused rather than fetched.
pub fn read(bytes: &[u8]) -> Result<Document, GlbError> {
    check_glb_header(bytes)?;
    check_indices(bytes)?;
    let gltf = gltf::Gltf::from_slice(bytes)?;

    // Every buffer must be the BIN chunk. glTF gives buffer 0 no URI in a GLB;
    // anything with a URI is external by definition, and is refused here rather
    // than resolved. `get_buffer_data` below can then never see an unchecked
    // buffer, because this loop has already rejected the file if one exists.
    //
    // Checked before the blob so a plain `.gltf` pointing at external buffers
    // says so, rather than reporting the missing BIN chunk it was never going
    // to have. Both refuse the file; only one explains why.
    for buffer in gltf.buffers() {
        if !matches!(buffer.source(), gltf::buffer::Source::Bin) {
            return Err(GlbError::ExternalBuffer {
                index: buffer.index(),
            });
        }
    }
    // A file that declares no buffers needs no BIN chunk, and demanding one
    // would reject a legitimate skeleton-only or empty document.
    let blob = match gltf.blob.as_deref() {
        Some(blob) => blob,
        None if gltf.buffers().len() == 0 => &[],
        None => return Err(GlbError::NoBinaryChunk),
    };
    let get_buffer_data = |_: gltf::Buffer| Some(blob);

    let mut report = GlbReport::default();
    let nodes = read_nodes(&gltf, &mut report);
    let mesh_owner = mesh_owners(&gltf);
    let primitives = read_primitives(&gltf, &mesh_owner, get_buffer_data, &mut report)?;
    let skins = read_skins(&gltf, get_buffer_data, &mut report);
    let clips = read_clips(&gltf, get_buffer_data, &mut report);

    Ok(Document {
        nodes,
        primitives,
        skins,
        clips,
        report,
    })
}

/// Replaces non-finite floats with zero, counting what it replaced.
///
/// Individually, not wholesale: a vertex with one bad component keeps its other
/// two, which matches how the FBX reader repairs a model's transform.
fn sanitize(values: &mut [f32], count: &mut usize) {
    for value in values.iter_mut() {
        if !value.is_finite() {
            *value = 0.0;
            *count += 1;
        }
    }
}

/// Rejects a file whose JSON contains an index pointing past the array it
/// indexes, before the `gltf` crate dereferences one.
///
/// glTF is a graph of integer references, and the crate resolves several of
/// them by direct indexing inside `validate()` — so an out-of-range value
/// panics rather than being reported. This walks the same references first.
/// Only structural references are covered: everything the crate might
/// dereference, not only what this reader goes on to use.
fn check_indices(bytes: &[u8]) -> Result<(), GlbError> {
    let Some(json) = json_chunk(bytes) else {
        return Ok(());
    };
    let Ok(root) = serde_json::from_slice::<serde_json::Value>(json) else {
        // Malformed JSON is the `gltf` crate's error to report, not ours.
        return Ok(());
    };
    let len = |key: &str| root.get(key).and_then(|v| v.as_array()).map_or(0, Vec::len);
    let (accessors, views, buffers) = (len("accessors"), len("bufferViews"), len("buffers"));
    let (nodes, meshes, skins) = (len("nodes"), len("meshes"), len("skins"));
    let (materials, images, scenes) = (len("materials"), len("images"), len("scenes"));

    let check = |value: Option<&serde_json::Value>,
                 owner: &'static str,
                 target: &'static str,
                 count: usize|
     -> Result<(), GlbError> {
        let Some(index) = value.and_then(serde_json::Value::as_u64) else {
            return Ok(());
        };
        let index = index as usize;
        if index >= count {
            return Err(GlbError::IndexOutOfRange {
                owner,
                target,
                index,
                len: count,
            });
        }
        Ok(())
    };
    let each = |value: Option<&serde_json::Value>| -> Vec<serde_json::Value> {
        value
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
    };

    for view in each(root.get("bufferViews")) {
        check(view.get("buffer"), "bufferView.buffer", "buffers", buffers)?;
    }
    for accessor in each(root.get("accessors")) {
        check(
            accessor.get("bufferView"),
            "accessor.bufferView",
            "bufferViews",
            views,
        )?;
        if let Some(sparse) = accessor.get("sparse") {
            for part in ["indices", "values"] {
                check(
                    sparse.get(part).and_then(|p| p.get("bufferView")),
                    "accessor.sparse.bufferView",
                    "bufferViews",
                    views,
                )?;
            }
        }
    }
    for image in each(root.get("images")) {
        check(
            image.get("bufferView"),
            "image.bufferView",
            "bufferViews",
            views,
        )?;
    }
    for texture in each(root.get("textures")) {
        check(texture.get("source"), "texture.source", "images", images)?;
    }
    for mesh in each(root.get("meshes")) {
        for primitive in each(mesh.get("primitives")) {
            for (_, accessor) in primitive
                .get("attributes")
                .and_then(|a| a.as_object())
                .into_iter()
                .flatten()
            {
                check(
                    Some(accessor),
                    "primitive.attributes",
                    "accessors",
                    accessors,
                )?;
            }
            check(
                primitive.get("indices"),
                "primitive.indices",
                "accessors",
                accessors,
            )?;
            check(
                primitive.get("material"),
                "primitive.material",
                "materials",
                materials,
            )?;
            for target in each(primitive.get("targets")) {
                for (_, accessor) in target.as_object().into_iter().flatten() {
                    check(Some(accessor), "primitive.targets", "accessors", accessors)?;
                }
            }
        }
    }
    for node in each(root.get("nodes")) {
        check(node.get("mesh"), "node.mesh", "meshes", meshes)?;
        check(node.get("skin"), "node.skin", "skins", skins)?;
        for child in each(node.get("children")) {
            check(Some(&child), "node.children", "nodes", nodes)?;
        }
    }
    for skin in each(root.get("skins")) {
        check(
            skin.get("inverseBindMatrices"),
            "skin.inverseBindMatrices",
            "accessors",
            accessors,
        )?;
        check(skin.get("skeleton"), "skin.skeleton", "nodes", nodes)?;
        for joint in each(skin.get("joints")) {
            check(Some(&joint), "skin.joints", "nodes", nodes)?;
        }
    }
    for scene in each(root.get("scenes")) {
        for node in each(scene.get("nodes")) {
            check(Some(&node), "scene.nodes", "nodes", nodes)?;
        }
    }
    check(root.get("scene"), "scene", "scenes", scenes)?;
    for animation in each(root.get("animations")) {
        let sampler_count = each(animation.get("samplers")).len();
        for channel in each(animation.get("channels")) {
            check(
                channel.get("sampler"),
                "animation.channel.sampler",
                "samplers",
                sampler_count,
            )?;
            check(
                channel.get("target").and_then(|t| t.get("node")),
                "animation.channel.target.node",
                "nodes",
                nodes,
            )?;
        }
        for sampler in each(animation.get("samplers")) {
            for part in ["input", "output"] {
                check(
                    sampler.get(part),
                    "animation.sampler",
                    "accessors",
                    accessors,
                )?;
            }
        }
    }
    // Material, texture-sampler and camera references are deliberately not
    // walked: this reader never resolves them, and 7.4M fuzz runs found no path
    // where the crate dereferences one before validating it. Add them here if
    // one ever turns up.
    Ok(())
}

/// The JSON chunk of a GLB, or the whole slice for a plain `.gltf`.
fn json_chunk(bytes: &[u8]) -> Option<&[u8]> {
    if !bytes.starts_with(b"glTF") {
        return Some(bytes);
    }
    // 12-byte header, then chunks of (u32 length, u32 type, payload). The JSON
    // chunk is required to be first.
    let length = u32::from_le_bytes(bytes.get(12..16)?.try_into().ok()?) as usize;
    let kind = u32::from_le_bytes(bytes.get(16..20)?.try_into().ok()?);
    if kind != 0x4E4F_534A {
        return None;
    }
    bytes.get(20..20usize.checked_add(length)?)
}

/// Whether an accessor declares one of the layouts glTF allows for its use, and
/// so can be handed to the `gltf` crate's typed readers without tripping their
/// debug assertions.
///
/// A zero count is rejected for the same reason: the crate computes
/// `stride * (count - 1)`, which underflows.
fn accessor_is<'a>(
    accessor: &gltf::Accessor<'a>,
    dimensions: gltf::accessor::Dimensions,
    types: &[gltf::accessor::DataType],
) -> bool {
    accessor.count() > 0
        && accessor.dimensions() == dimensions
        && types.contains(&accessor.data_type())
}

/// Validates the 12-byte GLB header before the `gltf` crate arithmetic on it.
///
/// Only applies to files that claim to be GLB; a plain JSON `.gltf` has no
/// header and is left to the crate's own parser.
fn check_glb_header(bytes: &[u8]) -> Result<(), GlbError> {
    if !bytes.starts_with(b"glTF") {
        return Ok(());
    }
    let header: &[u8; 12] =
        bytes
            .get(..12)
            .and_then(|h| h.try_into().ok())
            .ok_or(GlbError::MalformedHeader {
                reason: "shorter than the 12-byte header",
            })?;
    let version = u32::from_le_bytes([header[4], header[5], header[6], header[7]]);
    if version != 2 {
        return Err(GlbError::MalformedHeader {
            reason: "only glTF 2.0 binary is supported",
        });
    }
    // The declared total length, which the spec says includes this header.
    // Anything less is what underflows upstream.
    let length = u32::from_le_bytes([header[8], header[9], header[10], header[11]]);
    if (length as usize) < header.len() {
        return Err(GlbError::MalformedHeader {
            reason: "declared length is shorter than the header itself",
        });
    }
    Ok(())
}

/// Flattens the node graph, recording each node's parent.
///
/// glTF stores children, not parents, and guarantees the graph is a forest, so
/// one pass over every node's children names every parent exactly once.
fn read_nodes(gltf: &gltf::Gltf, report: &mut GlbReport) -> Vec<Node> {
    let mut parents: HashMap<usize, usize> = HashMap::new();
    for node in gltf.nodes() {
        for child in node.children() {
            parents.insert(child.index(), node.index());
        }
    }
    let mut out = Vec::new();
    for node in gltf.nodes() {
        let (mut translation, mut rotation, mut scale) = node.transform().decomposed();
        sanitize(&mut translation, &mut report.non_finite_values);
        sanitize(&mut rotation, &mut report.non_finite_values);
        sanitize(&mut scale, &mut report.non_finite_values);
        out.push({
            Node {
                name: node.name().unwrap_or_default().to_owned(),
                parent: parents.get(&node.index()).copied(),
                transform: Trs {
                    translation,
                    rotation,
                    scale,
                },
                skin: node.skin().map(|s| s.index()),
            }
        });
    }
    out
}

/// Which node instances each mesh.
///
/// glTF lets several nodes share one mesh; this keeps the first, which is what
/// every file this app reads actually has. A shared mesh would otherwise lose
/// the transform that places it.
fn mesh_owners(gltf: &gltf::Gltf) -> HashMap<usize, usize> {
    let mut owners = HashMap::new();
    for node in gltf.nodes() {
        if let Some(mesh) = node.mesh() {
            owners.entry(mesh.index()).or_insert(node.index());
        }
    }
    owners
}

fn read_primitives<'a, F>(
    gltf: &gltf::Gltf,
    mesh_owner: &HashMap<usize, usize>,
    get_buffer_data: F,
    report: &mut GlbReport,
) -> Result<Vec<Primitive>, GlbError>
where
    F: Clone + Fn(gltf::Buffer) -> Option<&'a [u8]>,
{
    let mut out = Vec::new();
    for mesh in gltf.meshes() {
        for primitive in mesh.primitives() {
            if primitive.mode() != gltf::mesh::Mode::Triangles {
                report.non_triangle_primitives += 1;
                continue;
            }
            // Check every accessor's declared layout before reading it. The
            // spec fixes these, and the `gltf` crate assumes them.
            let mut usable = true;
            let mut extra_influence_sets = false;
            for (semantic, accessor) in primitive.attributes() {
                if let gltf::Semantic::Joints(set) = semantic {
                    extra_influence_sets |= set > 0;
                }
                let ok = match semantic {
                    gltf::Semantic::Positions | gltf::Semantic::Normals => accessor_is(
                        &accessor,
                        gltf::accessor::Dimensions::Vec3,
                        &[gltf::accessor::DataType::F32],
                    ),
                    gltf::Semantic::Joints(_) => accessor_is(
                        &accessor,
                        gltf::accessor::Dimensions::Vec4,
                        &[gltf::accessor::DataType::U8, gltf::accessor::DataType::U16],
                    ),
                    gltf::Semantic::Weights(_) => accessor_is(
                        &accessor,
                        gltf::accessor::Dimensions::Vec4,
                        &[
                            gltf::accessor::DataType::F32,
                            gltf::accessor::DataType::U8,
                            gltf::accessor::DataType::U16,
                        ],
                    ),
                    // Attributes this reader does not touch cannot panic it.
                    _ => true,
                };
                if !ok {
                    usable = false;
                }
            }
            if let Some(accessor) = primitive.indices() {
                if !accessor_is(
                    &accessor,
                    gltf::accessor::Dimensions::Scalar,
                    &[
                        gltf::accessor::DataType::U8,
                        gltf::accessor::DataType::U16,
                        gltf::accessor::DataType::U32,
                    ],
                ) {
                    usable = false;
                }
            }
            if !usable {
                report.invalid_accessors += 1;
                continue;
            }
            if extra_influence_sets {
                report.primitives_over_influence_limit += 1;
            }

            let reader = primitive.reader(get_buffer_data.clone());
            let mut positions: Vec<f32> = reader
                .read_positions()
                .ok_or(GlbError::NoPositions {
                    mesh: mesh.index(),
                    primitive: primitive.index(),
                })?
                .flatten()
                .collect();
            sanitize(&mut positions, &mut report.non_finite_values);

            // A primitive with no index buffer draws its vertices in order.
            let vertex_count = positions.len() / 3;
            let raw: Vec<u32> = match reader.read_indices() {
                Some(indices) => indices.into_u32().collect(),
                None => (0..vertex_count as u32).collect(),
            };

            // Every corner must be a vertex this primitive actually has.
            // Callers — renderers above all — treat `indices` as valid offsets
            // into `positions`, so that has to be true here rather than
            // hopefully true.
            if raw.len() % 3 != 0 {
                report.incomplete_triangles += 1;
            }
            let mut indices = Vec::with_capacity(raw.len());
            for triangle in raw.chunks_exact(3) {
                if triangle.iter().all(|&i| (i as usize) < vertex_count) {
                    indices.extend_from_slice(triangle);
                } else {
                    report.out_of_range_triangles += 1;
                }
            }

            let joints: Vec<u16> = reader
                .read_joints(0)
                .map(|j| j.into_u16().flatten().collect())
                .unwrap_or_default();
            let mut weights: Vec<f32> = reader
                .read_weights(0)
                .map(|w| w.into_f32().flatten().collect())
                .unwrap_or_default();
            sanitize(&mut weights, &mut report.non_finite_values);
            if joints.is_empty() != weights.is_empty() {
                report.half_skinned_primitives += 1;
            }

            out.push(Primitive {
                mesh: mesh.index(),
                node: mesh_owner.get(&mesh.index()).copied(),
                positions,
                indices,
                joints,
                weights,
            });
        }
    }
    Ok(out)
}

fn read_skins<'a, F>(gltf: &gltf::Gltf, get_buffer_data: F, report: &mut GlbReport) -> Vec<Skin>
where
    F: Clone + Fn(gltf::Buffer) -> Option<&'a [u8]>,
{
    let mut out = Vec::new();
    for skin in gltf.skins() {
        out.push({
            let valid = skin.inverse_bind_matrices().is_none_or(|a| {
                accessor_is(
                    &a,
                    gltf::accessor::Dimensions::Mat4,
                    &[gltf::accessor::DataType::F32],
                )
            });
            let reader = skin.reader(get_buffer_data.clone());
            Skin {
                joints: skin.joints().map(|j| j.index()).collect(),
                inverse_bind_matrices: if valid {
                    let mut matrices: Vec<[f32; 16]> = reader
                        .read_inverse_bind_matrices()
                        .map(|m| m.map(flatten_matrix).collect())
                        .unwrap_or_default();
                    for matrix in &mut matrices {
                        sanitize(matrix, &mut report.non_finite_values);
                    }
                    matrices
                } else {
                    report.invalid_accessors += 1;
                    Vec::new()
                },
            }
        });
    }
    out
}

/// glTF hands matrices back as four columns; the rest of this crate wants the
/// same 16 floats flat, still column-major.
fn flatten_matrix(columns: [[f32; 4]; 4]) -> [f32; 16] {
    let mut out = [0.0; 16];
    for (i, column) in columns.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(column);
    }
    out
}

fn read_clips<'a, F>(gltf: &gltf::Gltf, get_buffer_data: F, report: &mut GlbReport) -> Vec<Clip>
where
    F: Clone + Fn(gltf::Buffer) -> Option<&'a [u8]>,
{
    gltf.animations()
        .map(|animation| {
            let mut channels = Vec::new();
            let mut duration = 0.0f32;
            for channel in animation.channels() {
                let path = match channel.target().property() {
                    gltf::animation::Property::Translation => Path::Translation,
                    gltf::animation::Property::Rotation => Path::Rotation,
                    gltf::animation::Property::Scale => Path::Scale,
                    gltf::animation::Property::MorphTargetWeights => {
                        report.morph_channels += 1;
                        Path::MorphWeights
                    }
                };
                // Key times are always SCALAR/f32; the values' layout follows
                // the path. Anything else is skipped rather than read, because
                // the crate's typed readers assert on the size.
                let sampler = channel.sampler();
                let output_dimensions = match path {
                    Path::Translation | Path::Scale => gltf::accessor::Dimensions::Vec3,
                    Path::Rotation => gltf::accessor::Dimensions::Vec4,
                    Path::MorphWeights => gltf::accessor::Dimensions::Scalar,
                };
                let normalised = &[
                    gltf::accessor::DataType::F32,
                    gltf::accessor::DataType::U8,
                    gltf::accessor::DataType::U16,
                    gltf::accessor::DataType::I8,
                    gltf::accessor::DataType::I16,
                ];
                if !accessor_is(
                    &sampler.input(),
                    gltf::accessor::Dimensions::Scalar,
                    &[gltf::accessor::DataType::F32],
                ) || !accessor_is(&sampler.output(), output_dimensions, normalised)
                {
                    report.invalid_accessors += 1;
                    continue;
                }

                let reader = channel.reader(get_buffer_data.clone());
                let (Some(times), Some(outputs)) = (reader.read_inputs(), reader.read_outputs())
                else {
                    report.channels_without_data += 1;
                    continue;
                };
                let mut times: Vec<f32> = times.collect();
                sanitize(&mut times, &mut report.non_finite_values);
                let mut values: Vec<f32> = match outputs {
                    gltf::animation::util::ReadOutputs::Translations(v) => v.flatten().collect(),
                    gltf::animation::util::ReadOutputs::Scales(v) => v.flatten().collect(),
                    gltf::animation::util::ReadOutputs::Rotations(v) => {
                        v.into_f32().flatten().collect()
                    }
                    gltf::animation::util::ReadOutputs::MorphTargetWeights(v) => {
                        v.into_f32().collect()
                    }
                };
                sanitize(&mut values, &mut report.non_finite_values);
                duration = times.iter().copied().fold(duration, f32::max);
                channels.push(Channel {
                    node: channel.target().node().index(),
                    path,
                    times,
                    values,
                });
            }
            Clip {
                name: animation.name().unwrap_or_default().to_owned(),
                duration,
                channels,
            }
        })
        .collect()
}
