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
const VERSION: u32 = 7700;

/// A mesh to write.
pub struct Mesh<'a> {
    /// Name as it should appear in the importing application.
    pub name: &'a str,
    /// Three floats per vertex.
    pub positions: &'a [f32],
    /// Triangle corners, three per face, indexing `positions`.
    pub triangles: &'a [u32],
}

/// What to write.
pub struct Scene<'a> {
    /// Meshes, each becoming a `Geometry` and the `Model` that holds it.
    pub meshes: &'a [Mesh<'a>],
}

/// Builds a document from a scene.
pub fn build(scene: &Scene) -> FbxDocument {
    // Object ids only have to be unique and stable within the file. Counting
    // from a fixed base keeps a rebuild byte-identical, which makes a diff
    // between two runs mean something.
    let mut next_id = 1_000_000i64;
    let mut id = || {
        next_id += 1;
        next_id
    };

    let mut objects = Vec::new();
    let mut connections = Vec::new();
    let mut model_count = 0usize;

    for mesh in scene.meshes {
        let geometry_id = id();
        let model_id = id();
        model_count += 1;

        objects.push(geometry_node(geometry_id, mesh));
        objects.push(model_node(model_id, mesh.name));
        // Geometry hangs off its Model; the Model hangs off the scene root, 0.
        connections.push(connection(geometry_id, model_id));
        connections.push(connection(model_id, 0));
    }

    let definitions = definitions_node(scene.meshes.len(), model_count);

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
            global_settings_node(),
            documents_node(id()),
            node("References", vec![], vec![]),
            definitions,
            node("Objects", vec![], objects),
            node("Connections", vec![], connections),
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
fn global_settings_node() -> FbxNode {
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
fn definitions_node(geometry_count: usize, model_count: usize) -> FbxNode {
    let object_type = |name: &str, count: usize| {
        node(
            "ObjectType",
            vec![FbxProperty::Str(name.into())],
            vec![node("Count", vec![FbxProperty::I32(count as i32)], vec![])],
        )
    };
    node(
        "Definitions",
        vec![],
        vec![
            node("Version", vec![FbxProperty::I32(100)], vec![]),
            node(
                "Count",
                vec![FbxProperty::I32((geometry_count + model_count + 1) as i32)],
                vec![],
            ),
            object_type("GlobalSettings", 1),
            object_type("Geometry", geometry_count),
            object_type("Model", model_count),
        ],
    )
}

/// The mesh itself: vertices, and the polygon corners that index them.
fn geometry_node(id: i64, mesh: &Mesh) -> FbxNode {
    let vertices: Vec<f64> = mesh.positions.iter().map(|&v| f64::from(v)).collect();

    // FBX marks a polygon's LAST corner by writing its index bitwise-negated,
    // which is how a flat array encodes variable-length faces. Everything here
    // is a triangle, so every third corner is negated.
    let polygon_indices: Vec<i32> = mesh
        .triangles
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
        .collect();

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
fn model_node(id: i64, name: &str) -> FbxNode {
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
