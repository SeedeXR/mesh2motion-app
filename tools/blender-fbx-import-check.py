"""Imports an FBX in headless Blender and reports what it found, as JSON.

Blender's importer is the strictest FBX reader available here, and the only
independent one: our reader and three.js's share a design — `end_offset` is
authoritative, the footer is detected heuristically, an uncompressed array's
declared byte length is ignored — so neither can check the conformance details
the other gets wrong.

Measured on the encoder's output: of four details that survive both our round
trip and the three.js gate, Blender catches two (a missing null record after a
child list, and an array whose declared byte length is wrong). It also caught
the object-name truncation that made our first encoder output unopenable.

    blender --background --factory-startup \
        --python tools/blender-fbx-import-check.py -- <file.fbx>

Prints one `BLENDER_JSON {...}` line. Exits non-zero if the import fails.
"""

import json
import os
import sys

import bpy


def main() -> int:
    path = sys.argv[sys.argv.index("--") + 1]
    bpy.ops.wm.read_factory_settings(use_empty=True)
    out = {"file": os.path.basename(path)}
    try:
        bpy.ops.import_scene.fbx(filepath=path)
    except Exception as error:  # noqa: BLE001 - report whatever Blender raised
        out["imported"] = False
        out["error"] = f"{type(error).__name__}: {error}"
        print("BLENDER_JSON " + json.dumps(out))
        return 1

    out["imported"] = True
    out["armatures"] = sum(1 for o in bpy.data.objects if o.type == "ARMATURE")
    out["meshes"] = sum(1 for o in bpy.data.objects if o.type == "MESH")
    out["bones"] = sum(len(a.bones) for a in bpy.data.armatures)
    out["actions"] = sorted(a.name for a in bpy.data.actions)
    out["mesh_vertices"] = sorted(len(m.vertices) for m in bpy.data.meshes)
    out["mesh_polygons"] = sorted(len(m.polygons) for m in bpy.data.meshes)
    # Corner counts per face, so a polygon-index encoding that merges or splits
    # faces shows up rather than only changing a total.
    out["polygon_sizes"] = sorted({len(poly.vertices) for m in bpy.data.meshes for poly in m.polygons})
    out["loose_vertices"] = sum(
        1 for m in bpy.data.meshes for v in m.vertices
        if not any(v.index in poly.vertices for poly in m.polygons)
    )
    out["bone_names"] = sorted(b.name for a in bpy.data.armatures for b in a.bones)
    print("BLENDER_JSON " + json.dumps(out))
    return 0


if __name__ == "__main__":
    sys.exit(main())
