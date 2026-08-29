//! Blender DCC bridge.
//!
//! Two modes:
//! - **headless** — spawn `Blender -b --python`, used for automated visual
//!   regression (`memory/test.md` §9)
//! - **live** — attach to a running Blender via the companion add-on for
//!   artist round-tripping
//!
//! Protocol is JSON-RPC over stdio. See `memory/architecture.md` §6.
//!
//! Empty until P4-1. The Blender path is resolved at runtime, not hardcoded —
//! `/Applications/Blender.app` is where it happens to sit on the reference
//! machine, not a guarantee.

#![forbid(unsafe_code)]
#![warn(missing_docs)]
