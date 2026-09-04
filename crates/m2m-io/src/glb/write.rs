//! Writing glTF 2.0 in its self-contained binary form (`.glb`).
//!
//! The inverse of [`super::read`], and it takes the same [`Document`] that
//! reader produces — so a round trip is `write(&read(bytes)?)` rather than a
//! translation between two shapes. A caller with a scene of its own (the
//! auto-rigger, eventually) builds a `Document` and writes that.
//!
//! # What is carried, and what is not
//!
//! Positions, triangle indices, the node hierarchy with its local TRS, skins
//! with their inverse bind matrices, joint indices and weights, animation
//! channels, and — when a primitive carries them — normals and UVs. **Not**
//! materials or textures: those are preserved by grafting onto the source file
//! (see [`super::graft`]), not rebuilt here. A file rebuilt through this writer
//! keeps its rig, its shape, and its shading coordinates.
//!
//! Everything lands in one buffer — the GLB's BIN chunk — because this writer
//! deliberately never emits an external URI. See the module docs on why.

use std::collections::BTreeMap;

use gltf::json;
use json::validation::{Checked, USize64};

use super::{Document, GlbError, Path};

/// Accumulates the BIN chunk and the buffer views that address it.
///
/// Each accessor gets its own view. Sharing one view between accessors is
/// legal and marginally smaller, but the offsets then have to satisfy both
/// accessors' component alignment at once, and the saving is not worth a class
/// of bug that only shows up in someone else's importer.
struct Bin {
    bytes: Vec<u8>,
    views: Vec<json::buffer::View>,
}

impl Bin {
    fn new() -> Self {
        Self {
            bytes: Vec::new(),
            views: Vec::new(),
        }
    }

    /// Appends `data` as a new buffer view and returns its index.
    ///
    /// Pads to four bytes first: glTF requires an accessor's offset within the
    /// buffer to be a multiple of its component size, and four covers every
    /// component this writer emits.
    fn push(&mut self, data: &[u8]) -> json::Index<json::buffer::View> {
        while self.bytes.len() % 4 != 0 {
            self.bytes.push(0);
        }
        let offset = self.bytes.len();
        self.bytes.extend_from_slice(data);
        let index = self.views.len() as u32;
        self.views.push(json::buffer::View {
            buffer: json::Index::new(0),
            byte_length: USize64(data.len() as u64),
            byte_offset: Some(USize64(offset as u64)),
            byte_stride: None,
            name: None,
            target: None,
            extensions: None,
            extras: Default::default(),
        });
        json::Index::new(index)
    }
}

/// Builds an accessor over a whole buffer view.
fn accessor(
    view: json::Index<json::buffer::View>,
    count: usize,
    component: json::accessor::ComponentType,
    kind: json::accessor::Type,
) -> json::Accessor {
    json::Accessor {
        buffer_view: Some(view),
        byte_offset: Some(USize64(0)),
        count: USize64(count as u64),
        component_type: Checked::Valid(json::accessor::GenericComponentType(component)),
        type_: Checked::Valid(kind),
        min: None,
        max: None,
        name: None,
        normalized: false,
        sparse: None,
        extensions: None,
        extras: Default::default(),
    }
}

fn as_bytes<T: bytemuck_lite::Pod>(values: &[T]) -> Vec<u8> {
    let mut out = Vec::with_capacity(std::mem::size_of_val(values));
    for value in values {
        out.extend_from_slice(value.to_le_bytes().as_ref());
    }
    out
}

/// A tiny stand-in for `bytemuck`, so this does not add a dependency to turn
/// four primitive types into little-endian bytes.
mod bytemuck_lite {
    /// Something with a fixed little-endian byte form.
    pub trait Pod: Copy {
        /// Its bytes.
        type Bytes: AsRef<[u8]>;
        /// Little-endian, because glTF buffers always are.
        fn to_le_bytes(self) -> Self::Bytes;
    }
    impl Pod for f32 {
        type Bytes = [u8; 4];
        fn to_le_bytes(self) -> [u8; 4] {
            f32::to_le_bytes(self)
        }
    }
    impl Pod for u32 {
        type Bytes = [u8; 4];
        fn to_le_bytes(self) -> [u8; 4] {
            u32::to_le_bytes(self)
        }
    }
    impl Pod for u16 {
        type Bytes = [u8; 2];
        fn to_le_bytes(self) -> [u8; 2] {
            u16::to_le_bytes(self)
        }
    }
}

/// Writes one optional Vec4 vertex attribute (joints, weights or colours) into
/// `attributes`, doing nothing when the primitive does not carry it.
fn vec4_attribute<T: bytemuck_lite::Pod>(
    bin: &mut Bin,
    accessors: &mut Vec<json::Accessor>,
    attributes: &mut BTreeMap<Checked<json::mesh::Semantic>, json::Index<json::Accessor>>,
    data: &[T],
    semantic: json::mesh::Semantic,
    component: json::accessor::ComponentType,
) {
    if data.is_empty() {
        return;
    }
    let view = bin.push(&as_bytes(data));
    let acc = accessor(view, data.len() / 4, component, json::accessor::Type::Vec4);
    attributes.insert(Checked::Valid(semantic), push(accessors, acc));
}

/// Builds one glTF primitive: POSITION with its required bounds, the optional
/// skin and colour attributes, and the triangle indices.
fn write_primitive(
    bin: &mut Bin,
    accessors: &mut Vec<json::Accessor>,
    primitive: &super::Primitive,
) -> Result<json::mesh::Primitive, GlbError> {
    let vertices = primitive.positions.len() / 3;
    let view = bin.push(&as_bytes(&primitive.positions));
    let mut positions = accessor(
        view,
        vertices,
        json::accessor::ComponentType::F32,
        json::accessor::Type::Vec3,
    );
    // POSITION must carry min and max; the crate's own validator
    // rejects the file without them.
    let (min, max) = bounds(&primitive.positions);
    positions.min = Some(json::serialize::to_value(min).map_err(json_error)?);
    positions.max = Some(json::serialize::to_value(max).map_err(json_error)?);
    let position_index = push(accessors, positions);

    let mut attributes = BTreeMap::new();
    attributes.insert(
        Checked::Valid(json::mesh::Semantic::Positions),
        position_index,
    );
    vec4_attribute(
        bin,
        accessors,
        &mut attributes,
        &primitive.joints,
        json::mesh::Semantic::Joints(0),
        json::accessor::ComponentType::U16,
    );
    vec4_attribute(
        bin,
        accessors,
        &mut attributes,
        &primitive.weights,
        json::mesh::Semantic::Weights(0),
        json::accessor::ComponentType::F32,
    );
    vec4_attribute(
        bin,
        accessors,
        &mut attributes,
        &primitive.colors,
        json::mesh::Semantic::Colors(0),
        json::accessor::ComponentType::F32,
    );

    // Normals (Vec3) and UVs (Vec2) when the primitive carries them — the
    // shading a rigged export keeps from its source. Written only when present,
    // so a rig template or the weight overlay, which carry neither, are unchanged.
    if !primitive.normals.is_empty() {
        let view = bin.push(&as_bytes(&primitive.normals));
        let acc = accessor(
            view,
            primitive.normals.len() / 3,
            json::accessor::ComponentType::F32,
            json::accessor::Type::Vec3,
        );
        attributes.insert(
            Checked::Valid(json::mesh::Semantic::Normals),
            push(accessors, acc),
        );
    }
    if !primitive.uvs.is_empty() {
        let view = bin.push(&as_bytes(&primitive.uvs));
        let acc = accessor(
            view,
            primitive.uvs.len() / 2,
            json::accessor::ComponentType::F32,
            json::accessor::Type::Vec2,
        );
        attributes.insert(
            Checked::Valid(json::mesh::Semantic::TexCoords(0)),
            push(accessors, acc),
        );
    }

    let view = bin.push(&as_bytes(&primitive.indices));
    let indices = accessor(
        view,
        primitive.indices.len(),
        json::accessor::ComponentType::U32,
        json::accessor::Type::Scalar,
    );
    Ok(json::mesh::Primitive {
        attributes,
        indices: Some(push(accessors, indices)),
        material: None,
        mode: Checked::Valid(json::mesh::Mode::Triangles),
        targets: None,
        extensions: None,
        extras: Default::default(),
    })
}

/// Writes the document's skins, each with its optional inverse-bind-matrix
/// accessor, in document order.
fn write_skins(
    document: &Document,
    bin: &mut Bin,
    accessors: &mut Vec<json::Accessor>,
) -> Vec<json::Skin> {
    document
        .skins
        .iter()
        .map(|skin| {
            let inverse_bind_matrices = if skin.inverse_bind_matrices.is_empty() {
                None
            } else {
                let flat: Vec<f32> = skin
                    .inverse_bind_matrices
                    .iter()
                    .flatten()
                    .copied()
                    .collect();
                let count = skin.inverse_bind_matrices.len();
                let view = bin.push(&as_bytes(&flat));
                Some(push(
                    accessors,
                    accessor(
                        view,
                        count,
                        json::accessor::ComponentType::F32,
                        json::accessor::Type::Mat4,
                    ),
                ))
            };
            json::Skin {
                joints: skin
                    .joints
                    .iter()
                    .map(|&j| json::Index::new(j as u32))
                    .collect(),
                inverse_bind_matrices,
                skeleton: None,
                name: None,
                extensions: None,
                extras: Default::default(),
            }
        })
        .collect()
}

/// Writes the node hierarchy (glTF stores children; the document stores
/// parents) and returns the nodes together with the scene's root indices.
fn write_nodes(
    document: &Document,
    mesh_indices: &BTreeMap<usize, u32>,
) -> (Vec<json::Node>, Vec<json::Index<json::Node>>) {
    let mut children: Vec<Vec<json::Index<json::Node>>> = vec![Vec::new(); document.nodes.len()];
    for (index, node) in document.nodes.iter().enumerate() {
        if let Some(parent) = node.parent {
            if parent < children.len() {
                children[parent].push(json::Index::new(index as u32));
            }
        }
    }
    // Which node instances each mesh, so the mesh is written back onto it.
    let mut node_mesh: BTreeMap<usize, u32> = BTreeMap::new();
    for primitive in &document.primitives {
        if let (Some(node), Some(&mesh)) = (primitive.node, mesh_indices.get(&primitive.mesh)) {
            node_mesh.insert(node, mesh);
        }
    }

    let nodes: Vec<json::Node> = document
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| json::Node {
            camera: None,
            children: if children[index].is_empty() {
                None
            } else {
                Some(children[index].clone())
            },
            matrix: None,
            mesh: node_mesh.get(&index).map(|&m| json::Index::new(m)),
            name: (!node.name.is_empty()).then(|| node.name.clone()),
            rotation: Some(json::scene::UnitQuaternion(node.transform.rotation)),
            scale: Some(node.transform.scale),
            translation: Some(node.transform.translation),
            skin: node.skin.map(|s| json::Index::new(s as u32)),
            weights: None,
            extensions: None,
            extras: Default::default(),
        })
        .collect();

    let roots: Vec<json::Index<json::Node>> = document
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, node)| node.parent.is_none())
        .map(|(index, _)| json::Index::new(index as u32))
        .collect();

    (nodes, roots)
}

/// Writes a document as a self-contained `.glb`.
///
/// # Errors
///
/// [`GlbError::Invalid`] if the assembled document fails the `gltf` crate's own
/// serialization, which in practice means a count that does not fit a `u32`.
pub fn write(document: &Document) -> Result<Vec<u8>, GlbError> {
    let mut bin = Bin::new();
    let mut accessors: Vec<json::Accessor> = Vec::new();
    let mut meshes: Vec<json::Mesh> = Vec::new();

    // Primitives are grouped back into the meshes they came from, so a mesh of
    // 22 primitives is written as one mesh again rather than 22.
    let mut by_mesh: BTreeMap<usize, Vec<&super::Primitive>> = BTreeMap::new();
    for primitive in &document.primitives {
        // A primitive with nothing to draw is skipped rather than written as
        // empty: glTF requires an accessor's count to be at least one, so a
        // zero-count POSITION or index accessor produces a file that readers —
        // ours included — reject. This is reachable from real input, because
        // the reader drops triangles that name vertices the mesh does not have,
        // and dropping all of them leaves a primitive with no indices.
        if primitive.indices.is_empty() || primitive.positions.is_empty() {
            continue;
        }
        by_mesh.entry(primitive.mesh).or_default().push(primitive);
    }
    // Which written mesh each source mesh became, since a source mesh with no
    // primitives is not written at all and would otherwise shift the indices.
    let mut mesh_indices: BTreeMap<usize, u32> = BTreeMap::new();

    for (source, primitives) in &by_mesh {
        let mut written = Vec::new();
        for primitive in primitives {
            written.push(write_primitive(&mut bin, &mut accessors, primitive)?);
        }
        mesh_indices.insert(*source, meshes.len() as u32);
        meshes.push(json::Mesh {
            primitives: written,
            name: None,
            weights: None,
            extensions: None,
            extras: Default::default(),
        });
    }

    let skins = write_skins(document, &mut bin, &mut accessors);
    let animations = write_animations(document, &mut bin, &mut accessors)?;
    let (nodes, roots) = write_nodes(document, &mesh_indices);

    let empty_scene = roots.is_empty();
    let root = json::Root {
        asset: json::Asset {
            generator: Some("mesh2motion".to_owned()),
            version: "2.0".to_owned(),
            ..Default::default()
        },
        accessors,
        animations,
        // glTF requires a buffer's byteLength to be at least one, so a document
        // with no geometry declares no buffer rather than an empty one.
        buffers: if bin.bytes.is_empty() {
            Vec::new()
        } else {
            vec![json::Buffer {
                byte_length: USize64(bin.bytes.len() as u64),
                // No URI: the data is the GLB's BIN chunk. This is the same rule
                // the reader enforces from the other side.
                uri: None,
                name: None,
                extensions: None,
                extras: Default::default(),
            }]
        },
        buffer_views: bin.views,
        meshes,
        nodes,
        skins,
        // A scene with no root nodes is not written at all. `json::Scene`'s
        // `nodes` field is `skip_serializing_if = "Vec::is_empty"` with no
        // `serde(default)` (`gltf-json-1.4.1/src/scene.rs`), so an empty scene
        // serializes to `{}` and then fails to deserialize with "missing field
        // `nodes`" — the crate writes a file it cannot read. glTF makes
        // `scenes` optional, so omitting it is the correct file anyway.
        scenes: if roots.is_empty() {
            Vec::new()
        } else {
            vec![json::Scene {
                nodes: roots,
                name: None,
                extensions: None,
                extras: Default::default(),
            }]
        },
        scene: (!empty_scene).then(|| json::Index::new(0)),
        ..Default::default()
    };

    let json_chunk = json::serialize::to_vec(&root).map_err(json_error)?;
    let glb = gltf::Glb {
        header: gltf::binary::Header {
            magic: *b"glTF",
            version: 2,
            // `to_vec` recomputes this from the chunks, so the value here is
            // never the one written.
            length: 0,
        },
        json: json_chunk.into(),
        bin: (!bin.bytes.is_empty()).then(|| bin.bytes.into()),
    };
    glb.to_vec().map_err(GlbError::Invalid)
}

fn write_animations(
    document: &Document,
    bin: &mut Bin,
    accessors: &mut Vec<json::Accessor>,
) -> Result<Vec<json::Animation>, GlbError> {
    let mut out = Vec::new();
    for clip in &document.clips {
        let mut channels = Vec::new();
        let mut samplers = Vec::new();
        for channel in &clip.channels {
            // Key times need min and max: importers read the clip's range from
            // them rather than scanning every key, so omitting them is how an
            // animation ends up with the right keys and the wrong length.
            let view = bin.push(&as_bytes(&channel.times));
            let mut input = accessor(
                view,
                channel.times.len(),
                json::accessor::ComponentType::F32,
                json::accessor::Type::Scalar,
            );
            let first = channel.times.first().copied().unwrap_or(0.0);
            let last = channel.times.iter().copied().fold(first, f32::max);
            input.min = Some(json::serialize::to_value([first]).map_err(json_error)?);
            input.max = Some(json::serialize::to_value([last]).map_err(json_error)?);

            let kind = match channel.path {
                Path::Rotation => json::accessor::Type::Vec4,
                Path::MorphWeights => json::accessor::Type::Scalar,
                _ => json::accessor::Type::Vec3,
            };
            let stride = channel.path.stride().unwrap_or(1);
            let view = bin.push(&as_bytes(&channel.values));
            let output = accessor(
                view,
                channel.values.len() / stride,
                json::accessor::ComponentType::F32,
                kind,
            );

            samplers.push(json::animation::Sampler {
                input: push(accessors, input),
                output: push(accessors, output),
                interpolation: Checked::Valid(json::animation::Interpolation::Linear),
                extensions: None,
                extras: Default::default(),
            });
            channels.push(json::animation::Channel {
                sampler: json::Index::new(samplers.len() as u32 - 1),
                target: json::animation::Target {
                    node: json::Index::new(channel.node as u32),
                    path: Checked::Valid(match channel.path {
                        Path::Translation => json::animation::Property::Translation,
                        Path::Rotation => json::animation::Property::Rotation,
                        Path::Scale => json::animation::Property::Scale,
                        Path::MorphWeights => json::animation::Property::MorphTargetWeights,
                    }),
                    extensions: None,
                    extras: Default::default(),
                },
                extensions: None,
                extras: Default::default(),
            });
        }
        out.push(json::Animation {
            channels,
            samplers,
            name: (!clip.name.is_empty()).then(|| clip.name.clone()),
            extensions: None,
            extras: Default::default(),
        });
    }
    Ok(out)
}

/// Appends an accessor and returns its index.
fn push(
    accessors: &mut Vec<json::Accessor>,
    accessor: json::Accessor,
) -> json::Index<json::Accessor> {
    accessors.push(accessor);
    json::Index::new(accessors.len() as u32 - 1)
}

/// Component-wise bounds of a position array, as glTF requires on POSITION.
///
/// An empty array yields zeroes rather than infinities, which would serialize
/// as `null` and fail validation.
fn bounds(positions: &[f32]) -> ([f32; 3], [f32; 3]) {
    let mut min = [0.0f32; 3];
    let mut max = [0.0f32; 3];
    let mut first = true;
    for vertex in positions.chunks_exact(3) {
        for axis in 0..3 {
            if first {
                min[axis] = vertex[axis];
                max[axis] = vertex[axis];
            } else {
                min[axis] = min[axis].min(vertex[axis]);
                max[axis] = max[axis].max(vertex[axis]);
            }
        }
        first = false;
    }
    (min, max)
}

fn json_error(error: json::Error) -> GlbError {
    GlbError::Serialize(error.to_string())
}
