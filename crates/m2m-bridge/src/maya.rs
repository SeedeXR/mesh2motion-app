//! Maya DCC bridge (headless).
//!
//! Maya is the *strictest* independent FBX reader the project has — it rejects
//! the footer-CRC and DefaultAttributeIndex non-conformances that Blender and
//! our own reader accept (memory: "FBX: Maya is the strict reader"). This spawns
//! `mayapy` on `tools/maya-fbx-import-check.py`, the Maya counterpart to the
//! Blender inspector, so an export can be checked through both engines and
//! their reports compared.
//!
//! As with the Blender bridge, the JSON report is read from a FILE, never
//! stdout — Maya prints plug-in and licensing chatter that would otherwise be
//! concatenated onto the JSON.

use crate::BridgeError;
use std::path::{Path, PathBuf};

/// The Maya inspection script, embedded so the crate is self-contained.
const MAYA_SCRIPT: &str = include_str!("../../../tools/maya-fbx-import-check.py");

/// What Maya found when it imported an FBX. Mirrors the keys
/// `tools/maya-fbx-import-check.py` writes; joints stand in for bones and
/// skinClusters for vertex groups, so it lines up with [`crate::BlenderReport`].
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct MayaReport {
    /// The file's base name, as Maya saw it.
    pub file: String,
    /// Whether the import succeeded at all.
    pub imported: bool,
    /// The importer's error, when `imported` is false.
    #[serde(default)]
    pub error: Option<String>,
    /// Joint nodes (Maya's bones).
    #[serde(default)]
    pub joints: Option<u32>,
    /// Mesh shapes.
    #[serde(default)]
    pub meshes: Option<u32>,
    /// skinCluster deformers.
    #[serde(default)]
    pub skin_clusters: Option<u32>,
    /// Joint names.
    #[serde(default)]
    pub joint_names: Vec<String>,
    /// `joint<-parent` per joint — the hierarchy, so a flattened skeleton with
    /// the right names still shows up.
    #[serde(default)]
    pub joint_parents: Vec<String>,
    /// Joints with no parent joint.
    #[serde(default)]
    pub root_joints: Vec<String>,
    /// Vertices per mesh, ascending.
    #[serde(default)]
    pub mesh_vertices: Vec<u32>,
    /// Vertices carrying any skin weight.
    #[serde(default)]
    pub weighted_vertices: Option<u32>,
    /// Sum of every vertex's total weight; equals the weighted-vertex count
    /// when every vertex is normalised to one.
    #[serde(default)]
    pub weight_total: Option<f64>,
}

/// Parses the JSON report the Maya script writes.
///
/// Separated from [`inspect`] so parsing is testable without Maya — the part CI
/// (which has no Maya) can cover.
pub fn parse_report(json: &str) -> Result<MayaReport, BridgeError> {
    serde_json::from_str(json).map_err(|e| BridgeError::BadReport(e.to_string()))
}

/// Resolves the `mayapy` executable.
///
/// `M2M_MAYAPY` wins when set; otherwise the newest Maya under the macOS default
/// `/Applications/Autodesk/maya*` is tried. Never assumed to exist without a
/// check.
pub fn mayapy_path() -> Result<PathBuf, BridgeError> {
    if let Ok(explicit) = std::env::var("M2M_MAYAPY") {
        let path = PathBuf::from(explicit);
        return if path.is_file() {
            Ok(path)
        } else {
            Err(BridgeError::MayaNotFound)
        };
    }
    // Newest install first, so a 2027 is preferred over a 2024 sitting beside it.
    let mut candidates: Vec<PathBuf> = std::fs::read_dir("/Applications/Autodesk")
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("maya"))
        })
        .map(|path| path.join("Maya.app/Contents/bin/mayapy"))
        .filter(|path| path.is_file())
        .collect();
    candidates.sort();
    candidates.pop().ok_or(BridgeError::MayaNotFound)
}

/// Imports `fbx` in headless Maya and returns what it found.
///
/// # Errors
///
/// [`BridgeError`] when `mayapy` cannot be found, the subprocess fails, or the
/// report does not parse.
pub fn inspect(fbx: &[u8], mayapy: &Path) -> Result<MayaReport, BridgeError> {
    let dir = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let model = dir.join(format!("m2m-maya-{stamp}.fbx"));
    let script = dir.join(format!("m2m-maya-{stamp}.py"));
    let report = dir.join(format!("m2m-maya-{stamp}.json"));

    struct Cleanup(Vec<PathBuf>);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            for path in &self.0 {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let _cleanup = Cleanup(vec![model.clone(), script.clone(), report.clone()]);

    std::fs::write(&model, fbx).map_err(|e| BridgeError::Spawn(e.to_string()))?;
    std::fs::write(&script, MAYA_SCRIPT).map_err(|e| BridgeError::Spawn(e.to_string()))?;

    let output = std::process::Command::new(mayapy)
        .arg(&script)
        .arg(&model)
        .arg(&report)
        .output()
        .map_err(|e| BridgeError::Spawn(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BridgeError::Failed {
            code: output.status.code().unwrap_or(-1),
            stderr: stderr.lines().rev().take(5).collect::<Vec<_>>().join(" | "),
        });
    }

    let json = std::fs::read_to_string(&report)
        .map_err(|e| BridgeError::Spawn(format!("Maya wrote no report file: {e}")))?;
    parse_report(&json)
}
