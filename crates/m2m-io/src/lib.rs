//! Mesh and animation file I/O for mesh2motion.
//!
//! # Trust boundary
//!
//! Every parser here reads **hostile input**. A malformed file must return an
//! error — never panic, never hang, never OOM. No `unwrap`/`expect` outside
//! tests. Fuzz targets are required for each format (`memory/test.md` §5).
//!
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod fbx;
