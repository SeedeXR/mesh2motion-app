"""Mesh2Motion live bridge — a Blender add-on and headless server.

The companion to `m2m-bridge`'s headless mode (architecture.md §6). Where the
headless path spawns Blender per inspection, this LIVE mode lets a rig be pushed
into an *already-running* Blender the artist is working in, imported, reported
on, and rendered back — the round trip a live session needs.

Two ways to run it:

* **As an add-on.** Install this file (Edit → Preferences → Add-ons → Install),
  enable "Mesh2Motion: Live Bridge". Start/stop the server from the *Mesh2Motion*
  panel in the View3D sidebar (press N), or from the add-on preferences. A
  background thread accepts connections; the actual work runs on Blender's main
  thread via `bpy.app.timers`, because `bpy` is not thread-safe.

* **Headless, single-shot.** `blender -b --python mesh2motion_bridge.py -- <port>`
  serves exactly one request on the main thread and exits.

## Wire protocol

Localhost TCP. A request is a JSON header line, then exactly `len` raw bytes:

    {"cmd": "import", "name": "rig.glb", "len": 12345}\n<12345 bytes>

Commands:
* ``import`` — import the payload glb and reply with a report (see `mesh_report`).
* ``ping``   — reply ``{"ok": true, "pong": true}``.
* ``render`` — render the current scene to a PNG and reply with it base64-encoded
               in ``image`` (no payload needed).

If ``M2M_BRIDGE_TOKEN`` is set, every request must carry a matching ``token``
field or it is refused — a small guard for a socket on a shared machine.

Logs go to stderr (and to ``M2M_BRIDGE_LOG`` if set), never onto the wire.
"""

import base64
import json
import os
import socket
import sys
import tempfile
import time

bl_info = {
    "name": "Mesh2Motion: Live Bridge",
    "author": "Mesh2Motion",
    "version": (0, 2, 0),
    "blender": (3, 0, 0),
    "location": "View3D → Sidebar (N) → Mesh2Motion",
    "description": "Receives rigs pushed from the Mesh2Motion app, reports on them, and renders back.",
    "category": "Import-Export",
}

DEFAULT_PORT = 47829


# --- logging (stderr + optional file), never onto the socket ----------------

def _log(level, msg):
    """Write one timestamped log line to stderr and, if `M2M_BRIDGE_LOG` is set,
    append it to that file. Never writes to the wire (stdout stays clean)."""
    line = f"[m2m-bridge {int(time.time() * 1000)} {level}] {msg}"
    sys.stderr.write(line + "\n")
    sys.stderr.flush()
    path = os.environ.get("M2M_BRIDGE_LOG")
    if path:
        try:
            with open(path, "a", encoding="utf-8") as handle:
                handle.write(line + "\n")
        except OSError:
            pass  # logging must never take down the server


# --- pure helpers (no bpy; unit-tested in tests/test_bridge.py) -------------

def check_token(header, expected):
    """Is this request allowed? With no `expected` token, everything is; with one,
    the header's `token` must match exactly. Returns (ok, error_or_None)."""
    if not expected:
        return True, None
    if header.get("token") != expected:
        return False, "unauthorized: missing or wrong token"
    return True, None


def mesh_report(name, meshes, armatures, actions):
    """Summarise the given meshes/armatures/actions into the reply dict — the
    shape `BlenderReport` parses, plus richer bind/geometry detail.

    Takes explicit lists (not the whole scene) so that in a live session, where
    the artist's own objects are present, the report describes only what was just
    pushed. Objects are duck-typed (`.data.vertices`, `.matrix_world`, …) so this
    is testable with plain stand-ins and no bpy.
    """
    report = {"ok": True, "file": name, "imported": True}
    report["armatures"] = len(armatures)
    report["meshes"] = len(meshes)
    report["bones"] = sum(len(o.data.bones) for o in armatures)
    report["actions"] = sorted(a.name for a in actions)
    report["mesh_vertices"] = sorted(len(o.data.vertices) for o in meshes)

    weighted = 0
    unweighted = 0
    over_influence = 0
    weight_total = 0.0
    lo = [float("inf")] * 3
    hi = [float("-inf")] * 3
    materials = set()
    for obj in meshes:
        matrix = obj.matrix_world
        for slot in getattr(obj, "material_slots", []):
            if slot.material is not None:
                materials.add(slot.material.name)
        for vert in obj.data.vertices:
            groups = [g for g in vert.groups if g.weight > 1e-6]
            summed = sum(g.weight for g in groups)
            if len(groups) == 0:
                unweighted += 1
            else:
                weighted += 1
            if len(groups) > 4:
                over_influence += 1
            weight_total += summed
            world = matrix @ vert.co
            for i in range(3):
                lo[i] = min(lo[i], world[i])
                hi[i] = max(hi[i], world[i])

    report["weighted_vertices"] = weighted
    report["unweighted_vertices"] = unweighted
    report["over_influence_vertices"] = over_influence
    report["weight_total"] = weight_total
    report["materials"] = sorted(materials)
    if lo[0] != float("inf"):
        report["bbox_min"] = lo
        report["bbox_max"] = hi
    return report


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


# --- request handling (main thread; uses bpy) -------------------------------

def build_report(name, objects):
    """Live-session report over just the objects an import added."""
    import bpy

    meshes = [o for o in objects if o.type == "MESH"]
    armatures = [o for o in objects if o.type == "ARMATURE"]
    return mesh_report(name, meshes, armatures, bpy.data.actions)


def _render_scene():
    """Renders the current scene to a temp PNG (Workbench, framing the meshes if
    there is no camera) and returns its base64. Main thread only."""
    import bpy
    from mathutils import Vector

    scene = bpy.context.scene
    created = []
    try:
        if scene.camera is None:
            meshes = [o for o in bpy.data.objects if o.type == "MESH"]
            lo = Vector((float("inf"),) * 3)
            hi = Vector((float("-inf"),) * 3)
            for obj in meshes:
                for corner in obj.bound_box:
                    world = obj.matrix_world @ Vector(corner)
                    lo = Vector(min(a, b) for a, b in zip(lo, world))
                    hi = Vector(max(a, b) for a, b in zip(hi, world))
            center = (lo + hi) * 0.5 if meshes else Vector((0, 0, 0))
            radius = max((hi - lo).length * 0.5, 1.0) if meshes else 3.0
            cam_data = bpy.data.cameras.new("m2m_cam")
            cam = bpy.data.objects.new("m2m_cam", cam_data)
            scene.collection.objects.link(cam)
            created.append(cam)
            cam.location = center + Vector((radius * 2.6, -radius * 2.6, radius * 1.6))
            direction = (center - cam.location).normalized()
            cam.rotation_euler = direction.to_track_quat("-Z", "Y").to_euler()
            scene.camera = cam

        scene.render.engine = "BLENDER_WORKBENCH"
        scene.render.resolution_x = 512
        scene.render.resolution_y = 512
        handle, path = tempfile.mkstemp(suffix=".png")
        os.close(handle)
        scene.render.filepath = path
        bpy.ops.render.render(write_still=True)
        with open(path, "rb") as image:
            data = base64.b64encode(image.read()).decode("ascii")
        os.remove(path)
        return data
    finally:
        for obj in created:
            bpy.data.objects.remove(obj, do_unlink=True)


def handle_request(header, payload, reset=False):
    """Turns one decoded request into a reply dict. Main thread only.

    `reset` is for the throwaway headless server only — it clears the file first.
    In a live session it MUST stay false: resetting would wipe the artist's open
    scene (and, found the hard way, disconnect other add-ons).
    """
    import bpy

    ok, error = check_token(header, os.environ.get("M2M_BRIDGE_TOKEN"))
    if not ok:
        _log("error", error)
        return {"ok": False, "error": error}

    cmd = header.get("cmd")
    if cmd == "ping":
        return {"ok": True, "pong": True}
    if cmd == "render":
        try:
            return {"ok": True, "image": _render_scene(), "mimeType": "image/png"}
        except Exception as error:  # noqa: BLE001 - report whatever Blender raised
            _log("error", f"render failed: {type(error).__name__}: {error}")
            return {"ok": False, "error": f"{type(error).__name__}: {error}"}
    if cmd != "import":
        return {"ok": False, "error": f"unknown command {cmd!r}"}

    name = header.get("name", "pushed.glb")
    _log("info", f"import {name} ({len(payload)} bytes)")
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
            _log("error", f"import failed: {type(error).__name__}: {error}")
            return {"ok": True, "file": name, "imported": False,
                    "error": f"{type(error).__name__}: {error}"}
        added = [o for o in bpy.data.objects if o.name not in before]
        report = build_report(name, added)
        _log("info", f"imported {name}: {report['bones']} bones, {report['meshes']} meshes")
        return report
    finally:
        try:
            os.remove(path)
        except OSError:
            pass


def serve_once(port):
    """Accepts one connection on the main thread, handles it, and returns."""
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as server:
        server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        server.bind(("127.0.0.1", port))
        server.listen(1)
        _log("info", f"listening on {port} (single-shot)")
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


def is_running():
    return _server_state["thread"] is not None


def _drain_queue():
    """Main-thread timer: imports any queued request and answers it."""
    import bpy  # noqa: F401 - proves we are on the main thread

    while _server_state["queue"]:
        conn, header, payload = _server_state["queue"].pop(0)
        try:
            reply = handle_request(header, payload)
        except Exception as error:  # noqa: BLE001
            _log("error", f"handler crashed: {type(error).__name__}: {error}")
            reply = {"ok": False, "error": f"{type(error).__name__}: {error}"}
        try:
            conn.sendall((json.dumps(reply) + "\n").encode("utf-8"))
        finally:
            conn.close()
    return 0.1  # re-arm the timer


def _accept_loop():
    """Background thread: accept connections and queue them for the main thread."""
    server = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
    server.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    _server_state["socket"] = server
    try:
        server.bind(("127.0.0.1", _server_state["port"]))
        server.listen(4)
        server.settimeout(0.5)
        _log("info", f"listening on {_server_state['port']}")
        while not _server_state["stop"]:
            try:
                conn, _ = server.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            try:
                header, payload = read_request(conn)
            except (ValueError, OSError) as error:
                _log("error", f"bad request dropped: {error}")
                conn.close()
                continue
            if header is None:
                conn.close()
                continue
            # bpy is main-thread only, so the actual work is handed to the timer.
            _server_state["queue"].append((conn, header, payload))
    finally:
        try:
            server.close()
        except OSError:
            pass
        _server_state["socket"] = None
        _log("info", "stopped")


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

    class M2M_PT_bridge(bpy.types.Panel):
        bl_label = "Mesh2Motion"
        bl_idname = "M2M_PT_bridge"
        bl_space_type = "VIEW_3D"
        bl_region_type = "UI"
        bl_category = "Mesh2Motion"

        def draw(self, context):
            layout = self.layout
            running = is_running()
            layout.label(text=f"Server: {'listening on ' + str(DEFAULT_PORT) if running else 'stopped'}")
            if running:
                layout.operator("m2m.stop_bridge", text="Stop Bridge", icon="PAUSE")
            else:
                layout.operator("m2m.start_bridge", text="Start Bridge", icon="PLAY")

    return (M2M_OT_start_bridge, M2M_OT_stop_bridge, M2M_PT_bridge)


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
    argv = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else []
    serve_once(int(argv[0]) if argv else DEFAULT_PORT)
