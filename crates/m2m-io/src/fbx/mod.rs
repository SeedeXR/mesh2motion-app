//! FBX reading.
//!
//! Ported from `legacy/src/lib/io/fbx/`, which is a hand-written parser proven
//! against real Mixamo exports. `fbxcel` was evaluated and rejected in ADR A3:
//! binary-only, read-only, no ASCII, no export.
//!
//! # Divergence from the original, on purpose
//!
//! The TypeScript `BinaryParser` interleaves two jobs — decoding the binary
//! container, and reshaping nodes into a convenient object (`Properties70`
//! flattening, `Connections` collection, single-property collapsing). The
//! container format itself is only "named nodes with properties and children";
//! the reshaping is interpretation of what those nodes *mean*.
//!
//! They are split here: this module produces a faithful, typed node tree, and
//! the semantic reshaping belongs with the DOM layer in P2-3. That makes the
//! container decoder testable on its own, which the original is not.
//!
//! ## What the split costs, and the invariant that covers it
//!
//! Everything `parseSubNode` consumes is recoverable from the tree — `id`,
//! `attrName` and `attrType` are just properties 0-2, and `Connections` and
//! `Properties70` are children — with one exception. The original computes
//! `singleProperty = numProperties == 1 && offset == endOffset`, which is a
//! fact about **file position**, and position is not retained here.
//!
//! The DOM layer must therefore approximate it as
//! `properties.len() == 1 && children.is_empty()`. Those agree unless a writer
//! emits a null record after a childless one-property node. Measured on the
//! reference export: 3414 nodes are `singleProperty`, and **zero** nodes have
//! one property and no children while sitting short of their end offset — so
//! the approximation is exact there. If a file ever disagrees, the node's byte
//! extent would have to be retained.

pub mod animation;
pub mod binary;
pub mod build;
pub mod dom;
pub mod encode;
pub mod geometry;
pub mod model;
pub mod reader;
pub mod skin;
pub mod text;
pub mod transform;

/// Errors produced while reading an FBX file.
///
/// Every variant is reachable from a malformed file. Parsing hostile input must
/// never panic (`memory/test.md` §4).
#[derive(Debug, thiserror::Error)]
pub enum FbxError {
    /// The file does not begin with the FBX binary magic.
    #[error("not an FBX binary file")]
    BadMagic,

    /// The declared FBX version predates what this parser supports.
    #[error("FBX version {0} is not supported; 6400 or later is required")]
    UnsupportedVersion(u32),

    /// A structure ran past the end of the buffer.
    #[error("truncated: needed {needed} bytes, file has {available}")]
    Truncated {
        /// Byte offset the read required.
        needed: usize,
        /// Bytes actually present.
        available: usize,
    },

    /// A property carried a type code this parser does not know.
    #[error("unknown property type {0:?} at offset {1}")]
    UnknownPropertyType(char, usize),

    /// Node nesting exceeded the depth limit.
    ///
    /// A guard against a crafted file whose nesting would otherwise recurse
    /// until the stack overflows — which aborts the process rather than
    /// unwinding, so it cannot be caught.
    #[error("node nesting deeper than {0}")]
    TooDeep(usize),

    /// A declared array or buffer length is impossible for the file's size.
    #[error("declared length {declared} exceeds the {remaining} bytes remaining")]
    ImplausibleLength {
        /// Length the file claimed.
        declared: usize,
        /// Bytes actually left.
        remaining: usize,
    },

    /// One file's arrays would decompress to more than the reader allows.
    ///
    /// Distinct from [`Self::ImplausibleLength`], which is about a length the
    /// file could not possibly satisfy. Nothing here is truncated or
    /// inconsistent: the file is simply asking for more memory than a reader
    /// will spend, and saying so in the language of "bytes remaining" would
    /// send anyone reading the message hunting a truncation that is not there.
    #[error("decompressing this file would need {total} bytes, over the {limit}-byte limit")]
    InflateBudgetExceeded {
        /// Running total, including the array that broke the limit.
        total: usize,
        /// The ceiling that was exceeded.
        limit: usize,
    },

    /// Decompressing a property array failed.
    #[error("zlib decompression failed: {0}")]
    Inflate(String),

    /// The file does not end with the FBX footer magic.
    ///
    /// Almost always truncation. The end-of-content test is a heuristic on
    /// offsets, so a file cut inside its last root node otherwise parses to a
    /// document that looks whole but has sections missing.
    #[error("missing FBX footer; the file is probably truncated")]
    MissingFooter,

    /// A value did not match the shape its context requires.
    #[error("malformed {what}: {detail}")]
    Malformed {
        /// What was being read.
        what: &'static str,
        /// What was found instead.
        detail: String,
    },

    /// A node's declared end offset points backwards or past the file.
    #[error("node at {at} declares end offset {end}, which is not reachable")]
    BadNodeExtent {
        /// Where the node started.
        at: usize,
        /// The end offset it declared.
        end: usize,
    },
}
