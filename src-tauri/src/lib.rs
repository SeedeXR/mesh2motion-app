//! Tauri command layer.
//!
//! # Boundary rule
//!
//! This crate is a **thin adapter**: it deserialises arguments, calls into the
//! `m2m-*` crates, and serialises results. No algorithms live here. Anything
//! worth testing belongs in a library crate where it can be tested without a
//! window. See `memory/architecture.md` §2.

#![forbid(unsafe_code)]

// The rigging pipeline lives in its own library crate now, shared with the MCP
// server; `rig::…` call sites keep working through this alias.
pub use m2m_pipeline as rig;

use m2m_io::import::Import;
use serde::Serialize;
use tauri::Emitter;
use tauri_plugin_dialog::DialogExt;

/// One progress step of a pipeline command, sent to the webview as a
/// `rig-progress` event and mirrored to stdout. `fraction` is 0..1.
#[derive(Clone, Serialize)]
struct Progress {
    /// The command this belongs to (`fit_skeleton`, `bind_weights`, …).
    command: &'static str,
    /// A short, human phase label (`reading`, `solving`, `encoding`, `done`).
    phase: &'static str,
    /// How far along, 0 at the start and 1 when done.
    fraction: f32,
}

/// Emits a progress step and logs it. The emit is best-effort — a dropped event
/// must never fail the command that reported it (todo P3-8b).
fn progress(app: &tauri::AppHandle, command: &'static str, phase: &'static str, fraction: f32) {
    println!("[ipc] {command}: {phase} ({:.0}%)", fraction * 100.0);
    let _ = app.emit(
        "rig-progress",
        Progress {
            command,
            phase,
            fraction,
        },
    );
}

/// Runs a command's blocking work, logging its start, its duration, and — this
/// is the point — every error, so an IPC failure is visible in the terminal and
/// in bug reports rather than only bubbling to the webview. The result is passed
/// through unchanged.
fn timed<T>(command: &'static str, work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    let start = std::time::Instant::now();
    println!("[ipc] {command}: start");
    let result = work();
    let ms = start.elapsed().as_millis();
    match &result {
        Ok(_) => println!("[ipc] {command}: ok in {ms} ms"),
        Err(e) => eprintln!("[ipc] {command}: FAILED in {ms} ms: {e}"),
    }
    result
}

/// Build and environment information surfaced in the About panel and in bug
/// reports, so a report always carries the versions it was produced against.
#[derive(Debug, Serialize)]
pub struct BuildInfo {
    /// Application version from `Cargo.toml`.
    pub version: &'static str,
    /// Target triple this binary was compiled for.
    pub target: &'static str,
}

/// Records what the webview resolved at startup: render backend and whether
/// the vendored font loaded.
///
/// Logged rather than merely displayed. A performance report is far less
/// useful without knowing whether the viewport ran on WebGPU or WebGL2, and a
/// font that silently failed to load is a visual regression nobody reports.
#[tauri::command]
fn report_startup(diagnostics: String) {
    println!("[m2m] startup: {diagnostics}");
}

/// Mirrors a webview console line to stdout, so the dev terminal and bug reports
/// see the frontend's logs — otherwise they are only visible in the inspector.
#[tauri::command]
fn log_line(level: String, message: String) {
    println!("[webview:{level}] {message}");
}

/// Dev/screenshot harness: a model path to auto-import at startup, from
/// `M2M_AUTOLOAD`, so the viewport can be exercised without the native picker.
#[tauri::command]
fn dev_autoload() -> Option<String> {
    std::env::var("M2M_AUTOLOAD").ok()
}

/// Dev/screenshot harness: a creature template to auto-fit after the autoload,
/// from `M2M_AUTOFIT`, so the Fit step can be exercised (and its auto-placement
/// screenshotted) without clicking through the workflow.
#[tauri::command]
fn dev_autofit() -> Option<String> {
    std::env::var("M2M_AUTOFIT").ok()
}

/// Dev/screenshot harness: a clip name to bind and preview after auto-fit, from
/// `M2M_AUTOCLIP`, so the Animate step (the retargeted preview and the clip
/// thumbnails) can be screenshotted without clicking through the workflow.
#[tauri::command]
fn dev_autoclip() -> Option<String> {
    std::env::var("M2M_AUTOCLIP").ok()
}

/// Dev/screenshot harness: when `M2M_AUTOPAINT` is set, bind and show the
/// weight-paint overlay after auto-fit, so the Bind step can be screenshotted.
#[tauri::command]
fn dev_autopaint() -> bool {
    std::env::var("M2M_AUTOPAINT").is_ok()
}

/// Dev/screenshot harness: when `M2M_AUTOMARK` is set, drive the marker-placement
/// flow after auto-fit (seed markers from the auto-fit joints, then solve) so the
/// marker Fit step can be screenshotted without clicking in the viewport.
#[tauri::command]
fn dev_automark() -> bool {
    std::env::var("M2M_AUTOMARK").is_ok()
}

/// Dev/screenshot harness: when `M2M_AUTOMARK_SOLVE` is set, the marker harness
/// also runs the solve, so the fitted skeleton (not just placement) can be shot.
#[tauri::command]
fn dev_automark_solve() -> bool {
    std::env::var("M2M_AUTOMARK_SOLVE").is_ok()
}

/// A file the user chose, and what it turned out to contain.
#[derive(Debug, Serialize)]
pub struct ImportedFile {
    /// The file's name, for the UI to echo back.
    pub name: String,
    /// Where it came from, so the viewport can ask for its geometry without a
    /// second trip through the picker.
    pub path: String,
    /// What reading it found.
    pub import: Import,
}

/// Opens a file picker and reports what the chosen model contains.
///
/// Nothing is stripped, which is the whole point: objective **O9** says a file
/// that arrives with a skeleton keeps it, so the app's job here is to *say* what
/// it found. Re-rigging is then something the user asks for, not what happens to
/// anyone who does not read a warning.
///
/// `Ok(None)` means the user cancelled, which is not an error.
#[tauri::command]
async fn import_model(app: tauri::AppHandle) -> Result<Option<ImportedFile>, String> {
    // The picker blocks until the user answers, and it needs the main thread
    // free to pump the event loop while it waits.
    tauri::async_runtime::spawn_blocking(move || pick_and_inspect(&app))
        .await
        .map_err(|e| e.to_string())?
}

fn pick_and_inspect(app: &tauri::AppHandle) -> Result<Option<ImportedFile>, String> {
    // Dev/screenshot harness: skip the native picker and use this path.
    let path = if let Ok(preset) = std::env::var("M2M_AUTOLOAD") {
        std::path::PathBuf::from(preset)
    } else {
        let Some(chosen) = app
            .dialog()
            .file()
            .add_filter("Model", &["glb", "fbx"])
            .blocking_pick_file()
        else {
            return Ok(None);
        };
        chosen.into_path().map_err(|e| e.to_string())?
    };
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read the file: {e}"))?;

    Ok(Some(ImportedFile {
        path: path.to_string_lossy().into_owned(),
        name: path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        import: m2m_io::import::inspect(&bytes).map_err(|e| e.to_string())?,
    }))
}

/// Returns a model's geometry as a `.glb`, for the viewport to draw.
///
/// **The bulk channel** (`memory/architecture.md` §4): the body is raw bytes,
/// never JSON. A 50k-vertex mesh is about 1.2 MB binary and about 9 MB as a
/// JSON number array, and the parse cost on the webview side would dominate
/// everything the Rust core just did.
///
/// glTF is the wire format because it already *is* the thing §4 describes — a
/// JSON header and a binary chunk — so neither side needs a private encoding.
/// An FBX is converted on the way out; see `m2m_io::convert` for what that
/// carries and what it does not.
#[tauri::command]
async fn load_model(path: String) -> Result<tauri::ipc::Response, String> {
    tauri::async_runtime::spawn_blocking(move || timed("load_model", || read_as_glb(&path)))
        .await
        .map_err(|e| e.to_string())?
}

fn read_as_glb(path: &str) -> Result<tauri::ipc::Response, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read the file: {e}"))?;
    // A glb the viewport can render itself is sent as-is, so its materials and
    // textures reach the screen. `glb::write` carries only geometry and skin
    // weights — all the rigging backend needs, but it shows as an untextured
    // grey mesh, which is not the model the user recognises. It is still read
    // first, so a corrupt file fails here with a clear error rather than in the
    // loader. FBX has no direct viewer path (three cannot read our FBX), so it
    // is converted — losing appearance the FBX reader does not carry anyway.
    if bytes.starts_with(b"glTF") {
        m2m_io::glb::read(&bytes).map_err(|e| e.to_string())?;
        return Ok(tauri::ipc::Response::new(bytes));
    }
    let document = m2m_io::import::load(&bytes).map_err(|e| e.to_string())?;
    let glb = m2m_io::glb::write(&document).map_err(|e| e.to_string())?;
    Ok(tauri::ipc::Response::new(glb))
}

/// Reads a creature's animation library.
///
/// The libraries are **bundled resources**, not embedded in the binary: they
/// total about 16 MB against a measured 8 MB bundle and a 40 MB budget, so
/// carrying them in the executable would be wasteful where the rigs' 158 KB was
/// not. The bundled copy is preferred and the repository is the fallback, so
/// `tauri dev` works from a checkout with nothing staged.
///
/// Most creatures use `<name>-animations.glb`; the human's is
/// `human-base-animations.glb`. Both are tried rather than kept in a table that
/// would drift from the files.
fn library_bytes(app: &tauri::AppHandle, template: &str) -> Result<Vec<u8>, String> {
    use tauri::Manager;

    let repository = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../assets/animations");
    for name in [
        format!("{template}-animations.glb"),
        format!("{template}-base-animations.glb"),
    ] {
        let bundled = app
            .path()
            .resolve(
                format!("animations/{name}"),
                tauri::path::BaseDirectory::Resource,
            )
            .ok();
        for candidate in [bundled, Some(repository.join(&name))]
            .into_iter()
            .flatten()
        {
            if candidate.is_file() {
                return std::fs::read(&candidate)
                    .map_err(|e| format!("cannot read {}: {e}", candidate.display()));
            }
        }
    }
    Err(format!("no animation library ships for {template}"))
}

/// The clips a creature's animation library offers.
#[tauri::command]
async fn animation_clips(
    app: tauri::AppHandle,
    template: String,
) -> Result<Vec<rig::ClipSummary>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = library_bytes(&app, &template)?;
        rig::library_clips(&bytes).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Returns a creature's animation-library `.glb` — the library character with
/// every clip on it.
///
/// Bulk channel: the whole library, so the clip chooser can play a moving
/// preview of any clip on the library character without a round-trip per clip
/// or retargeting onto the user's rig.
#[tauri::command]
async fn animation_library(
    app: tauri::AppHandle,
    template: String,
) -> Result<tauri::ipc::Response, String> {
    tauri::async_runtime::spawn_blocking(move || {
        Ok(tauri::ipc::Response::new(library_bytes(&app, &template)?))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Returns the bound model with a weight-paint overlay baked into vertex colours.
///
/// Bulk channel: a whole model whose vertices carry a `COLOR_0` the viewer draws
/// directly — dominant bone as a hue, the solver's straight-line guesses flagged.
#[tauri::command]
async fn weight_overlay(
    path: String,
    skeleton: rig::FittedSkeleton,
    falloff: f32,
) -> Result<tauri::ipc::Response, String> {
    tauri::async_runtime::spawn_blocking(move || {
        timed("weight_overlay", || {
            let source = std::fs::read(&path).map_err(|e| format!("cannot read the model: {e}"))?;
            let glb = rig::overlay_glb(&source, &skeleton, falloff).map_err(|e| e.to_string())?;
            Ok(tauri::ipc::Response::new(glb))
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Returns the rigged model with a clip retargeted onto it, as a `.glb`.
///
/// The **bulk channel** again: this is a whole animated model, and the viewport
/// hands it straight to the same loader it draws the imported mesh with. These
/// are the very bytes `export_model` writes to disk for a `.glb`, so the
/// preview and the export cannot drift — one code path, two destinations.
#[tauri::command]
async fn preview_animation(
    app: tauri::AppHandle,
    path: String,
    skeleton: rig::FittedSkeleton,
    falloff: f32,
    template: String,
    clip: String,
) -> Result<tauri::ipc::Response, String> {
    tauri::async_runtime::spawn_blocking(move || {
        timed("preview_animation", || {
            progress(&app, "preview_animation", "reading", 0.0);
            let source = std::fs::read(&path).map_err(|e| format!("cannot read the model: {e}"))?;
            let library = library_bytes(&app, &template)?;
            progress(&app, "preview_animation", "retargeting", 0.4);
            let glb = rig::export_glb(&source, &skeleton, falloff, Some((&library, &clip)))
                .map_err(|e| e.to_string())?;
            progress(&app, "preview_animation", "done", 1.0);
            Ok(tauri::ipc::Response::new(glb))
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// The creature templates the Choose Skeleton step offers.
#[tauri::command]
fn skeleton_templates() -> Result<Vec<rig::SkeletonTemplate>, String> {
    rig::templates().map_err(|e| e.to_string())
}

/// Places a template's skeleton on the imported model.
///
/// Runs off the main thread: fitting voxelises the mesh at 128 cubed, which is
/// real work and would otherwise stall the window while it ran.
///
/// The result travels as JSON rather than over the bulk channel — a skeleton is
/// a few hundred bones, not a vertex buffer, and `architecture.md` §4 draws the
/// line at bulk geometry, not at everything numeric.
#[tauri::command]
async fn fit_skeleton(
    app: tauri::AppHandle,
    template: String,
    path: String,
) -> Result<rig::FittedSkeleton, String> {
    tauri::async_runtime::spawn_blocking(move || {
        timed("fit_skeleton", || {
            progress(&app, "fit_skeleton", "reading", 0.0);
            let bytes = std::fs::read(&path).map_err(|e| format!("cannot read the file: {e}"))?;
            progress(&app, "fit_skeleton", "fitting", 0.3);
            let fitted = rig::fit(&template, &bytes).map_err(|e| e.to_string())?;
            progress(&app, "fit_skeleton", "done", 1.0);
            Ok(fitted)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Places a template's skeleton from user-placed markers.
///
/// The marker-placement flow: the app sends where a person dropped the markers
/// (chin, wrists, elbows, knees, groin) and the rig fits them. Needs no model
/// bytes — the solve is rig-and-markers only — so it is fast, but it is spawned
/// off the main thread for consistency with [`fit_skeleton`] and to keep the
/// window responsive.
#[tauri::command]
async fn fit_from_markers(
    app: tauri::AppHandle,
    template: String,
    markers: Vec<rig::Marker>,
) -> Result<rig::FittedSkeleton, String> {
    tauri::async_runtime::spawn_blocking(move || {
        timed("fit_from_markers", || {
            progress(&app, "fit_from_markers", "fitting", 0.3);
            let fitted = rig::fit_from_markers(&template, &markers).map_err(|e| e.to_string())?;
            progress(&app, "fit_from_markers", "done", 1.0);
            Ok(fitted)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Binds the mesh to the fitted skeleton.
///
/// Off the main thread for the same reason as fitting, and more so: this
/// voxelises, solves a geodesic field per bone and then assigns weights.
/// Measured on the reference body — 7,399 vertices, 66 bones — the whole thing
/// takes about 56 ms. Coarse `rig-progress` phase events are emitted at the
/// command boundary (P3-8b); a model an order of magnitude larger, where the
/// solve dominates, is where finer per-bone progress from inside the solver
/// would start to matter.
#[tauri::command]
async fn bind_weights(
    app: tauri::AppHandle,
    path: String,
    skeleton: rig::FittedSkeleton,
    falloff: f32,
) -> Result<rig::BindReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        timed("bind_weights", || {
            progress(&app, "bind_weights", "reading", 0.0);
            let bytes = std::fs::read(&path).map_err(|e| format!("cannot read the file: {e}"))?;
            progress(&app, "bind_weights", "solving", 0.3);
            let report = rig::bind(&bytes, &skeleton, falloff).map_err(|e| e.to_string())?;
            progress(&app, "bind_weights", "done", 1.0);
            Ok(report)
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Writes the rigged model to a file the user chooses.
///
/// The weights are recomputed rather than carried in from the frontend: they
/// are about 600 KB, the whole solve takes ~56 ms, and a round trip through the
/// webview would cost more than doing it twice — with a cache to invalidate
/// every time a bone moved.
///
/// `Ok(None)` means the user cancelled, which is not an error.
#[tauri::command]
async fn export_model(
    app: tauri::AppHandle,
    path: String,
    skeleton: rig::FittedSkeleton,
    falloff: f32,
    format: String,
    template: String,
    clip: Option<String>,
) -> Result<Option<String>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        timed("export_model", || {
            // Unknown formats are refused rather than defaulted: silently writing
            // a `.glb` for someone who asked for FBX is worse than an error.
            let (label, extension) = match format.as_str() {
                "glb" => ("glTF binary", "glb"),
                "fbx" => ("FBX binary", "fbx"),
                other => return Err(format!("unknown export format {other}")),
            };

            let Some(chosen) = app
                .dialog()
                .file()
                .add_filter(label, &[extension])
                .set_file_name(format!("rigged.{extension}"))
                .blocking_save_file()
            else {
                // A cancelled save is not an error and reports no progress.
                return Ok(None);
            };
            let target = chosen.into_path().map_err(|e| e.to_string())?;

            progress(&app, "export_model", "reading", 0.1);
            let source = std::fs::read(&path).map_err(|e| format!("cannot read the model: {e}"))?;
            let library = match &clip {
                Some(_) => Some(library_bytes(&app, &template)?),
                None => None,
            };
            progress(&app, "export_model", "solving", 0.4);
            let bytes = match extension {
                "fbx" => rig::export_fbx(
                    &source,
                    &skeleton,
                    falloff,
                    library.as_deref().zip(clip.as_deref()),
                ),
                _ => rig::export_glb(
                    &source,
                    &skeleton,
                    falloff,
                    library.as_deref().zip(clip.as_deref()),
                ),
            }
            .map_err(|e| e.to_string())?;
            progress(&app, "export_model", "writing", 0.8);
            std::fs::write(&target, &bytes).map_err(|e| format!("cannot write the export: {e}"))?;
            progress(&app, "export_model", "done", 1.0);

            Ok(Some(target.file_name().map_or_else(String::new, |n| {
                n.to_string_lossy().into_owned()
            })))
        })
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn build_info() -> BuildInfo {
    BuildInfo {
        version: env!("CARGO_PKG_VERSION"),
        target: env!("M2M_TARGET"),
    }
}

/// Starts the application.
///
/// # Panics
///
/// Panics if the Tauri runtime cannot be initialised, which is unrecoverable.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            build_info,
            report_startup,
            dev_autoload,
            dev_autofit,
            dev_autoclip,
            dev_autopaint,
            dev_automark,
            dev_automark_solve,
            log_line,
            import_model,
            load_model,
            skeleton_templates,
            fit_skeleton,
            fit_from_markers,
            bind_weights,
            export_model,
            animation_clips,
            animation_library,
            preview_animation,
            weight_overlay
        ])
        .run(tauri::generate_context!())
        .expect("failed to start mesh2motion");
}

#[cfg(test)]
mod ipc_tests {
    use tauri::ipc::{InvokeResponseBody, IpcResponse};

    /// The viewport gets a model over the bulk channel as raw bytes, which the
    /// frontend reads as an `ArrayBuffer`. If it ever came back as JSON, that
    /// `ArrayBuffer` would be a number array instead — `byteLength` undefined,
    /// so the model never draws (the "Geometry NaN MB" bug).
    #[test]
    fn read_as_glb_delivers_raw_bytes_not_json() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../assets/models/model-human.glb"
        );
        let response = super::read_as_glb(path).expect("reads the fixture");
        match response.body().expect("has a body") {
            InvokeResponseBody::Raw(bytes) => assert!(bytes.len() > 1000, "raw but tiny"),
            InvokeResponseBody::Json(json) => {
                panic!("delivered as JSON ({} chars), not raw bytes", json.len())
            }
        }
    }

    /// The bundle config actually ships the animation libraries the app looks
    /// for. Lives here because it validates THIS crate's `tauri.conf.json`; the
    /// pipeline that reads the libraries moved to `m2m-pipeline`.
    #[test]
    fn the_bundle_config_carries_the_animation_libraries() {
        let config: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/tauri.conf.json"))
                .expect("reads the config"),
        )
        .expect("parses");

        let resources = config["bundle"]["resources"]
            .as_object()
            .expect("bundle.resources is a source-to-target map");
        let (source, target) = resources
            .iter()
            .find(|(source, _)| source.contains("animations"))
            .expect("no resource entry mentions the animation libraries");
        assert_eq!(target.as_str(), Some("animations/"));

        let pattern = source.rsplit('/').next().expect("a file pattern");
        assert_eq!(pattern, "*.glb", "the glob is {source}");
        let directory = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR")))
            .join(source.trim_end_matches("/*.glb"));
        assert!(
            directory.join("human-base-animations.glb").is_file(),
            "{} does not hold the libraries",
            directory.display()
        );
    }

    /// The IPC logging wrapper is transparent: it times and logs, but returns the
    /// closure's own result — success and failure alike — unchanged.
    #[test]
    fn timed_passes_the_result_through() {
        assert_eq!(super::timed("t", || Ok::<_, String>(42)), Ok(42));
        assert_eq!(
            super::timed("t", || Err::<i32, _>("boom".to_string())),
            Err("boom".to_string())
        );
    }
}
