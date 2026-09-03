"""Imports an FBX in headless Maya (mayapy) and reports what it found, as JSON.

Maya is the strictest independent FBX reader available here — it catches the
footer-CRC and DefaultAttributeIndex conformance details that Blender and our
own reader wave through (see memory: "FBX: Maya is the strict reader"). This is
the Maya counterpart to `blender-fbx-import-check.py`; the report fields line up
(joints ≈ bones, skinClusters ≈ vertex groups) so the two engines can be
compared on the same export.

    /path/to/mayapy tools/maya-fbx-import-check.py <file.fbx> [report.json]

Prints one `MAYA_JSON {...}` line and writes the same JSON to `report.json`
when a second argument is given. **Prefer the file** — Maya writes plug-in and
licensing chatter to stdout that a caller scraping stdout can concatenate onto
the JSON. Exits non-zero if the import fails.
"""

import json
import os
import sys

import maya.standalone

maya.standalone.initialize()

import maya.cmds as cmds  # noqa: E402 - only importable after initialize()
import maya.mel as mel  # noqa: E402


def short(name):
    """The leaf of a Maya dag path (`|a|b|joint` -> `joint`)."""
    return name.rsplit("|", 1)[-1]


def main():
    args = sys.argv[1:]
    if not args:
        print("usage: mayapy maya-fbx-import-check.py <file.fbx> [report.json]")
        return 2
    path = args[0]
    report_path = args[1] if len(args) > 1 else None
    out = {"file": os.path.basename(path)}

    def emit(data):
        text = json.dumps(data)
        print("MAYA_JSON " + text)
        if report_path is not None:
            with open(report_path, "w", encoding="utf-8") as handle:
                handle.write(text)

    try:
        cmds.file(new=True, force=True)
        cmds.loadPlugin("fbxmaya", quiet=True)
        # The FBX plug-in is driven through MEL; the Python wrapper is a thin
        # shim over the same commands. Forward-slash the path so a Windows path
        # would not break the MEL string either.
        mel.eval('FBXImport -f "{}"'.format(path.replace("\\", "/")))
    except Exception as error:  # noqa: BLE001 - report whatever Maya raised
        out["imported"] = False
        out["error"] = "{}: {}".format(type(error).__name__, error)
        emit(out)
        return 1

    joints = cmds.ls(type="joint", long=True) or []
    meshes = cmds.ls(type="mesh", noIntermediate=True, long=True) or []
    skins = cmds.ls(type="skinCluster") or []

    out["imported"] = True
    out["joints"] = len(joints)
    out["meshes"] = len(meshes)
    out["skin_clusters"] = len(skins)
    out["joint_names"] = sorted(short(j) for j in joints)

    # Hierarchy, not just names: a flattened skeleton has the right count and
    # names and is still the wrong rig.
    def parent_joint(j):
        parents = cmds.listRelatives(j, parent=True, type="joint", fullPath=True) or []
        return short(parents[0]) if parents else ""

    out["joint_parents"] = sorted("{}<-{}".format(short(j), parent_joint(j)) for j in joints)
    out["root_joints"] = sorted(short(j) for j in joints if not parent_joint(j))
    out["mesh_vertices"] = sorted(cmds.polyEvaluate(m, vertex=True) for m in meshes)

    # Skin weights are what "the rig survived" actually means. Totalled rather
    # than per-vertex so the numbers stay comparable between two files.
    weighted_vertices = 0
    weight_sum = 0.0
    influence_counts = {}
    for skin in skins:
        geometry = cmds.skinCluster(skin, query=True, geometry=True) or []
        influences = cmds.skinCluster(skin, query=True, influence=True) or []
        for mesh in geometry:
            count = cmds.polyEvaluate(mesh, vertex=True)
            for index in range(count):
                vertex = "{}.vtx[{}]".format(mesh, index)
                values = cmds.skinPercent(skin, vertex, query=True, value=True) or []
                nonzero = [w for w in values if w > 0.0]
                if nonzero:
                    weighted_vertices += 1
                    weight_sum += sum(nonzero)
                influence_counts[len(nonzero)] = influence_counts.get(len(nonzero), 0) + 1
        _ = influences  # influence list kept for parity; counts come from weights

    out["weighted_vertices"] = weighted_vertices
    out["weight_total"] = round(weight_sum, 2)
    out["influences_per_vertex"] = {str(k): v for k, v in sorted(influence_counts.items())}
    emit(out)
    return 0


if __name__ == "__main__":
    sys.exit(main())
