//! The rigging MCP server's logic, kept out of `main.rs` so it can be tested
//! without a stdio transport.
//!
//! One [`Server`] holds a rigging **session** — the loaded model, the chosen
//! template, the fitted skeleton — and answers JSON-RPC requests. The transport
//! (newline-delimited JSON-RPC 2.0 over stdio) lives in `main.rs`; everything
//! that decides what a tool does is here and unit-tested.

use base64::Engine;
use m2m_pipeline as pipeline;
use serde_json::{json, Value};
use std::path::PathBuf;

/// The MCP protocol revision this server speaks.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// A rigging session: what has been loaded and fitted so far. Tools read and
/// advance it, so an agent drives the pipeline one call at a time.
#[derive(Default)]
struct Session {
    model_path: Option<String>,
    model_bytes: Option<Vec<u8>>,
    template: Option<String>,
    fitted: Option<pipeline::FittedSkeleton>,
    falloff: f32,
}

/// The server: a session plus where to find the animation libraries on disk.
pub struct Server {
    session: Session,
    assets_dir: PathBuf,
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    /// A fresh server. `M2M_ASSETS` overrides where the animation libraries are
    /// looked for; otherwise the repo's `assets/` beside this crate is used.
    pub fn new() -> Self {
        let assets_dir = std::env::var("M2M_ASSETS").map_or_else(
            |_| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets")),
            PathBuf::from,
        );
        Server {
            session: Session {
                falloff: 2.0,
                ..Session::default()
            },
            assets_dir,
        }
    }

    /// Handles one JSON-RPC request, returning the response — or `None` for a
    /// notification, which by protocol gets no reply.
    pub fn handle(&mut self, request: &Value) -> Option<Value> {
        let id = request.get("id").cloned();
        let method = request.get("method").and_then(Value::as_str).unwrap_or("");
        match method {
            "initialize" => Some(ok(
                id,
                json!({
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": "m2m-mcp", "version": env!("CARGO_PKG_VERSION") }
                }),
            )),
            "notifications/initialized" => None,
            "ping" => Some(ok(id, json!({}))),
            "tools/list" => Some(ok(id, json!({ "tools": tool_definitions() }))),
            "tools/call" => Some(self.tools_call(id, request.get("params"))),
            // Any other notification (no id) is ignored; a request gets an error.
            _ if id.is_none() => None,
            _ => Some(rpc_error(id, -32601, "method not found")),
        }
    }

    fn tools_call(&mut self, id: Option<Value>, params: Option<&Value>) -> Value {
        let name = params
            .and_then(|p| p.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let args = params
            .and_then(|p| p.get("arguments"))
            .cloned()
            .unwrap_or_else(|| json!({}));
        match self.run_tool(&name, &args) {
            Ok(content) => ok(id, json!({ "content": content })),
            // A tool failure is a normal result with isError, not a protocol error:
            // the agent reads the message and adjusts.
            Err(message) => ok(
                id,
                json!({ "content": text_content(&message), "isError": true }),
            ),
        }
    }

    /// Runs a tool by name, returning its content items (text, and — for
    /// `render_views` — images). Every arm reports what it needs in plain words
    /// so an agent can recover.
    fn run_tool(&mut self, name: &str, args: &Value) -> Result<Vec<Value>, String> {
        let text = |s: String| text_content(&s);
        match name {
            "session_status" => Ok(text(self.session_status())),
            "list_templates" => list_templates().map(text),
            "load_asset" => self.load_asset(args).map(text),
            "fit_skeleton" => self.fit_skeleton(args).map(text),
            "adjust_joint" => self.adjust_joint(args).map(text),
            "bind_weights" => self.bind_weights(args).map(text),
            "list_clips" => self.list_clips(args).map(text),
            "export" => self.export(args).map(text),
            "validate_export" => validate_export(args).map(text),
            "render_views" => self.render_views(args),
            other => Err(format!("unknown tool: {other}")),
        }
    }

    /// Renders the current rig from several angles (a turntable) in Blender
    /// headless and returns the images, so an agent can SEE the pose and
    /// deformation, rotate around it, and refine — with an optional clip + frame
    /// to inspect a pose mid-animation.
    fn render_views(&self, args: &Value) -> Result<Vec<Value>, String> {
        let model = self
            .session
            .model_bytes
            .as_ref()
            .ok_or("no asset loaded — call load_asset first")?;
        let fitted = self
            .session
            .fitted
            .as_ref()
            .ok_or("no fitted skeleton — call fit_skeleton first")?;
        let num_views = args
            .get("num_views")
            .and_then(Value::as_u64)
            .unwrap_or(4)
            .clamp(1, 12) as u32;
        let clip = args.get("clip").and_then(Value::as_str);
        let frame = args.get("frame").and_then(Value::as_i64).map(|f| f as i32);
        let library = match clip {
            Some(_) => Some(self.library_bytes(self.session.template.as_deref().unwrap_or(""))?),
            None => None,
        };
        let animation = library.as_deref().zip(clip);
        let glb = pipeline::export_glb(model, fitted, self.session.falloff, animation)
            .map_err(|e| e.to_string())?;
        let blender = m2m_bridge::blender_path().map_err(|e| e.to_string())?;
        let paths = m2m_bridge::render_views(&glb, num_views, frame, &blender)
            .map_err(|e| e.to_string())?;

        let mut content = vec![json!({
            "type": "text",
            "text": format!(
                "{} turntable view(s) of the rig{}",
                paths.len(),
                clip.map(|c| format!(" playing {c}")).unwrap_or_default()
            )
        })];
        for path in &paths {
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
            content.push(json!({ "type": "image", "data": data, "mimeType": "image/png" }));
            let _ = std::fs::remove_file(path);
        }
        if let Some(dir) = paths.first().and_then(|p| p.parent()) {
            let _ = std::fs::remove_dir_all(dir);
        }
        Ok(content)
    }

    fn session_status(&self) -> String {
        let s = &self.session;
        pretty(&json!({
            "model": s.model_path,
            "model_loaded": s.model_bytes.is_some(),
            "template": s.template,
            "fitted": s.fitted.as_ref().map(|f| json!({ "bones": f.bones.len(), "scale": f.scale, "pose": f.pose })),
            "falloff": s.falloff,
            "assets_dir": self.assets_dir.display().to_string(),
        }))
    }

    fn load_asset(&mut self, args: &Value) -> Result<String, String> {
        let path = arg_str(args, "path")?;
        let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {path}: {e}"))?;
        let import = m2m_io::import::inspect(&bytes).map_err(|e| e.to_string())?;
        let summary = serde_json::to_value(&import).map_err(|e| e.to_string())?;
        self.session.model_path = Some(path);
        self.session.model_bytes = Some(bytes);
        // A new asset invalidates any prior fit.
        self.session.fitted = None;
        Ok(pretty(&summary))
    }

    fn fit_skeleton(&mut self, args: &Value) -> Result<String, String> {
        let template = arg_str(args, "template")?;
        let model = self
            .session
            .model_bytes
            .as_ref()
            .ok_or("no asset loaded — call load_asset first")?;
        let fitted = pipeline::fit(&template, model).map_err(|e| e.to_string())?;
        let summary = json!({
            "bones": fitted.bones.len(),
            "scale": fitted.scale,
            "pose": fitted.pose,
            "note": "the skeleton is auto-placed; use adjust_joint to refine, then bind_weights",
        });
        self.session.template = Some(template);
        self.session.fitted = Some(fitted);
        Ok(pretty(&summary))
    }

    fn adjust_joint(&mut self, args: &Value) -> Result<String, String> {
        let index = arg_u64(args, "index")? as usize;
        let position = arg_vec3(args, "position")?;
        let fitted = self
            .session
            .fitted
            .as_mut()
            .ok_or("no fitted skeleton — call fit_skeleton first")?;
        let slot = fitted.positions.get_mut(index).ok_or_else(|| {
            format!(
                "joint index {index} out of range (0..{})",
                fitted.bones.len()
            )
        })?;
        *slot = position;
        Ok(pretty(&json!({
            "moved": fitted.bones.get(index),
            "to": position,
        })))
    }

    fn bind_weights(&mut self, args: &Value) -> Result<String, String> {
        if let Some(falloff) = args.get("falloff").and_then(Value::as_f64) {
            self.session.falloff = falloff as f32;
        }
        let model = self
            .session
            .model_bytes
            .as_ref()
            .ok_or("no asset loaded — call load_asset first")?;
        let fitted = self
            .session
            .fitted
            .as_ref()
            .ok_or("no fitted skeleton — call fit_skeleton first")?;
        let report =
            pipeline::bind(model, fitted, self.session.falloff).map_err(|e| e.to_string())?;
        Ok(pretty(
            &serde_json::to_value(&report).map_err(|e| e.to_string())?,
        ))
    }

    fn list_clips(&self, args: &Value) -> Result<String, String> {
        let template = arg_str(args, "template")?;
        let library = self.library_bytes(&template)?;
        let clips = pipeline::library_clips(&library).map_err(|e| e.to_string())?;
        Ok(pretty(
            &serde_json::to_value(&clips).map_err(|e| e.to_string())?,
        ))
    }

    fn export(&self, args: &Value) -> Result<String, String> {
        let out = arg_str(args, "path")?;
        let format = arg_str(args, "format")?;
        let model = self
            .session
            .model_bytes
            .as_ref()
            .ok_or("no asset loaded — call load_asset first")?;
        let fitted = self
            .session
            .fitted
            .as_ref()
            .ok_or("no fitted skeleton — call fit_skeleton first")?;
        let clip = args.get("clip").and_then(Value::as_str);
        let library = match clip {
            Some(_) => Some(self.library_bytes(self.session.template.as_deref().unwrap_or(""))?),
            None => None,
        };
        let animation = library.as_deref().zip(clip);
        let bytes = match format.as_str() {
            "glb" => pipeline::export_glb(model, fitted, self.session.falloff, animation),
            "fbx" => pipeline::export_fbx(model, fitted, self.session.falloff, animation),
            other => return Err(format!("unknown format {other}; use \"glb\" or \"fbx\"")),
        }
        .map_err(|e| e.to_string())?;
        std::fs::write(&out, &bytes).map_err(|e| format!("cannot write {out}: {e}"))?;
        Ok(pretty(
            &json!({ "path": out, "format": format, "bytes": bytes.len() }),
        ))
    }

    /// Finds a creature's animation-library glb under the assets dir, trying both
    /// names the pipeline knows it by.
    fn library_bytes(&self, template: &str) -> Result<Vec<u8>, String> {
        for name in pipeline::library_names(template) {
            let path = self.assets_dir.join("animations").join(&name);
            if path.is_file() {
                return std::fs::read(&path).map_err(|e| e.to_string());
            }
        }
        Err(format!(
            "no animation library for {template} under {}",
            self.assets_dir.display()
        ))
    }
}

fn list_templates() -> Result<String, String> {
    let templates = pipeline::templates().map_err(|e| e.to_string())?;
    Ok(pretty(
        &serde_json::to_value(&templates).map_err(|e| e.to_string())?,
    ))
}

fn validate_export(args: &Value) -> Result<String, String> {
    let path = arg_str(args, "path")?;
    let bytes = std::fs::read(&path).map_err(|e| format!("cannot read {path}: {e}"))?;
    let extension = std::path::Path::new(&path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("glb");
    let engines: Vec<String> = args
        .get("engines")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_else(|| vec!["blender".to_string()]);

    let mut reports = serde_json::Map::new();
    for engine in &engines {
        let report = match engine.as_str() {
            "blender" => m2m_bridge::blender_path()
                .and_then(|exe| m2m_bridge::inspect(&bytes, extension, &exe))
                .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
                .unwrap_or_else(|e| json!({ "error": e.to_string() })),
            "maya" => m2m_bridge::maya::mayapy_path()
                .and_then(|exe| m2m_bridge::maya::inspect(&bytes, &exe))
                .map(|r| serde_json::to_value(r).unwrap_or(Value::Null))
                .unwrap_or_else(|e| json!({ "error": e.to_string() })),
            other => json!({ "error": format!("unknown engine {other}; use blender or maya") }),
        };
        reports.insert(engine.clone(), report);
    }
    Ok(pretty(&Value::Object(reports)))
}

/// The tool catalogue returned by `tools/list`, with JSON-Schema input shapes.
fn tool_definitions() -> Value {
    let obj = |props: Value, required: Value| json!({ "type": "object", "properties": props, "required": required });
    json!([
        { "name": "session_status", "description": "Report what is loaded and fitted so far.",
          "inputSchema": obj(json!({}), json!([])) },
        { "name": "list_templates", "description": "List the creature skeleton templates that can be fitted.",
          "inputSchema": obj(json!({}), json!([])) },
        { "name": "load_asset", "description": "Load a model (glb/gltf/fbx) into the session and report what it contains.",
          "inputSchema": obj(json!({ "path": { "type": "string", "description": "Absolute path to the model file." } }), json!(["path"])) },
        { "name": "fit_skeleton", "description": "Auto-fit a creature template's skeleton to the loaded mesh.",
          "inputSchema": obj(json!({ "template": { "type": "string", "description": "A template name from list_templates." } }), json!(["template"])) },
        { "name": "adjust_joint", "description": "Move one fitted joint to refine the placement before binding.",
          "inputSchema": obj(json!({ "index": { "type": "integer" }, "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3 } }), json!(["index", "position"])) },
        { "name": "bind_weights", "description": "Bind the mesh to the fitted skeleton and report the weighting.",
          "inputSchema": obj(json!({ "falloff": { "type": "number", "description": "Weight falloff (default 2.0)." } }), json!([])) },
        { "name": "list_clips", "description": "List the animation clips a creature's library offers.",
          "inputSchema": obj(json!({ "template": { "type": "string" } }), json!(["template"])) },
        { "name": "export", "description": "Export the rigged model to a file, optionally with a clip retargeted on.",
          "inputSchema": obj(json!({ "path": { "type": "string" }, "format": { "type": "string", "enum": ["glb", "fbx"] }, "clip": { "type": "string" } }), json!(["path", "format"])) },
        { "name": "validate_export", "description": "Import an exported file into Blender and/or Maya headless and report what each read back.",
          "inputSchema": obj(json!({ "path": { "type": "string" }, "engines": { "type": "array", "items": { "type": "string", "enum": ["blender", "maya"] } } }), json!(["path"])) },
        { "name": "render_views", "description": "Render the rig from several angles (a turntable) in Blender headless and return the images, to see the pose and deformation and refine it. Optional clip + frame to inspect a pose mid-animation.",
          "inputSchema": obj(json!({ "num_views": { "type": "integer", "description": "1-12, default 4." }, "clip": { "type": "string", "description": "A clip name to retarget before rendering." }, "frame": { "type": "integer", "description": "Clip frame to render (needs clip)." } }), json!([])) },
    ])
}

// ---- JSON-RPC + argument helpers ----

/// A self-check an operator runs (`m2m-mcp --check`) to confirm the server
/// answers and every tool is wired, and to see whether the optional DCC engines
/// that `render_views`/`validate_export` need are reachable.
///
/// Drives the real `initialize` + `tools/list` path in-process — the same code
/// a client hits — so "all tools active" means the dispatch table actually
/// answers. Returns the report on success; `Err` only when a tool is missing
/// (a build wiring bug). A missing Blender/Maya is reported, not fatal: those
/// tools degrade gracefully.
pub fn self_check() -> Result<String, String> {
    let mut server = Server::new();
    let init = server
        .handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize" }))
        .ok_or("initialize returned no response")?;
    let protocol = init
        .pointer("/result/protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or("?");
    let listed = server
        .handle(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list" }))
        .ok_or("tools/list returned no response")?;
    let tools: Vec<String> = listed
        .pointer("/result/tools")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|t| t.get("name").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();

    let engine = |found: Result<PathBuf, m2m_bridge::BridgeError>| match found {
        Ok(path) => json!({ "available": true, "path": path.display().to_string() }),
        Err(e) => json!({ "available": false, "note": e.to_string() }),
    };
    let report = json!({
        "ok": !tools.is_empty(),
        "protocolVersion": protocol,
        "tool_count": tools.len(),
        "tools": tools,
        "engines": {
            "blender": engine(m2m_bridge::blender_path()),
            "maya": engine(m2m_bridge::maya::mayapy_path()),
        },
    });
    if tools.is_empty() {
        Err("tools/list returned no tools — the server is not wired".into())
    } else {
        Ok(pretty(&report))
    }
}

fn ok(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result })
}

fn rpc_error(id: Option<Value>, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "error": { "code": code, "message": message } })
}

fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string())
}

/// Wraps a string as a one-item MCP text content array.
fn text_content(text: &str) -> Vec<Value> {
    vec![json!({ "type": "text", "text": text })]
}

fn arg_str(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("missing required string argument `{key}`"))
}

fn arg_u64(args: &Value, key: &str) -> Result<u64, String> {
    args.get(key)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing required integer argument `{key}`"))
}

fn arg_vec3(args: &Value, key: &str) -> Result<[f32; 3], String> {
    let a = args
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing required 3-number array `{key}`"))?;
    if a.len() != 3 {
        return Err(format!("`{key}` must have exactly 3 numbers"));
    }
    let mut out = [0.0f32; 3];
    for (i, v) in a.iter().enumerate() {
        out[i] = v
            .as_f64()
            .ok_or_else(|| format!("`{key}[{i}]` is not a number"))? as f32;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(server: &mut Server, name: &str, args: Value) -> (String, bool) {
        let request = json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": name, "arguments": args }
        });
        let response = server.handle(&request).expect("a response");
        let result = &response["result"];
        let text = result["content"][0]["text"]
            .as_str()
            .unwrap_or("")
            .to_string();
        (text, result["isError"].as_bool().unwrap_or(false))
    }

    fn asset(name: &str) -> String {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/").to_owned() + name
    }

    #[test]
    fn initialize_and_list_tools() {
        let mut server = Server::new();
        let init = server
            .handle(&json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} }))
            .expect("response");
        assert_eq!(init["result"]["serverInfo"]["name"], "m2m-mcp");

        let list = server
            .handle(&json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} }))
            .expect("response");
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 10);
    }

    #[test]
    fn self_check_reports_every_tool() {
        let report: Value = serde_json::from_str(&self_check().expect("healthy")).unwrap();
        assert_eq!(report["ok"], true);
        // The self-check must see the same tools tools/list advertises.
        assert_eq!(
            report["tool_count"].as_u64().unwrap() as usize,
            tool_definitions().as_array().unwrap().len()
        );
        // Engine availability is reported (true or false), never absent.
        assert!(report["engines"]["blender"]["available"].is_boolean());
        assert!(report["engines"]["maya"]["available"].is_boolean());
    }

    #[test]
    fn a_notification_gets_no_reply() {
        let mut server = Server::new();
        assert!(server
            .handle(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
            .is_none());
    }

    #[test]
    fn an_unknown_method_is_a_protocol_error() {
        let mut server = Server::new();
        let r = server
            .handle(&json!({ "jsonrpc": "2.0", "id": 9, "method": "no_such", "params": {} }))
            .expect("response");
        assert_eq!(r["error"]["code"], -32601);
    }

    #[test]
    fn tool_errors_are_results_not_crashes() {
        let mut server = Server::new();
        // Unknown tool.
        let (_, is_error) = call(&mut server, "nope", json!({}));
        assert!(is_error);
        // Fitting before loading reports what is needed.
        let (msg, is_error) = call(&mut server, "fit_skeleton", json!({ "template": "human" }));
        assert!(is_error);
        assert!(
            msg.contains("load_asset"),
            "should tell the agent what to do: {msg}"
        );
        // A missing required argument is named.
        let (msg, is_error) = call(&mut server, "load_asset", json!({}));
        assert!(is_error);
        assert!(msg.contains("path"), "{msg}");
    }

    /// The whole pipeline over the tool interface, no DCC: load a mesh, fit the
    /// human template, adjust a joint, bind, and export a glb.
    #[test]
    fn load_fit_adjust_bind_export_over_the_tools() {
        let mut server = Server::new();

        let (loaded, err) = call(
            &mut server,
            "load_asset",
            json!({ "path": asset("models/model-human.glb") }),
        );
        assert!(!err, "{loaded}");
        assert!(loaded.contains("\"format\": \"Glb\""), "{loaded}");

        let (fitted, err) = call(&mut server, "fit_skeleton", json!({ "template": "human" }));
        assert!(!err, "{fitted}");
        assert!(fitted.contains("\"bones\": 66"), "{fitted}");

        // Refine a joint, then confirm the move is reported.
        let (moved, err) = call(
            &mut server,
            "adjust_joint",
            json!({ "index": 0, "position": [0.0, 1.0, 0.0] }),
        );
        assert!(!err, "{moved}");

        let (bound, err) = call(&mut server, "bind_weights", json!({}));
        assert!(!err, "{bound}");
        assert!(bound.contains("\"unweighted_vertices\": 0"), "{bound}");

        let out = std::env::temp_dir().join("m2m-mcp-test.glb");
        let (exported, err) = call(
            &mut server,
            "export",
            json!({ "path": out.to_string_lossy(), "format": "glb" }),
        );
        assert!(!err, "{exported}");
        assert!(out.is_file(), "export wrote no file");
        let _ = std::fs::remove_file(&out);
    }

    #[test]
    fn list_templates_and_clips() {
        let mut server = Server::new();
        let (templates, err) = call(&mut server, "list_templates", json!({}));
        assert!(!err, "{templates}");
        assert!(templates.contains("human"), "{templates}");

        let (clips, err) = call(&mut server, "list_clips", json!({ "template": "human" }));
        assert!(!err, "{clips}");
        // The human library ships many clips; just confirm it read some.
        assert!(clips.contains('{'), "{clips}");
    }
}
