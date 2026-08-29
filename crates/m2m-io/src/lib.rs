//! Mesh and animation file I/O for mesh2motion.
//!
//! # Trust boundary
//!
//! Every parser here reads **hostile input**. A malformed file must return an
//! error — never panic, never hang, never OOM. No `unwrap`/`expect` outside
//! tests. Fuzz targets are required for each format (`memory/test.md` §5).
//!
//! Empty until P2-1 ports the FBX binary reader. The error type is deliberately
//! not pre-declared: the failure modes come from the parser, not from guessing.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
