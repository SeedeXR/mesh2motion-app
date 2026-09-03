//! FBX binary container decoding.
//!
//! Produces a faithful node tree. Interpretation of what the nodes mean is the
//! DOM layer's job — see the divergence note in [`crate::fbx`].

use crate::fbx::reader::Cursor;
use crate::fbx::FbxError;

/// Bytes of magic before the version field: `"Kaydara FBX Binary  "` plus the
/// three-byte terminator `00 1A 00`.
const MAGIC: &[u8] = b"Kaydara FBX Binary  \x00\x1a\x00";

/// The 16 bytes every FBX binary file ends with.
///
/// Verified identical across eight real Mixamo exports from two separate
/// batches. Checking it is the only reliable way to tell a complete file from a
/// truncated one: the end-of-content test is a heuristic on offsets, so a file
/// cut anywhere inside the last root node stops the parse early and returns a
/// document that looks structurally fine. Measured: removing 578 bytes from the
/// 2.1 MB reference file — 0.03% — silently dropped the entire `Takes` section,
/// which is every animation stack in the file.
const FOOTER_MAGIC: [u8; 16] = [
    0xf8, 0x5a, 0x8c, 0x6a, 0xde, 0xf5, 0xd9, 0x7e, 0xec, 0xe9, 0x0c, 0xe3, 0x75, 0x8f, 0x29, 0x0b,
];

/// Deflate cannot expand input by more than this ratio.
///
/// Bounds a compression bomb by what the format can actually produce, rather
/// than only by an absolute ceiling.
const MAX_DEFLATE_RATIO: usize = 1032;

/// Versions below this used a different node layout and are not supported,
/// matching the original parser.
const MIN_VERSION: u32 = 6400;

/// From this version on, node offsets and counts are 64-bit rather than 32-bit.
const WIDE_OFFSETS_FROM: u32 = 7500;

/// Maximum node nesting.
///
/// Real files are nested single digits deep; this only has to be far enough
/// above that to never trip legitimately. Without it a crafted file recurses
/// until the stack overflows, which aborts rather than unwinds.
const MAX_DEPTH: usize = 256;

/// Ceiling on a single decompressed property array.
///
/// A zlib stream can expand enormously from a few bytes. 256 MB is 32 million
/// f64s — a mesh of tens of millions of triangles, far above any character rig
/// and far below exhausting memory.
const MAX_INFLATED_BYTES: usize = 256 * 1024 * 1024;

/// Ceiling on everything one file may decompress, added together.
///
/// [`MAX_INFLATED_BYTES`] is per property, and the deflate-ratio guard only
/// requires about 254 KB of compressed input per 256 MB array — so a 2 MB file
/// can carry eight of them and retain 2 GB in the node tree while no single
/// property ever exceeds its own limit. Every inflated array stays alive for
/// the lifetime of the document, so the total is what actually bounds memory.
///
/// Note this bounds what is RETAINED. While the last array is decoded, its
/// inflate buffer and the typed `Vec` built from it are both alive on top of
/// everything already kept, so a file accepted at the limit peaks nearer twice
/// this. Budget accordingly when judging what a desktop app survives.
///
/// The reference rig decompresses 1.5 MB in total, and this tool's stated
/// priority is low memory use, so half a gigabyte is already generous: it is
/// two orders of magnitude above any real rig while keeping a malicious file
/// inside what a desktop app can survive.
const MAX_INFLATED_BYTES_PER_FILE: usize = 512 * 1024 * 1024;

/// Tracks what one file has decompressed so far.
///
/// Threaded by `&mut` rather than held in the `Cursor`: the cursor is a
/// general byte reader shared with the ASCII path, and a deflate budget is a
/// fact about this format, not about reading bytes.
///
/// # Why only inflation is charged
///
/// Deflate is the one path where output is not bounded by input. Every other
/// allocation sized from a declared length — an uncompressed array, an `S`
/// string, an `R` blob — calls `check_capacity` first, which rejects anything
/// larger than the bytes actually remaining, and each then consumes what it
/// read. So their total across a file cannot exceed the file's own size. A
/// zlib stream has no such ceiling, which is why it is the only thing here
/// that needs a running total.
struct InflateBudget {
    used: usize,
}

impl InflateBudget {
    /// Charges `bytes` against the budget, or fails if it would be exceeded.
    fn charge(&mut self, bytes: usize) -> Result<(), FbxError> {
        self.used = self.used.saturating_add(bytes);
        if self.used > MAX_INFLATED_BYTES_PER_FILE {
            return Err(FbxError::InflateBudgetExceeded {
                total: self.used,
                limit: MAX_INFLATED_BYTES_PER_FILE,
            });
        }
        Ok(())
    }
}

/// A property value attached to an FBX node.
#[derive(Debug, Clone, PartialEq)]
pub enum FbxProperty {
    /// `C` — boolean.
    Bool(bool),
    /// `Y` — 16-bit integer.
    I16(i16),
    /// `I` — 32-bit integer.
    I32(i32),
    /// `L` — 64-bit integer.
    I64(i64),
    /// `F` — 32-bit float.
    F32(f32),
    /// `D` — 64-bit float.
    F64(f64),
    /// `S` — string.
    Str(String),
    /// `R` — opaque bytes.
    Raw(Vec<u8>),
    /// `b` or `c` — boolean array.
    BoolArray(Vec<bool>),
    /// `i` — 32-bit integer array.
    I32Array(Vec<i32>),
    /// `l` — 64-bit integer array.
    I64Array(Vec<i64>),
    /// `f` — 32-bit float array.
    F32Array(Vec<f32>),
    /// `d` — 64-bit float array.
    F64Array(Vec<f64>),
}

impl FbxProperty {
    /// This property as an integer, whatever width it was stored at.
    ///
    /// ASCII FBX carries no type codes, so a reader cannot know whether `100`
    /// was written as an `i32` or an `i64` — binary yields `I32`, ASCII yields
    /// `I64` for the identical source line. Rather than guess a width, both
    /// stay faithful to what they can know and consumers ask by meaning.
    /// Returns `None` for a non-integral float.
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Self::I16(v) => Some(i64::from(*v)),
            Self::I32(v) => Some(i64::from(*v)),
            Self::I64(v) => Some(*v),
            Self::F32(v) if v.fract() == 0.0 => Some(*v as i64),
            Self::F64(v) if v.fract() == 0.0 => Some(*v as i64),
            Self::Bool(v) => Some(i64::from(*v)),
            _ => None,
        }
    }

    /// This property as a float, whatever width it was stored at.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::I16(v) => Some(f64::from(*v)),
            Self::I32(v) => Some(f64::from(*v)),
            Self::I64(v) => Some(*v as f64),
            Self::F32(v) => Some(f64::from(*v)),
            Self::F64(v) => Some(*v),
            _ => None,
        }
    }

    /// This property as a string, if it is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }

    /// This property as a numeric array, whatever element type it was stored at.
    ///
    /// Binary distinguishes `I32Array` from `F64Array`; ASCII cannot, and yields
    /// `F64Array` for both. Consumers that want numbers should ask for numbers.
    pub fn as_f64_vec(&self) -> Option<Vec<f64>> {
        match self {
            Self::F64Array(v) => Some(v.clone()),
            Self::F32Array(v) => Some(v.iter().map(|&x| f64::from(x)).collect()),
            Self::I32Array(v) => Some(v.iter().map(|&x| f64::from(x)).collect()),
            Self::I64Array(v) => Some(v.iter().map(|&x| x as f64).collect()),
            _ => None,
        }
    }

    /// This property as an integer array, whatever element type it was stored at.
    ///
    /// Returns `None` if any element is not integral.
    pub fn as_i64_vec(&self) -> Option<Vec<i64>> {
        match self {
            Self::I32Array(v) => Some(v.iter().map(|&x| i64::from(x)).collect()),
            Self::I64Array(v) => Some(v.clone()),
            Self::F32Array(v) => v
                .iter()
                .map(|&x| (x.fract() == 0.0).then_some(x as i64))
                .collect(),
            Self::F64Array(v) => v
                .iter()
                .map(|&x| (x.fract() == 0.0).then_some(x as i64))
                .collect(),
            _ => None,
        }
    }
}

/// Splits an FBX object name into its name and class.
///
/// The two formats encode this differently and neither is normalised in place,
/// because collapsing them would discard the class:
///
/// - binary writes `"Bob\0\x01Geometry"` — name, separator, class
/// - ASCII writes `"Geometry::Bob"` — class, separator, name
///
/// Returns `(name, class)`, with an empty class when there is no separator.
pub fn split_object_name(raw: &str) -> (&str, &str) {
    if let Some((name, class)) = raw.split_once('\u{0}') {
        return (name, class.trim_start_matches('\u{1}'));
    }
    if let Some((class, name)) = raw.split_once("::") {
        return (name, class);
    }
    (raw, "")
}

/// A node in the FBX tree.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FbxNode {
    /// Node name, e.g. `Objects`, `Geometry`, `P`.
    pub name: String,
    /// Properties, in file order.
    pub properties: Vec<FbxProperty>,
    /// Nested nodes, in file order.
    pub children: Vec<FbxNode>,
    /// Whether the record declared a nested list that turned out to be empty,
    /// as opposed to declaring none at all.
    ///
    /// Only meaningful when `children` is empty, and it looks like a
    /// distinction without a difference until you write the file back out.
    /// **assimp reads an `AnimationLayer` written without the empty list as no
    /// layer at all**, so its stack has no layers and the file loads with zero
    /// animations — mesh and skeleton perfect, every keyframe silently gone.
    /// The reference rig has exactly three such nodes: `References` and its two
    /// `AnimationLayer`s. Its other 5,144 childless nodes declare no list, so
    /// "always write one" is equally wrong and breaks the three.js loader.
    pub empty_scope: bool,
}

impl FbxNode {
    /// First child with the given name.
    pub fn child(&self, name: &str) -> Option<&FbxNode> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Every child with the given name.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a FbxNode> {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Total nodes in this subtree, including itself.
    pub fn node_count(&self) -> usize {
        1 + self.children.iter().map(FbxNode::node_count).sum::<usize>()
    }
}

/// A parsed FBX binary file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FbxDocument {
    /// Version from the header, e.g. 7700 for FBX 7.7.
    pub version: u32,
    /// Top-level nodes.
    pub roots: Vec<FbxNode>,
}

impl FbxDocument {
    /// First top-level node with the given name.
    pub fn root(&self, name: &str) -> Option<&FbxNode> {
        self.roots.iter().find(|n| n.name == name)
    }

    /// Total nodes in the document.
    pub fn node_count(&self) -> usize {
        self.roots.iter().map(FbxNode::node_count).sum()
    }
}

/// Parses an FBX binary file.
///
/// # Errors
///
/// Returns an [`FbxError`] for any malformed input. This is a trust boundary:
/// it must never panic, hang, or exhaust memory on a hostile file.
pub fn parse(data: &[u8]) -> Result<FbxDocument, FbxError> {
    let mut cursor = Cursor::new(data);

    if data.len() < MAGIC.len() || &data[..MAGIC.len()] != MAGIC {
        return Err(FbxError::BadMagic);
    }
    cursor.skip(MAGIC.len())?;

    let version = cursor.read_u32()?;
    if version < MIN_VERSION {
        return Err(FbxError::UnsupportedVersion(version));
    }

    // One budget for the whole file: every inflated array stays alive in the
    // tree, so the total is what bounds memory, not any single property.
    let mut budget = InflateBudget { used: 0 };
    let mut roots = Vec::new();
    while !at_footer(&cursor) {
        match parse_node(&mut cursor, version, 0, &mut budget)? {
            Some(node) => roots.push(node),
            // A zero end-offset marks the end of a node list.
            None => break,
        }
    }

    // A valid FBX always carries at least FBXHeaderExtension. Without this a
    // header-only fragment parses "successfully" into an empty document,
    // because the footer heuristic fires immediately and the node loop never
    // runs.
    if roots.is_empty() {
        return Err(FbxError::Truncated {
            needed: MAGIC.len() + 4 + 1,
            available: data.len(),
        });
    }

    // The footer magic is what actually proves the file is whole. The loop
    // above stops on an offset heuristic, so without this a file cut inside its
    // last root node parses to a plausible-looking document with sections
    // silently missing.
    if data.len() < FOOTER_MAGIC.len() || data[data.len() - FOOTER_MAGIC.len()..] != FOOTER_MAGIC {
        return Err(FbxError::MissingFooter);
    }

    Ok(FbxDocument { version, roots })
}

/// Whether the cursor has reached the file footer.
///
/// The footer is 160 bytes plus padding to a 16-byte boundary. Reproduced from
/// the original's `endOfContent`, including its alignment special case — some
/// exporters embed a fixed 15 or 16 bytes of padding.
fn at_footer(cursor: &Cursor<'_>) -> bool {
    let size = cursor.len();
    let offset = cursor.offset();
    if size % 16 == 0 {
        ((offset + 160 + 16) & !0xf) >= size
    } else {
        offset + 160 + 16 >= size
    }
}

/// Parses one node and its subtree. `None` marks a null record.
fn parse_node(
    cursor: &mut Cursor<'_>,
    version: u32,
    depth: usize,
    budget: &mut InflateBudget,
) -> Result<Option<FbxNode>, FbxError> {
    if depth > MAX_DEPTH {
        return Err(FbxError::TooDeep(MAX_DEPTH));
    }

    let start = cursor.offset();
    let wide = version >= WIDE_OFFSETS_FROM;

    let end_offset = read_size(cursor, wide)?;
    let property_count = read_size(cursor, wide)?;
    // The property-list byte length is declared but unused: properties are
    // self-describing, and trusting a redundant length is how a malformed file
    // gets to disagree with itself.
    let _property_bytes = read_size(cursor, wide)?;

    let name_len = cursor.read_u8()? as usize;
    let name = cursor.read_string(name_len)?;

    // A zero end offset is the null record that terminates a node list.
    if end_offset == 0 {
        return Ok(None);
    }
    if end_offset > cursor.len() || end_offset <= start {
        return Err(FbxError::BadNodeExtent {
            at: start,
            end: end_offset,
        });
    }

    let mut properties = Vec::new();
    for _ in 0..property_count {
        properties.push(parse_property(cursor, budget)?);
    }

    // Whether the record declared a nested list at all. A node whose extent
    // stops at its properties declared none; one that runs on declared a list,
    // even if that list turns out to hold only the terminating null record.
    let declares_scope = cursor.offset() < end_offset;
    let mut children = Vec::new();
    while cursor.offset() < end_offset {
        match parse_node(cursor, version, depth + 1, budget)? {
            Some(child) => children.push(child),
            None => {
                // A null record terminates the child list, and the original
                // simply skips it and keeps looping. Breaking here and seeking
                // to end_offset would silently discard any siblings after it —
                // a parser differential against three.js that could hide nodes
                // from validation. Real files never do this (checked: zero
                // occurrences in the reference export), so treat it as
                // malformed rather than guessing which reading is intended.
                if cursor.offset() < end_offset {
                    return Err(FbxError::BadNodeExtent {
                        at: cursor.offset(),
                        end: end_offset,
                    });
                }
                break;
            }
        }
    }

    // Trust the declared extent over where parsing happened to stop. Children
    // are followed by a null record whose length varies with the offset width,
    // and the original relies on the same thing implicitly.
    cursor.seek(end_offset)?;

    let empty_scope = declares_scope && children.is_empty();
    Ok(Some(FbxNode {
        name,
        properties,
        children,
        empty_scope,
    }))
}

/// Reads a node header field, 64-bit from FBX 7500 onward.
fn read_size(cursor: &mut Cursor<'_>, wide: bool) -> Result<usize, FbxError> {
    let raw = if wide {
        cursor.read_u64()?
    } else {
        u64::from(cursor.read_u32()?)
    };
    usize::try_from(raw).map_err(|_| FbxError::ImplausibleLength {
        declared: usize::MAX,
        remaining: cursor.remaining(),
    })
}

/// Rejects a declared element count that the remaining bytes cannot hold.
///
/// Checked before allocating, so a file claiming four billion elements fails
/// immediately instead of attempting the reservation.
fn check_capacity(cursor: &Cursor<'_>, count: usize, bytes_each: usize) -> Result<(), FbxError> {
    let needed = count.saturating_mul(bytes_each);
    if needed > cursor.remaining() {
        return Err(FbxError::ImplausibleLength {
            declared: needed,
            remaining: cursor.remaining(),
        });
    }
    Ok(())
}

/// Reads `count` elements with `read`, pre-checking the declared size.
fn read_array<T>(
    cursor: &mut Cursor<'_>,
    count: usize,
    bytes_each: usize,
    mut read: impl FnMut(&mut Cursor<'_>) -> Result<T, FbxError>,
) -> Result<Vec<T>, FbxError> {
    check_capacity(cursor, count, bytes_each)?;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        out.push(read(cursor)?);
    }
    Ok(out)
}

/// Parses one property value.
fn parse_property(
    cursor: &mut Cursor<'_>,
    budget: &mut InflateBudget,
) -> Result<FbxProperty, FbxError> {
    let at = cursor.offset();
    let type_code = cursor.read_u8()?;

    match type_code {
        b'C' => Ok(FbxProperty::Bool(cursor.read_bool()?)),
        b'Y' => Ok(FbxProperty::I16(cursor.read_i16()?)),
        b'I' => Ok(FbxProperty::I32(cursor.read_i32()?)),
        b'L' => Ok(FbxProperty::I64(cursor.read_i64()?)),
        b'F' => Ok(FbxProperty::F32(cursor.read_f32()?)),
        b'D' => Ok(FbxProperty::F64(cursor.read_f64()?)),
        b'S' => {
            let len = cursor.read_u32()? as usize;
            check_capacity(cursor, len, 1)?;
            Ok(FbxProperty::Str(cursor.read_string(len)?))
        }
        b'R' => {
            let len = cursor.read_u32()? as usize;
            check_capacity(cursor, len, 1)?;
            Ok(FbxProperty::Raw(cursor.take(len)?.to_vec()))
        }
        b'b' | b'c' | b'd' | b'f' | b'i' | b'l' => parse_array_property(cursor, type_code, budget),
        other => Err(FbxError::UnknownPropertyType(other as char, at)),
    }
}

/// Parses an array property, inflating it first if it is compressed.
fn parse_array_property(
    cursor: &mut Cursor<'_>,
    type_code: u8,
    budget: &mut InflateBudget,
) -> Result<FbxProperty, FbxError> {
    let count = cursor.read_u32()? as usize;
    let encoding = cursor.read_u32()?;
    let compressed_len = cursor.read_u32()? as usize;

    let element_bytes = match type_code {
        b'b' | b'c' => 1,
        b'i' | b'f' => 4,
        b'd' | b'l' => 8,
        _ => unreachable!("caller restricts the type code"),
    };

    if encoding == 0 {
        return read_typed_array(cursor, type_code, count, element_bytes);
    }

    check_capacity(cursor, compressed_len, 1)?;
    let compressed = cursor.take(compressed_len)?;

    // Bound the inflate three ways. A zlib stream can expand by orders of
    // magnitude, and the decompressed Vec is retained in the tree, so a handful
    // of bombs in one small file would otherwise peak at gigabytes.
    let expected = count.saturating_mul(element_bytes);
    if expected > MAX_INFLATED_BYTES {
        return Err(FbxError::ImplausibleLength {
            declared: expected,
            remaining: MAX_INFLATED_BYTES,
        });
    }
    // Deflate cannot exceed this ratio, so a declared size far above it means
    // the header is lying before a single byte is inflated.
    if expected > compressed_len.saturating_mul(MAX_DEFLATE_RATIO) {
        return Err(FbxError::ImplausibleLength {
            declared: expected,
            remaining: compressed_len.saturating_mul(MAX_DEFLATE_RATIO),
        });
    }
    // The limit is the DECLARED size, not the absolute ceiling: an FBX array
    // inflates to exactly count * element_bytes, so this costs nothing on a
    // legitimate file while stopping a property that declares one element from
    // inflating half a gigabyte.
    // Charged BEFORE inflating, so a file that would exceed the total is
    // rejected without allocating the array that pushes it over.
    budget.charge(expected)?;
    let inflated =
        miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(compressed, expected.max(1))
            .map_err(|e| FbxError::Inflate(format!("{e:?}")))?;

    let mut inner = Cursor::new(&inflated);
    read_typed_array(&mut inner, type_code, count, element_bytes)
}

/// Reads `count` elements of the given FBX array type.
fn read_typed_array(
    cursor: &mut Cursor<'_>,
    type_code: u8,
    count: usize,
    element_bytes: usize,
) -> Result<FbxProperty, FbxError> {
    Ok(match type_code {
        b'b' | b'c' => {
            FbxProperty::BoolArray(read_array(cursor, count, element_bytes, |c| c.read_bool())?)
        }
        b'i' => FbxProperty::I32Array(read_array(cursor, count, element_bytes, |c| c.read_i32())?),
        b'l' => FbxProperty::I64Array(read_array(cursor, count, element_bytes, |c| c.read_i64())?),
        b'f' => FbxProperty::F32Array(read_array(cursor, count, element_bytes, |c| c.read_f32())?),
        b'd' => FbxProperty::F64Array(read_array(cursor, count, element_bytes, |c| c.read_f64())?),
        _ => unreachable!("caller restricts the type code"),
    })
}
