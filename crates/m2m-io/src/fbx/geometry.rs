//! Mesh geometry extraction.
//!
//! Ported from `legacy/src/lib/io/fbx/GeometryParser.ts`, minus NURBS and morph
//! targets, which the rigging pipeline does not use.
//!
//! # Output shape
//!
//! Vertices are expanded **per polygon corner**, not per FBX vertex. FBX stores
//! normals and UVs per corner (`ByPolygonVertex`), so two faces meeting at a
//! vertex may disagree about its normal; a shared vertex cannot carry both.
//! This is what the legacy does too.
//!
//! That expansion is why [`MeshGeometry::vertex_source`] exists: skin weights
//! are indexed by *original* FBX vertex id, so binding them to the expanded
//! buffer needs the mapping back. The legacy calls the same step
//! `remapSkinIndices`.
//!
//! # Triangulation
//!
//! Measured on the reference rig: the two meshes contain **only triangles and
//! quads** — 172 tris and 14050 quads, and 1400 tris and 9720 quads. So the
//! legacy's earcut over a projected tangent plane is not needed here:
//!
//! - a triangle passes through,
//! - a quad splits along the diagonal whose two triangles both agree with the
//!   polygon's own normal — exact for a **planar** quad, and correct for the
//!   concave case a naive fan gets wrong. A bowtie, or a quad warped enough
//!   that Newell's average misrepresents it, satisfies neither diagonal; that
//!   is split arbitrarily and counted in the report,
//! - anything larger is fanned and **counted in the report**, so an
//!   approximation is never applied silently.

use crate::fbx::binary::{FbxNode, FbxProperty};
use crate::fbx::dom::{Object, Scene};
use crate::fbx::FbxError;
use glam::{DMat4, DVec3, EulerRot};

/// The mesh's offset from its Model node.
///
/// FBX lets a mesh sit at an offset from the node it hangs off, via
/// `GeometricTranslation`, `GeometricRotation` and `GeometricScaling` on the
/// Model. It is **not** part of the node transform and is not inherited by
/// children — it applies to the geometry alone.
///
/// This matters for rigging specifically: the skeleton is placed by node
/// transforms, so a mesh with a non-identity geometric offset that is not
/// applied lands somewhere the bones are not. Mixamo writes identity, which is
/// why it went unnoticed here; Maya and Max exports commonly do not.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GeometricTransform {
    /// Column-major transform applied to positions.
    pub matrix: DMat4,
}

impl Default for GeometricTransform {
    fn default() -> Self {
        Self {
            matrix: DMat4::IDENTITY,
        }
    }
}

impl GeometricTransform {
    /// True when this transform does nothing.
    pub fn is_identity(&self) -> bool {
        self.matrix.abs_diff_eq(DMat4::IDENTITY, 1e-12)
    }

    /// Reads the geometric offset from the Model a geometry hangs off.
    ///
    /// Returns the identity when the geometry has no Model parent or the Model
    /// declares no offset, which is the common case.
    pub fn for_geometry(scene: &Scene, geometry_id: i64) -> Self {
        let Some(model) = scene
            .parents_of(geometry_id, Some("Model"))
            .first()
            .and_then(|id| scene.object(*id))
        else {
            return Self::default();
        };

        let vec3 = |name: &str, fallback: f64| {
            model
                .property(name)
                .and_then(|p| p.as_vec3())
                .map(DVec3::from)
                .unwrap_or(DVec3::splat(fallback))
        };
        let translation = vec3("GeometricTranslation", 0.0);
        let rotation = vec3("GeometricRotation", 0.0);
        let scaling = vec3("GeometricScaling", 1.0);

        // FBX default rotation order is XYZ, applied as intrinsic rotations.
        let radians = rotation * (std::f64::consts::PI / 180.0);
        let quat = glam::DQuat::from_euler(EulerRot::XYZ, radians.x, radians.y, radians.z);

        Self {
            matrix: DMat4::from_scale_rotation_translation(scaling, quat, translation),
        }
    }
}

/// Most polygon corners one mesh may declare.
///
/// `PolygonVertexIndex` is bounded only by the reader's 256 MB per-property
/// inflate ceiling — 67 million i32 corners, from roughly a quarter of a
/// megabyte of deflate.
/// Every corner becomes an expanded vertex with a position, a normal and a UV,
/// so the output is tens of bytes per corner and nothing otherwise relates it
/// to the input size. The reference rig's two meshes are 84,816 and 62,520
/// corners; four million is a mesh of over a million triangles.
const MAX_CORNERS: usize = 4_194_304;

/// Most corners one polygon may have before it is dropped.
///
/// A single n-gon of N corners is fanned into N-2 triangles, so one absurd
/// polygon amplifies without any total being exceeded. Real geometry is
/// triangles and quads; a thousand-sided face is corruption, not a design.
const MAX_POLYGON_CORNERS: usize = 1024;

/// What a geometry parse had to approximate or discard.
///
/// Reported rather than logged: a caller that silently accepts an approximation
/// has no way to tell the user their mesh was altered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GeometryReport {
    /// Corners dropped because the mesh declared more than [`MAX_CORNERS`].
    pub corners_over_limit: usize,
    /// Polygons dropped for having more than [`MAX_POLYGON_CORNERS`] corners.
    pub polygons_over_corner_limit: usize,
    /// Polygons with more than four corners, triangulated by fanning.
    ///
    /// Correct for a convex polygon, wrong for a concave one. Non-zero means
    /// the mesh may need the earcut path the legacy uses.
    pub fanned_polygons: usize,
    /// Quads where neither diagonal produced two triangles agreeing with the
    /// polygon normal — a bowtie, or a quad warped enough that Newell's
    /// average does not represent it. Split arbitrarily.
    pub ambiguous_quads: usize,
    /// Polygons with fewer than three corners, which cannot form a triangle.
    pub degenerate_polygons: usize,
    /// Corners whose normal or UV index resolved out of range, filled with zero.
    ///
    /// Silently zero-filling is how a mesh comes back subtly wrong; counting it
    /// is what lets a caller say so.
    pub unresolved_attributes: usize,
    /// Layer elements dropped because their mapping type was unrecognised.
    ///
    /// Distinguishes "the file had no normals" from "the normals were in a
    /// form this parser does not know", which are otherwise identical.
    pub dropped_layers: usize,
}

/// A triangulated mesh with its per-corner attributes.
#[derive(Debug, Clone, Default)]
pub struct MeshGeometry {
    /// The Geometry object this was parsed from.
    ///
    /// Carried so a skin can confirm it is binding to the mesh it was painted
    /// on; vertex counts alone do not identify a mesh.
    pub id: i64,
    /// Positions, three floats per expanded vertex.
    pub positions: Vec<f32>,
    /// Triangle indices into the expanded vertices.
    ///
    /// Always the identity sequence `0..n`, because vertices are expanded per
    /// corner and nothing is shared. Kept so the type matches what a renderer
    /// expects rather than forcing every consumer to synthesise it.
    pub indices: Vec<u32>,
    /// Normals, three per expanded vertex, when the file carried them.
    pub normals: Option<Vec<f32>>,
    /// UVs, two per expanded vertex, when the file carried them.
    pub uvs: Option<Vec<f32>>,
    /// The original FBX vertex id behind each expanded vertex.
    ///
    /// Needed to bind skin weights, which are indexed by the original id.
    pub vertex_source: Vec<u32>,
    /// Vertices in the original FBX buffer, before per-corner expansion.
    ///
    /// The skin remap needs this to size its table and bounds-check cluster
    /// indices; `max(vertex_source) + 1` is only correct when every vertex
    /// takes part in a polygon, which is not guaranteed.
    pub source_vertex_count: usize,
    /// What had to be approximated.
    pub report: GeometryReport,
}

impl MeshGeometry {
    /// Number of expanded vertices.
    pub fn vertex_count(&self) -> usize {
        self.positions.len() / 3
    }

    /// Number of triangles.
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }
}

/// How a layer element maps its values onto the mesh.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mapping {
    /// One value per polygon corner.
    ByPolygonVertex,
    /// One value per polygon.
    ByPolygon,
    /// One value per original vertex.
    ByVertex,
    /// One value for the whole mesh.
    AllSame,
}

impl Mapping {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "ByPolygonVertex" => Some(Self::ByPolygonVertex),
            "ByPolygon" => Some(Self::ByPolygon),
            // FBX spells it "ByVertice"; some exporters write "ByVertex".
            // The reference rig uses BOTH ByPolygonVertex and ByVertice for
            // normals across its two meshes, so this is not hypothetical.
            "ByVertice" | "ByVertex" => Some(Self::ByVertex),
            "AllSame" => Some(Self::AllSame),
            _ => None,
        }
    }
}

/// A layer element — normals, UVs, colours — with its mapping.
struct Layer {
    values: Vec<f64>,
    indices: Vec<i64>,
    mapping: Mapping,
    /// True when `indices` redirects into `values`.
    indexed: bool,
    stride: usize,
}

impl Layer {
    /// The value for one corner, or `None` if the mapping resolves out of range.
    fn at(&self, corner: usize, polygon: usize, vertex: usize) -> Option<&[f64]> {
        let mut index = match self.mapping {
            Mapping::ByPolygonVertex => corner,
            Mapping::ByPolygon => polygon,
            Mapping::ByVertex => vertex,
            Mapping::AllSame => *self.indices.first().unwrap_or(&0) as usize,
        };
        // Deliberate divergence from the legacy `getData`, which applies the
        // indirection unconditionally and so computes `indices[indices[0]]` for
        // AllSame. The two agree whenever `indices[0] == 0`, which is the
        // normal case; this reading follows the FBX spec, where AllSame's index
        // IS the direct value. Do not "fix" it back without a file that needs it.
        if self.indexed && self.mapping != Mapping::AllSame {
            index = usize::try_from(*self.indices.get(index)?).ok()?;
        }
        let from = index.checked_mul(self.stride)?;
        let to = from.checked_add(self.stride)?;
        self.values.get(from..to)
    }
}

/// Reads a node's first property as a float array.
fn float_array(node: &FbxNode) -> Option<Vec<f64>> {
    node.properties.first().and_then(FbxProperty::as_f64_vec)
}

/// Reads a node's first property as an integer array.
fn int_array(node: &FbxNode) -> Option<Vec<i64>> {
    node.properties.first().and_then(FbxProperty::as_i64_vec)
}

/// Reads a child node's first property as a string.
fn child_str<'a>(node: &'a FbxNode, name: &str) -> Option<&'a str> {
    node.child(name)?.properties.first()?.as_str()
}

/// Builds a layer element from its FBX node.
fn read_layer(
    node: &FbxNode,
    values_key: &str,
    index_keys: &[&str],
    stride: usize,
    report: &mut GeometryReport,
) -> Option<Layer> {
    let Some(mapping) = child_str(node, "MappingInformationType").and_then(Mapping::parse) else {
        // An unrecognised mapping would otherwise drop the layer and look
        // exactly like a file that had no normals at all.
        report.dropped_layers += 1;
        return None;
    };
    let reference = child_str(node, "ReferenceInformationType").unwrap_or("Direct");
    let indexed = reference == "IndexToDirect" || reference == "Index";
    let Some(values) = node.child(values_key).and_then(float_array) else {
        report.dropped_layers += 1;
        return None;
    };

    // The index array's node name differs by element and by exporter: normals
    // use NormalIndex or NormalsIndex, UVs use UVIndex.
    let indices = index_keys
        .iter()
        .find_map(|k| node.child(k).and_then(int_array))
        .unwrap_or_default();

    Some(Layer {
        values,
        indices,
        mapping,
        indexed,
        stride,
    })
}

/// Newell's method: a polygon normal that is well-defined for non-planar faces.
fn polygon_normal(corners: &[[f64; 3]]) -> [f64; 3] {
    let mut n = [0.0f64; 3];
    for i in 0..corners.len() {
        let a = corners[i];
        let b = corners[(i + 1) % corners.len()];
        n[0] += (a[1] - b[1]) * (a[2] + b[2]);
        n[1] += (a[2] - b[2]) * (a[0] + b[0]);
        n[2] += (a[0] - b[0]) * (a[1] + b[1]);
    }
    n
}

/// Cross product of `(b - a)` and `(c - a)`.
fn triangle_normal(a: [f64; 3], b: [f64; 3], c: [f64; 3]) -> [f64; 3] {
    let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    [
        u[1] * v[2] - u[2] * v[1],
        u[2] * v[0] - u[0] * v[2],
        u[0] * v[1] - u[1] * v[0],
    ]
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Splits a polygon's corners into triangles, as offsets within the polygon.
///
/// See the module docs for why this is not earcut.
fn triangulate(corners: &[[f64; 3]], report: &mut GeometryReport) -> Vec<[usize; 3]> {
    match corners.len() {
        0..=2 => {
            report.degenerate_polygons += 1;
            Vec::new()
        }
        3 => vec![[0, 1, 2]],
        4 => {
            // Pick the diagonal whose two triangles both wind with the polygon.
            // For a concave quad one diagonal falls outside the shape, and its
            // triangles disagree with the polygon normal — which is what a
            // naive fan gets wrong.
            let n = polygon_normal(corners);
            // `>= 0.0`: three collinear corners give exactly zero, and a
            // degenerate triangle inside an otherwise good quad should not
            // reject a valid diagonal.
            let ok = |a: usize, b: usize, c: usize| {
                dot(triangle_normal(corners[a], corners[b], corners[c]), n) >= 0.0
            };
            if ok(0, 1, 2) && ok(0, 2, 3) {
                vec![[0, 1, 2], [0, 2, 3]]
            } else if ok(0, 1, 3) && ok(1, 2, 3) {
                vec![[0, 1, 3], [1, 2, 3]]
            } else {
                // Neither diagonal works: a bowtie, or a quad warped enough
                // that Newell's average does not represent it. Split anyway,
                // but say so — this is the one case where the result is a
                // guess.
                report.ambiguous_quads += 1;
                vec![[0, 1, 2], [0, 2, 3]]
            }
        }
        n => {
            // A fan is correct for a convex polygon and wrong for a concave
            // one. Counted so the caller knows an approximation was applied.
            report.fanned_polygons += 1;
            (1..n - 1).map(|i| [0, i, i + 1]).collect()
        }
    }
}

/// Extracts triangulated geometry from a `Geometry` object.
///
/// # Errors
///
/// Fails if the node has no vertex or polygon data, or if a polygon index
/// addresses a vertex that does not exist.
pub fn parse(object: &Object, pre_transform: GeometricTransform) -> Result<MeshGeometry, FbxError> {
    let node = &object.node;
    let identity = pre_transform.is_identity();
    let normal_matrix = pre_transform.matrix.inverse().transpose();

    let vertices = node
        .child("Vertices")
        .and_then(float_array)
        .ok_or(FbxError::Malformed {
            what: "Geometry",
            detail: "no Vertices array".into(),
        })?;
    let polygon_indices =
        node.child("PolygonVertexIndex")
            .and_then(int_array)
            .ok_or(FbxError::Malformed {
                what: "Geometry",
                detail: "no PolygonVertexIndex array".into(),
            })?;

    if vertices.len() % 3 != 0 {
        return Err(FbxError::Malformed {
            what: "Geometry",
            detail: format!(
                "{} vertex coordinates is not a multiple of 3",
                vertices.len()
            ),
        });
    }
    let vertex_count = vertices.len() / 3;

    let mut out = MeshGeometry {
        id: object.id,
        source_vertex_count: vertex_count,
        ..Default::default()
    };

    let normals = node.child("LayerElementNormal").and_then(|n| {
        read_layer(
            n,
            "Normals",
            &["NormalIndex", "NormalsIndex"],
            3,
            &mut out.report,
        )
    });
    let uvs = node
        .child("LayerElementUV")
        .and_then(|n| read_layer(n, "UV", &["UVIndex"], 2, &mut out.report));
    // Bound the work before any of it is sized from file content.
    let polygon_indices: &[i64] = if polygon_indices.len() > MAX_CORNERS {
        out.report.corners_over_limit = polygon_indices.len() - MAX_CORNERS;
        &polygon_indices[..MAX_CORNERS]
    } else {
        &polygon_indices[..]
    };

    // One polygon's corners, reset at each face boundary.
    let mut unresolved = 0usize;
    let mut oversized = false;
    let mut corner_ids: Vec<usize> = Vec::new();
    let mut corner_slots: Vec<usize> = Vec::new();
    let mut polygon = 0usize;

    for (slot, &raw) in polygon_indices.iter().enumerate() {
        // A negative index marks the polygon's last corner. The real id is the
        // bitwise complement: `-3` closes a face on vertex 2.
        let (id, last) = if raw < 0 { (!raw, true) } else { (raw, false) };
        let id = usize::try_from(id).map_err(|_| FbxError::Malformed {
            what: "PolygonVertexIndex",
            detail: format!("negative vertex id {id}"),
        })?;
        if id >= vertex_count {
            return Err(FbxError::Malformed {
                what: "PolygonVertexIndex",
                detail: format!("vertex {id} of {vertex_count}"),
            });
        }

        corner_ids.push(id);
        corner_slots.push(slot);

        // A polygon past the corner limit is abandoned, but its remaining
        // corners still have to be consumed to find the face boundary — and it
        // is counted once, as one polygon, not once per block of corners.
        if !oversized && corner_ids.len() > MAX_POLYGON_CORNERS {
            oversized = true;
            out.report.polygons_over_corner_limit += 1;
            corner_ids.clear();
            corner_slots.clear();
        }
        if oversized {
            corner_ids.clear();
            corner_slots.clear();
            if last {
                oversized = false;
                polygon += 1;
            }
            continue;
        }

        if !last {
            continue;
        }

        let positions: Vec<[f64; 3]> = corner_ids
            .iter()
            .map(|&v| [vertices[v * 3], vertices[v * 3 + 1], vertices[v * 3 + 2]])
            .collect();

        for tri in triangulate(&positions, &mut out.report) {
            for &c in &tri {
                let base = out.positions.len() / 3;
                let p = DVec3::from(positions[c]);
                let p = if identity {
                    p
                } else {
                    pre_transform.matrix.transform_point3(p)
                };
                out.positions.extend([p.x as f32, p.y as f32, p.z as f32]);
                out.vertex_source.push(corner_ids[c] as u32);
                out.indices.push(base as u32);

                if let Some(layer) = &normals {
                    let resolved = layer.at(corner_slots[c], polygon, corner_ids[c]);
                    if resolved.is_none() {
                        unresolved += 1;
                    }
                    let v = resolved.unwrap_or(&[0.0, 0.0, 0.0]);
                    let n = DVec3::new(v[0], v[1], v[2]);
                    // Normals transform by the inverse transpose, not the
                    // matrix: a non-uniform scale would otherwise tilt them.
                    let n = if identity {
                        n
                    } else {
                        normal_matrix.transform_vector3(n).normalize_or_zero()
                    };
                    out.normals
                        .get_or_insert_with(Vec::new)
                        .extend([n.x as f32, n.y as f32, n.z as f32]);
                }
                if let Some(layer) = &uvs {
                    let resolved = layer.at(corner_slots[c], polygon, corner_ids[c]);
                    if resolved.is_none() {
                        unresolved += 1;
                    }
                    let v = resolved.unwrap_or(&[0.0, 0.0]);
                    out.uvs
                        .get_or_insert_with(Vec::new)
                        .extend(v.iter().map(|&x| x as f32));
                }
            }
        }

        corner_ids.clear();
        corner_slots.clear();
        polygon += 1;
    }

    if !corner_ids.is_empty() {
        // A polygon list must end on a negative index. Trailing corners mean
        // the array was cut, and accepting them would silently drop a face.
        return Err(FbxError::Malformed {
            what: "PolygonVertexIndex",
            detail: format!(
                "{} trailing corners with no closing index",
                corner_ids.len()
            ),
        });
    }

    out.report.unresolved_attributes = unresolved;
    Ok(out)
}
