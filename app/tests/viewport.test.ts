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
import { applyFraming, frameBounds, parseModel } from '../src/viewport/model'

function glb(path: string): ArrayBuffer {
  const file = readFileSync(path)
  return file.buffer.slice(file.byteOffset, file.byteOffset + file.byteLength) as ArrayBuffer
}

const RIGGED = 'legacy/static/animations/human-base-animations.glb'
const PLAIN = 'legacy/static/models/model-human.glb'

describe('parseModel', () => {
  test('a rigged model arrives with its skeleton and its clips', async () => {
    const model = await parseModel(glb(RIGGED))

    expect(model.bones).toBe(66)
    expect(model.skinnedMeshes).toBe(1)
    expect(model.clips).toHaveLength(87)
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
