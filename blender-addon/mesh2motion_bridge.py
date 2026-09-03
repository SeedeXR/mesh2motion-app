"""Mesh2Motion live bridge — a Blender add-on and headless server.

The companion to `m2m-bridge`'s headless mode (architecture.md §6). Where the
headless path spawns Blender per inspection, this LIVE mode lets a rig be pushed
into an *already-running* Blender the artist is working in, imported, and
reported on — the round trip a live session needs.

Two ways to run it:

* **As an add-on.** Install this file (Edit → Preferences → Add-ons → Install),
  enable "Mesh2Motion: Live Bridge", and press *Start Bridge Server* (or set it
  to start on enable in the add-on preferences). A background thread accepts
  connections; the actual import runs on Blender's main thread via
  `bpy.app.timers`, because `bpy` is not thread-safe.

* **Headless, single-shot.** `blender -b --python mesh2motion_bridge.py -- <port>`
  serves exactly one request on the main thread and exits. This is what the
  Rust side's `#[ignore]`d live test uses, and the simplest way to see the
  protocol work.

## Wire protocol

Localhost TCP. A request is a JSON header line, then exactly `len` raw bytes of
the `.glb`:

    {"cmd": "import", "name": "rig.glb", "len": 12345}\n<12345 bytes>

The reply is one JSON line — the same shape `m2m-bridge`'s `BlenderReport`
parses:

    {"ok": true, "file": "rig.glb", "imported": true, "bones": 66, ...}\n

`{"cmd": "ping"}` (len 0 or omitted) replies `{"ok": true, "pong": true}`.
"""

import json
import os
import socket
import sys
import tempfile

bl_info = {
    "name": "Mesh2Motion: Live Bridge",
    "author": "Mesh2Motion",
    "version": (0, 1, 0),
    "blender": (3, 0, 0),
    "location": "View3D — started from Add-on Preferences",
    "description": "Receives rigs pushed from the Mesh2Motion app and imports them live.",
    "category": "Import-Export",
}

DEFAULT_PORT = 47829


def build_report(name, objects):
    """Summarises the given objects, in the shape `BlenderReport` expects.

    Called on the main thread only. Takes an explicit object list rather than
    the whole scene so that in a live session — where the artist's own objects
    are present — the report describes only the rig that was just pushed.
    """
    import bpy

    armatures = [o for o in objects if o.type == "ARMATURE"]
    meshes = [o for o in objects if o.type == "MESH"]

    report = {"ok": True, "file": name, "imported": True}
    report["armatures"] = len(armatures)
    report["meshes"] = len(meshes)
    report["bones"] = sum(len(o.data.bones) for o in armatures)
    report["actions"] = sorted(a.name for a in bpy.data.actions)
    report["mesh_vertices"] = sorted(len(o.data.vertices) for o in meshes)

    weighted = 0
    weight_total = 0.0
    for obj in meshes:
        for vert in obj.data.vertices:
            summed = sum(g.weight for g in vert.groups)
            if summed > 0.0:
                weighted += 1
            weight_total += summed
    report["weighted_vertices"] = weighted
    report["weight_total"] = weight_total
    return report


def handle_request(header, payload, reset=False):
    """Turns one decoded request into a reply dict. Main thread only.

    `reset` is for the throwaway headless server only — it clears the file to
    factory settings first. In a live session it MUST stay false: resetting
    would wipe the artist's open scene (and, found the hard way, disconnect
    other add-ons). Live mode instead imports alongside whatever is open and
    reports only the objects the import added.
    """
    import bpy

    if header.get("cmd") == "ping":
        return {"ok": True, "pong": True}
    if header.get("cmd") != "import":
        return {"ok": False, "error": f"unknown command {header.get('cmd')!r}"}

    name = header.get("name", "pushed.glb")
    # Blender imports from a path, so the pushed bytes land in a temp file that
    # is removed on every return path.
    handle, path = tempfile.mkstemp(suffix=".glb")
    try:
        with os.fdopen(handle, "wb") as file:
            file.write(payload)
        if reset:
            bpy.ops.wm.read_factory_settings(use_empty=True)
        before = {o.name for o in bpy.data.objects}
        try:
            bpy.ops.import_scene.gltf(filepath=path, disable_bone_shape=True)
        except Exception as error:  # noqa: BLE001 - report whatever Blender raised
            return {"ok": True, "file": name, "imported": False,
                    "error": f"{type(error).__name__}: {error}"}
        added = [o for o in bpy.data.objects if o.name not in before]
        return build_report(name, added)
    finally:
        try:
            os.remove(path)
        except OSError:
            pass


def read_request(conn):
    """Reads a header line then its `len` payload bytes from a socket."""
    buffer = b""
    while b"\n" not in buffer:
        chunk = conn.recv(4096)
        if not chunk:
            return None, b""
        buffer += chunk
    line, rest = buffer.split(b"\n", 1)
    header = json.loads(line.decode("utf-8"))
    length = int(header.get("len", 0))
    payload = rest
    while len(payload) < length:
        chunk = conn.recv(min(65536, length - len(payload)))
        if not chunk:
            break
        payload += chunk
    return header, payload[:length]


def serve_once(port):
    """Accepts one connection on the main thread, handles it, and returns.

    Used by the headless test entry point. Runs `bpy` directly because there is
    no modal loop to marshal onto.
    """
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as server:
        server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        server.bind(("127.0.0.1", port))
        server.listen(1)
        print(f"M2M_BRIDGE listening on {port}")
        sys.stdout.flush()
        conn, _ = server.accept()
        with conn:
            header, payload = read_request(conn)
            reply = {"ok": False, "error": "no request"} if header is None \
                else handle_request(header, payload, reset=True)
            conn.sendall((json.dumps(reply) + "\n").encode("utf-8"))


# --- Add-on plumbing (interactive mode) -----------------------------------

_server_state = {"thread": None, "stop": False, "queue": [], "port": DEFAULT_PORT, "socket": None}


def _drain_queue():
    """Main-thread timer: imports any queued request and answers it."""
    import bpy  # noqa: F401 - proves we are on the main thread

    while _server_state["queue"]:
        conn, header, payload = _server_state["queue"].pop(0)
        try:
            reply = handle_request(header, payload)
        except Exception as error:  # noqa: BLE001
            reply = {"ok": False, "error": f"{type(error).__name__}: {error}"}
        try:
            conn.sendall((json.dumps(reply) + "\n").encode("utf-8"))
        finally:
            conn.close()
    return 0.1  # re-arm the timer


def _accept_loop():
    """Background thread: accept connections and queue them for the main thread.

    The listening socket is kept on `_server_state` so `stop_server` can close it
    even if this loop is blocked, and is always closed on the way out so the port
    is never leaked to a later start (found the hard way).
    """
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    _server_state["socket"] = server
    try:
        server.bind(("127.0.0.1", _server_state["port"]))
        server.listen(4)
        server.settimeout(0.5)
        while not _server_state["stop"]:
            try:
                conn, _ = server.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            header, payload = read_request(conn)
            if header is None:
                conn.close()
                continue
            # bpy is main-thread only, so the actual import is handed to the timer.
            _server_state["queue"].append((conn, header, payload))
    finally:
        try:
            server.close()
        except OSError:
            pass
        _server_state["socket"] = None


def start_server(port=DEFAULT_PORT):
    import bpy
    import threading

    if _server_state["thread"] is not None:
        return
    _server_state.update(port=port, stop=False)
    thread = threading.Thread(target=_accept_loop, daemon=True)
    thread.start()
    _server_state["thread"] = thread
    if not bpy.app.timers.is_registered(_drain_queue):
        bpy.app.timers.register(_drain_queue)


def stop_server():
    import bpy

    _server_state["stop"] = True
    _server_state["thread"] = None
    # Close the listening socket directly so the port is released now, not only
    # when the accept loop next wakes from its timeout.
    server = _server_state.get("socket")
    if server is not None:
        try:
            server.close()
        except OSError:
            pass
        _server_state["socket"] = None
    if bpy.app.timers.is_registered(_drain_queue):
        bpy.app.timers.unregister(_drain_queue)


def _register_operators():
    import bpy

    class M2M_OT_start_bridge(bpy.types.Operator):
        bl_idname = "m2m.start_bridge"
        bl_label = "Start Mesh2Motion Bridge Server"

        def execute(self, context):
            start_server()
            self.report({"INFO"}, f"Mesh2Motion bridge listening on {DEFAULT_PORT}")
            return {"FINISHED"}

    class M2M_OT_stop_bridge(bpy.types.Operator):
        bl_idname = "m2m.stop_bridge"
        bl_label = "Stop Mesh2Motion Bridge Server"

        def execute(self, context):
            stop_server()
            self.report({"INFO"}, "Mesh2Motion bridge stopped")
            return {"FINISHED"}

    return (M2M_OT_start_bridge, M2M_OT_stop_bridge)


_classes = []


def register():
    import bpy

    global _classes
    _classes = _register_operators()
    for cls in _classes:
        bpy.utils.register_class(cls)


def unregister():
    import bpy

    stop_server()
    for cls in reversed(_classes):
        bpy.utils.unregister_class(cls)


if __name__ == "__main__":
    # Headless single-shot: `blender -b --python this.py -- <port>`.
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    serve_once(int(argv[0]) if argv else DEFAULT_PORT)
