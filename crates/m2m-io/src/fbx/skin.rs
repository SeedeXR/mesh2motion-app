//! Skin clusters: which bones deform which vertices, and by how much.
//!
//! Ported from the deformer path in `legacy/src/lib/io/fbx/FBXTreeParser.ts`
//! (`parseDeformers`, `parseSkeleton`, `bindSkeleton`).
//!
//! # The bind matrix, and how this differs from the legacy
//!
//! Each cluster carries two matrices: `TransformLink`, the bone's global
//! transform when the weights were painted, and `Transform`, the mesh's global
//! transform at that same moment.
//!
//! The legacy uses only `inverse(TransformLink)` for its bone inverses and
//! supplies the mesh transform separately, as three.js's `bindMatrix`, taken
//! from the reconstructed scene graph:
//! `model.bind(new Skeleton(bones, boneInverses), model.matrixWorld)`.
//!
//! This port folds them into one matrix per cluster,
//! `inverse(TransformLink) * Transform`, which is the same composition. It is
//! preferred because `Transform` is what the exporter *recorded* at bind time,
//! whereas `model.matrixWorld` depends on rebuilding the scene graph
//! identically. Measured on the reference rig: **all 129 clusters carry a
//! non-identity `Transform`** (worst component deviation 179.9), so this is not
//! a distinction without a difference — dropping it would misplace every mesh.
//!
//! At bind time the bone matrix *is* `TransformLink`, so
//! `TransformLink · inverse(TransformLink) · Transform · v` reduces to
//! `Transform · v`, the vertex's world position. That is the check that the
//! composition is the right way round.

use crate::fbx::binary::FbxProperty;
use crate::fbx::dom::{Object, Scene};
use crate::fbx::geometry::MeshGeometry;
use crate::fbx::FbxError;
use glam::DMat4;
use m2m_core::skinning::{SkinWeights, MAX_INFLUENCES};

/// What binding had to drop or approximate.
///
/// Every field counts data the file contained and the parser could not use.
/// All zero means nothing was lost. Vertices left unweighted are not counted
/// here — they are listed individually in [`SkinWeights::fallback_vertices`],
/// and a second count of the same thing in different units is how a report
/// starts lying.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SkinReport {
    /// Clusters with no bone Model in the file. Their influences are lost.
    pub clusters_without_bone: usize,
    /// Clusters whose index and weight arrays disagreed in length.
    ///
    /// Truncated to the shorter of the two rather than dropped, since the
    /// leading pairs are still meaningful.
    pub mismatched_arrays: usize,
    /// Clusters whose `Indexes` or `Weights` node was present but unreadable.
    ///
    /// Distinct from a cluster with no `Indexes` node at all, which is a bone
    /// that legitimately influences nothing — 40 of the 129 clusters on the
    /// reference rig are exactly that. Without the distinction a corrupt array
    /// is indistinguishable from an empty one and a bone's whole influence set
    /// disappears silently.
    pub undecodable_arrays: usize,
    /// Index/weight pairs whose vertex index was negative or beyond `u32`.
    ///
    /// Dropped as a pair, never as a lone index, so the remaining weights stay
    /// attached to the vertices they were painted on.
    pub unusable_indices: usize,
    /// Bind matrices that were absent, the wrong length, or unusable.
    ///
    /// Replaced with the identity, which misplaces the mesh — on the reference
    /// rig every cluster's `Transform` is non-identity, worst component
    /// deviation 179.9 — so this is never a harmless default.
    pub matrices_defaulted: usize,
    /// Vertices influenced by more bones than the GPU limit allows.
    ///
    /// The smallest influences are discarded and the rest renormalised.
    pub vertices_over_influence_limit: usize,
}

/// One bone's influence over a set of vertices.
#[derive(Debug, Clone)]
pub struct Cluster {
    /// The cluster object's id.
    pub id: i64,
    /// The Model this cluster drives.
    pub bone_id: i64,
    /// Original FBX vertex ids this cluster influences.
    pub indices: Vec<u32>,
    /// Influence per index, parallel to `indices`.
    pub weights: Vec<f64>,
    /// The bone's global transform when the weights were painted.
    pub transform_link: DMat4,
    /// The mesh's global transform at that moment.
    pub transform: DMat4,
}

impl Cluster {
    /// Maps a mesh-local vertex into this bone's space at bind time.
    ///
    /// See the module docs for why this composes both matrices where the legacy
    /// splits them.
    pub fn inverse_bind(&self) -> DMat4 {
        self.transform_link.inverse() * self.transform
    }
}

/// A skin: the clusters binding one geometry to a set of bones.
#[derive(Debug, Clone)]
pub struct Skin {
    /// The Skin deformer's id.
    pub id: i64,
    /// The geometry this skin deforms.
    pub geometry_id: i64,
    /// One per influencing bone.
    pub clusters: Vec<Cluster>,
    /// What parsing had to drop.
    pub report: SkinReport,
}

/// Reads a 4x4 matrix stored as 16 column-major floats.
///
/// `None` for an absent field, a field that is not 16 floats, or values that
/// are not finite. When `invertible`, a singular matrix is rejected too:
/// glam is built without `glam-assert`, so `DMat4::inverse` on a zero
/// determinant does not panic — it returns all-NaN, and that NaN would spread
/// into every vertex the bone touches with nothing to show it happened.
fn matrix(object: &Object, field: &str, invertible: bool) -> Option<DMat4> {
    let values = object
        .node
        .child(field)?
        .properties
        .first()
        .and_then(FbxProperty::as_f64_vec)?;
    let array: [f64; 16] = values.try_into().ok()?;
    let m = DMat4::from_cols_array(&array);
    if !m.is_finite() || (invertible && m.determinant() == 0.0) {
        return None;
    }
    Some(m)
}

/// Reads every skin in the scene.
///
/// Returns the skins alongside the number that were **skipped for having no
/// geometry**. A skin that deforms nothing cannot be bound to anything, but
/// silently returning fewer skins would leave the caller unable to tell a
/// truncated `Connections` section from a genuinely unskinned mesh — and that
/// drop is larger than any counted inside [`SkinReport`].
pub fn parse_all(scene: &Scene) -> (Vec<Skin>, usize) {
    let candidates: Vec<&Object> = scene
        .objects_of_kind("Deformer")
        .into_iter()
        .filter(|d| d.subclass == "Skin")
        .collect();
    let total = candidates.len();
    let mut skins: Vec<Skin> = candidates
        .into_iter()
        .filter_map(|s| parse(scene, s))
        .collect();
    skins.sort_by_key(|s| s.id);
    let skipped = total - skins.len();
    (skins, skipped)
}

/// Reads one skin and its clusters.
fn parse(scene: &Scene, skin: &Object) -> Option<Skin> {
    // A skin hangs off exactly one geometry. The legacy warns and takes the
    // first when there is more than one; there is no sensible alternative.
    let geometry_id = *scene.parents_of(skin.id, Some("Geometry")).first()?;

    let mut report = SkinReport::default();
    let mut clusters = Vec::new();

    for cluster_id in scene.children_of(skin.id, Some("Deformer")) {
        let Some(cluster) = scene.object(cluster_id) else {
            continue;
        };
        if cluster.subclass != "Cluster" {
            continue;
        }

        // A cluster with no bone has nothing to drive. The legacy drops these
        // too, warning that their influences are lost.
        let Some(&bone_id) = scene.children_of(cluster_id, Some("Model")).first() else {
            report.clusters_without_bone += 1;
            continue;
        };

        // Borrowed, not cloned: `as_*_vec` already allocates the Vec it
        // returns, and these arrays run to thousands of entries per cluster.
        let field = |name: &str| cluster.node.child(name)?.properties.first();

        // An absent node and an unreadable one mean different things: the
        // first is a bone that influences nothing, the second is influence
        // that was lost. Collapsing them hides the loss.
        let mut undecodable = false;
        let mut read =
            |name: &str, decode: &dyn Fn(&FbxProperty) -> Option<Vec<f64>>| match field(name) {
                None => Vec::new(),
                Some(p) => decode(p).unwrap_or_else(|| {
                    undecodable = true;
                    Vec::new()
                }),
            };
        let raw_indices = read("Indexes", &|p| {
            p.as_i64_vec()
                .map(|v| v.into_iter().map(|i| i as f64).collect())
        });
        let raw_weights = read("Weights", &FbxProperty::as_f64_vec);
        if undecodable {
            report.undecodable_arrays += 1;
        }

        if raw_indices.len() != raw_weights.len() {
            report.mismatched_arrays += 1;
        }
        // Only the leading pairs are meaningful; a trailing index with no
        // weight, or the reverse, says nothing.
        let keep = raw_indices.len().min(raw_weights.len());

        // Pair BEFORE discarding. An index that is not a valid u32 has to take
        // its weight with it: dropping the index alone would slide every later
        // weight onto the wrong vertex, and the length check above would then
        // report a merely ragged array while the surviving weights silently
        // deform the wrong part of the mesh.
        let (indices, weights): (Vec<u32>, Vec<f64>) = raw_indices[..keep]
            .iter()
            .zip(&raw_weights[..keep])
            .filter_map(|(&i, &w)| {
                let exact = i as i64 as f64 == i;
                u32::try_from(i as i64)
                    .ok()
                    .filter(|_| exact)
                    .map(|i| (i, w))
            })
            .unzip();
        report.unusable_indices += keep - indices.len();

        let mut bind_matrix = |name: &str, invertible: bool| match matrix(cluster, name, invertible)
        {
            Some(m) => m,
            None => {
                report.matrices_defaulted += 1;
                DMat4::IDENTITY
            }
        };
        let transform_link = bind_matrix("TransformLink", true);
        let transform = bind_matrix("Transform", false);

        clusters.push(Cluster {
            id: cluster_id,
            bone_id,
            indices,
            weights,
            transform_link,
            transform,
        });
    }

    // Deterministic order: the cluster index becomes a bone index downstream,
    // and a rig whose bones are numbered differently between runs is a
    // different rig.
    clusters.sort_by_key(|c| c.id);

    Some(Skin {
        id: skin.id,
        geometry_id,
        clusters,
        report,
    })
}

impl Skin {
    /// Bones influencing this skin, in cluster order.
    ///
    /// The position in this list is the bone index used by [`Self::bind`].
    pub fn bone_ids(&self) -> Vec<i64> {
        self.clusters.iter().map(|c| c.bone_id).collect()
    }

    /// Binds this skin's weights onto an expanded mesh.
    ///
    /// FBX weights are indexed by *original* vertex, while geometry is expanded
    /// per polygon corner so that per-corner normals and UVs can differ. The
    /// mapping back is [`MeshGeometry::vertex_source`]; the legacy calls the
    /// same step `remapSkinIndices`.
    ///
    /// # Errors
    ///
    /// Fails if the mesh is not the geometry this skin deforms, if the skin has
    /// no usable clusters, or if a cluster references a vertex outside the mesh.
    pub fn bind(&self, mesh: &MeshGeometry) -> Result<(SkinWeights, SkinReport), FbxError> {
        // Vertex counts do not identify a mesh. Two geometries of the same size
        // would bind silently with entirely wrong weights, so check identity.
        if mesh.id != self.geometry_id {
            return Err(FbxError::Malformed {
                what: "skin",
                detail: format!(
                    "skin deforms geometry {} but was handed geometry {}",
                    self.geometry_id, mesh.id
                ),
            });
        }
        // With no clusters every corner would take the fallback and be pinned
        // to bone 0 — an index into an empty bone list. Returning Ok here hands
        // the caller a rig that cannot be posed.
        if self.clusters.is_empty() {
            return Err(FbxError::Malformed {
                what: "skin",
                detail: "no usable clusters, so there is no bone to weight to".into(),
            });
        }
        // Gather influences per ORIGINAL vertex first, then expand. Doing it the
        // other way round would repeat the same lookup for every corner sharing
        // a vertex.
        let mut per_source: Vec<Vec<(u16, f32)>> = vec![Vec::new(); mesh.source_vertex_count];

        for (bone, cluster) in self.clusters.iter().enumerate() {
            let bone = u16::try_from(bone).map_err(|_| FbxError::Malformed {
                what: "skin",
                detail: format!(
                    "{} clusters exceeds the bone index range",
                    self.clusters.len()
                ),
            })?;
            for (&vertex, &weight) in cluster.indices.iter().zip(cluster.weights.iter()) {
                let vertex = vertex as usize;
                if vertex >= mesh.source_vertex_count {
                    return Err(FbxError::Malformed {
                        what: "skin cluster",
                        detail: format!(
                            "vertex {vertex} of {} — this skin does not belong to this mesh",
                            mesh.source_vertex_count
                        ),
                    });
                }
                // Narrow BEFORE testing. A weight of 1e-45 is positive as an
                // f64 and zero as an f32, so testing the f64 would admit an
                // influence that contributes nothing and leave the vertex
                // looking weighted while holding no weight at all.
                let weight = weight as f32;
                if weight.is_finite() && weight > 0.0 {
                    per_source[vertex].push((bone, weight));
                }
            }
        }

        let mut report = self.report.clone();
        for influences in per_source.iter_mut() {
            if influences.is_empty() {
                continue;
            }
            if influences.len() > MAX_INFLUENCES {
                report.vertices_over_influence_limit += 1;
                // Keep the strongest. Sorting descending by weight, with the
                // bone index breaking ties, so the choice is deterministic
                // rather than dependent on cluster iteration order.
                influences.sort_by(|a, b| {
                    b.1.partial_cmp(&a.1)
                        .unwrap_or(std::cmp::Ordering::Equal)
                        .then(a.0.cmp(&b.0))
                });
                influences.truncate(MAX_INFLUENCES);
            }
            let total: f32 = influences.iter().map(|(_, w)| w).sum();
            if total.is_finite() && total > 0.0 {
                for influence in influences.iter_mut() {
                    influence.1 /= total;
                }
            } else {
                // Influences that cannot be normalised (a sum that overflowed
                // to infinity) would divide down to zero and detach the vertex
                // just as surely as having no cluster at all — but silently.
                // Drop them so the expansion below applies the fallback and
                // the count says the file was not usable as written.
                influences.clear();
            }
        }

        let mut out = SkinWeights::zeroed(mesh.vertex_count());
        for (corner, &source) in mesh.vertex_source.iter().enumerate() {
            let base = corner * MAX_INFLUENCES;
            let influences = &per_source[source as usize];
            if influences.is_empty() {
                // Never leave a vertex unweighted: an unweighted vertex is not
                // pinned to anything and detaches when the rig moves. Bone 0
                // with full weight keeps it attached, and the corner is listed
                // so the caller can say the file was incomplete. Listed by
                // CORNER, which is the unit the output is in — a source vertex
                // expands to roughly six of them, and loose vertices belonging
                // to no polygon reach the output not at all.
                out.weights[base] = 1.0;
                out.fallback_vertices.push(corner as u32);
                continue;
            }
            for (slot, &(bone, weight)) in influences.iter().enumerate() {
                out.indices[base + slot] = bone;
                out.weights[base + slot] = weight;
            }
        }

        Ok((out, report))
    }
}
