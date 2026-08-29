//! Tauri command layer.
//!
//! # Boundary rule
//!
//! This crate is a **thin adapter**: it deserialises arguments, calls into the
//! `m2m-*` crates, and serialises results. No algorithms live here. Anything
//! worth testing belongs in a library crate where it can be tested without a
//! window. See `memory/architecture.md` §2.

#![forbid(unsafe_code)]

use serde::Serialize;

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
        .invoke_handler(tauri::generate_handler![build_info, report_startup])
        .run(tauri::generate_context!())
        .expect("failed to start mesh2motion");
}
