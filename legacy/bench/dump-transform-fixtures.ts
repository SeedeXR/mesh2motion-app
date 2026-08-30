// @vitest-environment node
import { describe, it } from 'vitest'
import { resolve } from 'node:path'
import { writeFileSync, mkdirSync } from 'node:fs'
import { Matrix4 } from 'three'
import { generateTransform, getEulerOrder, type FBXTransformData } from '../src/lib/io/fbx/fbx-utils.js'

/**
 * Exports `generateTransform` inputs paired with the matrix three.js produces,
 * so the Rust port can be checked against the reference implementation itself
 * rather than against my reading of the FBX SDK docs.
 *
 * This matters more here than anywhere else in the port. The FBX transform
 * pipeline is a fixed composition of nine matrices with three inheritance
 * modes, and three.js's Euler order strings are the INVERSE of the FBX
 * extrinsic order integers -- `getEulerOrder(0)` returns 'ZYX'. A port that
 * gets the convention backwards produces rotations that look plausible,
 * animate smoothly, and are wrong. Only a differential test catches that.
 *
 * Format: [u32 caseCount][u32 padding] then caseCount records of 78 f64,
 * little-endian. The padding is not decorative: Float64Array requires an
 * 8-byte-aligned offset, so a bare 4-byte header cannot be followed by f64s.
 *   translation[3] preRotation[3] rotation[3] postRotation[3] scale[3]
 *   scalingOffset[3] scalingPivot[3] rotationOffset[3] rotationPivot[3]
 *   fbxRotationOrder inheritType hasParent
 *   parentMatrix[16] parentMatrixWorld[16] expected[16]
 *
 *   cd legacy && npm run bench
 */
describe('dump transform fixtures', () => {
  it('writes generateTransform cases for the Rust port', () => {
    // Deterministic, so a rerun does not churn the fixture. Numerical Recipes LCG.
    let seed = 0x2545f491
    const rand = (): number => {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0
      return seed / 0x100000000
    }
    // Values in the range FBX actually carries: degrees, centimetre translations.
    const angle = (): number => (rand() * 720) - 360
    const dist = (): number => (rand() * 200) - 100
    const scl = (): number => 0.1 + rand() * 3

    const v3 = (f: () => number): number[] => [f(), f(), f()]

    type Case = { d: FBXTransformData; fbxOrder?: number; parent?: { local: Matrix4; world: Matrix4 } }
    const cases: Case[] = []

    // 1. Every field at its default -- must come back as the identity.
    cases.push({ d: {} })

    // 2. One field at a time, so a mis-ordered composition cannot be masked
    //    by another term.
    cases.push({ d: { translation: [1, 2, 3] } })
    cases.push({ d: { scale: [2, 3, 4] } })
    cases.push({ d: { preRotation: [10, 20, 30] } })
    cases.push({ d: { postRotation: [10, 20, 30] } })
    cases.push({ d: { rotationOffset: [5, 6, 7] } })
    cases.push({ d: { rotationPivot: [5, 6, 7] } })
    cases.push({ d: { scalingOffset: [5, 6, 7] } })
    cases.push({ d: { scalingPivot: [5, 6, 7] } })

    // 3. Rotation under every Euler order. The corpus only ever uses order 0,
    //    so without these five the mapping is entirely unverified.
    for (let order = 0; order < 6; order++) {
      cases.push({ d: { rotation: [30, 45, 60], eulerOrder: getEulerOrder(order) }, fbxOrder: order })
    }
    // And order 6 (Spherical XYZ), which the legacy warns about and folds to 0.
    cases.push({ d: { rotation: [30, 45, 60], eulerOrder: getEulerOrder(6) }, fbxOrder: 6 })

    // 4. All three rotations together -- pre/post use the DEFAULT order even
    //    when the node declares another, which is easy to get wrong.
    for (let order = 0; order < 6; order++) {
      cases.push({
        d: {
          preRotation: [11, 22, 33],
          rotation: [44, 55, 66],
          postRotation: [77, 88, 99],
          eulerOrder: getEulerOrder(order)
        },
        fbxOrder: order
      })
    }

    // 5. Each inheritance mode, with a parent that has rotation AND non-uniform
    //    scale -- the modes differ only in how parent scale propagates, so a
    //    uniformly-scaled parent would make all three agree.
    const parentLocal = new Matrix4().makeRotationY(0.4).scale({ x: 2, y: 3, z: 4 } as never)
    const parentWorld = new Matrix4().makeRotationX(0.3).multiply(parentLocal)
    for (const inheritType of [0, 1, 2]) {
      cases.push({
        d: {
          translation: [1, 2, 3],
          rotation: [10, 20, 30],
          scale: [1.5, 2.5, 3.5],
          inheritType,
          eulerOrder: getEulerOrder(0)
        },
        fbxOrder: 0,
        parent: { local: parentLocal, world: parentWorld }
      })
    }

    // 6. Randomised full-field cases, including pivots and offsets, which no
    //    file in the corpus carries -- this is their only coverage.
    for (let i = 0; i < 24; i++) {
      const withParent = i % 2 === 0
      const pl = new Matrix4().makeRotationFromEuler(
        { x: rand(), y: rand(), z: rand(), order: 'XYZ' } as never
      ).scale({ x: scl(), y: scl(), z: scl() } as never)
      pl.setPosition({ x: dist(), y: dist(), z: dist() } as never)
      const pw = new Matrix4().makeRotationZ(rand()).multiply(pl)
      cases.push({
        d: {
          translation: v3(dist),
          preRotation: v3(angle),
          rotation: v3(angle),
          postRotation: v3(angle),
          scale: v3(scl),
          scalingOffset: v3(dist),
          scalingPivot: v3(dist),
          rotationOffset: v3(dist),
          rotationPivot: v3(dist),
          inheritType: i % 3,
          eulerOrder: getEulerOrder(i % 6)
        },
        fbxOrder: i % 6,
        parent: withParent ? { local: pl, world: pw } : undefined
      })
    }

    const F = 78
    const buf = new ArrayBuffer(8 + cases.length * F * 8)
    new DataView(buf).setUint32(0, cases.length, true)
    const out = new Float64Array(buf, 8)

    cases.forEach((c, i) => {
      const d: FBXTransformData = { ...c.d }
      if (c.parent) {
        d.parentMatrix = c.parent.local
        d.parentMatrixWorld = c.parent.world
      }
      // generateTransform INVERTS lParentGX in place (`lTransform.premultiply(
      // lParentGX.invert())`), and lParentGX is a copy, so the caller's matrix
      // survives -- but pass clones anyway so a future change cannot corrupt
      // later cases through the shared parentLocal/parentWorld above.
      if (d.parentMatrix) d.parentMatrix = d.parentMatrix.clone()
      if (d.parentMatrixWorld) d.parentMatrixWorld = d.parentMatrixWorld.clone()

      const expected = generateTransform(d)

      let o = i * F
      const put3 = (v?: number[], dflt = 0): void => {
        out[o++] = v ? v[0] : dflt
        out[o++] = v ? v[1] : dflt
        out[o++] = v ? v[2] : dflt
      }
      put3(c.d.translation)
      put3(c.d.preRotation)
      put3(c.d.rotation)
      put3(c.d.postRotation)
      put3(c.d.scale, 1)
      put3(c.d.scalingOffset)
      put3(c.d.scalingPivot)
      put3(c.d.rotationOffset)
      put3(c.d.rotationPivot)
      // The FBX RotationOrder INTEGER, not three.js's string index: the Rust
      // side has to run its own mapping to get back to this matrix, so a
      // reversed mapping fails here instead of passing silently.
      out[o++] = c.fbxOrder ?? 0
      out[o++] = c.d.inheritType ?? 0
      out[o++] = c.parent ? 1 : 0
      // three.js stores column-major in `elements`; write it through unchanged
      // and let the Rust side read it with from_cols_array.
      const m = (x?: Matrix4): void => {
        const e = x ? x.elements : new Matrix4().elements
        for (let k = 0; k < 16; k++) out[o++] = e[k]
      }
      m(c.parent?.local)
      m(c.parent?.world)
      m(expected)
    })

    const dir = resolve(__dirname, '..', '..', 'crates/m2m-io/tests/fixtures')
    mkdirSync(dir, { recursive: true })
    writeFileSync(resolve(dir, 'fbx-transform.bin'), Buffer.from(buf))
    console.log(`transform fixtures: ${cases.length} cases`)
  })
})
