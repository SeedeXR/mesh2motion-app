# m2m-mcp — the rigging pipeline as an MCP server

An [MCP](https://modelcontextprotocol.io) server that exposes Mesh2Motion's
rigging pipeline as tools any MCP client — Claude Code, the Claude desktop app,
or another agent — can drive. An agent can load a mesh, choose and fit a
skeleton template, refine joints, bind weights, export, and confirm the export
opens in Blender and Maya, all through tool calls.

It speaks newline-delimited JSON-RPC 2.0 over **stdio**, the standard MCP local
transport, and holds a **session**: each tool advances the same loaded model and
fitted skeleton, so a rig is built one call at a time.

## Tools

| Tool | What it does |
|------|--------------|
| `session_status` | Report what is loaded and fitted so far. |
| `list_templates` | The creature skeleton templates that can be fitted. |
| `load_asset` | Load a model (glb/gltf/fbx) and report what it contains. |
| `fit_skeleton` | Auto-fit a template's skeleton to the loaded mesh (voxel + landmark, pose-aware). |
| `adjust_joint` | Move one fitted joint to refine placement before binding. |
| `bind_weights` | Bind the mesh to the skeleton and report the weighting. |
| `list_clips` | The animation clips a creature's library offers. |
| `export` | Export the rigged model (glb/fbx), optionally with a clip retargeted on. |
| `validate_export` | Import an export into Blender and/or Maya **headless** and report what each read back. |
| `render_views` | Render the rig from several angles (a turntable) in Blender headless and return the PNGs, so the agent can **see** the pose and deformation and refine it; optional clip + frame to inspect a pose mid-animation. |

## Build

```bash
cargo build --release -p m2m-mcp     # -> target/release/m2m-mcp
```

`scripts/install.sh` builds this binary, runs its self-check, and registers it
with Claude Code (when the `claude` CLI is present) as part of installing the
app — so a normal install leaves the MCP ready to use.

## Health check

```bash
m2m-mcp --check
```

Drives the server's own `initialize` + `tools/list` path and prints a JSON
report: the protocol version, every tool that is active (all 10 when healthy),
and whether the optional Blender / Maya engines that `render_views` and
`validate_export` need are reachable. Exits non-zero if the server is not wired.
Use it to confirm the server is alive and all tools are active.

## Register it

**Claude Code** (from the repo root, so the default asset path resolves):

```bash
claude mcp add -s user mesh2motion -- /absolute/path/to/target/release/m2m-mcp
```

`-s user` makes it available across all projects (drop it for this directory
only). `scripts/install.sh` registers it at user scope for you.

or in a project `.mcp.json`:

```json
{
  "mcpServers": {
    "mesh2motion": { "command": "/absolute/path/to/target/release/m2m-mcp" }
  }
}
```

**Claude desktop app** — add the same entry to its MCP config
(`claude_desktop_config.json`).

## Environment

- `M2M_ASSETS` — where the animation libraries live (default: the repo's
  `assets/`). Needed for `list_clips` and exporting with a clip.
- `M2M_BLENDER` / `M2M_MAYAPY` — override the Blender / `mayapy` executables that
  `validate_export` spawns; otherwise the macOS defaults are tried. Validation
  is skipped gracefully (reported as an error string) when the engine is absent.

## A session, end to end

```
initialize
tools/call load_asset      { "path": "…/model-human.glb" }
tools/call fit_skeleton    { "template": "human" }
tools/call adjust_joint    { "index": 12, "position": [0.0, 1.4, 0.0] }
tools/call bind_weights    { }
tools/call render_views    { "num_views": 4 }                     # see it, refine it
tools/call export          { "path": "/tmp/rig.glb", "format": "glb" }
tools/call validate_export { "path": "/tmp/rig.glb", "engines": ["blender", "maya"] }
```

`render_views` returns image content, so an agent that supports images sees the
turntable directly — rotate around the rig, spot a bad joint, `adjust_joint`,
render again. With a `clip` + `frame` it renders a pose mid-animation, to judge
whether the motion looks natural before exporting. It needs Blender (as
`validate_export` does).
