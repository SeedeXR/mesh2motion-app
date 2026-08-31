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
        --python tools/blender-fbx-import-check.py -- <file.fbx|file.glb> [report.json]

Prints one `BLENDER_JSON {...}` line, and writes the same JSON to
`report.json` when a second argument is given. **Prefer the file.** Blender
writes its own progress to stdout without always terminating the line, so a
caller scraping stdout can end up with the JSON concatenated to something
else — measured: a gate failed with `SyntaxError: Unexpected non-whitespace
character after JSON at position 5342` rather than the assertion it was
testing, which is a gate that can pass or fail for the wrong reason.

Exits non-zero if the import fails.
"""

import json
import os
import sys

import bpy


def main() -> int:
    args = sys.argv[sys.argv.index("--") + 1:]
    path = args[0]
    report_path = args[1] if len(args) > 1 else None

    def emit(data: dict) -> None:
        text = json.dumps(data)
        print("BLENDER_JSON " + text)
        if report_path is not None:
            with open(report_path, "w", encoding="utf-8") as handle:
                handle.write(text)

    bpy.ops.wm.read_factory_settings(use_empty=True)
    out = {"file": os.path.basename(path)}
    try:
        # Dispatch on extension so the same report can be produced for both
        # formats, which is what makes the FBX and glTF gates comparable.
        if path.lower().endswith((".glb", ".gltf")):
            # disable_bone_shape stops the importer adding an icosphere as a
            # bone display widget (blender/imp/node.py calls
            # primitive_ico_sphere_add). It is a viewport gizmo, not model
            # data, but it lands in bpy.data.objects as a real MESH and would
            # otherwise add a phantom 42-vertex, 80-polygon mesh to every
            # skinned file this compares.
            bpy.ops.import_scene.gltf(filepath=path, disable_bone_shape=True)
        else:
            bpy.ops.import_scene.fbx(filepath=path)
    except Exception as error:  # noqa: BLE001 - report whatever Blender raised
        out["imported"] = False
        out["error"] = f"{type(error).__name__}: {error}"
        emit(out)
        return 1

    out["imported"] = True
    out["armatures"] = sum(1 for o in bpy.data.objects if o.type == "ARMATURE")
    out["meshes"] = sum(1 for o in bpy.data.objects if o.type == "MESH")
    out["bones"] = sum(len(a.bones) for a in bpy.data.armatures)
    out["actions"] = sorted(a.name for a in bpy.data.actions)
    # "An action exists" is not "the animation survived". These make it
    # measurable: how many curves, how many keys, over what frame range, and
    # which properties are driven.
    #
    # Blender 4.4 introduced slotted actions and `Action.fcurves` is gone in
    # 5.2 — the curves live in `layers[].strips[].channelbags[].fcurves`. Both
    # shapes are handled because this tool has to keep working across versions.
    def fcurves_of(action):
        if hasattr(action, "fcurves"):
            return list(action.fcurves)
        found = []
        for layer in getattr(action, "layers", []):
            for strip in getattr(layer, "strips", []):
                for bag in getattr(strip, "channelbags", []):
                    found.extend(bag.fcurves)
        return found

    out["action_detail"] = sorted(
        "{}:curves={},keys={},range={:.2f}-{:.2f}".format(
            action.name,
            len(fcurves_of(action)),
            sum(len(fc.keyframe_points) for fc in fcurves_of(action)),
            *action.frame_range,
        )
        for action in bpy.data.actions
    )
    # Which properties are driven, so a clip that lost every rotation but kept
    # its translations does not read as intact.
    out["animated_paths"] = sorted(
        {
            fc.data_path.rsplit(".", 1)[-1]
            for action in bpy.data.actions
            for fc in fcurves_of(action)
        }
    )

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

    # The hierarchy, not just the names. A flattened skeleton has the right
    # bone count and the right names and is still not the same rig — measured:
    # parenting every bone to the root passed a names-and-counts comparison.
    out["bone_parents"] = sorted(
        f"{b.name}<-{b.parent.name if b.parent else ''}"
        for a in bpy.data.armatures
        for b in a.bones
    )
    out["root_bones"] = sorted(
        b.name for a in bpy.data.armatures for b in a.bones if b.parent is None
    )

    # Where the bones actually ARE. Names, counts, hierarchy and weights can
    # all match while every joint sits in the wrong place — measured: dropping
    # PreRotation, which 440 of 522 models in the reference corpus carry, was
    # invisible to every other field here.
    #
    # Rounded to 4 decimals: FBX stores these as f32 and Blender recomputes
    # roll, so the last digits are noise while a real change moves whole units.
    out["bone_rest"] = sorted(
        "{}:{}".format(
            b.name,
            ",".join(
                f"{v:.4f}"
                for v in (*b.head_local, *b.tail_local)
            ),
        )
        for a in bpy.data.armatures
        for b in a.bones
    )

    # Skin weights, which are what "the rig survived" actually means (O9).
    # Reported as totals rather than per-vertex so the numbers stay comparable
    # between two files without being enormous.
    groups = set()
    weighted_vertices = 0
    weight_sum = 0.0
    influence_counts = {}
    for obj in bpy.data.objects:
        if obj.type != "MESH":
            continue
        groups.update(g.name for g in obj.vertex_groups)
        for vertex in obj.data.vertices:
            influences = [g for g in vertex.groups if g.weight > 0.0]
            if influences:
                weighted_vertices += 1
                weight_sum += sum(g.weight for g in influences)
            count = len(influences)
            influence_counts[count] = influence_counts.get(count, 0) + 1
    out["vertex_groups"] = len(groups)
    out["weighted_vertices"] = weighted_vertices
    # Rounded, because f32 storage and Blender's own normalisation make the
    # last digits meaningless while a real change moves this by whole units.
    out["weight_total"] = round(weight_sum, 2)
    out["influences_per_vertex"] = {str(k): v for k, v in sorted(influence_counts.items())}
    emit(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
