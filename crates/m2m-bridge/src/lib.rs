//! Blender DCC bridge.
//!
//! Two modes are planned (`memory/architecture.md` §6): **headless** — spawn
//! `Blender -b --python`, used to check an export against the strictest,
//! independent FBX/glTF reader available — and **live** — attach to a running
//! Blender for artist round-tripping. This is the headless half.
//!
//! Blender is the only *independent* reader in the project: our own reader and
//! three.js share a design (`end_offset` authoritative, footer detected
//! heuristically, an uncompressed array's declared length ignored), so neither
//! can check the conformance details the other gets wrong. Blender can, which
//! is why every export this project ships has been checked through it.
//!
//! # Why the report is read from a file, never stdout
//!
//! Blender writes its own progress to stdout without always terminating the
//! line, so a caller scraping stdout can get the JSON concatenated to
//! something else — a real failure seen during development was a gate reporting
//! `SyntaxError: Unexpected non-whitespace character after JSON` instead of the
//! assertion it was testing. [`inspect`] tells the script to write its JSON to
//! a temp file and reads that.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod live;
pub mod maya;

use std::path::{Path, PathBuf};

/// The inspection script, embedded so the crate is self-contained and cannot
/// drift from a copy: it is the same file the dev tooling runs.
const INSPECT_SCRIPT: &str = include_str!("../../../tools/blender-fbx-import-check.py");

/// What Blender found when it imported a file.
///
/// Every field past `imported` is optional because a failed import emits only
/// `file`, `imported: false` and `error`. On success the counts are present.
/// These mirror the keys `tools/blender-fbx-import-check.py` writes.
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct BlenderReport {
    /// The file's base name, as Blender saw it.
    pub file: String,
    /// Whether the import succeeded at all.
    pub imported: bool,
    /// The importer's error, when `imported` is false.
    #[serde(default)]
    pub error: Option<String>,
    /// Bones across all armatures.
    #[serde(default)]
    pub bones: Option<u32>,
    /// Mesh objects.
    #[serde(default)]
    pub meshes: Option<u32>,
    /// Armature objects.
    #[serde(default)]
    pub armatures: Option<u32>,
    /// Animation actions, by name.
    #[serde(default)]
    pub actions: Vec<String>,
    /// Vertices per mesh, in object order.
    #[serde(default)]
    pub mesh_vertices: Vec<u32>,
    /// Vertices carrying any vertex-group weight.
    #[serde(default)]
    pub weighted_vertices: Option<u32>,
    /// Sum of every vertex's total weight; equals the weighted-vertex count
    /// when every vertex is normalised to one.
    #[serde(default)]
    pub weight_total: Option<f64>,
    /// `action|stack|layer:curves=..,keys=..,range=lo-hi` per action — the
    /// FRAME RANGE is what proves a clip's time axis survived, not its keys.
    #[serde(default)]
    pub action_detail: Vec<String>,
}

/// Why an inspection could not be carried out.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    /// Blender itself could not be located to run.
    #[error("Blender not found: set M2M_BLENDER, or install it at the default path")]
    BlenderNotFound,
    /// Maya (`mayapy`) could not be located to run.
    #[error("mayapy not found: set M2M_MAYAPY, or install Maya at the default path")]
    MayaNotFound,
    /// The subprocess or a temp-file operation failed.
    #[error("running the DCC subprocess: {0}")]
    Spawn(String),
    /// The DCC ran but exited non-zero (an import that failed hard).
    #[error("the DCC exited with {code}: {stderr}")]
    Failed {
        /// Exit code, or -1 when killed by a signal.
        code: i32,
        /// The tail of the DCC's stderr, for the reason.
        stderr: String,
    },
    /// The report the DCC wrote was not the JSON this expects.
    #[error("the DCC report did not parse: {0}")]
    BadReport(String),
}

/// Resolves the Blender executable.
///
/// `M2M_BLENDER` wins when set, so CI or another machine can point at its own
/// install; otherwise the macOS default is tried. The path is never assumed to
/// exist without a check — `/Applications/Blender.app` is where it happens to
/// sit on the reference machine, not a guarantee.
pub fn blender_path() -> Result<PathBuf, BridgeError> {
    if let Ok(explicit) = std::env::var("M2M_BLENDER") {
        let path = PathBuf::from(explicit);
        return if path.is_file() {
            Ok(path)
        } else {
            Err(BridgeError::BlenderNotFound)
        };
    }
    let default = PathBuf::from("/Applications/Blender.app/Contents/MacOS/Blender");
    if default.is_file() {
        Ok(default)
    } else {
        Err(BridgeError::BlenderNotFound)
    }
}

/// Parses the JSON report the inspection script writes.
///
/// Separated from [`inspect`] so the parsing is testable without Blender: the
/// runners have no Blender, so this is the part CI can cover.
pub fn parse_report(json: &str) -> Result<BlenderReport, BridgeError> {
    serde_json::from_str(json).map_err(|e| BridgeError::BadReport(e.to_string()))
}

/// Imports `bytes` in headless Blender and returns what it found.
///
/// `extension` is `"glb"` or `"fbx"` — Blender chooses its importer from the
/// file name, so the temp file is named accordingly.
///
/// # Errors
///
/// [`BridgeError`] when Blender cannot be found, the subprocess fails, or the
/// report does not parse.
pub fn inspect(
    bytes: &[u8],
    extension: &str,
    blender: &Path,
) -> Result<BlenderReport, BridgeError> {
    let dir = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let model = dir.join(format!("m2m-bridge-{stamp}.{extension}"));
    let script = dir.join(format!("m2m-bridge-{stamp}.py"));
    let report = dir.join(format!("m2m-bridge-{stamp}.json"));

    // A guard that removes the temp files however this function returns.
    struct Cleanup(Vec<PathBuf>);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            for path in &self.0 {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    let _cleanup = Cleanup(vec![model.clone(), script.clone(), report.clone()]);

    std::fs::write(&model, bytes).map_err(|e| BridgeError::Spawn(e.to_string()))?;
    std::fs::write(&script, INSPECT_SCRIPT).map_err(|e| BridgeError::Spawn(e.to_string()))?;

    let output = std::process::Command::new(blender)
        .args(["--background", "--factory-startup", "--python"])
        .arg(&script)
        .arg("--")
        .arg(&model)
        .arg(&report)
        .output()
        .map_err(|e| BridgeError::Spawn(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BridgeError::Failed {
            code: output.status.code().unwrap_or(-1),
            // The last few lines carry the reason; the rest is Blender's banner.
            stderr: stderr.lines().rev().take(5).collect::<Vec<_>>().join(" | "),
        });
    }

    // The report is read from the FILE, never stdout — see the module docs.
    let json = std::fs::read_to_string(&report)
        .map_err(|e| BridgeError::Spawn(format!("Blender wrote no report file: {e}")))?;
    parse_report(&json)
}
