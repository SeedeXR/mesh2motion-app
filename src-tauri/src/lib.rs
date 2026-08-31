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
        name: path
            .file_name()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        import: m2m_io::import::inspect(&bytes).map_err(|e| e.to_string())?,
    }))
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
            import_model
        ])
        .run(tauri::generate_context!())
        .expect("failed to start mesh2motion");
}
