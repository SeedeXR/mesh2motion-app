"""Renders every frame of an imported clip from one fixed 3/4 camera, headless.

The Rust side exports a glb with a clip baked on, hands it here, and encodes the
frames this writes into a video with ffmpeg (Blender in this build has no video
encoder, so frames go out as PNGs). Workbench solid shading — the point is to see
the motion, fast.

    blender -b --factory-startup --python tools/blender-render-animation.py -- \
        <model.glb> <out_dir> [max_frames]

`max_frames` (default 120) caps a very long clip so the encode stays quick.
Prints `FRAMES <n>` and `FPS <n>`, one `RENDERED <path>` per frame, exits
non-zero on failure.
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
    max_frames = int(args[2]) if len(args) > 2 and args[2] else 120
    os.makedirs(out_dir, exist_ok=True)

    bpy.ops.wm.read_factory_settings(use_empty=True)
    try:
        bpy.ops.import_scene.gltf(filepath=glb)
    except Exception as error:  # noqa: BLE001 - report whatever Blender raised
        sys.stderr.write(f"import failed: {type(error).__name__}: {error}\n")
        return 1

    meshes = [o for o in bpy.data.objects if o.type == "MESH"]
    if not meshes:
        sys.stderr.write("no mesh imported\n")
        return 1

    # Frame range from the imported clips; fall back to the scene's if none.
    scene = bpy.context.scene
    lo_f, hi_f = scene.frame_start, scene.frame_end
    ranges = [a.frame_range for a in bpy.data.actions if a.frame_range[1] > a.frame_range[0]]
    if ranges:
        lo_f = int(min(r[0] for r in ranges))
        hi_f = int(max(r[1] for r in ranges))
    total = max(1, hi_f - lo_f + 1)
    # Cap: sample evenly if the clip is longer than max_frames.
    step = max(1, math.ceil(total / max_frames))
    frames = list(range(lo_f, hi_f + 1, step))
    fps = int(round(scene.render.fps / max(1, scene.render.fps_base))) or 24
    print(f"FRAMES {len(frames)}")
    print(f"FPS {fps}")

    # Frame at the animation's mid frame so a raised limb is inside the view.
    scene.frame_set((lo_f + hi_f) // 2)
    bpy.context.view_layer.update()
    lo = Vector((float("inf"),) * 3)
    hi = Vector((float("-inf"),) * 3)
    for obj in meshes:
        for corner in obj.bound_box:
            world = obj.matrix_world @ Vector(corner)
            lo = Vector(min(a, b) for a, b in zip(lo, world))
            hi = Vector(max(a, b) for a, b in zip(hi, world))
    center = (lo + hi) * 0.5
    radius = max((hi - lo).length * 0.5, 0.1)

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
    # A fixed front-3/4 view for the whole clip so the motion, not the camera, moves.
    distance = radius * 3.2
    camera.location = center + Vector((distance * 0.85, radius * 0.5, distance * 0.55))

    scene.render.engine = "BLENDER_WORKBENCH"
    scene.render.resolution_x = 512
    scene.render.resolution_y = 512
    scene.render.film_transparent = False

    for i, frame in enumerate(frames):
        scene.frame_set(frame)
        bpy.context.view_layer.update()
        path = os.path.join(out_dir, f"frame_{i:04d}.png")
        scene.render.filepath = path
        bpy.ops.render.render(write_still=True)
        print("RENDERED " + path)
    return 0


if __name__ == "__main__":
    sys.exit(main())
