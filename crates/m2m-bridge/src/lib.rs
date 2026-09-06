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

/// The turntable-render script, embedded for the same reason.
const RENDER_SCRIPT: &str = include_str!("../../../tools/blender-render-views.py");

const ANIM_SCRIPT: &str = include_str!("../../../tools/blender-render-animation.py");

/// What Blender found when it imported a file.
///
/// Every field past `imported` is optional because a failed import emits only
/// `file`, `imported: false` and `error`. On success the counts are present.
/// These mirror the keys `tools/blender-fbx-import-check.py` writes.
#[derive(Debug, Clone, PartialEq, serde::Deserialize, serde::Serialize)]
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
    /// ffmpeg could not be located to encode a video.
    #[error("ffmpeg not found: set M2M_FFMPEG, or install it (e.g. `brew install ffmpeg`)")]
    FfmpegNotFound,
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

/// Runs a DCC on an embedded inspection script and returns the JSON report it
/// wrote to a file — the mechanism the Blender and Maya bridges share.
///
/// Writes `model_bytes` and `script_source` to temp files, spawns `exe` with the
/// arguments `build_args` derives from the (script, model, report) temp paths,
/// checks the exit status, and reads the report FILE — never stdout, since a
/// DCC's own progress/licensing chatter would otherwise corrupt the JSON. Temp
/// files are cleaned however this returns.
///
/// # Errors
///
/// [`BridgeError`] when the subprocess fails to spawn, exits non-zero, or writes
/// no report file.
pub(crate) fn run_report(
    exe: &Path,
    model_extension: &str,
    model_bytes: &[u8],
    script_source: &str,
    build_args: impl Fn(&Path, &Path, &Path) -> Vec<std::ffi::OsString>,
) -> Result<String, BridgeError> {
    let dir = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let model = dir.join(format!("m2m-bridge-{stamp}.{model_extension}"));
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

    std::fs::write(&model, model_bytes).map_err(|e| BridgeError::Spawn(e.to_string()))?;
    std::fs::write(&script, script_source).map_err(|e| BridgeError::Spawn(e.to_string()))?;

    let output = std::process::Command::new(exe)
        .args(build_args(&script, &model, &report))
        .output()
        .map_err(|e| BridgeError::Spawn(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BridgeError::Failed {
            code: output.status.code().unwrap_or(-1),
            // The last few lines carry the reason; the rest is the DCC's banner.
            stderr: stderr.lines().rev().take(5).collect::<Vec<_>>().join(" | "),
        });
    }

    std::fs::read_to_string(&report)
        .map_err(|e| BridgeError::Spawn(format!("the DCC wrote no report file: {e}")))
}

/// Renders a `.glb` from `num_views` angles (a turntable) in headless Blender,
/// returning the written PNG paths. An optional `frame` renders that frame of an
/// imported clip, for inspecting a pose mid-animation.
///
/// The PNGs are left in a temp directory for the caller to read and then remove
/// (unlike a report, they are the result). The model and script temp files are
/// cleaned here.
///
/// # Errors
///
/// [`BridgeError`] when the subprocess fails or writes no images.
pub fn render_views(
    glb: &[u8],
    num_views: u32,
    frame: Option<i32>,
    mode: &str,
    blender: &Path,
) -> Result<Vec<PathBuf>, BridgeError> {
    let dir = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let model = dir.join(format!("m2m-render-{stamp}.glb"));
    let script = dir.join(format!("m2m-render-{stamp}.py"));
    let out_dir = dir.join(format!("m2m-render-{stamp}"));
    std::fs::create_dir_all(&out_dir).map_err(|e| BridgeError::Spawn(e.to_string()))?;
    std::fs::write(&model, glb).map_err(|e| BridgeError::Spawn(e.to_string()))?;
    std::fs::write(&script, RENDER_SCRIPT).map_err(|e| BridgeError::Spawn(e.to_string()))?;

    let mut command = std::process::Command::new(blender);
    command
        .args(["-b", "--factory-startup", "--python"])
        .arg(&script)
        .arg("--")
        .arg(&model)
        .arg(&out_dir)
        .arg(num_views.max(1).to_string())
        .arg(if mode.is_empty() { "solid" } else { mode });
    if let Some(frame) = frame {
        command.arg(frame.to_string());
    }
    let output = command
        .output()
        .map_err(|e| BridgeError::Spawn(e.to_string()))?;
    // The model and script are ours to clean; the PNGs belong to the caller.
    let _ = std::fs::remove_file(&model);
    let _ = std::fs::remove_file(&script);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BridgeError::Failed {
            code: output.status.code().unwrap_or(-1),
            stderr: stderr.lines().rev().take(5).collect::<Vec<_>>().join(" | "),
        });
    }

    let mut pngs: Vec<PathBuf> = std::fs::read_dir(&out_dir)
        .map_err(|e| BridgeError::Spawn(e.to_string()))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "png"))
        .collect();
    pngs.sort();
    if pngs.is_empty() {
        return Err(BridgeError::Spawn("Blender rendered no images".to_string()));
    }
    Ok(pngs)
}

/// Resolves the ffmpeg executable (to encode rendered frames into a video, and
/// to sample frames from a reference video). `M2M_FFMPEG` wins; otherwise the
/// common install locations and `PATH` are searched. Never assumed present.
pub fn ffmpeg_path() -> Result<PathBuf, BridgeError> {
    if let Ok(explicit) = std::env::var("M2M_FFMPEG") {
        let path = PathBuf::from(explicit);
        return if path.is_file() {
            Ok(path)
        } else {
            Err(BridgeError::FfmpegNotFound)
        };
    }
    let mut candidates: Vec<PathBuf> = [
        "/opt/homebrew/bin/ffmpeg",
        "/usr/local/bin/ffmpeg",
        "/usr/bin/ffmpeg",
        "/opt/local/bin/ffmpeg",
    ]
    .iter()
    .map(PathBuf::from)
    .collect();
    if let Ok(path_var) = std::env::var("PATH") {
        candidates.extend(std::env::split_paths(&path_var).map(|d| d.join("ffmpeg")));
    }
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .ok_or(BridgeError::FfmpegNotFound)
}

/// Renders every frame of a clip baked into `glb` from one fixed camera, headless.
///
/// Returns the frame PNGs (sorted) and the clip's fps, for [`encode_video`] to
/// turn into a movie. The frames are named `frame_%04d.png` in a temp dir the
/// caller cleans; the model and script temp files are cleaned here.
pub fn render_animation(
    glb: &[u8],
    max_frames: u32,
    blender: &Path,
) -> Result<(Vec<PathBuf>, u32), BridgeError> {
    let dir = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let model = dir.join(format!("m2m-anim-{stamp}.glb"));
    let script = dir.join(format!("m2m-anim-{stamp}.py"));
    let out_dir = dir.join(format!("m2m-anim-{stamp}"));
    std::fs::create_dir_all(&out_dir).map_err(|e| BridgeError::Spawn(e.to_string()))?;
    std::fs::write(&model, glb).map_err(|e| BridgeError::Spawn(e.to_string()))?;
    std::fs::write(&script, ANIM_SCRIPT).map_err(|e| BridgeError::Spawn(e.to_string()))?;

    let output = std::process::Command::new(blender)
        .args(["-b", "--factory-startup", "--python"])
        .arg(&script)
        .arg("--")
        .arg(&model)
        .arg(&out_dir)
        .arg(max_frames.max(1).to_string())
        .output()
        .map_err(|e| BridgeError::Spawn(e.to_string()))?;
    let _ = std::fs::remove_file(&model);
    let _ = std::fs::remove_file(&script);

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BridgeError::Failed {
            code: output.status.code().unwrap_or(-1),
            stderr: stderr.lines().rev().take(5).collect::<Vec<_>>().join(" | "),
        });
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let fps = stdout
        .lines()
        .find_map(|l| l.strip_prefix("FPS "))
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(24);

    let mut pngs: Vec<PathBuf> = std::fs::read_dir(&out_dir)
        .map_err(|e| BridgeError::Spawn(e.to_string()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "png"))
        .collect();
    pngs.sort();
    if pngs.is_empty() {
        return Err(BridgeError::Spawn("Blender rendered no frames".to_string()));
    }
    Ok((pngs, fps))
}

/// Encodes `frames_dir`'s `frame_%04d.png` sequence into an mp4 at `out`, `fps`.
pub fn encode_video(
    frames_dir: &Path,
    fps: u32,
    out: &Path,
    ffmpeg: &Path,
) -> Result<(), BridgeError> {
    let pattern = frames_dir.join("frame_%04d.png");
    let output = std::process::Command::new(ffmpeg)
        .args(["-y", "-framerate", &fps.max(1).to_string(), "-i"])
        .arg(&pattern)
        .args([
            "-c:v",
            "libx264",
            "-pix_fmt",
            "yuv420p",
            "-movflags",
            "+faststart",
        ])
        .arg(out)
        .output()
        .map_err(|e| BridgeError::Spawn(e.to_string()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BridgeError::Failed {
            code: output.status.code().unwrap_or(-1),
            stderr: stderr.lines().rev().take(5).collect::<Vec<_>>().join(" | "),
        });
    }
    Ok(())
}

/// Samples up to `count` evenly-spaced frames from `video` into a temp dir, for
/// a side-by-side comparison against a rendered clip. Returns the frame PNGs.
pub fn sample_video_frames(
    video: &Path,
    count: u32,
    ffmpeg: &Path,
) -> Result<Vec<PathBuf>, BridgeError> {
    let dir = std::env::temp_dir();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let out_dir = dir.join(format!("m2m-ref-{stamp}"));
    std::fs::create_dir_all(&out_dir).map_err(|e| BridgeError::Spawn(e.to_string()))?;
    let pattern = out_dir.join("ref_%04d.png");
    // `thumbnail=N` picks representative frames; capped by -frames:v.
    let count = count.clamp(1, 64);
    let output = std::process::Command::new(ffmpeg)
        .arg("-i")
        .arg(video)
        .args([
            "-vf",
            &format!("thumbnail={},scale=512:-1", 300 / count.max(1)),
            "-frames:v",
            &count.to_string(),
            "-vsync",
            "0",
        ])
        .arg(&pattern)
        .output()
        .map_err(|e| BridgeError::Spawn(e.to_string()))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BridgeError::Failed {
            code: output.status.code().unwrap_or(-1),
            stderr: stderr.lines().rev().take(5).collect::<Vec<_>>().join(" | "),
        });
    }
    let mut pngs: Vec<PathBuf> = std::fs::read_dir(&out_dir)
        .map_err(|e| BridgeError::Spawn(e.to_string()))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "png"))
        .collect();
    pngs.sort();
    if pngs.is_empty() {
        return Err(BridgeError::Spawn(
            "ffmpeg sampled no frames from the reference".to_string(),
        ));
    }
    Ok(pngs)
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
    let json = run_report(
        blender,
        extension,
        bytes,
        INSPECT_SCRIPT,
        |script, model, report| {
            vec![
                std::ffi::OsString::from("--background"),
                std::ffi::OsString::from("--factory-startup"),
                std::ffi::OsString::from("--python"),
                script.as_os_str().to_owned(),
                std::ffi::OsString::from("--"),
                model.as_os_str().to_owned(),
                report.as_os_str().to_owned(),
            ]
        },
    )?;
    parse_report(&json)
}
