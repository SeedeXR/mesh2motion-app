//! The FBX document model: connections, objects, and flattened properties.
//!
//! This is the semantic layer both readers deliberately deferred to. The binary
//! and ASCII readers produce a faithful node tree; this turns that tree into
//! the thing the rest of the pipeline actually wants — objects addressable by
//! id, a connection graph, and `Properties70` entries as named values.
//!
//! Doing it once here, rather than in each reader, is why the two readers were
//! allowed to stay simple. The legacy implements this reshaping twice, once in
//! `BinaryParser.parseSubNode` and again in `TextParser`.
//!
//! # Scope
//!
//! Measured on `legacy/src/lib/io/fbx/FBXTreeParser.ts`: of its 1574 lines of
//! method code, **319 are textures and materials and a further 218 are lights
//! and cameras** — about 46% once material parameter parsing is counted. None
//! of that belongs in `m2m-io`, which exists to read geometry and rigs
//! (`memory/architecture.md` §2); the viewport loads materials itself. Only the
//! rigging path is ported.

use crate::fbx::binary::{split_object_name, FbxDocument, FbxNode, FbxProperty};
use std::collections::HashMap;

/// Rewrites a leading `Lcl ` to `Lcl_` so the name is a usable identifier.
///
/// Prefix only, matching the legacy's `indexOf('Lcl ') === 0`. A global replace
/// would also rewrite something like `Maya|Lcl Offset`, which the legacy leaves
/// alone.
fn normalise_name(name: &str) -> String {
    match name.strip_prefix("Lcl ") {
        Some(rest) => format!("Lcl_{rest}"),
        None => name.to_string(),
    }
}

/// One end of a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// The object at the other end.
    pub id: i64,
    /// For an object-to-property connection, the property name it targets.
    ///
    /// `None` for a plain object-to-object connection.
    ///
    /// Normalised the same way as [`Object::properties`] keys, so
    /// `scene.object(link.id)?.property(link.property.as_deref()?)` resolves.
    /// Without that they disagree on exactly the `Lcl ` channels — 215 of the
    /// 666 connections in the reference rig are object-to-property, and the
    /// animation path is the consumer that walks them back to a property.
    pub property: Option<String>,
}

/// The parents and children of one object.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Links {
    /// Objects this one is connected *to*.
    pub parents: Vec<Link>,
    /// Objects connected *to* this one.
    pub children: Vec<Link>,
}

/// A `Properties70` entry: FBX's typed, named property.
#[derive(Debug, Clone, PartialEq)]
pub struct TypedProperty {
    /// FBX type name, e.g. `int`, `double`, `Lcl Translation`, `KString`.
    pub type_name: String,
    /// Secondary type, often empty.
    pub type2: String,
    /// Flags, e.g. `A` for animatable.
    pub flags: String,
    /// The value or values.
    pub values: Vec<FbxProperty>,
}

impl TypedProperty {
    /// The single value as a float, if there is exactly one and it is numeric.
    pub fn as_f64(&self) -> Option<f64> {
        match self.values.as_slice() {
            [one] => one.as_f64(),
            _ => None,
        }
    }

    /// The single value as an integer.
    pub fn as_i64(&self) -> Option<i64> {
        match self.values.as_slice() {
            [one] => one.as_i64(),
            _ => None,
        }
    }

    /// The single value as a string.
    pub fn as_str(&self) -> Option<&str> {
        match self.values.as_slice() {
            [one] => one.as_str(),
            _ => None,
        }
    }

    /// The value as three floats — a colour, a vector, or a local transform.
    pub fn as_vec3(&self) -> Option<[f64; 3]> {
        match self.values.as_slice() {
            [x, y, z] => Some([x.as_f64()?, y.as_f64()?, z.as_f64()?]),
            _ => None,
        }
    }
}

/// An addressable FBX object.
#[derive(Debug, Clone)]
pub struct Object {
    /// The object's id, unique within the document.
    pub id: i64,
    /// Node name: `Model`, `Geometry`, `Deformer`, `AnimationCurve`, and so on.
    pub kind: String,
    /// Object name, with the class prefix or suffix removed.
    pub name: String,
    /// Subclass from the third attribute: `Mesh`, `LimbNode`, `Skin`, `Cluster`.
    pub subclass: String,
    /// Flattened `Properties70` entries, by name.
    pub properties: HashMap<String, TypedProperty>,
    /// The raw node, for the per-kind parsers that come next.
    pub node: FbxNode,
}

impl Object {
    /// A `Properties70` entry by name.
    pub fn property(&self, name: &str) -> Option<&TypedProperty> {
        self.properties.get(name)
    }
}

/// An FBX document with its objects and relationships resolved.
#[derive(Debug, Clone, Default)]
pub struct Scene {
    /// Version from the file header.
    pub version: u32,
    /// Objects by id.
    pub objects: HashMap<i64, Object>,
    /// Connections by object id.
    pub links: HashMap<i64, Links>,
}

impl Scene {
    /// Builds the model from a parsed document.
    ///
    /// Never fails: a document missing `Objects` or `Connections` yields an
    /// empty scene rather than an error, matching the legacy behaviour of
    /// checking `'Connections' in fbxTree` before reading it. Whether an empty
    /// scene is acceptable is the caller's judgement, not the parser's.
    pub fn from_document(doc: FbxDocument) -> Self {
        // Consumes the document rather than borrowing it. Cloning each object
        // node duplicated every vertex, index, normal and weight array for the
        // lifetime of the scene — roughly doubling peak memory on a large mesh,
        // since the document is normally still alive alongside it.
        let links = collect_links(&doc);
        Self {
            version: doc.version,
            objects: collect_objects(doc),
            links,
        }
    }

    /// An object by id.
    pub fn object(&self, id: i64) -> Option<&Object> {
        self.objects.get(&id)
    }

    /// Every object of a given kind, in ascending id order.
    ///
    /// Sorted so iteration is deterministic: `HashMap` order varies per run,
    /// and a rig built in a different bone order is a different rig.
    pub fn objects_of_kind(&self, kind: &str) -> Vec<&Object> {
        let mut found: Vec<&Object> = self.objects.values().filter(|o| o.kind == kind).collect();
        found.sort_by_key(|o| o.id);
        found
    }

    /// Ids connected *to* `id`, optionally filtered by object kind.
    pub fn children_of(&self, id: i64, kind: Option<&str>) -> Vec<i64> {
        self.links
            .get(&id)
            .map(|l| {
                l.children
                    .iter()
                    .map(|c| c.id)
                    .filter(|c| match kind {
                        Some(k) => self.objects.get(c).is_some_and(|o| o.kind == k),
                        None => true,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Ids this object connects *to*, optionally filtered by kind.
    pub fn parents_of(&self, id: i64, kind: Option<&str>) -> Vec<i64> {
        self.links
            .get(&id)
            .map(|l| {
                l.parents
                    .iter()
                    .map(|p| p.id)
                    .filter(|p| match kind {
                        Some(k) => self.objects.get(p).is_some_and(|o| o.kind == k),
                        None => true,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Indexes every node under `Objects` by its id.
fn collect_objects(doc: FbxDocument) -> HashMap<i64, Object> {
    let mut out = HashMap::new();
    let Some(objects) = doc.roots.into_iter().find(|r| r.name == "Objects") else {
        return out;
    };

    let mut skipped = 0usize;
    for node in objects.children {
        // Object attributes are the first three properties: id, name, subclass.
        let Some(id) = node.properties.first().and_then(FbxProperty::as_i64) else {
            // FBX 6.x-style objects have no numeric id. Counted rather than
            // dropped in silence: an empty scene from a file that parsed fine
            // is otherwise indistinguishable from a file with no objects.
            skipped += 1;
            continue;
        };
        let raw_name = node
            .properties
            .get(1)
            .and_then(FbxProperty::as_str)
            .unwrap_or_default();
        let subclass = node
            .properties
            .get(2)
            .and_then(FbxProperty::as_str)
            .unwrap_or_default()
            .to_string();
        // The two formats encode name and class in opposite orders.
        let (name, _class) = split_object_name(raw_name);
        let name = name.to_string();
        let properties = flatten_properties(&node);
        let kind = node.name.clone();

        if let Some(existing) = out.insert(
            id,
            Object {
                id,
                kind,
                name,
                subclass,
                properties,
                node,
            },
        ) {
            // The legacy keys objects per kind (`Objects.Model[id]`), so an id
            // shared between two kinds keeps both. Flat keying is the right
            // shape for a connection graph, which is also id-keyed, but it must
            // not lose an object without saying so.
            let replaced: &Object = &existing;
            debug_assert!(
                false,
                "duplicate object id {id}: {} replaced by {}",
                replaced.kind, out[&id].kind
            );
        }
    }

    debug_assert_eq!(skipped, 0, "{skipped} Objects children had no numeric id");
    out
}

/// Flattens a node's `Properties70` block into named, typed values.
///
/// Each entry is a `P` node whose properties are
/// `[name, type, type2, flags, value...]`. A colour or vector carries three
/// values; everything else carries one.
fn flatten_properties(node: &FbxNode) -> HashMap<String, TypedProperty> {
    let mut out = HashMap::new();
    let Some(block) = node.child("Properties70") else {
        return out;
    };

    for p in block.children_named("P") {
        let name = match p.properties.first().and_then(FbxProperty::as_str) {
            Some(n) => n,
            None => continue,
        };
        let field = |i: usize| {
            p.properties
                .get(i)
                .and_then(FbxProperty::as_str)
                .unwrap_or_default()
                .to_string()
        };
        out.insert(
            normalise_name(name),
            TypedProperty {
                type_name: normalise_name(&field(1)),
                type2: field(2),
                flags: field(3),
                // Colour, vector and Lcl types carry three components; every
                // other type carries one. Taking everything from index 4
                // regardless means an exporter that appends a trailing field to
                // a scalar yields two values, and the accessors — which match
                // on a single value — then report the property as absent.
                values: {
                    let ty = normalise_name(&field(1));
                    let wanted =
                        if matches!(ty.as_str(), "Color" | "ColorRGB" | "Vector" | "Vector3D")
                            || ty.starts_with("Lcl_")
                        {
                            3
                        } else {
                            1
                        };
                    p.properties.iter().skip(4).take(wanted).cloned().collect()
                },
            },
        );
    }
    out
}

/// Builds the connection graph from the `Connections` section.
fn collect_links(doc: &FbxDocument) -> HashMap<i64, Links> {
    let mut out: HashMap<i64, Links> = HashMap::new();
    let Some(connections) = doc.root("Connections") else {
        return out;
    };

    for c in connections.children_named("C") {
        // `C` properties are [type, from, to, propertyName?]. The type — "OO"
        // for object-to-object, "OP" for object-to-property — is what decides
        // whether the fourth field is present.
        let from = c.properties.get(1).and_then(FbxProperty::as_i64);
        let to = c.properties.get(2).and_then(FbxProperty::as_i64);
        let (Some(from), Some(to)) = (from, to) else {
            continue;
        };
        let property = c
            .properties
            .get(3)
            .and_then(FbxProperty::as_str)
            .map(normalise_name);

        out.entry(from).or_default().parents.push(Link {
            id: to,
            property: property.clone(),
        });
        out.entry(to)
            .or_default()
            .children
            .push(Link { id: from, property });
    }
    out
}
