//! Tauri command layer.
//!
//! # Boundary rule
//!
//! This crate is a **thin adapter**: it deserialises arguments, calls into the
//! `m2m-*` crates, and serialises results. No algorithms live here. Anything
//! worth testing belongs in a library crate where it can be tested without a
//! window. See `memory/architecture.md` §2.

#![forbid(unsafe_code)]

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
    let document = m2m_io::import::load(&bytes).map_err(|e| e.to_string())?;
    let glb = m2m_io::glb::write(&document).map_err(|e| e.to_string())?;
    Ok(tauri::ipc::Response::new(glb))
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
            load_model
        ])
        .run(tauri::generate_context!())
        .expect("failed to start mesh2motion");
}
