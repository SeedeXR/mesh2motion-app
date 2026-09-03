//! Live mode: push a rig into an already-running Blender.
//!
//! The counterpart to [`crate::inspect`], which spawns Blender per call. When an
//! artist has Blender open with the companion add-on
//! (`blender-addon/mesh2motion_bridge.py`) running its server, this sends a
//! `.glb` over a localhost socket, Blender imports it, and the same
//! [`BlenderReport`] comes back — no process spawn, no temp files on this side.
//!
//! # Protocol
//!
//! A request is a JSON header line, then exactly `len` raw bytes of the `.glb`:
//!
//! ```text
//! {"cmd":"import","name":"rig.glb","len":12345}\n<12345 bytes>
//! ```
//!
//! The reply is one JSON line in the [`BlenderReport`] shape. Length-prefixed
//! framing rather than base64 keeps the payload as bytes, not a 33%-larger
//! string, and needs no extra dependency.

use crate::{BlenderReport, BridgeError};
use std::io::{BufRead, BufReader, Write};
use std::net::TcpStream;
use std::time::Duration;

/// The port the add-on listens on by default (matches `DEFAULT_PORT` in the
/// add-on).
pub const DEFAULT_PORT: u16 = 47829;

/// Frames a request: the JSON header line, then the raw `.glb` bytes.
fn encode_request(name: &str, glb: &[u8]) -> Vec<u8> {
    let header = serde_json::json!({ "cmd": "import", "name": name, "len": glb.len() });
    let mut out = serde_json::to_vec(&header).expect("a header serialises");
    out.push(b'\n');
    out.extend_from_slice(glb);
    out
}

/// Parses the add-on's one-line JSON reply into a report.
fn decode_response(line: &str) -> Result<BlenderReport, BridgeError> {
    let line = line.trim();
    if line.is_empty() {
        return Err(BridgeError::BadReport("empty reply from Blender".into()));
    }
    serde_json::from_str(line).map_err(|e| BridgeError::BadReport(e.to_string()))
}

/// Sends a `.glb` to a running Blender's bridge server and returns its report.
///
/// `addr` is a socket address like `"127.0.0.1:47829"`. `name` is the file name
/// Blender reports back; the bytes are the model.
///
/// # Errors
///
/// [`BridgeError::Spawn`] if the connection or transfer fails (no server
/// listening, the session closed), [`BridgeError::BadReport`] if the reply is
/// not the expected JSON.
pub fn inspect_live(addr: &str, name: &str, glb: &[u8]) -> Result<BlenderReport, BridgeError> {
    let stream = TcpStream::connect(addr).map_err(|e| BridgeError::Spawn(e.to_string()))?;
    // A large import can take a while; do not wait forever on a wedged session.
    stream
        .set_read_timeout(Some(Duration::from_secs(120)))
        .map_err(|e| BridgeError::Spawn(e.to_string()))?;

    let request = encode_request(name, glb);
    {
        let mut writer = &stream;
        writer
            .write_all(&request)
            .map_err(|e| BridgeError::Spawn(e.to_string()))?;
        writer.flush().ok();
    }

    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    reader
        .read_line(&mut line)
        .map_err(|e| BridgeError::Spawn(e.to_string()))?;
    decode_response(&line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_is_a_header_line_then_the_raw_bytes() {
        let glb = b"GLBDATA\x00\x01\x02";
        let request = encode_request("rig.glb", glb);
        let newline = request
            .iter()
            .position(|&b| b == b'\n')
            .expect("a header line");
        let header: serde_json::Value =
            serde_json::from_slice(&request[..newline]).expect("valid JSON header");
        assert_eq!(header["cmd"], "import");
        assert_eq!(header["name"], "rig.glb");
        assert_eq!(header["len"], glb.len());
        // The exact payload follows the newline, byte for byte.
        assert_eq!(&request[newline + 1..], glb);
    }

    #[test]
    fn a_reply_parses_into_a_report() {
        let line = r#"{"ok":true,"file":"rig.glb","imported":true,"bones":66,"meshes":1,"mesh_vertices":[7399],"weight_total":7399.0}"#;
        let report = decode_response(line).expect("parses");
        assert!(report.imported);
        assert_eq!(report.bones, Some(66));
        assert_eq!(report.mesh_vertices, vec![7399]);
        assert_eq!(report.weight_total, Some(7399.0));
    }

    #[test]
    fn an_import_failure_reply_is_a_report_not_an_error() {
        let line =
            r#"{"ok":true,"file":"bad.glb","imported":false,"error":"RuntimeError: unreadable"}"#;
        let report = decode_response(line).expect("parses");
        assert!(!report.imported);
        assert_eq!(report.error.as_deref(), Some("RuntimeError: unreadable"));
    }

    #[test]
    fn an_empty_reply_is_an_error() {
        assert!(matches!(
            decode_response("\n"),
            Err(BridgeError::BadReport(_))
        ));
    }
}
