"""Unit tests for the Blender bridge add-on's pure logic — no Blender needed.

The add-on imports `bpy` only inside functions, so the module imports cleanly
here and its pure helpers (`check_token`, `mesh_report`, `read_request`) are
tested with plain stand-ins. Run directly (`python test_bridge.py`) or under
pytest. The bpy-dependent paths (import/render) are covered by the Rust
`m2m-bridge` live test (`#[ignore]`, needs a running Blender).
"""

import importlib.util
import os
import sys

_HERE = os.path.dirname(os.path.abspath(__file__))
_SPEC = importlib.util.spec_from_file_location(
    "m2m_bridge_addon", os.path.join(_HERE, "..", "mesh2motion_bridge.py")
)
bridge = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(bridge)


# --- stand-ins that duck-type the bits of bpy the pure code touches ---------

class Vec(list):
    """A 3-vector that supports `matrix @ co` by identity (matrix is Identity)."""


class Identity:
    def __matmul__(self, other):
        return other


class Group:
    def __init__(self, weight):
        self.weight = weight


class Vert:
    def __init__(self, weights, co=(0.0, 0.0, 0.0)):
        self.groups = [Group(w) for w in weights]
        self.co = Vec(co)


class Data:
    def __init__(self, verts=None, bones=None):
        self.vertices = verts or []
        self.bones = bones or []


class Obj:
    def __init__(self, kind, data, materials=()):
        self.type = kind
        self.data = data
        self.matrix_world = Identity()
        self.material_slots = [type("Slot", (), {"material": m})() for m in materials]


class Action:
    def __init__(self, name):
        self.name = name


def test_check_token():
    assert bridge.check_token({}, None) == (True, None)  # no token → open
    assert bridge.check_token({"token": "x"}, None) == (True, None)
    ok, err = bridge.check_token({}, "secret")
    assert not ok and "unauthorized" in err
    assert bridge.check_token({"token": "secret"}, "secret") == (True, None)
    ok, _ = bridge.check_token({"token": "wrong"}, "secret")
    assert not ok


def test_mesh_report_counts_weights_and_geometry():
    mat_l = type("Mat", (), {"name": "skin"})()
    mesh = Obj(
        "MESH",
        Data(verts=[
            Vert([1.0], co=(0, 0, 0)),          # 1 influence
            Vert([0.5, 0.5], co=(2, 0, 0)),     # 2 influences
            Vert([], co=(1, 3, 0)),             # unweighted
            Vert([0.2, 0.2, 0.2, 0.2, 0.2], co=(0, 0, 1)),  # over-influence (5)
        ]),
        materials=[mat_l],
    )
    arm = Obj("ARMATURE", Data(bones=[object(), object(), object()]))
    report = bridge.mesh_report("rig.glb", [mesh], [arm], [Action("Walk"), Action("Idle")])

    assert report["ok"] and report["imported"]
    assert report["bones"] == 3
    assert report["meshes"] == 1 and report["armatures"] == 1
    assert report["actions"] == ["Idle", "Walk"]  # sorted
    assert report["weighted_vertices"] == 3
    assert report["unweighted_vertices"] == 1
    assert report["over_influence_vertices"] == 1
    assert report["materials"] == ["skin"]
    assert report["bbox_min"] == [0, 0, 0]
    assert report["bbox_max"] == [2, 3, 1]


class FakeConn:
    """A socket whose recv() hands back a scripted byte stream in chunks."""

    def __init__(self, blob, chunk=7):
        self.blob = blob
        self.chunk = chunk
        self.pos = 0

    def recv(self, _size):
        piece = self.blob[self.pos:self.pos + self.chunk]
        self.pos += len(piece)
        return piece


def test_read_request_splits_header_and_payload():
    payload = b"\x00\x01\x02\x03\x04"
    blob = b'{"cmd": "import", "name": "a.glb", "len": 5}\n' + payload
    header, body = bridge.read_request(FakeConn(blob))
    assert header["cmd"] == "import" and header["name"] == "a.glb"
    assert body == payload

    header, body = bridge.read_request(FakeConn(b'{"cmd": "ping"}\n'))
    assert header["cmd"] == "ping" and body == b""


def _run():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for test in tests:
        test()
        print(f"ok  {test.__name__}")
    print(f"\n{len(tests)} passed")


if __name__ == "__main__":
    sys.exit(_run())
