# Mesh2Motion Live Bridge (Blender add-on)

Push a rig from the Mesh2Motion app straight into a **running** Blender, import
it, and get a report back — the live counterpart to the headless bridge
(`crates/m2m-bridge`, architecture.md §6).

## Install

1. Blender → Edit → Preferences → Add-ons → **Install…**
2. Pick `mesh2motion_bridge.py` from this folder.
3. Tick **Mesh2Motion: Live Bridge** to enable it.
4. Run the **Start Mesh2Motion Bridge Server** operator (F3 → search "bridge"),
   or call `mesh2motion_bridge.start_server()` from the Python console.

The server listens on `127.0.0.1:47829`. It accepts connections on a background
thread and runs each import on Blender's main thread (via `bpy.app.timers`),
because `bpy` is not thread-safe.

## Protocol

Localhost TCP. A request is a JSON header line then the raw `.glb` bytes:

```
{"cmd":"import","name":"rig.glb","len":12345}\n<12345 bytes>
```

The reply is one JSON line in the `BlenderReport` shape
(`{"ok":true,"imported":true,"bones":66,"meshes":1,...}`). `{"cmd":"ping"}`
replies `{"ok":true,"pong":true}`.

## Headless mode (used by the tests)

`blender -b --python mesh2motion_bridge.py -- <port>` serves exactly one request
on the main thread and exits. The Rust side's `#[ignore]`d
`a_rig_pushed_to_a_live_blender_reads_back` test drives this end to end.
