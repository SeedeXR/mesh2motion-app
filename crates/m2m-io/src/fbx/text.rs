//! FBX ASCII parsing.
//!
//! Produces the same [`FbxDocument`] as the binary reader, so the DOM layer in
//! P2-3 never has to care which format a file arrived in.
//!
//! # Normalisation, and where the line is
//!
//! The two formats express the same tree differently. Binary stores a vertex
//! array as a property directly on its `Vertices` node; ASCII writes
//! `Vertices: *9 { a: 0,0,0,... }`, putting the numbers on a child called `a`.
//! That difference is a **format** quirk, so it is reconciled here: an `a:`
//! array is hoisted onto its parent as a property, matching the binary layout.
//!
//! **Semantic** reshaping — `Properties70` flattening, `Connections`
//! collection — stays in P2-3, exactly as for binary. The split is: each reader
//! knows its own file format's quirks; only the DOM layer knows what the nodes
//! mean.
//!
//! # Ported bug fixes
//!
//! The legacy `TextParser` carries three fixes over upstream three.js, each
//! with a regression test that comes across with it:
//!
//! 1. A `{` inside a property *value* — a Windows path containing `{Project}`,
//!    a GUID, a material name — was read as a block delimiter, desynchronising
//!    the indent so every later node was silently discarded.
//! 2. Document-level properties (`CreationTime:`, `Creator:`) sit outside any
//!    block, and upstream dereferenced the absent node.
//! 3. A stray closing brace emptied the node stack and corrupted the tree.

use crate::fbx::binary::{FbxDocument, FbxNode, FbxProperty};
use crate::fbx::FbxError;

/// How much of a file to inspect when sniffing the format.
const SNIFF_BYTES: usize = 1024;

/// Maximum node nesting, matching the binary reader's guard.
const MAX_DEPTH: usize = 256;

/// Whether `text` looks like an ASCII FBX document.
///
/// Sniffs for the header node rather than sampling a fixed offset. Upstream
/// compared byte 6 against the binary magic, which rejected any file lacking
/// the usual `; FBX 7.x.x project file` comment block.
pub fn is_ascii_fbx(text: &str) -> bool {
    if text.starts_with("Kaydara FBX Binary") {
        return false;
    }
    // Byte-wise, not by slicing `&str`. `&text[..1024]` panics when the cut
    // lands inside a multibyte character, and exporters put non-ASCII creator
    // names and paths in the header — a panic on the trust boundary.
    let head = &text.as_bytes()[..text.len().min(SNIFF_BYTES)];
    head.windows(18).any(|w| w == b"FBXHeaderExtension")
        || head.windows(11).any(|w| w == b"FBXVersion:")
}

/// Reads the version from an ASCII document, tolerating any spacing after the colon.
pub fn ascii_version(text: &str) -> Option<u32> {
    let at = text.find("FBXVersion:")? + "FBXVersion:".len();
    let digits: String = text[at..]
        .chars()
        .skip_while(|c| c.is_whitespace())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse().ok()
}

/// One classified line of an ASCII FBX document.
enum Line<'a> {
    /// Blank or a comment.
    Ignored,
    /// `Name: attrs {` — opens a block.
    BlockStart { name: &'a str, attrs: &'a str },
    /// `}` — closes a block.
    BlockEnd,
    /// `Name: value`.
    Property { name: &'a str, value: &'a str },
    /// A bare line continuing the previous array or `Content:` payload.
    Continuation(&'a str),
}

/// Leading tab count, which is the node depth this line belongs at.
fn indent_of(line: &str) -> usize {
    line.bytes().take_while(|&b| b == b'\t').count()
}

/// Classifies a line by its content.
fn classify(line: &str) -> Line<'_> {
    let body = line.trim_start_matches('\t');
    let trimmed = body.trim();

    if trimmed.is_empty() || trimmed.starts_with(';') {
        return Line::Ignored;
    }
    if trimmed == "}" {
        return Line::BlockEnd;
    }

    // A colon before any brace means a named line. Splitting on the FIRST colon
    // matters: values contain colons (timestamps, `Geometry::name`).
    if let Some(colon) = body.find(':') {
        let name = body[..colon].trim();
        let rest = body[colon + 1..].trim();

        // Only a brace at END of line opens a block. This is the fix for a
        // brace inside a value: `P: "DocumentUrl", ..., "D:\Art\{Project}\x.fbx"`
        // must be a property, not a block start.
        if let Some(attrs) = rest.strip_suffix('{') {
            if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Line::BlockStart {
                    name,
                    attrs: attrs.trim(),
                };
            }
        }
        if !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') {
            return Line::Property { name, value: rest };
        }
    }

    Line::Continuation(trimmed)
}

/// Strips one layer of surrounding double quotes.
fn unquote(s: &str) -> &str {
    s.strip_prefix('"')
        .and_then(|t| t.strip_suffix('"'))
        .unwrap_or(s)
}

/// Converts one ASCII token into a typed property.
///
/// ASCII carries no type codes, so the type is inferred: quoted is a string, an
/// integral literal is an `i64`, anything else numeric is an `f64`, and what is
/// left stays a string.
fn value_of(token: &str) -> FbxProperty {
    let t = token.trim();
    if t.starts_with('"') {
        return FbxProperty::Str(unquote(t).to_string());
    }
    if let Ok(i) = t.parse::<i64>() {
        return FbxProperty::I64(i);
    }
    if let Ok(f) = t.parse::<f64>() {
        return FbxProperty::F64(f);
    }
    FbxProperty::Str(t.to_string())
}

/// Splits a comma-separated value list, respecting quotes.
///
/// Naive `split(',')` corrupts any value containing a comma — an object named
/// `"Model::Bob, Jr"` becomes two properties, and every positional index after
/// it shifts, so the DOM layer reading `properties[2]` as the attribute type
/// gets a fragment of the name instead.
fn split_values(list: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut in_quotes = false;
    for (i, c) in list.char_indices() {
        match c {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                out.push(&list[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    out.push(&list[start..]);
    out
}

/// Splits a comma-separated value list into typed properties.
fn values_of(list: &str) -> Vec<FbxProperty> {
    if list.trim().is_empty() {
        return Vec::new();
    }
    split_values(list).into_iter().map(value_of).collect()
}

/// Parses the numbers of an `a:` array line.
///
/// A token that is not a number is an error, not something to skip. Silently
/// dropping it would build a mesh from part of its vertices with nothing
/// reported anywhere.
fn number_array(text: &str) -> Result<Vec<f64>, FbxError> {
    text.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<f64>().map_err(|_| FbxError::Malformed {
                what: "array element",
                detail: s.chars().take(32).collect(),
            })
        })
        .collect()
}

/// Pops the innermost open node onto its parent, or into `roots` if it had no
/// parent. A stack that is already empty is left alone.
fn close_top(stack: &mut Vec<FbxNode>, roots: &mut Vec<FbxNode>) {
    if let Some(node) = stack.pop() {
        match stack.last_mut() {
            Some(parent) => parent.children.push(node),
            None => roots.push(node),
        }
    }
}

/// Closes open blocks down to `target` depth, flushing each one's pending array
/// and its declared length as it goes.
fn close_to(
    stack: &mut Vec<FbxNode>,
    roots: &mut Vec<FbxNode>,
    declared_len: &mut Vec<Option<usize>>,
    pending_array: &mut Option<Vec<f64>>,
    target: usize,
) -> Result<(), FbxError> {
    while stack.len() > target {
        finish_array(stack, pending_array)?;
        declared_len.pop();
        close_top(stack, roots);
    }
    Ok(())
}

/// Parses an ASCII FBX document.
///
/// # Errors
///
/// Returns an [`FbxError`] if the text is not ASCII FBX or nests too deeply.
/// Structural oddities that the legacy parser tolerates — a stray closing
/// brace, an unclassifiable line — are tolerated here too, because real
/// exporters produce them and rejecting the file would lose a model over a
/// cosmetic defect.
pub fn parse(text: &str) -> Result<FbxDocument, FbxError> {
    if !is_ascii_fbx(text) {
        return Err(FbxError::BadMagic);
    }

    let version = ascii_version(text).unwrap_or(0);

    // Nodes under construction, outermost first. Roots collect at the bottom.
    let mut roots: Vec<FbxNode> = Vec::new();
    let mut stack: Vec<FbxNode> = Vec::new();
    // Declared `*N` length per open block, parallel to `stack`.
    let mut declared_len: Vec<Option<usize>> = Vec::new();
    // Numbers accumulated for an `a:` array still spanning lines.
    let mut pending_array: Option<Vec<f64>> = None;
    // True when the previous line was `Content: ,` and its payload follows.
    let mut pending_content = false;

    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');

        let classified = classify(line);
        let indent = indent_of(line);

        // Resync to the line's own depth before acting on it.
        //
        // A missing closing brace otherwise leaves the stack too deep, and
        // every later section is nested inside whatever was left open — a
        // document missing one `}` parsed to roots ["FBXVersion", "Objects"]
        // with Connections swallowed as a descendant and no error anywhere.
        // The legacy parser anchored its patterns to the exact indent, so such
        // lines simply failed to match and were skipped with a warning; closing
        // back to the right depth recovers the structure instead of losing it.
        let target = match classified {
            // A `}` at indent i closes the block opened at depth i.
            Line::BlockEnd => indent + 1,
            Line::BlockStart { .. } | Line::Property { .. } => indent,
            Line::Ignored | Line::Continuation(_) => stack.len(),
        };
        close_to(
            &mut stack,
            &mut roots,
            &mut declared_len,
            &mut pending_array,
            target,
        )?;

        if !matches!(classified, Line::Continuation(_) | Line::Ignored) {
            pending_content = false;
        }

        match classified {
            Line::Ignored => {}

            Line::BlockStart { name, attrs } => {
                finish_array(&mut stack, &mut pending_array)?;
                if stack.len() >= MAX_DEPTH {
                    return Err(FbxError::TooDeep(MAX_DEPTH));
                }

                // `Vertices: *9 {` declares the array length. It is metadata
                // about the block, not a property of it: emitting it as a
                // leading Str would make the ASCII tree differ from the binary
                // one at properties[0], which is exactly what this reader
                // exists to avoid. Kept aside to validate the array instead.
                let (attrs, declared) = match attrs.strip_prefix('*') {
                    Some(rest) => ("", rest.trim().parse::<usize>().ok()),
                    None => (attrs, None),
                };

                declared_len.push(declared);
                stack.push(FbxNode {
                    name: name.to_string(),
                    properties: values_of(attrs),
                    children: Vec::new(),
                    // ASCII has no null records, so it cannot express the
                    // empty-scope distinction the binary format carries.
                    empty_scope: false,
                });
            }

            Line::BlockEnd => {
                finish_array(&mut stack, &mut pending_array)?;
                declared_len.pop();
                // A stray closing brace with nothing open is skipped: the
                // legacy parser warns rather than failing, and its regression
                // test asserts the nodes after it survive.
                close_top(&mut stack, &mut roots);
            }

            Line::Property { name, value } => {
                finish_array(&mut stack, &mut pending_array)?;

                // An `a:` line is the array payload of its parent, and may
                // continue across lines when it ends in a comma.
                if name == "a" {
                    let numbers = number_array(value)?;
                    if value.trim_end().ends_with(',') {
                        pending_array = Some(numbers);
                    } else {
                        attach_array(&mut stack, &declared_len, numbers)?;
                    }
                    continue;
                }

                // `Content: ,` puts its base64 payload on the following line.
                // Not porting this dropped embedded textures silently — the
                // node came out empty and the payload line was discarded as an
                // unclassifiable continuation.
                if name == "Content" && value.trim() == "," {
                    pending_content = true;
                }

                let node = FbxNode {
                    name: name.to_string(),
                    properties: values_of(value),
                    children: Vec::new(),
                    empty_scope: false,
                };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    // Document-level properties sit outside any block. Upstream
                    // dereferenced the absent node here and crashed.
                    None => roots.push(node),
                }
            }

            Line::Continuation(rest) => {
                if pending_content {
                    pending_content = false;
                    let payload = rest.trim().trim_end_matches(',').trim_matches('"');
                    if let Some(node) = stack
                        .last_mut()
                        .and_then(|n| n.children.last_mut())
                        .filter(|n| n.name == "Content")
                    {
                        node.properties = vec![FbxProperty::Str(payload.to_string())];
                    }
                    continue;
                }
                if let Some(acc) = pending_array.as_mut() {
                    acc.extend(number_array(rest)?);
                    if !rest.trim_end().ends_with(',') {
                        let done = pending_array.take().unwrap_or_default();
                        attach_array(&mut stack, &declared_len, done)?;
                    }
                }
                // Anything else is a line the classifier could not place. The
                // legacy parser warns and skips; there is nothing to attach it
                // to either way.
            }
        }
    }

    // Close anything left open, so a truncated document still yields the nodes
    // it did contain rather than dropping them entirely.
    finish_array(&mut stack, &mut pending_array)?;
    while !stack.is_empty() {
        close_top(&mut stack, &mut roots);
    }

    Ok(FbxDocument { version, roots })
}

/// Flushes a multi-line array that ended without a trailing comma.
fn finish_array(stack: &mut [FbxNode], pending: &mut Option<Vec<f64>>) -> Result<(), FbxError> {
    match pending.take() {
        // No declared length available on this path; the block-end check is
        // where a short array is caught.
        Some(numbers) => attach_array(stack, &[], numbers),
        None => Ok(()),
    }
}

/// Hoists an `a:` array onto its enclosing node, matching the binary layout.
///
/// Validates the length against the block's `*N` when one was declared: an
/// array shorter than its header is the quiet partial success that would
/// otherwise reach the DOM layer as a mesh with missing vertices.
fn attach_array(
    stack: &mut [FbxNode],
    declared_len: &[Option<usize>],
    numbers: Vec<f64>,
) -> Result<(), FbxError> {
    if let Some(Some(expected)) = declared_len.last() {
        if *expected != numbers.len() {
            return Err(FbxError::Malformed {
                what: "array length",
                detail: format!("header declares {expected}, found {}", numbers.len()),
            });
        }
    }
    if let Some(node) = stack.last_mut() {
        node.properties.push(FbxProperty::F64Array(numbers));
    }
    Ok(())
}
