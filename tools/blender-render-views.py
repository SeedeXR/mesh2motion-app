"""Renders a model from several angles in headless Blender, for visual review.

Imports a glb, frames it, and renders `num_views` shots evenly spaced around it
(a turntable) into `out_dir` as `view_0.png` … The Workbench engine is used —
solid shading, no ray tracing — because the point is to see the mesh, the pose
and the deformation, fast, not to light a scene. An optional frame number renders
that frame of an imported clip, so a pose mid-animation can be inspected.

    blender -b --factory-startup --python tools/blender-render-views.py -- \
        <model.glb> <out_dir> <num_views> [frame]

Prints one `RENDERED <path>` line per image and exits non-zero on failure.
"""

import math
import os
import sys

import bpy
from mathutils import Vector


def main() -> int:
    args = sys.argv[sys.argv.index("--") + 1:]
    glb = args[0]
    out_dir = args[1]
    num_views = max(1, int(args[2]))
    frame = int(args[3]) if len(args) > 3 else None
    os.makedirs(out_dir, exist_ok=True)

    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=glb)
    if frame is not None:
        bpy.context.scene.frame_set(frame)
        bpy.context.view_layer.update()

    meshes = [o for o in bpy.data.objects if o.type == "MESH"]
    if not meshes:
        sys.stderr.write("no mesh imported\n")
        return 1

    # Combined world-space bounding box of every mesh.
    lo = Vector((float("inf"),) * 3)
    hi = Vector((float("-inf"),) * 3)
    for obj in meshes:
        for corner in obj.bound_box:
            world = obj.matrix_world @ Vector(corner)
            lo = Vector(min(a, b) for a, b in zip(lo, world))
            hi = Vector(max(a, b) for a, b in zip(hi, world))
    center = (lo + hi) * 0.5
    radius = max((hi - lo).length * 0.5, 0.1)

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
    bpy.context.scene.camera = camera

    scene = bpy.context.scene
    scene.render.engine = "BLENDER_WORKBENCH"
    scene.render.resolution_x = 512
    scene.render.resolution_y = 512
    scene.render.film_transparent = False

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
