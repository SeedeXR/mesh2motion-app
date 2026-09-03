//! Writing the binary FBX container.
//!
//! The exact inverse of [`crate::fbx::binary::parse`], and the first half of
//! the FBX writer: this turns an [`FbxDocument`] back into bytes, while the
//! builders that turn scene data into a document are a separate concern.
//!
//! Splitting it that way buys a test that needs no reference implementation at
//! all — `parse(encode(parse(bytes)))` must equal `parse(bytes)` for every real
//! file we have. A round trip through the document, rather than through the
//! bytes, is the right equality: the format has legitimate freedoms (whether an
//! array is deflated, how a footer is padded) that change the bytes without
//! changing what the file means.
//!
//! # What this writes, and what it does not
//!
//! Arrays are deflated when that is smaller (`encoding = 1`), which is what
//! real exporters emit; raw arrays are legal but make the file about 2.5x
//! larger. Nothing else about the container is simplified: offsets, the null
//! records that terminate node lists, and the 16-byte footer magic are all as
//! the format requires, because [`crate::fbx::binary::parse`] validates each of
//! them.

use crate::fbx::binary::{FbxDocument, FbxNode, FbxProperty};
use crate::fbx::FbxError;

/// The 23-byte header, including the two bytes that follow the text.
const MAGIC: &[u8; 23] = b"Kaydara FBX Binary  \x00\x1a\x00";

/// The 16 bytes every binary FBX ends with.
const FOOTER_MAGIC: [u8; 16] = [
    0xf8, 0x5a, 0x8c, 0x6a, 0xde, 0xf5, 0xd9, 0x7e, 0xec, 0xe9, 0x0c, 0xe3, 0x75, 0x8f, 0x29, 0x0b,
];

/// The footer's leading 16-byte id. The Autodesk FBX SDK validates it as a CRC
/// over the file's `CreationTime` and `FileId`, so the three are one fixed set:
/// this id is valid only with `CREATION_TIME`/`FILE_ID` in `build.rs`. These are
/// the constants Blender writes (its "timedate hack"), a triple the SDK accepts.
const FOOTER_ID: [u8; 16] = [
    0xfa, 0xbc, 0xab, 0x09, 0xd0, 0xc8, 0xd4, 0x66, 0xb1, 0x76, 0xfb, 0x83, 0x1c, 0xf7, 0x26, 0x7e,
];

/// From this version on, node offsets and counts are 64-bit.
const WIDE_OFFSETS_FROM: u32 = 7500;

/// A node name must fit in the single byte that precedes it.
const MAX_NAME_LEN: usize = u8::MAX as usize;

/// Encodes a document as a binary FBX file.
///
/// # Errors
///
/// Fails only on data the format cannot represent: a node name longer than
/// 255 bytes, or a file so large that a node offset does not fit the width the
/// declared version uses.
pub fn encode(document: &FbxDocument) -> Result<Vec<u8>, FbxError> {
    let wide = document.version >= WIDE_OFFSETS_FROM;
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&document.version.to_le_bytes());

    for node in &document.roots {
        write_node(&mut out, node, wide)?;
    }
    // The top-level list is terminated the same way a child list is.
    write_null_record(&mut out, wide);
    write_footer(&mut out, document.version);
    Ok(out)
}

/// Writes one node and its subtree.
///
/// The end offset is absolute and cannot be known until the subtree has been
/// written, so it is reserved and backpatched. Computing sizes bottom-up first
/// would work too, and would mean two places that must agree about how large
/// every property is.
fn write_node(out: &mut Vec<u8>, node: &FbxNode, wide: bool) -> Result<(), FbxError> {
    if node.name.len() > MAX_NAME_LEN {
        return Err(FbxError::Malformed {
            what: "node name",
            detail: format!(
                "{} bytes exceeds the 255 a name may occupy",
                node.name.len()
            ),
        });
    }

    let end_offset_at = out.len();
    write_size(out, 0, wide); // end offset, backpatched below
    write_size(out, node.properties.len(), wide);
    let property_bytes_at = out.len();
    write_size(out, 0, wide); // property byte length, backpatched below

    out.push(node.name.len() as u8);
    out.extend_from_slice(node.name.as_bytes());

    let properties_start = out.len();
    for property in &node.properties {
        write_property(out, property);
    }
    let property_bytes = out.len() - properties_start;

    if !node.children.is_empty() || node.empty_scope {
        for child in &node.children {
            write_node(out, child, wide)?;
        }
        // A node gets one when it has children, and also when it declared an
        // empty list. Measured on the reference rig, which an FBX SDK exporter
        // wrote: 5,144 childless nodes declare no list and exactly 3 declare an
        // empty one — `References` and its two `AnimationLayer`s. Writing it
        // always breaks the three.js loader; writing it never makes assimp read
        // the file with no animation at all. Neither blanket rule is right, so
        // the distinction is carried on the node.
        write_null_record(out, wide);
    }

    let end = out.len();
    backpatch(out, end_offset_at, end, wide)?;
    backpatch(out, property_bytes_at, property_bytes, wide)?;
    Ok(())
}

/// Overwrites a reserved size field.
fn backpatch(out: &mut [u8], at: usize, value: usize, wide: bool) -> Result<(), FbxError> {
    if wide {
        let bytes = (value as u64).to_le_bytes();
        out[at..at + 8].copy_from_slice(&bytes);
    } else {
        let narrow = u32::try_from(value).map_err(|_| FbxError::Malformed {
            what: "node offset",
            detail: format!(
                "{value} does not fit the 32-bit offsets this FBX version uses; \
                 write version {WIDE_OFFSETS_FROM} or later for a file this large"
            ),
        })?;
        out[at..at + 4].copy_from_slice(&narrow.to_le_bytes());
    }
    Ok(())
}

/// Writes a size field at the width the version uses.
fn write_size(out: &mut Vec<u8>, value: usize, wide: bool) {
    if wide {
        out.extend_from_slice(&(value as u64).to_le_bytes());
    } else {
        out.extend_from_slice(&(value as u32).to_le_bytes());
    }
}

/// The all-zero record that ends a node list.
fn write_null_record(out: &mut Vec<u8>, wide: bool) {
    write_size(out, 0, wide);
    write_size(out, 0, wide);
    write_size(out, 0, wide);
    out.push(0); // name length
}

/// Writes the 160-byte trailer and the magic the reader checks for.
///
/// The padding is not decorative: the reader treats "within about 176 bytes of
/// the end" as the footer and stops parsing nodes there, so a trailer shorter
/// than that would make the last node unreachable.
fn write_footer(out: &mut Vec<u8>, version: u32) {
    out.extend_from_slice(&FOOTER_ID); // CRC over CreationTime/FileId; see FOOTER_ID.
                                       // Pad to a 16-byte boundary, as the format does.
    let padding = (16 - (out.len() % 16)) % 16;
    out.extend(std::iter::repeat_n(0u8, padding));
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&version.to_le_bytes());
    out.extend_from_slice(&[0u8; 120]);
    out.extend_from_slice(&FOOTER_MAGIC);
}

/// Writes one property, type code first.
fn write_property(out: &mut Vec<u8>, property: &FbxProperty) {
    match property {
        FbxProperty::Bool(v) => {
            out.push(b'C');
            // The reader takes the low bit, and real files write 'Y'/'T' here
            // rather than 1/0. Either round-trips; 1 and 0 are the clearer
            // choice for a file we are writing ourselves.
            out.push(u8::from(*v));
        }
        FbxProperty::I16(v) => {
            out.push(b'Y');
            out.extend_from_slice(&v.to_le_bytes());
        }
        FbxProperty::I32(v) => {
            out.push(b'I');
            out.extend_from_slice(&v.to_le_bytes());
        }
        FbxProperty::I64(v) => {
            out.push(b'L');
            out.extend_from_slice(&v.to_le_bytes());
        }
        FbxProperty::F32(v) => {
            out.push(b'F');
            out.extend_from_slice(&v.to_le_bytes());
        }
        FbxProperty::F64(v) => {
            out.push(b'D');
            out.extend_from_slice(&v.to_le_bytes());
        }
        FbxProperty::Str(v) => {
            out.push(b'S');
            out.extend_from_slice(&(v.len() as u32).to_le_bytes());
            out.extend_from_slice(v.as_bytes());
        }
        FbxProperty::Raw(v) => {
            out.push(b'R');
            out.extend_from_slice(&(v.len() as u32).to_le_bytes());
            out.extend_from_slice(v);
        }
        FbxProperty::BoolArray(v) => {
            write_array(out, b'b', v.len(), v.len(), |out| {
                out.extend(v.iter().map(|&b| u8::from(b)));
            });
        }
        FbxProperty::I32Array(v) => {
            write_array(out, b'i', v.len(), v.len() * 4, |out| {
                out.extend(v.iter().flat_map(|x| x.to_le_bytes()));
            });
        }
        FbxProperty::I64Array(v) => {
            write_array(out, b'l', v.len(), v.len() * 8, |out| {
                out.extend(v.iter().flat_map(|x| x.to_le_bytes()));
            });
        }
        FbxProperty::F32Array(v) => {
            write_array(out, b'f', v.len(), v.len() * 4, |out| {
                out.extend(v.iter().flat_map(|x| x.to_le_bytes()));
            });
        }
        FbxProperty::F64Array(v) => {
            write_array(out, b'd', v.len(), v.len() * 8, |out| {
                out.extend(v.iter().flat_map(|x| x.to_le_bytes()));
            });
        }
    }
}

/// Writes an array header and its payload, deflating it when that is smaller.
///
/// Real exporters compress array payloads, and a file with raw arrays is around
/// 2.5x larger than the same data as an FBX SDK writer would emit. The encoding
/// word says which was used, so a reader handles either — but "legal" and "what
/// other tools expect" are not the same thing, and the size alone is worth it.
///
/// Compression is skipped when it would not shrink the payload, which is the
/// case for small arrays where the zlib header costs more than it saves.
fn write_array(
    out: &mut Vec<u8>,
    type_code: u8,
    count: usize,
    byte_len: usize,
    payload: impl FnOnce(&mut Vec<u8>),
) {
    let mut raw = Vec::with_capacity(byte_len);
    payload(&mut raw);

    // Level 6 is zlib's default: the point is interoperability and size, not
    // the last few percent, and higher levels cost noticeably more time on the
    // multi-megabyte arrays a rig produces.
    let deflated = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
    let (encoding, body) = if deflated.len() < raw.len() {
        (1u32, deflated)
    } else {
        (0u32, raw)
    };

    out.push(type_code);
    out.extend_from_slice(&(count as u32).to_le_bytes());
    out.extend_from_slice(&encoding.to_le_bytes());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
}
