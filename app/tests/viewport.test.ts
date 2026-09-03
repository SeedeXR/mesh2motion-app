/**
 * The viewport's decidable half: what a `.glb` from the bulk channel turns into,
 * and where the camera has to sit to see it.
 *
 * Pixels are not tested here — nothing in this environment can look at them.
 * What *is* tested is the chain that produces them: the payload the Rust side
 * writes, parsed by the very loader the viewport uses.
 */

import { readFileSync } from 'node:fs'
import { describe, expect, test } from 'vitest'
import { Box3, PerspectiveCamera, Vector3 } from 'three'
import {
  applyFraming,
  findClip,
  frameBounds,
  hasVertexColors,
  parseAnimated,
  parseModel,
  presetCameraPosition,
  skeletonSegments,
  withJointMoved
} from '../src/viewport/model'

function glb(path: string): ArrayBuffer {
  const file = readFileSync(path)
  return file.buffer.slice(file.byteOffset, file.byteOffset + file.byteLength) as ArrayBuffer
}

const RIGGED = 'assets/animations/human-base-animations.glb'
const PLAIN = 'assets/models/model-human.glb'

describe('parseModel', () => {
  test('a rigged model arrives with its skeleton and its clips', async () => {
    const model = await parseModel(glb(RIGGED))

    expect(model.bones).toBe(66)
    expect(model.skinnedMeshes).toBe(1)
    expect(model.clips).toHaveLength(94)
    expect(model.clips[0]).toBe('Chest_Open')
  })

  test('the model is in metres, the units the converter promises', async () => {
    // The Rust side scales FBX out of centimetres; if that ever regressed, the
    // camera framing would still "work" and the model would be 180 m tall.
    const { bounds } = await parseModel(glb(RIGGED))
    const height = bounds.max.y - bounds.min.y

    expect(height).toBeGreaterThan(1.2)
    expect(height).toBeLessThan(2.5)
  })

  test('a plain mesh has no skeleton to draw', async () => {
    const model = await parseModel(glb(PLAIN))

    expect(model.bones).toBe(0)
    expect(model.skinnedMeshes).toBe(0)
    expect(model.clips).toHaveLength(0)
  })
})

describe('frameBounds', () => {
  /** Half-angle the bounding sphere subtends from the camera, in degrees. */
  function subtended(bounds: Box3, fov: number, aspect: number): number {
    const { position, target } = frameBounds(bounds, fov, aspect)
    const radius = bounds.getSize(new Vector3()).length() / 2
    return (Math.asin(radius / position.distanceTo(target)) * 180) / Math.PI
  }

  test('the whole subject fits inside the vertical field of view', () => {
    const human = new Box3(new Vector3(-0.5, 0, -0.3), new Vector3(0.5, 1.8, 0.3))
    expect(subtended(human, 45, 16 / 9)).toBeLessThan(45 / 2)
  })

  test('a narrow viewport pulls the camera back further than a wide one', () => {
    // Fitting height alone crops a character's arms on a portrait window.
    const wide = new Box3(new Vector3(-2, 0, -0.2), new Vector3(2, 1.8, 0.2))
    const centre = wide.getCenter(new Vector3())

    const landscape = frameBounds(wide, 45, 16 / 9).position.distanceTo(centre)
    const portrait = frameBounds(wide, 45, 9 / 16).position.distanceTo(centre)

    expect(portrait).toBeGreaterThan(landscape)
  })

  test('a degenerate box still leaves the camera somewhere useful', () => {
    // An empty scene has a zero-radius sphere; dividing by it would put the
    // camera exactly on the target and render nothing, with no error.
    const point = new Box3(new Vector3(), new Vector3())
    const { position, target } = frameBounds(point, 45, 1)

    expect(position.distanceTo(target)).toBeGreaterThan(0.5)
  })

  test('a bigger subject is viewed from further away, in proportion', () => {
    const small = new Box3(new Vector3(-0.1, 0, -0.1), new Vector3(0.1, 0.2, 0.1))
    const large = new Box3(new Vector3(-10, 0, -10), new Vector3(10, 20, 10))
    const distance = (b: Box3): number =>
      frameBounds(b, 45, 1).position.distanceTo(b.getCenter(new Vector3()))

    expect(distance(large) / distance(small)).toBeCloseTo(100, 0)
  })
})

describe('applyFraming', () => {
  /** Frames `bounds` with a fresh camera and reports its depth range. */
  function depthRange(bounds: Box3): { near: number; far: number } {
    const camera = new PerspectiveCamera(45, 1, 0.01, 100)
    applyFraming(camera, frameBounds(bounds, 45, 1))
    return { near: camera.near, far: camera.far }
  }

  test('the depth range scales with the subject rather than staying fixed', () => {
    // One hard-coded near/far cannot serve a 20 cm model and a 20 m one: the
    // small one z-fights and the large one is clipped. A fixed pair passes any
    // test that only asks near < distance < far, so this asks them to MOVE.
    const small = depthRange(new Box3(new Vector3(-0.1, 0, -0.1), new Vector3(0.1, 0.2, 0.1)))
    const large = depthRange(new Box3(new Vector3(-10, 0, -10), new Vector3(10, 20, 10)))

    expect(large.near / small.near).toBeCloseTo(100, 0)
    expect(large.far / small.far).toBeCloseTo(100, 0)
  })

  test('the subject sits between the near and far planes', async () => {
    const camera = new PerspectiveCamera(45, 1, 0.01, 100)
    const { bounds } = await parseModel(glb(RIGGED))
    applyFraming(camera, frameBounds(bounds, 45, 1))

    const distance = camera.position.distanceTo(bounds.getCenter(new Vector3()))
    expect(camera.near).toBeGreaterThan(0)
    expect(camera.near).toBeLessThan(distance - bounds.getSize(new Vector3()).length() / 2)
    expect(camera.far).toBeGreaterThan(distance)
  })
})

describe('presetCameraPosition', () => {
  const target = new Vector3(1, 2, 3)

  test('each preset sits the given distance from the target', () => {
    for (const preset of ['front', 'back', 'left', 'right', 'top', 'bottom'] as const) {
      const p = presetCameraPosition(preset, target, 5)
      expect(p.distanceTo(target)).toBeCloseTo(5)
    }
  })

  test('front looks down +Z, top looks down +Y, right looks down +X', () => {
    expect(presetCameraPosition('front', target, 5).toArray()).toEqual([1, 2, 8])
    expect(presetCameraPosition('top', target, 5).toArray()).toEqual([1, 7, 3])
    expect(presetCameraPosition('right', target, 5).toArray()).toEqual([6, 2, 3])
  })

  test('opposite presets sit on opposite sides of the target', () => {
    const front = presetCameraPosition('front', target, 4)
    const back = presetCameraPosition('back', target, 4)
    expect(front.clone().add(back).multiplyScalar(0.5).toArray()).toEqual(target.toArray())
  })
})

describe('skeletonSegments', () => {
  const line: [number, number, number][] = [
    [0, 0, 0],
    [0, 1, 0],
    [0, 2, 0]
  ]

  test('one segment per bone that has a parent', () => {
    // Three bones in a chain is two lines, not three: the root has nothing to
    // draw back to.
    const points = skeletonSegments(line, [null, 0, 1])

    expect(points).toHaveLength(2 * 2 * 3)
    expect(Array.from(points)).toEqual([0, 1, 0, 0, 0, 0, 0, 2, 0, 0, 1, 0])
  })

  test('a skeleton of only roots draws nothing', () => {
    expect(skeletonSegments(line, [null, null, null])).toHaveLength(0)
  })

  test('a parent that does not exist is skipped, not read', () => {
    // `parents` comes from a file's node graph by way of IPC. Reading past the
    // end would put undefined into a Float32Array as NaN, and one NaN takes the
    // whole overlay off screen — with nothing on the console to say why.
    const points = skeletonSegments(line, [null, 0, 99])

    expect(points).toHaveLength(6)
    expect(Array.from(points).every(Number.isFinite)).toBe(true)
  })
})

describe('withJointMoved', () => {
  const line: Array<[number, number, number]> = [
    [0, 0, 0],
    [1, 0, 0],
    [2, 0, 0]
  ]

  test('moves only the named joint, leaving the rest', () => {
    const moved = withJointMoved(line, 1, [1, 5, 0])
    expect(moved[1]).toEqual([1, 5, 0])
    expect(moved[0]).toEqual([0, 0, 0])
    expect(moved[2]).toEqual([2, 0, 0])
  })

  test('returns a fresh array, never mutating the input', () => {
    const moved = withJointMoved(line, 0, [9, 9, 9])
    expect(moved).not.toBe(line)
    expect(line[0]).toEqual([0, 0, 0])
  })

  test('an out-of-range index changes nothing', () => {
    const moved = withJointMoved(line, 99, [9, 9, 9])
    expect(moved).toEqual(line)
  })
})

describe('parseAnimated and findClip', () => {
  const animated = glb('assets/animations/human-base-animations.glb')

  test('an animated glb comes back with its clips as AnimationClips', async () => {
    const model = await parseAnimated(animated)

    expect(model.clips.length).toBe(94)
    const chest = model.clips.find((c) => c.name === 'Chest_Open')
    expect(chest).toBeDefined()
    // The duration is what plays; assert it, not the track count, which three
    // resamples.
    expect(chest?.duration).toBeCloseTo(1.375, 2)
  })

  test('findClip matches an exact name', () => {
    const clips = [
      { name: 'Walk' },
      { name: 'Chest_Open' }
    ] as unknown as Parameters<typeof findClip>[0]
    expect(findClip(clips, 'Chest_Open')?.name).toBe('Chest_Open')
  })

  test('findClip tolerates an importer-decorated name', () => {
    // An FBX round trip names the action `Armature|Chest_Open|Layer0`; an exact
    // match would miss it and the preview would silently show nothing.
    const clips = [
      { name: 'Armature|Chest_Open|Layer0' }
    ] as unknown as Parameters<typeof findClip>[0]
    expect(findClip(clips, 'Chest_Open')?.name).toBe('Armature|Chest_Open|Layer0')
  })

  test('an exact match wins over a longer name that merely contains it', () => {
    // `Chest_Open_Slow` contains `Chest_Open`, and comes first. A
    // contains-first search would return the wrong clip; the exact name has to
    // win.
    const clips = [
      { name: 'Chest_Open_Slow' },
      { name: 'Chest_Open' }
    ] as unknown as Parameters<typeof findClip>[0]
    expect(findClip(clips, 'Chest_Open')?.name).toBe('Chest_Open')
  })

  test('findClip returns undefined rather than guessing', () => {
    const clips = [{ name: 'Walk' }] as unknown as Parameters<typeof findClip>[0]
    expect(findClip(clips, 'Chest_Open')).toBeUndefined()
  })
})

describe('hasVertexColors', () => {
  test('true when a mesh carries a color attribute, false otherwise', async () => {
    const { Group, Mesh, BufferGeometry, BufferAttribute, MeshBasicMaterial } = await import('three')

    const plain = new Group()
    plain.add(new Mesh(new BufferGeometry(), new MeshBasicMaterial()))
    expect(hasVertexColors(plain)).toBe(false)

    const painted = new Group()
    const geometry = new BufferGeometry()
    geometry.setAttribute('color', new BufferAttribute(new Float32Array([1, 0, 0, 1]), 4))
    painted.add(new Mesh(geometry, new MeshBasicMaterial()))
    expect(hasVertexColors(painted)).toBe(true)
  })
})
