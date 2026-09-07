//! The rigging MCP server over stdio.
//!
//! MCP's stdio transport is newline-delimited JSON-RPC 2.0: one request per
//! line in, one response per line out. All the decisions live in
//! [`m2m_mcp::Server`]; this is just the loop that reads, dispatches and writes.

use m2m_mcp::Server;
use std::io::{BufRead, Write};

fn main() {
    // `--check`: confirm the server answers and every tool is wired (and report
    // Blender/Maya availability), then exit — no stdio loop. For operators and
    // install scripts to verify a healthy server.
    if std::env::args().any(|a| a == "--check") {
        match m2m_mcp::self_check() {
            Ok(report) => {
                println!("{report}");
            }
            Err(e) => {
                eprintln!("m2m-mcp self-check FAILED: {e}");
                std::process::exit(1);
            }
        }
        return;
    }

    let mut server = Server::new();
    m2m_mcp::log::info("server started (stdio JSON-RPC); set M2M_MCP_LOG=debug for per-call detail, =off to silence");
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let request = match serde_json::from_str::<serde_json::Value>(&line) {
            Ok(value) => value,
            Err(e) => {
                // A parse error is answered with a null-id JSON-RPC error, as the
                // spec requires, rather than crashing the transport.
                m2m_mcp::log::error(&format!("parse error, replying -32700: {e}"));
                let error = serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": serde_json::Value::Null,
                    "error": { "code": -32700, "message": format!("parse error: {e}") }
                });
                let _ = writeln!(stdout, "{error}");
                let _ = stdout.flush();
                continue;
            }
        };
        if let Some(response) = server.handle(&request) {
            let _ = writeln!(stdout, "{response}");
            let _ = stdout.flush();
        }
    }
}
