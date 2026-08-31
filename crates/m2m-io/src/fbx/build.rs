//! Building an [`FbxDocument`] from scene data.
//!
//! The other half of the writer: [`crate::fbx::encode`] turns a document into
//! bytes, and this turns geometry into a document. Splitting them means the
//! container rules and the scene rules are tested separately — a file Blender
//! refuses is either malformed (encode) or misdescribed (here), never both at
//! once.
//!
//! # What "correct" means here
//!
//! Not "our reader accepts it". Our reader shares a design with three.js, and
//! measured, neither notices four conformance details the format requires.
//! Blender's importer is the independent check, and it is what
//! `legacy/bench/fbx-conformance.ts` runs.

use crate::fbx::binary::{FbxDocument, FbxNode, FbxProperty};

/// FBX version this writes. 7500 and later use 64-bit node offsets.
/// A built clip's ids: its stack, its layer, and per channel the curve-node id
/// and its curve ids. Threaded to `definitions_node`, which must count them.
type ClipIds = (i64, i64, Vec<(i64, Vec<i64>)>);

const VERSION: u32 = 7700;

/// How a mesh's faces are supplied.
///
/// Two forms because there are two callers with genuinely different data, and
/// collapsing them would lose something either way.
pub enum Faces<'a> {
    /// Triangle corners, three per face, indexing the positions.
    ///
    /// For geometry we generated, where triangles are all there is.
    Triangles(&'a [u32]),
    /// An FBX `PolygonVertexIndex` array, used verbatim.
    ///
    /// Each face's last corner is bitwise-negated, which is how the format
    /// encodes variable-length faces. Passing it through unchanged is what
    /// keeps an imported mesh's **quads**: the reference rig's two meshes are
    /// 11,120 and 14,222 polygons, mostly quads, and rebuilding them from the
    /// triangulated form Blender reports 20,840 and 28,272 instead — every
    /// quad split. Artists notice.
    Polygons(&'a [i32]),
}

/// A mesh to write.
pub struct Mesh<'a> {
    /// Name as it should appear in the importing application.
    pub name: &'a str,
    /// Three floats per vertex.
    pub positions: &'a [f32],
    /// The faces.
    pub faces: Faces<'a>,
}

/// One bone of a skeleton.
pub struct Bone<'a> {
    /// Name as it should appear in the importing application.
    pub name: &'a str,
    /// Index into [`Scene::bones`] of this bone's parent, or `None` for a root.
    pub parent: Option<usize>,
    /// `Lcl Translation`, in the file's units.
    pub translation: [f64; 3],
    /// `Lcl Rotation`, in degrees.
    pub rotation: [f64; 3],
    /// `Lcl Scaling`.
    pub scale: [f64; 3],
    /// `PreRotation`, in degrees. Mixamo rigs use it heavily — 440 of 522
    /// models in the reference corpus carry one — so dropping it would move
    /// every joint.
    pub pre_rotation: [f64; 3],
}

/// One bone's influence over a mesh's vertices.
pub struct Cluster<'a> {
    /// Index into [`Scene::bones`].
    pub bone: usize,
    /// Original vertex ids this bone influences.
    pub indices: &'a [u32],
    /// Influence per index, parallel to `indices`.
    pub weights: &'a [f64],
    /// The mesh's global transform when the weights were painted, column-major.
    pub transform: [f64; 16],
    /// The bone's global transform at that moment, column-major.
    pub transform_link: [f64; 16],
}

/// A skin binding one mesh to a set of bones.
pub struct Skin<'a> {
    /// Index into [`Scene::meshes`].
    pub mesh: usize,
    /// One per influencing bone.
    pub clusters: &'a [Cluster<'a>],
}

/// One animated axis of one channel.
pub struct Curve<'a> {
    /// The FBX axis property: `d|X`, `d|Y` or `d|Z`.
    pub axis: &'a str,
    /// Key times in FBX ticks — 1/46186158000 of a second each.
    pub times: &'a [i64],
    /// One value per key, in the channel's own units (degrees for a rotation).
    pub values: &'a [f32],
    /// The value an importer uses where the curve does not reach.
    pub default: f64,
}

/// One animated property of one bone.
pub struct Channel<'a> {
    /// Index into [`Scene::bones`].
    ///
    /// Bones only for now: all 52 animated models in the reference rig are
    /// `LimbNode`s. Animating a mesh's own transform needs a target that can
    /// name either, and nothing asks for it yet.
    pub bone: usize,
    /// The property driven: `Lcl Translation`, `Lcl Rotation` or `Lcl Scaling`.
    pub property: &'a str,
    /// The curve node's name, which FBX abbreviates: `T`, `R` or `S`.
    pub kind: &'a str,
    /// One per animated axis. An axis with no curve is simply absent.
    pub curves: &'a [Curve<'a>],
}

/// One animation clip, becoming an `AnimationStack` and its layer.
///
/// # Why this is described in FBX's own terms
///
/// [`crate::fbx::animation`] hands back quaternion tracks with times in
/// seconds, which is what a player and a retargeter want. FBX stores Euler
/// curves with tick times, and converting back is both lossy and ambiguous —
/// many Euler triples give the same quaternion, and the reader also merges the
/// three axes onto one time array and can insert sub-keys for large steps.
///
/// So for O9, where an existing animation must come back as it went in, the
/// original curves are passed through unchanged. Same decision as
/// [`Faces::Polygons`], for the same reason: the semantic layer exists to be
/// used, not to be re-encoded.
pub struct Clip<'a> {
    /// The stack's name, e.g. `mixamo.com`.
    pub name: &'a str,
    /// Clip length in FBX ticks.
    pub duration: i64,
    /// The layer's name, e.g. `Layer0`.
    ///
    /// Carried because an importer can build its own name from it — Blender
    /// composes `armature|stack|layer`, so writing `Base Layer` where the
    /// source said `Layer0` renames the action and breaks anything matching on
    /// it.
    pub layer: &'a str,
    /// The animated channels.
    pub channels: &'a [Channel<'a>],
}

/// What to write.
pub struct Scene<'a> {
    /// Meshes, each becoming a `Geometry` and the `Model` that holds it.
    pub meshes: &'a [Mesh<'a>],
    /// Bones, each becoming a `Model`/`LimbNode` and its `NodeAttribute`.
    ///
    /// A bone's parent must appear earlier in this slice, so a single forward
    /// pass can emit connections — and a cycle cannot be expressed at all.
    pub bones: &'a [Bone<'a>],
    /// Skins, binding meshes to bones.
    pub skins: &'a [Skin<'a>],
    /// Animation clips.
    pub clips: &'a [Clip<'a>],
    /// FBX `TimeMode`, which fixes the frame rate the tick times are read at.
    ///
    /// **Not cosmetic.** Key times are stored in ticks, so a reader needs the
    /// frame rate to turn them into frames. Measured: omitting it made Blender
    /// read a 148-frame clip as 123.5 — the same keys at 25fps instead of 30,
    /// i.e. the animation plays 20% slow with every other number identical.
    ///
    /// 6 is 30fps, verified by writing it and checking Blender's frame range.
    pub time_mode: i32,
}

/// Builds a document from a scene.
///
/// # Panics
///
/// Panics if an index is out of range — a `Skin::mesh` with no such mesh, or a
/// `Cluster::bone` with no such bone. These are programming errors in the
/// caller, not file content, so they are not `Result`: a scene assembled with
/// a dangling index has no correct file to write.
pub fn build(scene: &Scene) -> FbxDocument {
    // Ids are allocated up front because connections reference them, and a
    // connection can point backwards (a cluster to a bone declared later).
    // Counting from a fixed base keeps a rebuild byte-identical, which makes a
    // diff between two runs mean something.
    let mut next = 1_000_000i64;
    let mut id = || {
        next += 1;
        next
    };

    let mesh_ids: Vec<(i64, i64)> = scene.meshes.iter().map(|_| (id(), id())).collect();
    let bone_ids: Vec<(i64, i64)> = scene.bones.iter().map(|_| (id(), id())).collect();
    let skin_ids: Vec<(i64, Vec<i64>)> = scene
        .skins
        .iter()
        .map(|skin| (id(), skin.clusters.iter().map(|_| id()).collect()))
        .collect();
    // Per clip: a stack, a layer, and for each channel a curve node plus one
    // curve per axis.
    let clip_ids: Vec<ClipIds> = scene
        .clips
        .iter()
        .map(|clip| {
            let stack = id();
            let layer = id();
            let channels = clip
                .channels
                .iter()
                .map(|channel| (id(), channel.curves.iter().map(|_| id()).collect()))
                .collect();
            (stack, layer, channels)
        })
        .collect();
    let document_id = id();

    let mut objects = Vec::new();
    let mut connections = Vec::new();

    for (mesh, &(geometry_id, model_id)) in scene.meshes.iter().zip(&mesh_ids) {
        objects.push(geometry_node(geometry_id, mesh));
        objects.push(mesh_model_node(model_id, mesh.name));
        connections.push(connection(geometry_id, model_id));
        connections.push(connection(model_id, 0));
    }

    for (bone, &(model_id, attribute_id)) in scene.bones.iter().zip(&bone_ids) {
        objects.push(bone_model_node(model_id, bone));
        objects.push(bone_attribute_node(attribute_id, bone.name));
        // The attribute is what marks the Model as a skeleton joint rather
        // than an empty transform; without it Blender builds no armature.
        connections.push(connection(attribute_id, model_id));
        let parent = bone.parent.map(|i| bone_ids[i].0).unwrap_or(0);
        connections.push(connection(model_id, parent));
    }

    for (skin, (skin_id, cluster_ids)) in scene.skins.iter().zip(&skin_ids) {
        let (geometry_id, _) = mesh_ids[skin.mesh];
        let mesh_name = scene.meshes[skin.mesh].name;
        objects.push(skin_node(*skin_id, mesh_name));
        connections.push(connection(*skin_id, geometry_id));

        for (cluster, &cluster_id) in skin.clusters.iter().zip(cluster_ids) {
            let bone = &scene.bones[cluster.bone];
            objects.push(cluster_node(cluster_id, bone.name, cluster));
            connections.push(connection(cluster_id, *skin_id));
            // The bone drives the cluster, so the Model is the CHILD here —
            // the direction that surprises everyone reading FBX connections.
            connections.push(connection(bone_ids[cluster.bone].0, cluster_id));
        }
    }

    for (clip, (stack_id, layer_id, channel_ids)) in scene.clips.iter().zip(&clip_ids) {
        objects.push(stack_node(*stack_id, clip));
        objects.push(layer_node(*layer_id, clip.layer));
        connections.push(connection(*layer_id, *stack_id));

        for (channel, (node_id, curve_ids)) in clip.channels.iter().zip(channel_ids) {
            objects.push(curve_node(*node_id, channel));
            connections.push(connection(*node_id, *layer_id));
            // Object-to-PROPERTY: the curve node drives one named property of
            // the bone, not the bone itself.
            connections.push(property_connection(
                *node_id,
                bone_ids[channel.bone].0,
                channel.property,
            ));

            for (curve, &curve_id) in channel.curves.iter().zip(curve_ids) {
                objects.push(animation_curve_node(curve_id, curve));
                connections.push(property_connection(curve_id, *node_id, curve.axis));
            }
        }
    }

    let definitions = definitions_node(scene, &skin_ids, &clip_ids);

    FbxDocument {
        version: VERSION,
        roots: vec![
            header_node(),
            node("FileId", vec![FbxProperty::Raw(vec![0; 16])], vec![]),
            node(
                "CreationTime",
                vec![FbxProperty::Str("1970-01-01 00:00:00:000".into())],
                vec![],
            ),
            node("Creator", vec![FbxProperty::Str(creator())], vec![]),
            global_settings_node(
                scene.time_mode,
                scene.clips.iter().map(|c| c.duration).max().unwrap_or(0),
            ),
            documents_node(document_id),
            node("References", vec![], vec![]),
            definitions,
            node("Objects", vec![], objects),
            node("Connections", vec![], connections),
            takes_node(scene.clips),
        ],
    }
}

/// `Name\0\x01Class`, which is how binary FBX pairs the two.
///
/// Blender splits on this separator and raises
/// `ValueError: not enough values to unpack` without it, so a name written
/// bare produces a file that will not open.
fn object_name(name: &str, class: &str) -> String {
    format!("{name}\u{0}\u{1}{class}")
}

fn node(name: &str, properties: Vec<FbxProperty>, children: Vec<FbxNode>) -> FbxNode {
    FbxNode {
        name: name.into(),
        properties,
        children,
        empty_scope: false,
    }
}

/// A node that declares a nested list and puts nothing in it.
///
/// The difference is invisible to our reader and decides whether assimp sees an
/// `AnimationLayer` at all — see `FbxNode::empty_scope`.
fn empty_scope_node(name: &str, properties: Vec<FbxProperty>) -> FbxNode {
    FbxNode {
        name: name.into(),
        properties,
        children: Vec::new(),
        empty_scope: true,
    }
}

fn creator() -> String {
    format!("mesh2motion {}", env!("CARGO_PKG_VERSION"))
}

/// A `P` entry inside a `Properties70` block.
fn property(name: &str, kind: &str, sub: &str, flags: &str, values: Vec<FbxProperty>) -> FbxNode {
    let mut properties = vec![
        FbxProperty::Str(name.into()),
        FbxProperty::Str(kind.into()),
        FbxProperty::Str(sub.into()),
        FbxProperty::Str(flags.into()),
    ];
    properties.extend(values);
    node("P", properties, vec![])
}

fn header_node() -> FbxNode {
    node(
        "FBXHeaderExtension",
        vec![],
        vec![
            node("FBXHeaderVersion", vec![FbxProperty::I32(1003)], vec![]),
            node("FBXVersion", vec![FbxProperty::I32(VERSION as i32)], vec![]),
            node("EncryptionType", vec![FbxProperty::I32(0)], vec![]),
            node(
                "CreationTimeStamp",
                vec![],
                vec![
                    node("Version", vec![FbxProperty::I32(1000)], vec![]),
                    node("Year", vec![FbxProperty::I32(1970)], vec![]),
                    node("Month", vec![FbxProperty::I32(1)], vec![]),
                    node("Day", vec![FbxProperty::I32(1)], vec![]),
                    node("Hour", vec![FbxProperty::I32(0)], vec![]),
                    node("Minute", vec![FbxProperty::I32(0)], vec![]),
                    node("Second", vec![FbxProperty::I32(0)], vec![]),
                    node("Millisecond", vec![FbxProperty::I32(0)], vec![]),
                ],
            ),
            node("Creator", vec![FbxProperty::Str(creator())], vec![]),
        ],
    )
}

/// Y-up, Z-forward, X-right — the axes three.js and Blender's importer expect.
fn global_settings_node(time_mode: i32, time_span_stop: i64) -> FbxNode {
    node(
        "GlobalSettings",
        vec![],
        vec![
            node("Version", vec![FbxProperty::I32(1000)], vec![]),
            node(
                "Properties70",
                vec![],
                vec![
                    property("UpAxis", "int", "Integer", "", vec![FbxProperty::I32(1)]),
                    property(
                        "UpAxisSign",
                        "int",
                        "Integer",
                        "",
                        vec![FbxProperty::I32(1)],
                    ),
                    property("FrontAxis", "int", "Integer", "", vec![FbxProperty::I32(2)]),
                    property(
                        "FrontAxisSign",
                        "int",
                        "Integer",
                        "",
                        vec![FbxProperty::I32(1)],
                    ),
                    property("CoordAxis", "int", "Integer", "", vec![FbxProperty::I32(0)]),
                    property(
                        "CoordAxisSign",
                        "int",
                        "Integer",
                        "",
                        vec![FbxProperty::I32(1)],
                    ),
                    property(
                        "UnitScaleFactor",
                        "double",
                        "Number",
                        "",
                        vec![FbxProperty::F64(1.0)],
                    ),
                    property(
                        "TimeMode",
                        "enum",
                        "",
                        "",
                        vec![FbxProperty::I32(time_mode)],
                    ),
                    property(
                        "TimeSpanStart",
                        "KTime",
                        "Time",
                        "",
                        vec![FbxProperty::I64(0)],
                    ),
                    property(
                        "TimeSpanStop",
                        "KTime",
                        "Time",
                        "",
                        vec![FbxProperty::I64(time_span_stop)],
                    ),
                ],
            ),
        ],
    )
}

fn documents_node(document_id: i64) -> FbxNode {
    node(
        "Documents",
        vec![],
        vec![
            node("Count", vec![FbxProperty::I32(1)], vec![]),
            node(
                "Document",
                vec![
                    FbxProperty::I64(document_id),
                    FbxProperty::Str(String::new()),
                    FbxProperty::Str("Scene".into()),
                ],
                vec![
                    node("Properties70", vec![], vec![]),
                    node("RootNode", vec![FbxProperty::I64(0)], vec![]),
                ],
            ),
        ],
    )
}

/// Declares how many of each object type follow.
///
/// Importers use this to size their tables; a count that disagrees with the
/// `Objects` block is a file that describes itself wrongly.
fn definitions_node(scene: &Scene, skin_ids: &[(i64, Vec<i64>)], clip_ids: &[ClipIds]) -> FbxNode {
    let object_type = |name: &str, count: usize| {
        node(
            "ObjectType",
            vec![FbxProperty::Str(name.into())],
            vec![node("Count", vec![FbxProperty::I32(count as i32)], vec![])],
        )
    };
    let geometries = scene.meshes.len();
    // Every mesh gets a Model and every bone gets one too, which is why this
    // is a sum rather than a mesh count.
    let models = scene.meshes.len() + scene.bones.len();
    let attributes = scene.bones.len();
    let deformers = skin_ids.len() + skin_ids.iter().map(|(_, c)| c.len()).sum::<usize>();
    let stacks = clip_ids.len();
    let layers = clip_ids.len();
    let curve_nodes: usize = clip_ids.iter().map(|(_, _, c)| c.len()).sum();
    let curves: usize = clip_ids
        .iter()
        .flat_map(|(_, _, channels)| channels.iter())
        .map(|(_, curves)| curves.len())
        .sum();
    let total =
        1 + geometries + models + attributes + deformers + stacks + layers + curve_nodes + curves;

    let mut children = vec![
        node("Version", vec![FbxProperty::I32(100)], vec![]),
        node("Count", vec![FbxProperty::I32(total as i32)], vec![]),
        object_type("GlobalSettings", 1),
    ];
    if geometries > 0 {
        children.push(object_type("Geometry", geometries));
    }
    if models > 0 {
        children.push(object_type("Model", models));
    }
    if attributes > 0 {
        children.push(object_type("NodeAttribute", attributes));
    }
    if deformers > 0 {
        children.push(object_type("Deformer", deformers));
    }
    if stacks > 0 {
        children.push(object_type("AnimationStack", stacks));
        children.push(object_type("AnimationLayer", layers));
        children.push(object_type("AnimationCurveNode", curve_nodes));
        children.push(object_type("AnimationCurve", curves));
    }
    node("Definitions", vec![], children)
}

/// The mesh itself: vertices, and the polygon corners that index them.
fn geometry_node(id: i64, mesh: &Mesh) -> FbxNode {
    let vertices: Vec<f64> = mesh.positions.iter().map(|&v| f64::from(v)).collect();

    // FBX marks a polygon's LAST corner by writing its index bitwise-negated,
    // which is how a flat array encodes variable-length faces.
    let polygon_indices: Vec<i32> = match &mesh.faces {
        Faces::Polygons(existing) => existing.to_vec(),
        Faces::Triangles(triangles) => triangles
            .chunks(3)
            .flat_map(|tri| {
                let last = tri.len() - 1;
                tri.iter()
                    .enumerate()
                    .map(move |(i, &v)| {
                        let v = v as i32;
                        if i == last {
                            !v
                        } else {
                            v
                        }
                    })
                    .collect::<Vec<i32>>()
            })
            .collect(),
    };

    node(
        "Geometry",
        vec![
            FbxProperty::I64(id),
            FbxProperty::Str(object_name(mesh.name, "Geometry")),
            FbxProperty::Str("Mesh".into()),
        ],
        vec![
            node("GeometryVersion", vec![FbxProperty::I32(124)], vec![]),
            node("Vertices", vec![FbxProperty::F64Array(vertices)], vec![]),
            node(
                "PolygonVertexIndex",
                vec![FbxProperty::I32Array(polygon_indices)],
                vec![],
            ),
        ],
    )
}

/// The scene node that holds a geometry and places it in the world.
fn mesh_model_node(id: i64, name: &str) -> FbxNode {
    node(
        "Model",
        vec![
            FbxProperty::I64(id),
            FbxProperty::Str(object_name(name, "Model")),
            FbxProperty::Str("Mesh".into()),
        ],
        vec![
            node("Version", vec![FbxProperty::I32(232)], vec![]),
            node(
                "Properties70",
                vec![],
                vec![property(
                    "Lcl Translation",
                    "Lcl Translation",
                    "",
                    "A",
                    vec![
                        FbxProperty::F64(0.0),
                        FbxProperty::F64(0.0),
                        FbxProperty::F64(0.0),
                    ],
                )],
            ),
        ],
    )
}

/// A skeleton joint.
///
/// `LimbNode` is what marks this Model as a bone rather than an empty
/// transform, and the reader treats `Root` the same way.
fn bone_model_node(id: i64, bone: &Bone) -> FbxNode {
    let vector = |name: &str, v: [f64; 3], flags: &str| {
        property(
            name,
            if name.starts_with("Lcl") {
                name
            } else {
                "Vector3D"
            },
            if name.starts_with("Lcl") {
                ""
            } else {
                "Vector"
            },
            flags,
            vec![
                FbxProperty::F64(v[0]),
                FbxProperty::F64(v[1]),
                FbxProperty::F64(v[2]),
            ],
        )
    };
    let mut properties = vec![vector("Lcl Translation", bone.translation, "A")];
    if bone.rotation != [0.0; 3] {
        properties.push(vector("Lcl Rotation", bone.rotation, "A"));
    }
    if bone.scale != [1.0; 3] {
        properties.push(vector("Lcl Scaling", bone.scale, "A"));
    }
    if bone.pre_rotation != [0.0; 3] {
        // Written before the local rotation and in the DEFAULT Euler order, per
        // the transform pipeline. Mixamo puts joint orientation here.
        properties.push(vector("PreRotation", bone.pre_rotation, ""));
        properties.push(property(
            "RotationActive",
            "bool",
            "",
            "",
            vec![FbxProperty::I32(1)],
        ));
    }

    node(
        "Model",
        vec![
            FbxProperty::I64(id),
            FbxProperty::Str(object_name(bone.name, "Model")),
            FbxProperty::Str("LimbNode".into()),
        ],
        vec![
            node("Version", vec![FbxProperty::I32(232)], vec![]),
            node("Properties70", vec![], properties),
            node("Shading", vec![FbxProperty::Bool(true)], vec![]),
            node(
                "Culling",
                vec![FbxProperty::Str("CullingOff".into())],
                vec![],
            ),
        ],
    )
}

/// The `NodeAttribute` that tells an importer a Model is a skeleton joint.
///
/// Measured, and NOT what I first assumed: removing the attribute's connection
/// to its Model leaves Blender still building a 65-bone armature, because
/// `LimbNode` on the Model is enough for it. So this is written because the
/// format specifies it and other importers may rely on it, not because any
/// reader here fails without it — our own Rust test is what catches its
/// absence. The claim that Blender "builds no armature at all" was wrong; the
/// reading that suggested it came from a file that was also carrying a
/// reversed cluster connection.
fn bone_attribute_node(id: i64, name: &str) -> FbxNode {
    node(
        "NodeAttribute",
        vec![
            FbxProperty::I64(id),
            FbxProperty::Str(object_name(name, "NodeAttribute")),
            FbxProperty::Str("LimbNode".into()),
        ],
        vec![node(
            "TypeFlags",
            vec![FbxProperty::Str("Skeleton".into())],
            vec![],
        )],
    )
}

/// The skin deformer that binds a geometry to its clusters.
fn skin_node(id: i64, mesh_name: &str) -> FbxNode {
    node(
        "Deformer",
        vec![
            FbxProperty::I64(id),
            FbxProperty::Str(object_name(&format!("Skin {mesh_name}"), "Deformer")),
            FbxProperty::Str("Skin".into()),
        ],
        vec![
            node("Version", vec![FbxProperty::I32(101)], vec![]),
            node("Link_DeformAcuracy", vec![FbxProperty::F64(50.0)], vec![]),
        ],
    )
}

/// One bone's weights over a mesh.
fn cluster_node(id: i64, bone_name: &str, cluster: &Cluster) -> FbxNode {
    let indices: Vec<i32> = cluster.indices.iter().map(|&i| i as i32).collect();
    let mut children = vec![
        node("Version", vec![FbxProperty::I32(100)], vec![]),
        node(
            "UserData",
            vec![
                FbxProperty::Str(String::new()),
                FbxProperty::Str(String::new()),
            ],
            vec![],
        ),
    ];
    // A bone bound to the skin but influencing nothing writes no arrays at
    // all — 40 of the reference rig's 129 clusters are exactly that, every one
    // a finger bone, and the reader guards for it the same way.
    if !indices.is_empty() {
        children.push(node(
            "Indexes",
            vec![FbxProperty::I32Array(indices)],
            vec![],
        ));
        children.push(node(
            "Weights",
            vec![FbxProperty::F64Array(cluster.weights.to_vec())],
            vec![],
        ));
    }
    children.push(node(
        "Transform",
        vec![FbxProperty::F64Array(cluster.transform.to_vec())],
        vec![],
    ));
    children.push(node(
        "TransformLink",
        vec![FbxProperty::F64Array(cluster.transform_link.to_vec())],
        vec![],
    ));

    node(
        "Deformer",
        vec![
            FbxProperty::I64(id),
            FbxProperty::Str(object_name(&format!("Cluster {bone_name}"), "SubDeformer")),
            FbxProperty::Str("Cluster".into()),
        ],
        children,
    )
}

/// An `AnimationStack`, which is what an importer turns into one action.
fn stack_node(id: i64, clip: &Clip) -> FbxNode {
    node(
        "AnimationStack",
        vec![
            FbxProperty::I64(id),
            FbxProperty::Str(object_name(clip.name, "AnimStack")),
            FbxProperty::Str(String::new()),
        ],
        vec![node(
            "Properties70",
            vec![],
            vec![
                // KTime, so the value is ticks rather than seconds.
                property(
                    "LocalStop",
                    "KTime",
                    "Time",
                    "",
                    vec![FbxProperty::I64(clip.duration)],
                ),
                property(
                    "ReferenceStop",
                    "KTime",
                    "Time",
                    "",
                    vec![FbxProperty::I64(clip.duration)],
                ),
            ],
        )],
    )
}

/// The single layer each stack gets.
///
/// FBX allows a stack to blend several, but the reader takes only the first —
/// as three.js does — so writing more than one would produce a file we cannot
/// read back.
/// The layer an animation's curve nodes hang from.
///
/// Written with an **empty scope**, which the reference export also does. A
/// layer written without one is invisible to assimp: its stack then has no
/// layers, and the file loads with the mesh and skeleton intact and not a
/// single keyframe. Blender and three.js read it either way, which is exactly
/// why this needed a third reader to catch.
fn layer_node(id: i64, name: &str) -> FbxNode {
    empty_scope_node(
        "AnimationLayer",
        vec![
            FbxProperty::I64(id),
            FbxProperty::Str(object_name(name, "AnimLayer")),
            FbxProperty::Str(String::new()),
        ],
    )
}

/// The curve node grouping one property's axes.
///
/// Its `Properties70` carries each axis's default, which is the value an
/// importer uses for an axis that has no curve at all.
fn curve_node(id: i64, channel: &Channel) -> FbxNode {
    let defaults: Vec<FbxNode> = channel
        .curves
        .iter()
        .map(|curve| {
            property(
                curve.axis,
                "Number",
                "",
                "A",
                vec![FbxProperty::F64(curve.default)],
            )
        })
        .collect();
    node(
        "AnimationCurveNode",
        vec![
            FbxProperty::I64(id),
            FbxProperty::Str(object_name(channel.kind, "AnimCurveNode")),
            FbxProperty::Str(String::new()),
        ],
        vec![node("Properties70", vec![], defaults)],
    )
}

/// One axis's keys.
///
/// `KeyValueFloat` is f32 and `KeyTime` is i64 ticks, which is what the format
/// specifies — not a narrowing we chose.
fn animation_curve_node(id: i64, curve: &Curve) -> FbxNode {
    node(
        "AnimationCurve",
        vec![
            FbxProperty::I64(id),
            FbxProperty::Str(object_name("", "AnimCurve")),
            FbxProperty::Str(String::new()),
        ],
        vec![
            node("Default", vec![FbxProperty::F64(curve.default)], vec![]),
            node("KeyVer", vec![FbxProperty::I32(4009)], vec![]),
            node(
                "KeyTime",
                vec![FbxProperty::I64Array(curve.times.to_vec())],
                vec![],
            ),
            node(
                "KeyValueFloat",
                vec![FbxProperty::F32Array(curve.values.to_vec())],
                vec![],
            ),
            // One flag for the whole curve: 24840 is the constant every
            // exporter in the reference corpus writes for linear keys.
            node(
                "KeyAttrFlags",
                vec![FbxProperty::I32Array(vec![24840])],
                vec![],
            ),
            node(
                "KeyAttrDataFloat",
                vec![FbxProperty::F32Array(vec![0.0, 0.0, 0.0, 0.0])],
                vec![],
            ),
            node(
                "KeyAttrRefCount",
                vec![FbxProperty::I32Array(vec![curve.times.len() as i32])],
                vec![],
            ),
        ],
    )
}

/// The `Takes` section, which names the clips a file contains.
///
/// Redundant with the animation stacks for readers that use those, but the
/// format carries it and some importers read it first.
fn takes_node(clips: &[Clip]) -> FbxNode {
    let mut children = vec![node(
        "Current",
        vec![FbxProperty::Str(
            clips.first().map(|c| c.name).unwrap_or("").into(),
        )],
        vec![],
    )];
    for clip in clips {
        children.push(node(
            "Take",
            vec![FbxProperty::Str(clip.name.into())],
            vec![
                node(
                    "FileName",
                    vec![FbxProperty::Str(format!("{}.tak", clip.name))],
                    vec![],
                ),
                node(
                    "LocalTime",
                    vec![FbxProperty::I64(0), FbxProperty::I64(clip.duration)],
                    vec![],
                ),
                node(
                    "ReferenceTime",
                    vec![FbxProperty::I64(0), FbxProperty::I64(clip.duration)],
                    vec![],
                ),
            ],
        ));
    }
    node("Takes", vec![], children)
}

/// An object-to-PROPERTY connection: `child` drives one named property.
fn property_connection(child: i64, parent: i64, property: &str) -> FbxNode {
    node(
        "C",
        vec![
            FbxProperty::Str("OP".into()),
            FbxProperty::I64(child),
            FbxProperty::I64(parent),
            FbxProperty::Str(property.into()),
        ],
        vec![],
    )
}

/// An object-to-object connection: `child` belongs to `parent`.
fn connection(child: i64, parent: i64) -> FbxNode {
    node(
        "C",
        vec![
            FbxProperty::Str("OO".into()),
            FbxProperty::I64(child),
            FbxProperty::I64(parent),
        ],
        vec![],
    )
}
