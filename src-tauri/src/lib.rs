//! Tauri command layer.
//!
//! # Boundary rule
//!
//! This crate is a **thin adapter**: it deserialises arguments, calls into the
//! `m2m-*` crates, and serialises results. No algorithms live here. Anything
//! worth testing belongs in a library crate where it can be tested without a
//! window. See `memory/architecture.md` §2.

#![forbid(unsafe_code)]

pub mod rig;

use m2m_io::import::Import;
use serde::Serialize;
use tauri_plugin_dialog::DialogExt;

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
    let Some(chosen) = app
        .dialog()
        .file()
        .add_filter("Model", &["glb", "fbx"])
        .blocking_pick_file()
    else {
        return Ok(None);
    };
    let path = chosen.into_path().map_err(|e| e.to_string())?;
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
    tauri::async_runtime::spawn_blocking(move || read_as_glb(&path))
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
        let source = std::fs::read(&path).map_err(|e| format!("cannot read the model: {e}"))?;
        let glb = rig::overlay_glb(&source, &skeleton, falloff).map_err(|e| e.to_string())?;
        Ok(tauri::ipc::Response::new(glb))
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
        let source = std::fs::read(&path).map_err(|e| format!("cannot read the model: {e}"))?;
        let library = library_bytes(&app, &template)?;
        let glb = rig::export_glb(&source, &skeleton, falloff, Some((&library, &clip)))
            .map_err(|e| e.to_string())?;
        Ok(tauri::ipc::Response::new(glb))
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
async fn fit_skeleton(template: String, path: String) -> Result<rig::FittedSkeleton, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| format!("cannot read the file: {e}"))?;
        rig::fit(&template, &bytes).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Binds the mesh to the fitted skeleton.
///
/// Off the main thread for the same reason as fitting, and more so: this
/// voxelises, solves a geodesic field per bone and then assigns weights.
/// Measured on the reference body — 7,399 vertices, 66 bones — the whole thing
/// takes about 56 ms, which is why there is still no progress event to report
/// (todo P3-8b). A model an order of magnitude larger will change that.
#[tauri::command]
async fn bind_weights(
    path: String,
    skeleton: rig::FittedSkeleton,
    falloff: f32,
) -> Result<rig::BindReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| format!("cannot read the file: {e}"))?;
        rig::bind(&bytes, &skeleton, falloff).map_err(|e| e.to_string())
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
            return Ok(None);
        };
        let target = chosen.into_path().map_err(|e| e.to_string())?;

        let source = std::fs::read(&path).map_err(|e| format!("cannot read the model: {e}"))?;
        let library = match &clip {
            Some(_) => Some(library_bytes(&app, &template)?),
            None => None,
        };
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
        std::fs::write(&target, &bytes).map_err(|e| format!("cannot write the export: {e}"))?;

        Ok(Some(target.file_name().map_or_else(String::new, |n| {
            n.to_string_lossy().into_owned()
        })))
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
            import_model,
            load_model,
            skeleton_templates,
            fit_skeleton,
            bind_weights,
            export_model,
            animation_clips,
            preview_animation,
            weight_overlay
        ])
        .run(tauri::generate_context!())
        .expect("failed to start mesh2motion");
}
