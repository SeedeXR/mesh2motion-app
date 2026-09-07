"""Renders a model from several angles in headless Blender, for visual review.

Imports a glb, frames it, and renders `num_views` shots evenly spaced around it
(a turntable) into `out_dir` as `view_0.png` … Three modes give an agent
different *eyes* on the same rig:

* ``solid``    — the mesh as shaded geometry (the default; see the pose + form).
* ``skeleton`` — the fitted bones drawn as bright geometry through an X-rayed
                 mesh, so you can see where the skeleton sits inside the body.
* ``weights``  — the mesh tinted by how many bones drive each vertex (1=red …
                 4=green) with UNWEIGHTED vertices flagged magenta, so a bad
                 bind (detached islands, single-bone rigidity) is visible at a
                 glance.

The Workbench engine is used — solid shading, no ray tracing — because the point
is to see the mesh, the pose and the deformation fast, not to light a scene. An
optional frame renders that frame of an imported clip, to inspect a pose
mid-animation.

    blender -b --factory-startup --python tools/blender-render-views.py -- \
        <model.glb> <out_dir> <num_views> [mode] [frame]

`mode` defaults to ``solid``; `frame` is optional. Prints one `RENDERED <path>`
line per image and exits non-zero on failure.
"""

import math
import os
import sys

import bpy
from mathutils import Vector

# Influence-count → RGBA. 0 (unweighted) is magenta so a detached vertex screams;
# 1 bone red (rigid), 2 orange, 3 yellow-green, 4 green (well blended).
INFLUENCE_COLORS = {
    0: (1.0, 0.0, 1.0, 1.0),
    1: (0.90, 0.12, 0.12, 1.0),
    2: (0.95, 0.55, 0.10, 1.0),
    3: (0.85, 0.85, 0.15, 1.0),
    4: (0.20, 0.80, 0.25, 1.0),
}


def _combined_bounds(objects):
    """World-space (min, max) corner over every object's bounding box."""
    lo = Vector((float("inf"),) * 3)
    hi = Vector((float("-inf"),) * 3)
    for obj in objects:
        for corner in obj.bound_box:
            world = obj.matrix_world @ Vector(corner)
            lo = Vector(min(a, b) for a, b in zip(lo, world))
            hi = Vector(max(a, b) for a, b in zip(hi, world))
    return lo, hi


def build_skeleton_geometry(armatures):
    """A single mesh of small diamonds at each joint joined to their parents, in
    bright emissive material, so the skeleton reads through an X-rayed mesh."""
    verts = []
    edges = []
    faces = []
    diamond = 0.0
    joints = []
    for arm in armatures:
        mw = arm.matrix_world
        for bone in arm.data.bones:
            joints.append((mw @ bone.head_local, mw @ bone.tail_local))
    if not joints:
        return None
    # Diamond radius scales with the median bone length so it reads at any size.
    lengths = sorted((t - h).length for h, t in joints if (t - h).length > 1e-6)
    diamond = (lengths[len(lengths) // 2] if lengths else 0.05) * 0.22
    diamond = max(diamond, 1e-4)

    def add_diamond(center):
        base = len(verts)
        offsets = [
            Vector((diamond, 0, 0)), Vector((-diamond, 0, 0)),
            Vector((0, diamond, 0)), Vector((0, -diamond, 0)),
            Vector((0, 0, diamond)), Vector((0, 0, -diamond)),
        ]
        for off in offsets:
            verts.append(center + off)
        for a, b, c in [
            (0, 2, 4), (2, 1, 4), (1, 3, 4), (3, 0, 4),
            (2, 0, 5), (1, 2, 5), (3, 1, 5), (0, 3, 5),
        ]:
            faces.append((base + a, base + b, base + c))

    for head, tail in joints:
        add_diamond(head)
        add_diamond(tail)
        verts.append(head)
        verts.append(tail)
        edges.append((len(verts) - 2, len(verts) - 1))

    mesh = bpy.data.meshes.new("m2m_skeleton")
    mesh.from_pydata(verts, edges, faces)
    mesh.update()
    obj = bpy.data.objects.new("m2m_skeleton", mesh)
    # Workbench colours by object (not shader nodes), so the bone colour rides on
    # obj.color with color_type='OBJECT' set in main().
    obj.color = (0.05, 0.9, 1.0, 1.0)
    bpy.context.collection.objects.link(obj)
    return obj


def paint_weight_colors(meshes):
    """Tints every mesh by per-vertex influence count into a colour attribute,
    and switches Workbench to render that attribute. Returns the count of
    unweighted vertices found (the number the bind report also reports)."""
    unweighted = 0
    for obj in meshes:
        mesh = obj.data
        attr = mesh.color_attributes.new(name="m2m_weights", type="BYTE_COLOR", domain="POINT")
        for i, vert in enumerate(mesh.vertices):
            influences = sum(1 for g in vert.groups if g.weight > 1e-4)
            if influences == 0:
                unweighted += 1
            attr.data[i].color = INFLUENCE_COLORS.get(min(influences, 4), INFLUENCE_COLORS[4])
        mesh.color_attributes.active_color = attr
        mesh.attributes.active_color = attr
    return unweighted


def main() -> int:
    args = sys.argv[sys.argv.index("--") + 1:]
    glb = args[0]
    out_dir = args[1]
    num_views = max(1, int(args[2]))
    mode = args[3] if len(args) > 3 and args[3] else "solid"
    frame = int(args[4]) if len(args) > 4 and args[4] not in ("", "-") else None
    if mode not in ("solid", "skeleton", "weights"):
        sys.stderr.write(f"unknown mode {mode!r}; use solid, skeleton or weights\n")
        return 2
    os.makedirs(out_dir, exist_ok=True)

    bpy.ops.wm.read_factory_settings(use_empty=True)
    try:
        bpy.ops.import_scene.gltf(filepath=glb)
    except Exception as error:  # noqa: BLE001 - report whatever Blender raised
        sys.stderr.write(f"import failed: {type(error).__name__}: {error}\n")
        return 1
    if frame is not None:
        bpy.context.scene.frame_set(frame)
        bpy.context.view_layer.update()

    meshes = [o for o in bpy.data.objects if o.type == "MESH"]
    armatures = [o for o in bpy.data.objects if o.type == "ARMATURE"]
    if not meshes:
        sys.stderr.write("no mesh imported\n")
        return 1

    lo, hi = _combined_bounds(meshes)
    center = (lo + hi) * 0.5
    radius = max((hi - lo).length * 0.5, 0.1)

    scene = bpy.context.scene
    scene.render.engine = "BLENDER_WORKBENCH"
    scene.render.resolution_x = 512
    scene.render.resolution_y = 512
    scene.render.film_transparent = False
    shading = scene.display.shading

    if mode == "skeleton":
        if not armatures:
            sys.stderr.write("skeleton mode needs an armature; none imported\n")
            return 1
        build_skeleton_geometry(armatures)
        # Workbench colours per object: grey mesh, cyan bones, X-rayed so the
        # bones read through the body.
        for obj in meshes:
            obj.color = (0.6, 0.6, 0.6, 1.0)
        shading.color_type = "OBJECT"
        shading.show_xray = True
        shading.xray_alpha = 0.22
    elif mode == "weights":
        found_unweighted = paint_weight_colors(meshes)
        shading.color_type = "VERTEX"
        shading.light = "FLAT"
        print(f"UNWEIGHTED {found_unweighted}")

    # An empty at the centre for the camera to track, and a key light.
    target = bpy.data.objects.new("target", None)
    bpy.context.collection.objects.link(target)
    target.location = center
    sun = bpy.data.lights.new("sun", type="SUN")
    sun.energy = 3.0
    sun_obj = bpy.data.objects.new("sun", sun)
    bpy.context.collection.objects.link(sun_obj)
    sun_obj.rotation_euler = (math.radians(50), 0, math.radians(30))

    camera_data = bpy.data.cameras.new("camera")
    camera = bpy.data.objects.new("camera", camera_data)
    bpy.context.collection.objects.link(camera)
    track = camera.constraints.new(type="TRACK_TO")
    track.target = target
    track.track_axis = "TRACK_NEGATIVE_Z"
    track.up_axis = "UP_Y"
    scene.camera = camera

    distance = radius * 3.2
    elevation = math.radians(15)
    for i in range(num_views):
        azimuth = 2.0 * math.pi * i / num_views
        camera.location = center + Vector((
            distance * math.cos(azimuth) * math.cos(elevation),
            distance * math.sin(elevation),
            distance * math.sin(azimuth) * math.cos(elevation),
        ))
        path = os.path.join(out_dir, f"view_{i}.png")
        scene.render.filepath = path
        bpy.ops.render.render(write_still=True)
        print("RENDERED " + path)
    return 0


if __name__ == "__main__":
    sys.exit(main())
