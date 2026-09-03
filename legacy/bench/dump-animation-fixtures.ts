// @vitest-environment node
import { describe, it } from 'vitest'
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import { resolve } from 'node:path'
import { FBXLoader } from '../src/lib/io/FBXLoader.js'

/**
 * Exports the AnimationClips the legacy builds from the reference rig, so the
 * Rust port can be diffed key-for-key.
 *
 * The rotation path is why this matters. A quaternion track is the product of
 * Euler-order composition, a PreRotation premultiply, and a sign-unroll pass —
 * every one of which produces a smooth, unit-length, entirely plausible track
 * when it is wrong. 43 of the 52 animated models carry a PreRotation, so this
 * is not a corner.
 *
 * Format: `fbx-anim.bin` is [u32 clipCount][u32 padding] then, per clip,
 *   [duration][trackCount] then per track [kind][keyCount][valueCount]
 *   [times…][values…]
 * all f64 little-endian, kind 0 = position, 1 = quaternion, 2 = scale. The
 * padding keeps the f64 body 8-byte aligned. Names go to `fbx-anim-names.txt`
 * as `clip <name>` / `track <name>` lines in the same order — strings do not
 * belong in a float array, and a failing assertion needs to say which bone.
 *
 *   cd legacy && npm run bench
 */
describe('dump animation fixtures', () => {
  it('writes the reference rig clips', () => {
    const src = resolve(__dirname, '..', 'static/test-files/retarget testing/mixamo-original-rig.fbx')
    const buf = readFileSync(src)
    const ab = buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength)
    const root = new FBXLoader().parse(ab, '') as unknown as {
      animations: Array<{
        name: string
        duration: number
        tracks: Array<{ name: string; times: ArrayLike<number>; values: ArrayLike<number> }>
      }>
    }
    const clips = root.animations

    const KIND: Record<string, number> = { position: 0, quaternion: 1, scale: 2 }
    const names: string[] = []
    const body: number[] = []

    for (const clip of clips) {
      names.push(`clip ${clip.name}`)
      body.push(clip.duration, clip.tracks.length)
      for (const t of clip.tracks) {
        names.push(`track ${t.name}`)
        const suffix = t.name.slice(t.name.indexOf('.') + 1)
        const kind = KIND[suffix]
        if (kind === undefined) throw new Error(`unhandled track kind: ${t.name}`)
        body.push(kind, t.times.length, t.values.length)
        for (let i = 0; i < t.times.length; i++) body.push(t.times[i])
        for (let i = 0; i < t.values.length; i++) body.push(t.values[i])
      }
    }

    const out = new ArrayBuffer(8 + body.length * 8)
    new DataView(out).setUint32(0, clips.length, true)
    new Float64Array(out, 8).set(body)

    const dir = resolve(__dirname, '..', '..', 'crates/m2m-io/tests/fixtures')
    mkdirSync(dir, { recursive: true })
    writeFileSync(resolve(dir, 'fbx-anim.bin'), Buffer.from(out))
    writeFileSync(resolve(dir, 'fbx-anim-names.txt'), names.join('\n') + '\n')
  })
})
