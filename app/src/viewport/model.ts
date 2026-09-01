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

/**
 * Line-segment endpoints for a fitted skeleton, two points per bone-to-parent
 * link.
 *
 * A template skeleton is not a glTF skeleton: it arrives as bone positions and
 * parent indices, with no scene graph and no `Bone` objects, so three's
 * `SkeletonHelper` has nothing to attach to. This builds the same picture from
 * the plain data.
 *
 * Root bones contribute no segment — a bone with no parent has nothing to draw
 * a line to. A parent index outside the list is skipped rather than read: it
 * would otherwise put `undefined` into a `Float32Array` as `NaN` and take the
 * whole overlay off screen.
 */
export function skeletonSegments(
  positions: readonly (readonly [number, number, number])[],
  parents: readonly (number | null)[]
): Float32Array {
  const points: number[] = []
  parents.forEach((parent, bone) => {
    if (parent === null) return
    const from = positions[bone]
    const to = positions[parent]
    if (from === undefined || to === undefined) return
    points.push(from[0], from[1], from[2], to[0], to[1], to[2])
  })
  return new Float32Array(points)
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
