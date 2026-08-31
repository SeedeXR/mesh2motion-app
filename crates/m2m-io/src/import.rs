//! Reading a model file without being told what it is.
//!
//! # Why this exists
//!
//! Objective **O9**: a file that already carries a skeleton must keep it. The
//! legacy app did the opposite by default —
//! `ModelCleanupUtility.strip_out_all_unecessary_model_data` turns every
//! `SkinnedMesh` into a plain `Mesh` and deletes `skinIndex`/`skinWeight`, and
//! `ModelAnalysisReport.ts:353` warns *"Mesh is already rigged. This workflow
//! drops the existing skeleton"*.
//!
//! Nothing here strips anything. [`inspect`] reports what a file contains so
//! the app can say what it found and keep it, which makes re-rigging an
//! explicit choice rather than the default nobody read the warning about.
//!
//! # What this is not
//!
//! This is a **summary**, not a scene. It answers "what is in this file, and is
//! it already rigged" for the import step and for a caller deciding which
//! pipeline to run. Anything that needs the geometry, the weights or the curves
//! goes to [`crate::fbx`] or [`crate::glb`] directly.

use crate::fbx::{self, animation, binary, dom::Scene, model, skin, text};
use crate::glb;
use m2m_core::skinning::MAX_INFLUENCES;
use std::collections::HashMap;

/// A container format this reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum Format {
    /// FBX, binary or ASCII — both reach the same semantic layers.
    Fbx,
    /// Binary glTF.
    Glb,
}

/// Why a file could not be summarised.
#[derive(Debug, thiserror::Error)]
pub enum ImportError {
    /// The bytes match no format this reads.
    #[error("not a file this reads: expected binary or ASCII FBX, or a .glb")]
    UnknownFormat,

    /// The file is FBX and reading it failed.
    #[error(transparent)]
    Fbx(#[from] fbx::FbxError),

    /// The file is glTF and reading it failed.
    #[error(transparent)]
    Glb(#[from] glb::GlbError),
}

/// What a file turned out to contain.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Import {
    /// The format it was read as.
    pub format: Format,
    /// Distinct meshes.
    pub meshes: usize,
    /// Bone names, parents before children.
    ///
    /// Empty is a real answer, not a failure: a plain mesh has no bones. For
    /// glTF that means the joints its skins name — glTF has no bone type, so a
    /// node is a bone only because a skin says so, and a file with no skin has
    /// no way to tell a bone from any other empty node.
    pub bones: Vec<String>,
    /// Meshes bound to a skin.
    pub skinned_meshes: usize,
    /// Animation clip names, in file order.
    pub clips: Vec<String>,
    /// Vertices (FBX) or primitives (glTF) whose influences past the fourth are
    /// not carried.
    ///
    /// The unit differs because it is what each format's reader can honestly
    /// count: FBX clusters are per-vertex, while glTF declares extra influence
    /// sets per primitive. Non-zero means the imported weights are not the
    /// file's weights, which O9 requires the app to say out loud rather than
    /// discover later as a deformation that looks wrong.
    pub over_influence_limit: usize,
}

impl Import {
    /// Whether the file arrived with a skeleton already on it.
    pub fn already_rigged(&self) -> bool {
        !self.bones.is_empty()
    }
}

/// Summarises a model file, deciding its format from its contents.
///
/// The format is decided by magic and shape, never by the file's extension: an
/// extension is a claim the user's filesystem makes, and a `.fbx` holding glTF
/// should be read as what it is.
///
/// # Errors
///
/// [`ImportError::UnknownFormat`] when nothing matches, or the underlying
/// reader's error. Hostile input must reach one of those rather than a panic.
pub fn inspect(bytes: &[u8]) -> Result<Import, ImportError> {
    if bytes.starts_with(b"glTF") {
        return Ok(from_glb(&glb::read(bytes)?));
    }
    let binary_error = match binary::parse(bytes) {
        Ok(document) => return Ok(from_fbx(&Scene::from_document(document))),
        Err(error) => error,
    };
    // ASCII FBX is tried last, and only on valid UTF-8.
    if let Ok(source) = std::str::from_utf8(bytes) {
        if text::is_ascii_fbx(source) {
            return Ok(from_fbx(&Scene::from_document(text::parse(source)?)));
        }
    }
    // A file carrying the FBX magic that then failed to parse gets to report
    // why. Anything else never claimed to be FBX, and saying "truncated" about
    // a JPEG would send the user looking for a damaged model.
    if matches!(binary_error, fbx::FbxError::BadMagic) {
        return Err(ImportError::UnknownFormat);
    }
    Err(binary_error.into())
}

fn from_fbx(scene: &Scene) -> Import {
    let models = model::parse_all(scene);
    let (skins, _) = skin::parse_all(scene);
    let (clips, _) = animation::parse_all(scene, &models);

    // Bones in hierarchy order, roots first, which is the order a rebuild
    // needs and the order a user reads a skeleton in.
    let mut bones = Vec::new();
    let mut queue: Vec<i64> = models.roots.clone();
    while let Some(id) = queue.pop() {
        let Some(node) = models.get(id) else { continue };
        if node.is_bone() {
            bones.push(node.name.clone());
        }
        queue.extend(node.children.iter().rev().copied());
    }

    Import {
        format: Format::Fbx,
        meshes: models
            .models
            .iter()
            .filter(|m| m.subclass == "Mesh")
            .count(),
        bones,
        skinned_meshes: skins.len(),
        clips: clips.into_iter().map(|c| c.name).collect(),
        over_influence_limit: skins.iter().map(over_weighted_vertices).sum(),
    }
}

/// Vertices that more than four clusters claim with a positive weight.
///
/// [`skin::Skin::bind`] is what actually truncates them, and it fills
/// `SkinReport::vertices_over_influence_limit` — but it needs the mesh geometry
/// to do it, and the report `skin::parse_all` returns has never been through
/// `bind`, so that field is always zero here. Reading it would have made this a
/// metric that could not move.
///
/// A summary must not pay for triangulating every mesh to answer one question,
/// so the same criterion is applied to the clusters alone. A cluster's indices
/// are already source-vertex indices, which is exactly what `bind` counts, so
/// the two agree — except for a weight small enough to narrow to zero as an
/// `f32`, which `bind` discards and this counts.
fn over_weighted_vertices(skin: &skin::Skin) -> usize {
    let mut per_vertex: HashMap<u32, usize> = HashMap::new();
    for cluster in &skin.clusters {
        for (&vertex, &weight) in cluster.indices.iter().zip(&cluster.weights) {
            if weight.is_finite() && weight > 0.0 {
                *per_vertex.entry(vertex).or_default() += 1;
            }
        }
    }
    per_vertex
        .values()
        .filter(|&&claims| claims > MAX_INFLUENCES)
        .count()
}

fn from_glb(document: &glb::Document) -> Import {
    let mut bones = Vec::new();
    for skin in &document.skins {
        for &joint in &skin.joints {
            let Some(node) = document.nodes.get(joint) else {
                continue;
            };
            // A joint shared between skins is one bone, not two.
            if !bones.contains(&node.name) {
                bones.push(node.name.clone());
            }
        }
    }

    Import {
        format: Format::Glb,
        meshes: document.mesh_count(),
        bones,
        skinned_meshes: document.nodes.iter().filter(|n| n.skin.is_some()).count(),
        clips: document.clips.iter().map(|c| c.name.clone()).collect(),
        over_influence_limit: document.report.primitives_over_influence_limit,
    }
}

#[cfg(test)]
mod tests {
    use super::{over_weighted_vertices, MAX_INFLUENCES};
    use glam::DMat4;

    fn cluster(vertex: u32, weight: f64) -> crate::fbx::skin::Cluster {
        crate::fbx::skin::Cluster {
            id: 0,
            bone_id: 0,
            indices: vec![vertex],
            weights: vec![weight],
            transform_link: DMat4::IDENTITY,
            transform: DMat4::IDENTITY,
        }
    }

    fn skin(clusters: Vec<crate::fbx::skin::Cluster>) -> crate::fbx::skin::Skin {
        crate::fbx::skin::Skin {
            id: 0,
            geometry_id: 0,
            clusters,
            report: Default::default(),
        }
    }

    /// No real fixture exceeds the limit — the reference rig maxes out at three
    /// influences — so the counting is pinned here instead.
    #[test]
    fn a_vertex_is_over_the_limit_only_past_the_fourth_claim() {
        let at_limit: Vec<_> = (0..MAX_INFLUENCES).map(|_| cluster(7, 0.25)).collect();
        assert_eq!(over_weighted_vertices(&skin(at_limit.clone())), 0);

        let mut over = at_limit;
        over.push(cluster(7, 0.25));
        assert_eq!(over_weighted_vertices(&skin(over)), 1);
    }

    /// A cluster can name a vertex and give it nothing. `bind` drops those, so
    /// counting them would report a truncation that never happens.
    #[test]
    fn a_zero_weight_claim_is_not_an_influence() {
        let mut clusters: Vec<_> = (0..MAX_INFLUENCES).map(|_| cluster(7, 0.25)).collect();
        clusters.push(cluster(7, 0.0));
        assert_eq!(over_weighted_vertices(&skin(clusters)), 0);
    }
}
