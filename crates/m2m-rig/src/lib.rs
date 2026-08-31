//! Creature rig templates, skeleton fitting, and animation retargeting.
//!
//! # Design rule
//!
//! Templates are **data, not code**. Adding a creature must never require a
//! change in this crate — if it does, the template format is wrong and the
//! format gets fixed, not special-cased. See `memory/instruction.md` §6.
//!
//! This is the direct lesson of the legacy per-body-part weight correctors,
//! where every new creature type meant new hand-written correction code.
//!
//! [`template`] defines that format: a skeleton described as typed chains.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod automap;
pub mod fit;
pub mod retarget;
pub mod template;
