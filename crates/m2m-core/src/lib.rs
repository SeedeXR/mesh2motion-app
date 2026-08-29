//! Pure geometry and skinning solver for mesh2motion.
//!
//! # Boundary rule
//!
//! This crate depends on nothing else in the workspace and performs **no I/O**.
//! Everything here is a pure function over plain data, which is what makes it
//! unit-testable and benchmarkable with no window, webview, or Tauri runtime.
//!
//! Enforced in CI by the `arch-gate` job. See `memory/architecture.md` §2.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod mesh;
pub mod skinning;

/// Errors produced by core geometry and solver operations.
#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    /// The mesh has no vertices, or a buffer length is not a multiple of its stride.
    #[error("invalid mesh: {0}")]
    InvalidMesh(String),
}

/// Convenience alias for results in this crate.
pub type Result<T> = std::result::Result<T, CoreError>;
