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
use std::path::{Path, PathBuf};

/// The MCP protocol revision this server speaks.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// Structured logging to **stderr** — stdout is the JSON-RPC transport, so a log
/// line there would corrupt the protocol. Gated by `M2M_MCP_LOG`: `off` silences
/// everything; `debug` adds per-call detail; anything else (or unset) logs
/// info + errors. MCP hosts capture a server's stderr, so this is where an
/// operator sees what the server did and why a call failed.
pub mod log {
    /// Is a message at this verbosity enabled? `debug` messages need `M2M_MCP_LOG=debug`.
    fn enabled(is_debug: bool) -> bool {
        match std::env::var("M2M_MCP_LOG").ok().as_deref() {
            Some("off") => false,
            Some("debug") => true,
            _ => !is_debug,
        }
    }

    /// The formatted line, without the trailing newline — pure, so it is tested.
    #[must_use]
    pub fn line(level: &str, msg: &str) -> String {
        let millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_millis());
        format!("[m2m-mcp {millis} {level}] {msg}")
    }

    pub fn info(msg: &str) {
        if enabled(false) {
            eprintln!("{}", line("info", msg));
        }
    }

    pub fn error(msg: &str) {
        if enabled(false) {
            eprintln!("{}", line("error", msg));
        }
    }

    pub fn debug(msg: &str) {
        if enabled(true) {
            eprintln!("{}", line("debug", msg));
        }
    }
}

/// A rigging session: what has been loaded and fitted so far. Tools read and
/// advance it, so an agent drives the pipeline one call at a time.
#[derive(Default)]
struct Session {
    model_path: Option<String>,
    model_bytes: Option<Vec<u8>>,
    template: Option<String>,
    fitted: Option<pipeline::FittedSkeleton>,
    falloff: f32,
    /// Snapshots of joint positions before each adjust, for `undo` (newest last).
    history: Vec<Vec<[f32; 3]>>,
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
        log::info(&format!("tool {name}"));
        log::debug(&format!("tool {name} args {args}"));
        let started = std::time::Instant::now();
        let outcome = self.run_tool(&name, &args);
        let ms = started.elapsed().as_millis();
        match outcome {
            Ok(content) => {
                log::info(&format!("tool {name} ok ({ms} ms)"));
                ok(id, json!({ "content": content }))
            }
            // A tool failure is a normal result with isError, not a protocol error:
            // the agent reads the message and adjusts.
            Err(message) => {
                log::error(&format!("tool {name} failed ({ms} ms): {message}"));
                ok(
                    id,
                    json!({ "content": text_content(&message), "isError": true }),
                )
            }
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
            "list_joints" => self.list_joints().map(text),
            "adjust_joint" => self.adjust_joint(args).map(text),
            "undo" => self.undo().map(text),
            "bind_weights" => self.bind_weights(args).map(text),
            "diagnose" => self.diagnose(args).map(text),
            "list_clips" => self.list_clips(args).map(text),
            "export" => self.export(args).map(text),
            "validate_export" => validate_export(args).map(text),
            "render_views" => self.render_views(args),
            "render_animation" => self.render_animation(args),
            "compare_to_reference" => self.compare_to_reference(args),
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
        let overlay = args
            .get("overlay")
            .and_then(Value::as_str)
            .unwrap_or("solid");
        if !["solid", "skeleton", "weights"].contains(&overlay) {
            return Err(format!(
                "unknown overlay {overlay:?}; use \"solid\", \"skeleton\" or \"weights\""
            ));
        }
        let library = match clip {
            Some(_) => Some(self.library_bytes(self.session.template.as_deref().unwrap_or(""))?),
            None => None,
        };
        let animation = library
            .as_deref()
            .zip(clip)
            .map(|(lib, c)| pipeline::Animation {
                library: lib,
                clip: c,
                mirror: false,
                arm_space: 50.0,
                options: pipeline::ClipOptions::full(),
                skin: true,
            });
        let glb = pipeline::export_glb(model, fitted, self.session.falloff, animation)
            .map_err(|e| e.to_string())?;
        let blender = m2m_bridge::blender_path().map_err(|e| e.to_string())?;
        let paths = m2m_bridge::render_views(&glb, num_views, frame, overlay, &blender)
            .map_err(|e| e.to_string())?;

        let mut content = vec![json!({
            "type": "text",
            "text": format!(
                "{} {} view(s) of the rig{}",
                paths.len(),
                match overlay {
                    "skeleton" => "skeleton-overlay",
                    "weights" => "weight-heatmap (magenta=unweighted, red=1 bone .. green=4)",
                    _ => "turntable",
                },
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

    /// Renders the whole clip from a fixed camera, encodes it to an mp4 with
    /// ffmpeg (a file path the agent can open), and returns a few sample frames
    /// inline so the motion is visible without opening the video. Degrades to
    /// frames-only if ffmpeg is missing.
    fn render_animation(&self, args: &Value) -> Result<Vec<Value>, String> {
        let clip = arg_str(args, "clip")?;
        let max_frames = args
            .get("max_frames")
            .and_then(Value::as_u64)
            .unwrap_or(120)
            .clamp(8, 240) as u32;
        let glb = self.animated_glb(&clip)?;
        let blender = m2m_bridge::blender_path().map_err(|e| e.to_string())?;
        let (frames, fps) =
            m2m_bridge::render_animation(&glb, max_frames, &blender).map_err(|e| e.to_string())?;
        let frames_dir = frames
            .first()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);

        let out = args
            .get("path")
            .and_then(Value::as_str)
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join(format!("m2m-{}.mp4", sanitize(&clip))));

        let mut content = Vec::new();
        match (m2m_bridge::ffmpeg_path(), frames_dir.as_deref()) {
            (Ok(ffmpeg), Some(dir)) => {
                m2m_bridge::encode_video(dir, fps, &out, &ffmpeg).map_err(|e| e.to_string())?;
                content.push(json!({ "type": "text", "text": format!(
                    "rendered {} frames of '{clip}' at {fps} fps, encoded to {}",
                    frames.len(), out.display()
                )}));
            }
            _ => content.push(json!({ "type": "text", "text": format!(
                "rendered {} frames of '{clip}' at {fps} fps; ffmpeg not found so no mp4 (set M2M_FFMPEG). Sample frames below.",
                frames.len()
            )})),
        }
        for path in sample_evenly(&frames, 5) {
            content.push(image_content(&path)?);
        }
        if let Some(dir) = frames_dir {
            let _ = std::fs::remove_dir_all(dir);
        }
        Ok(content)
    }

    /// Renders the clip and samples the same number of frames from a reference
    /// video, returning both sets inline for a side-by-side visual comparison of
    /// pose and timing. A qualitative aid — not a numeric similarity score, which
    /// would mislead across differently framed footage.
    fn compare_to_reference(&self, args: &Value) -> Result<Vec<Value>, String> {
        let reference = arg_str(args, "reference")?;
        if !Path::new(&reference).is_file() {
            return Err(format!("reference video not found: {reference}"));
        }
        let clip = arg_str(args, "clip")?;
        let n = args
            .get("frames")
            .and_then(Value::as_u64)
            .unwrap_or(5)
            .clamp(2, 12) as u32;
        let ffmpeg = m2m_bridge::ffmpeg_path()
            .map_err(|e| format!("{e} — needed to sample the reference video"))?;
        let glb = self.animated_glb(&clip)?;
        let blender = m2m_bridge::blender_path().map_err(|e| e.to_string())?;
        let (rendered, _fps) =
            m2m_bridge::render_animation(&glb, 120, &blender).map_err(|e| e.to_string())?;
        let rendered_dir = rendered
            .first()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);
        let reference_frames = m2m_bridge::sample_video_frames(Path::new(&reference), n, &ffmpeg)
            .map_err(|e| e.to_string())?;
        let reference_dir = reference_frames
            .first()
            .and_then(|p| p.parent())
            .map(Path::to_path_buf);

        let mut content = vec![json!({ "type": "text", "text": format!(
            "Comparing clip '{clip}' (rendered) against reference '{reference}'. {n} evenly-spaced frames of each follow — judge pose and timing qualitatively (not a numeric metric).",
        )})];
        content.push(json!({ "type": "text", "text": "— rendered clip:" }));
        for path in sample_evenly(&rendered, n as usize) {
            content.push(image_content(&path)?);
        }
        content.push(json!({ "type": "text", "text": "— reference video:" }));
        for path in &reference_frames {
            content.push(image_content(path)?);
        }
        for dir in [rendered_dir, reference_dir].into_iter().flatten() {
            let _ = std::fs::remove_dir_all(dir);
        }
        Ok(content)
    }

    /// Exports the loaded model with `clip` retargeted on, as a glb ready to
    /// render — the shared front of render_animation and compare_to_reference.
    fn animated_glb(&self, clip: &str) -> Result<Vec<u8>, String> {
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
        let library = self.library_bytes(self.session.template.as_deref().unwrap_or(""))?;
        let animation = pipeline::Animation {
            library: &library,
            clip,
            mirror: false,
            arm_space: 50.0,
            options: pipeline::ClipOptions::full(),
            skin: true,
        };
        pipeline::export_glb(model, fitted, self.session.falloff, Some(animation))
            .map_err(|e| e.to_string())
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
        // A fresh fit invalidates the previous skeleton's edit history.
        self.session.history.clear();
        Ok(pretty(&summary))
    }

    /// List every fitted joint — index, name, world position and parent name —
    /// so an agent can pick one to adjust by name instead of guessing an index.
    fn list_joints(&self) -> Result<String, String> {
        let fitted = self
            .session
            .fitted
            .as_ref()
            .ok_or("no fitted skeleton — call fit_skeleton first")?;
        let joints: Vec<Value> = (0..fitted.bones.len())
            .map(|i| {
                json!({
                    "index": i,
                    "name": fitted.bones[i],
                    "position": fitted.positions[i],
                    "parent": fitted.parents[i].map(|p| fitted.bones[p].clone()),
                })
            })
            .collect();
        Ok(pretty(&json!({ "count": joints.len(), "joints": joints })))
    }

    /// Move one fitted joint before binding. Address it by `index` or `name`; set
    /// an absolute `position` or a relative `nudge`; `mirror` also moves the
    /// left/right counterpart (X negated). Every move is snapshotted for `undo`.
    fn adjust_joint(&mut self, args: &Value) -> Result<String, String> {
        let mirror = args.get("mirror").and_then(Value::as_bool).unwrap_or(false);
        // Resolve everything that only needs a shared borrow first.
        let (index, is_nudge, value, midline) = {
            let fitted = self
                .session
                .fitted
                .as_ref()
                .ok_or("no fitted skeleton — call fit_skeleton first")?;
            let index = resolve_joint(fitted, args)?;
            let (is_nudge, value) = match args.get("nudge") {
                Some(nudge) => (true, vec3_of(nudge, "nudge")?),
                None => (false, arg_vec3(args, "position")?),
            };
            // Midline for an absolute-position mirror: the mean joint X (a
            // symmetric rig sits centred there).
            let midline = fitted.positions.iter().map(|p| p[0]).sum::<f32>()
                / fitted.positions.len().max(1) as f32;
            (index, is_nudge, value, midline)
        };

        // Snapshot for undo, then apply.
        self.session
            .history
            .push(self.session.fitted.as_ref().unwrap().positions.clone());
        let fitted = self.session.fitted.as_mut().unwrap();
        apply_move(&mut fitted.positions[index], is_nudge, value);
        let mut moved =
            vec![json!({ "joint": fitted.bones[index], "to": fitted.positions[index] })];

        if mirror {
            let counterpart = mirror_name(&fitted.bones[index])
                .and_then(|name| fitted.bones.iter().position(|b| *b == name));
            if let Some(other) = counterpart {
                let mirrored = if is_nudge {
                    [-value[0], value[1], value[2]]
                } else {
                    [2.0 * midline - value[0], value[1], value[2]]
                };
                apply_move(&mut fitted.positions[other], is_nudge, mirrored);
                moved.push(json!({ "joint": fitted.bones[other], "to": fitted.positions[other] }));
            }
        }
        Ok(pretty(&json!({ "moved": moved })))
    }

    /// Undo the last adjust_joint, restoring the joint positions before it.
    fn undo(&mut self) -> Result<String, String> {
        let previous = self
            .session
            .history
            .pop()
            .ok_or("nothing to undo — no joint adjustments have been made")?;
        let fitted = self
            .session
            .fitted
            .as_mut()
            .ok_or("no fitted skeleton — call fit_skeleton first")?;
        fitted.positions = previous;
        Ok(pretty(&json!({
            "undone": true,
            "adjustments_left_to_undo": self.session.history.len(),
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

    /// Grade the current fit + bind: disconnected islands, the influence
    /// histogram, unweighted vertices, and which joints poke outside the mesh,
    /// rolled into a pass/warn/fail verdict with plain findings to act on.
    fn diagnose(&self, args: &Value) -> Result<String, String> {
        let falloff = args
            .get("falloff")
            .and_then(Value::as_f64)
            .map_or(self.session.falloff, |f| f as f32);
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
        let report = pipeline::diagnose(model, fitted, falloff).map_err(|e| e.to_string())?;
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
        let animation = library
            .as_deref()
            .zip(clip)
            .map(|(lib, c)| pipeline::Animation {
                library: lib,
                clip: c,
                mirror: false,
                arm_space: 50.0,
                options: pipeline::ClipOptions::full(),
                skin: true,
            });
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
        { "name": "list_joints", "description": "List every fitted joint — index, name, world position, parent — so you can adjust one by name. Needs fit_skeleton first.",
          "inputSchema": obj(json!({}), json!([])) },
        { "name": "adjust_joint", "description": "Move one fitted joint to refine placement before binding. Address it by `index` OR `name`; give an absolute `position` OR a relative `nudge`; set `mirror` to also move the left/right counterpart (X mirrored). Undoable with `undo`.",
          "inputSchema": obj(json!({ "index": { "type": "integer" }, "name": { "type": "string", "description": "Joint name (from list_joints); an alternative to index." }, "position": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "New absolute world position." }, "nudge": { "type": "array", "items": { "type": "number" }, "minItems": 3, "maxItems": 3, "description": "Relative delta added to the current position." }, "mirror": { "type": "boolean", "description": "Also move the L/R counterpart (default false)." } }), json!([])) },
        { "name": "undo", "description": "Undo the last adjust_joint, restoring the joint positions before it.",
          "inputSchema": obj(json!({}), json!([])) },
        { "name": "bind_weights", "description": "Bind the mesh to the fitted skeleton and report the weighting.",
          "inputSchema": obj(json!({ "falloff": { "type": "number", "description": "Weight falloff (default 2.0)." } }), json!([])) },
        { "name": "diagnose", "description": "Grade the current fit + bind (pass/warn/fail) with plain findings: unweighted vertices, disconnected islands, the influence histogram, and which joints sit outside the mesh. Needs load_asset + fit_skeleton first.",
          "inputSchema": obj(json!({ "falloff": { "type": "number", "description": "Weight falloff to grade at (default: the session's)." } }), json!([])) },
        { "name": "list_clips", "description": "List the animation clips a creature's library offers.",
          "inputSchema": obj(json!({ "template": { "type": "string" } }), json!(["template"])) },
        { "name": "export", "description": "Export the rigged model to a file, optionally with a clip retargeted on.",
          "inputSchema": obj(json!({ "path": { "type": "string" }, "format": { "type": "string", "enum": ["glb", "fbx"] }, "clip": { "type": "string" } }), json!(["path", "format"])) },
        { "name": "validate_export", "description": "Import an exported file into Blender and/or Maya headless and report what each read back.",
          "inputSchema": obj(json!({ "path": { "type": "string" }, "engines": { "type": "array", "items": { "type": "string", "enum": ["blender", "maya"] } } }), json!(["path"])) },
        { "name": "render_views", "description": "Render the rig from several angles (a turntable) in Blender headless and return the images, to see the pose and deformation and refine it. `overlay` picks what to see: \"solid\" (the shaded mesh), \"skeleton\" (the fitted bones through an X-rayed mesh), or \"weights\" (the mesh tinted by influence count — magenta=unweighted, red=1 bone .. green=4 — so a bad bind is visible). Optional clip + frame to inspect a pose mid-animation.",
          "inputSchema": obj(json!({ "num_views": { "type": "integer", "description": "1-12, default 4." }, "overlay": { "type": "string", "enum": ["solid", "skeleton", "weights"], "description": "What to render (default solid)." }, "clip": { "type": "string", "description": "A clip name to retarget before rendering." }, "frame": { "type": "integer", "description": "Clip frame to render (needs clip)." } }), json!([])) },
        { "name": "render_animation", "description": "Render a whole clip playing on the rigged model in Blender headless, encode it to an mp4 with ffmpeg (a file path you can open), and return a few sample frames inline so the motion is visible. Needs load_asset + fit_skeleton. Degrades to frames-only if ffmpeg is missing.",
          "inputSchema": obj(json!({ "clip": { "type": "string", "description": "A clip name from list_clips." }, "path": { "type": "string", "description": "Output mp4 path (default: a temp file, path reported back)." }, "max_frames": { "type": "integer", "description": "Cap on frames rendered (8-240, default 120); a longer clip is sampled evenly." } }), json!(["clip"])) },
        { "name": "compare_to_reference", "description": "Render a clip and sample the same number of frames from a REFERENCE VIDEO (e.g. real footage of the animal), returning both sets of frames inline for a side-by-side visual comparison of pose and timing. A qualitative aid, not a numeric score. Needs ffmpeg + load_asset + fit_skeleton.",
          "inputSchema": obj(json!({ "clip": { "type": "string" }, "reference": { "type": "string", "description": "Absolute path to a reference video file." }, "frames": { "type": "integer", "description": "Frames of each to compare (2-12, default 5)." } }), json!(["clip", "reference"])) },
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

/// Reads a PNG and wraps it as an MCP image content item (base64).
fn image_content(path: &Path) -> Result<Value, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let data = base64::engine::general_purpose::STANDARD.encode(&bytes);
    Ok(json!({ "type": "image", "data": data, "mimeType": "image/png" }))
}

/// Up to `n` evenly-spaced items across `items` (endpoints included), so a long
/// frame sequence can be previewed with a handful of images.
fn sample_evenly(items: &[PathBuf], n: usize) -> Vec<PathBuf> {
    if items.is_empty() || n == 0 {
        return Vec::new();
    }
    if items.len() <= n {
        return items.to_vec();
    }
    (0..n)
        .map(|i| items[i * (items.len() - 1) / (n - 1).max(1)].clone())
        .collect()
}

/// A filesystem-safe stem from a clip name (for a default output file).
fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}

fn arg_str(args: &Value, key: &str) -> Result<String, String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("missing required string argument `{key}`"))
}

/// Sets a position absolutely, or adds a delta when `is_nudge`.
fn apply_move(slot: &mut [f32; 3], is_nudge: bool, value: [f32; 3]) {
    if is_nudge {
        for i in 0..3 {
            slot[i] += value[i];
        }
    } else {
        *slot = value;
    }
}

/// Resolves a joint index from an `index` or a `name` argument.
fn resolve_joint(fitted: &pipeline::FittedSkeleton, args: &Value) -> Result<usize, String> {
    if let Some(index) = args.get("index").and_then(Value::as_u64) {
        let index = index as usize;
        return if index < fitted.bones.len() {
            Ok(index)
        } else {
            Err(format!(
                "joint index {index} out of range (0..{})",
                fitted.bones.len()
            ))
        };
    }
    if let Some(name) = args.get("name").and_then(Value::as_str) {
        return fitted
            .bones
            .iter()
            .position(|b| b == name)
            .ok_or_else(|| format!("no joint named {name:?}; call list_joints to see the names"));
    }
    Err("give the joint to move as `index` or `name`".to_string())
}

/// The left/right counterpart of a bone name, swapping the common side markers.
/// Returns `None` when the name carries no side (a spine or centre bone).
fn mirror_name(name: &str) -> Option<String> {
    const PAIRS: [(&str, &str); 6] = [
        ("_l", "_r"),
        ("_L", "_R"),
        (".l", ".r"),
        (".L", ".R"),
        ("left", "right"),
        ("Left", "Right"),
    ];
    for (a, b) in PAIRS {
        if let Some(stem) = name.strip_suffix(a) {
            return Some(format!("{stem}{b}"));
        }
        if let Some(stem) = name.strip_suffix(b) {
            return Some(format!("{stem}{a}"));
        }
        if name.contains(a) {
            return Some(name.replacen(a, b, 1));
        }
        if name.contains(b) {
            return Some(name.replacen(b, a, 1));
        }
    }
    None
}

/// Parses a 3-number array from an arbitrary JSON value (for `nudge`).
fn vec3_of(value: &Value, key: &str) -> Result<[f32; 3], String> {
    let a = value
        .as_array()
        .ok_or_else(|| format!("`{key}` must be a 3-number array"))?;
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
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 15);
    }

    #[test]
    fn mirror_name_and_apply_move() {
        assert_eq!(mirror_name("upperarm_l").as_deref(), Some("upperarm_r"));
        assert_eq!(mirror_name("upperarm_r").as_deref(), Some("upperarm_l"));
        assert_eq!(mirror_name("Hand_L").as_deref(), Some("Hand_R"));
        assert_eq!(mirror_name("spine"), None);

        let mut p = [1.0, 2.0, 3.0];
        apply_move(&mut p, false, [5.0, 5.0, 5.0]);
        assert_eq!(p, [5.0, 5.0, 5.0], "absolute set");
        apply_move(&mut p, true, [1.0, -1.0, 0.0]);
        assert_eq!(p, [6.0, 4.0, 5.0], "relative nudge");
    }

    /// Adjust by name + nudge + mirror moves both sides; undo restores them and
    /// then reports there is nothing left to undo.
    #[test]
    fn adjust_by_name_nudge_mirror_and_undo() {
        let mut server = Server::new();
        call(
            &mut server,
            "load_asset",
            json!({ "path": asset("models/model-human.glb") }),
        );
        call(&mut server, "fit_skeleton", json!({ "template": "human" }));

        let (listed, err) = call(&mut server, "list_joints", json!({}));
        assert!(!err, "{listed}");
        let joints: Value = serde_json::from_str(&listed).unwrap();
        let names: Vec<String> = joints["joints"]
            .as_array()
            .unwrap()
            .iter()
            .map(|j| j["name"].as_str().unwrap().to_string())
            .collect();
        // A bone that has a side AND whose counterpart exists in this rig.
        let sided = names
            .iter()
            .find(|n| mirror_name(n).is_some_and(|m| names.contains(&m)))
            .cloned()
            .expect("the human rig has a left/right bone");

        let (moved, err) = call(
            &mut server,
            "adjust_joint",
            json!({ "name": sided, "nudge": [0.05, 0.0, 0.0], "mirror": true }),
        );
        assert!(!err, "{moved}");
        let moved: Value = serde_json::from_str(&moved).unwrap();
        assert_eq!(
            moved["moved"].as_array().unwrap().len(),
            2,
            "moved both sides"
        );

        let (undone, err) = call(&mut server, "undo", json!({}));
        assert!(!err, "{undone}");
        // The one adjustment is now undone; a second undo has nothing to do.
        let (msg, err) = call(&mut server, "undo", json!({}));
        assert!(err && msg.contains("nothing to undo"), "{msg}");

        // Addressing an unknown name points at list_joints.
        let (msg, err) = call(
            &mut server,
            "adjust_joint",
            json!({ "name": "no_such_bone" }),
        );
        assert!(err && msg.contains("list_joints"), "{msg}");
    }

    #[test]
    fn sample_evenly_and_sanitize() {
        let paths: Vec<PathBuf> = (0..10)
            .map(|i| PathBuf::from(format!("f{i}.png")))
            .collect();
        let five = sample_evenly(&paths, 5);
        assert_eq!(five.len(), 5);
        assert_eq!(five.first(), Some(&paths[0]), "endpoints included");
        assert_eq!(five.last(), Some(&paths[9]));
        assert_eq!(sample_evenly(&paths[..3], 5).len(), 3, "fewer than n → all");
        assert!(sample_evenly(&[], 5).is_empty());
        assert_eq!(sanitize("Swim Horizontal/2"), "Swim_Horizontal_2");
    }

    /// The animation tools name their missing inputs before spending a Blender or
    /// ffmpeg launch. (The renders themselves need those tools, so only the guard
    /// rails are unit-tested here.)
    #[test]
    fn animation_tools_guard_their_inputs() {
        let mut server = Server::new();

        let (msg, err) = call(&mut server, "render_animation", json!({}));
        assert!(err && msg.contains("clip"), "missing clip: {msg}");
        let (msg, err) = call(&mut server, "render_animation", json!({ "clip": "Idle" }));
        assert!(err && msg.contains("load_asset"), "before load: {msg}");

        let (msg, err) = call(
            &mut server,
            "compare_to_reference",
            json!({ "clip": "Idle", "reference": "/no/such/video.mp4" }),
        );
        assert!(err && msg.contains("reference video not found"), "{msg}");
    }

    #[test]
    fn log_line_is_prefixed_and_levelled() {
        let line = log::line("info", "tool fit_skeleton ok");
        assert!(line.starts_with("[m2m-mcp "), "{line}");
        assert!(line.contains(" info] tool fit_skeleton ok"), "{line}");
    }

    /// diagnose grades a real fit+bind, and refuses (with a next-step hint)
    /// before a skeleton exists. Integration over the tool interface + a
    /// regression guard on the "fit first" precondition.
    #[test]
    fn diagnose_grades_a_fit_and_guards_its_preconditions() {
        let mut server = Server::new();

        // Precondition: diagnosing before loading names what to do first.
        let (msg, err) = call(&mut server, "diagnose", json!({}));
        assert!(err && msg.contains("load_asset"), "{msg}");

        call(
            &mut server,
            "load_asset",
            json!({ "path": asset("models/model-human.glb") }),
        );
        // ...and before fitting.
        let (msg, err) = call(&mut server, "diagnose", json!({}));
        assert!(err && msg.contains("fit_skeleton"), "{msg}");

        call(&mut server, "fit_skeleton", json!({ "template": "human" }));
        let (report, err) = call(&mut server, "diagnose", json!({}));
        assert!(!err, "{report}");
        let value: Value = serde_json::from_str(&report).unwrap();
        assert!(
            ["pass", "warn", "fail"].contains(&value["grade"].as_str().unwrap()),
            "{report}"
        );
        assert!(value["findings"].as_array().is_some_and(|f| !f.is_empty()));
        assert_eq!(value["joints"], 66);
        // The human template fits cleanly, so no vertex should detach.
        assert_eq!(value["bind"]["unweighted_vertices"], 0, "{report}");
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

    /// render_views validates its overlay before spending a Blender launch, and
    /// names what it needs before an asset is loaded. (The render itself needs
    /// Blender, so only the guard rails are unit-tested here.)
    #[test]
    fn render_views_validates_before_launching_blender() {
        let mut server = Server::new();

        // Nothing loaded: it names the first step, not a Blender error.
        let (msg, err) = call(&mut server, "render_views", json!({}));
        assert!(err && msg.contains("load_asset"), "{msg}");

        call(
            &mut server,
            "load_asset",
            json!({ "path": asset("models/model-human.glb") }),
        );
        call(&mut server, "fit_skeleton", json!({ "template": "human" }));
        // A bad overlay is rejected up front, listing the valid choices.
        let (msg, err) = call(&mut server, "render_views", json!({ "overlay": "bogus" }));
        assert!(err, "{msg}");
        assert!(
            msg.contains("skeleton") && msg.contains("weights"),
            "should list the valid overlays: {msg}"
        );
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
