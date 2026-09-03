/**
 * Turning a `.glb` from the bulk channel into something drawable, and the
 * arithmetic for looking at it.
 *
 * Kept apart from `scene.ts` because everything here is decidable without a
 * renderer, a canvas or a GPU — which is what makes it testable. `scene.ts` is
 * the part that needs a screen, and it is deliberately thin.
 */

import {
  type AnimationClip,
  Bone,
  Box3,
  Mesh,
  type Camera,
  Group,
  type Object3D,
  PerspectiveCamera,
  SkinnedMesh,
  Sphere,
  Vector3
} from 'three'
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js'

/** What a loaded model turned out to contain. */
export interface ModelContents {
  /** The scene graph to add to the viewport. */
  readonly root: Group
  /** Bones found anywhere under it. */
  readonly bones: number
  /** Meshes bound to a skeleton. */
  readonly skinnedMeshes: number
  /** Animation clip names. */
  readonly clips: readonly string[]
  /** World-space bounds, in metres. */
  readonly bounds: Box3
}

/**
 * Parses a `.glb` that arrived over the bulk channel.
 *
 * The payload is already bytes, so nothing is fetched and no URL is involved —
 * `parseAsync` takes the buffer directly.
 *
 * Models converted from FBX carry no normals, which glTF allows and which the
 * loader answers with flat shading. That is a real visual difference from a
 * file authored with normals, not a failure to load.
 */
export async function parseModel(data: ArrayBuffer): Promise<ModelContents> {
  const gltf = await new GLTFLoader().parseAsync(data, '')

  let bones = 0
  let skinnedMeshes = 0
  gltf.scene.traverse((object: Object3D) => {
    if (object instanceof Bone) bones++
    if (object instanceof SkinnedMesh) skinnedMeshes++
  })

  return {
    root: gltf.scene,
    bones,
    skinnedMeshes,
    clips: gltf.animations.map((clip) => clip.name),
    bounds: new Box3().setFromObject(gltf.scene)
  }
}

/** Where a camera should sit to see the whole of `bounds`. */
export interface Framing {
  readonly position: Vector3
  readonly target: Vector3
}

/**
 * Frames a bounding box for a perspective camera.
 *
 * Fits the box's bounding **sphere** rather than the box, so the framing does
 * not change as the model is orbited — fitting the box makes the camera lurch
 * every time a wider silhouette rotates into view.
 *
 * Both axes are fitted: a wide model in a tall viewport is limited by width,
 * and using the vertical fit alone crops its arms.
 */
export function frameBounds(
  bounds: Box3,
  fovDegrees: number,
  aspect: number,
  direction = new Vector3(0, 0.25, 1)
): Framing {
  const sphere = bounds.getBoundingSphere(new Sphere())
  // An empty or degenerate box yields a zero radius, which would put the
  // camera exactly on the target and leave nothing on screen.
  const radius = sphere.radius > 0 ? sphere.radius : 1

  const vertical = radius / Math.sin((fovDegrees * Math.PI) / 360)
  const horizontal = vertical / Math.max(aspect, 0.01)
  const distance = 1.15 * Math.max(vertical, horizontal)

  const offset = direction.clone().normalize().multiplyScalar(distance)
  return { position: sphere.center.clone().add(offset), target: sphere.center.clone() }
}

/** Points `camera` at a framing. */
export function applyFraming(camera: Camera, framing: Framing): void {
  camera.position.copy(framing.position)
  camera.lookAt(framing.target)
  if (camera instanceof PerspectiveCamera) {
    // Near and far follow the subject, so a 2 m character and a 200 m one both
    // get usable depth precision instead of one fixed guess. No floor on
    // `near`: depth precision depends on the near/far RATIO, which stays
    // constant here, and a floor would clamp exactly the small subjects it
    // looks like it protects — a 20 cm model wants a near of half a
    // millimetre. `frameBounds` guarantees a positive distance.
    const distance = camera.position.distanceTo(framing.target)
    camera.near = distance / 1000
    camera.far = distance * 100
    camera.updateProjectionMatrix()
  }
}

/** The frame a playback time falls on, at a given frame rate. */
export function frameOfTime(seconds: number, fps: number): number {
  return Math.round(seconds * fps)
}

/** The time at the start of a frame, at a given frame rate. */
export function timeOfFrame(frame: number, fps: number): number {
  return frame / fps
}

/** How many frames a clip of `duration` seconds spans at `fps` (at least one). */
export function totalFrames(duration: number, fps: number): number {
  return Math.max(1, Math.round(duration * fps))
}

/**
 * A joint's edit-handle radius, sized to how close its nearest neighbour joint
 * is: capped at `max`, floored at `max * 0.12` so it stays grabbable.
 *
 * Dense joints — a hand's fingers — get small, separable handles instead of
 * merging into one unpickable clump; limb joints, whose neighbours are far,
 * keep the full size. A joint with no neighbour (a lone skeleton) gets `max`.
 */
export function localHandleRadius(
  positions: readonly (readonly [number, number, number])[],
  index: number,
  max: number
): number {
  const p = positions[index]
  if (p === undefined) return max
  let nearest = Infinity
  positions.forEach((q, j) => {
    if (j === index) return
    const d = Math.hypot(p[0] - q[0], p[1] - q[1], p[2] - q[2])
    if (d < nearest) nearest = d
  })
  if (!Number.isFinite(nearest)) return max
  return Math.min(Math.max(nearest * 0.4, max * 0.12), max)
}

/** A canonical camera direction: the six orthographic-style views. */
export type ViewPreset = 'front' | 'back' | 'left' | 'right' | 'top' | 'bottom'

/** The unit direction from the target toward the camera for each preset. */
const PRESET_DIRECTIONS: Readonly<Record<ViewPreset, readonly [number, number, number]>> = {
  front: [0, 0, 1],
  back: [0, 0, -1],
  right: [1, 0, 0],
  left: [-1, 0, 0],
  top: [0, 1, 0],
  bottom: [0, -1, 0]
}

/**
 * Where the camera sits for a preset view: `distance` from `target` along the
 * preset's axis. Keeping the current distance means a preset snaps the angle
 * without also zooming, which is what "look from the front" should do.
 */
export function presetCameraPosition(
  preset: ViewPreset,
  target: Vector3,
  distance: number
): Vector3 {
  const [x, y, z] = PRESET_DIRECTIONS[preset]
  return new Vector3(x, y, z).multiplyScalar(distance).add(target)
}

/**
 * Blender-style octahedral "bone" geometry for a fitted skeleton — one
 * octahedron per bone-to-parent link, merged into a single indexed mesh.
 *
 * A template skeleton is not a glTF skeleton: it arrives as bone positions and
 * parent indices, with no scene graph and no `Bone` objects, so three's
 * `SkeletonHelper` has nothing to attach to. This builds the picture from the
 * plain data — but as solid octahedra, not lines, so the eye reads each bone's
 * direction and length (which a line throws away) the way Blender's default
 * armature display does.
 *
 * Each bone runs from its PARENT joint — the head, where the octahedron is
 * widest — to the joint itself — the tail, the far tip — so the taper points
 * down the chain. The ring sits at 10% of the bone length from the head with a
 * radius of `max(10% of length, minWidth)`, matching Blender while `minWidth`
 * keeps short bones from collapsing to a sliver.
 *
 * Root bones (no parent) contribute nothing — a bone with no parent has no head
 * to draw from — and a parent index outside the list, or a zero-length bone, is
 * skipped rather than read, so no `NaN` reaches the buffer and takes the overlay
 * off screen. The caller builds a `BufferGeometry` from `positions` + `indices`
 * and computes normals for the shading.
 */
export function skeletonOctahedra(
  positions: readonly (readonly [number, number, number])[],
  parents: readonly (number | null)[],
  minWidth: number
): { positions: Float32Array; indices: Uint32Array } {
  const verts: number[] = []
  const indices: number[] = []
  parents.forEach((parent, bone) => {
    if (parent === null) return
    const tail = positions[bone]
    const head = positions[parent]
    if (head === undefined || tail === undefined) return
    const ax = tail[0] - head[0]
    const ay = tail[1] - head[1]
    const az = tail[2] - head[2]
    const length = Math.hypot(ax, ay, az)
    if (length < 1e-6) return
    const dx = ax / length
    const dy = ay / length
    const dz = az / length
    // A reference axis not parallel to the bone, so the cross product below is
    // well-conditioned; swap to X only when the bone is almost vertical.
    const [rx, ry] = Math.abs(dy) < 0.99 ? [0, 1] : [1, 0]
    // u ⟂ bone (unit), v = bone × u (unit): the ring plane's two axes.
    let ux = dy * 0 - dz * ry
    let uy = dz * rx - dx * 0
    let uz = dx * ry - dy * rx
    const ul = Math.hypot(ux, uy, uz) || 1
    ux /= ul
    uy /= ul
    uz /= ul
    const vx = dy * uz - dz * uy
    const vy = dz * ux - dx * uz
    const vz = dx * uy - dy * ux

    const radius = Math.max(length * 0.1, minWidth)
    const cx = head[0] + dx * length * 0.1
    const cy = head[1] + dy * length * 0.1
    const cz = head[2] + dz * length * 0.1
    const base = verts.length / 3
    verts.push(
      head[0], head[1], head[2], // 0 head tip
      tail[0], tail[1], tail[2], // 1 tail tip
      cx + ux * radius, cy + uy * radius, cz + uz * radius, // 2 +u
      cx + vx * radius, cy + vy * radius, cz + vz * radius, // 3 +v
      cx - ux * radius, cy - uy * radius, cz - uz * radius, // 4 -u
      cx - vx * radius, cy - vy * radius, cz - vz * radius // 5 -v
    )
    const [h, t, r0, r1, r2, r3] = [base, base + 1, base + 2, base + 3, base + 4, base + 5]
    indices.push(
      h, r0, r1, h, r1, r2, h, r2, r3, h, r3, r0, // head fan
      t, r1, r0, t, r2, r1, t, r3, r2, t, r0, r3 // tail fan
    )
  })
  return { positions: new Float32Array(verts), indices: new Uint32Array(indices) }
}

/**
 * A copy of `positions` with joint `index` moved to `to`.
 *
 * The gizmo edits one joint at a time; the fitted skeleton's positions are
 * `readonly`, so binding gets a fresh array rather than a mutated one. An
 * out-of-range index returns the positions unchanged.
 */
export function withJointMoved(
  positions: readonly (readonly [number, number, number])[],
  index: number,
  to: readonly [number, number, number]
): Array<[number, number, number]> {
  return positions.map((p, i): [number, number, number] =>
    i === index ? [to[0], to[1], to[2]] : [p[0], p[1], p[2]]
  )
}

/** Whether any mesh under a parsed model carries per-vertex colours. */
export function hasVertexColors(root: Object3D): boolean {
  let found = false
  root.traverse((object) => {
    if (object instanceof Mesh && object.geometry.getAttribute('color') !== undefined) {
      found = true
    }
  })
  return found
}

/**
 * A rigged, animated `.glb` parsed for playback.
 *
 * Kept apart from [`parseModel`] because a preview is a different object with a
 * different lifetime: it carries `AnimationClip`s and is swapped in and out as
 * the user scrubs between clips, where the imported model persists.
 */
export interface AnimatedModel {
  readonly root: Group
  /** The clips the file carries, by the name the library gave them. */
  readonly clips: readonly AnimationClip[]
  readonly bounds: Box3
}

/**
 * Parses an animated `.glb` from `preview_animation`.
 *
 * Nothing here needs a GPU — three parses the clips into `AnimationClip`s on
 * the CPU — so the whole path from Rust bytes to a playable clip is testable in
 * Node.
 */
export async function parseAnimated(data: ArrayBuffer): Promise<AnimatedModel> {
  const gltf = await new GLTFLoader().parseAsync(data, '')
  return {
    root: gltf.scene,
    clips: gltf.animations,
    bounds: new Box3().setFromObject(gltf.scene)
  }
}

/**
 * Finds a clip by name, tolerating the prefixes importers add.
 *
 * Our exporter names the clip exactly (`Chest_Open`), but a name can arrive
 * decorated — `Armature|Chest_Open|Layer0` from an FBX round trip — so an exact
 * match is tried first and a contains-match second. Returns `undefined` rather
 * than guessing when nothing matches.
 */
export function findClip(
  clips: readonly AnimationClip[],
  name: string
): AnimationClip | undefined {
  return clips.find((clip) => clip.name === name) ?? clips.find((clip) => clip.name.includes(name))
}
