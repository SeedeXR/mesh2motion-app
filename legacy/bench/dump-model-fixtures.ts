// @vitest-environment node
import { describe, it } from 'vitest'
import { readFileSync, writeFileSync, mkdirSync } from 'node:fs'
import { resolve } from 'node:path'
import type { Object3D } from 'three'
import { FBXLoader } from '../src/lib/io/FBXLoader.js'

/**
 * Exports the Model hierarchy the legacy builds from the reference rig, so the
 * Rust port can be checked node-for-node instead of against hand-written
 * expectations about where a hip ought to be.
 *
 * The legacy graph is three.js-shaped and needs flattening first. It holds 132
 * nodes for 67 Model ids, because `buildSkeleton` creates a SEPARATE `Bone`
 * object per skeleton when a Model is shared between skins -- both of this
 * rig's skins cover the same 64 bones -- and nests the second inside the first
 * (`bone.add(subBone)`). Only the outer Bone is the one `parseModels` put in
 * its model map and gave transform properties to; the inner duplicate keeps an
 * identity local matrix and a world copied straight from the cluster's
 * `TransformLink`. Taking "the first node with this id" would export the wrong
 * matrix for every shared bone -- for the hips, one off by the entire 104.27cm
 * hip height.
 *
 * So the authoritative node is selected by the presence of
 * `userData.transformData`, which `getTransformData` sets on exactly the model
 * map's entry, and that uniqueness is asserted rather than assumed.
 *
 * Format: [u32 modelCount][u32 padding] then modelCount records of 34 f64:
 *   id  parentId(-1 for a root)  local[16]  world[16]
 * Names go to a sibling .txt, one per line, in the same order -- purely so a
 * failing Rust assertion can say which bone.
 *
 *   cd legacy && npm run bench
 */
describe('dump model fixtures', () => {
  it('writes the reference rig hierarchy', () => {
    const src = resolve(__dirname, '..', 'static/test-files/retarget testing/mixamo-original-rig.fbx')
    const buf = readFileSync(src)
    const ab = buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength)
    const root = new FBXLoader().parse(ab, '')

    // The loader hangs an `ID` on every node it creates from a Model; three.js
    // has no such field, so it has to be named here.
    type ModelNode = Object3D & { ID?: number }

    const byId = new Map<number, ModelNode>()
    let carriers = 0
    root.traverse((o: Object3D) => {
      const n = o as ModelNode
      if (n.ID === undefined) return
      // The duplicate Bones created for a second skeleton carry no transform
      // data; they are three.js's way of letting one bone belong to two
      // Skeletons, not a second node of the FBX hierarchy.
      if (n.userData?.transformData === undefined) return
      carriers++
      if (byId.has(n.ID)) {
        throw new Error(`Model ${n.ID} (${n.name}) has two nodes carrying transformData`)
      }
      byId.set(n.ID, n)
    })
    if (carriers !== byId.size) throw new Error('transformData carrier count disagrees with id count')

    const ids = [...byId.keys()].sort((a, b) => a - b)
    const F = 34
    const bufOut = new ArrayBuffer(8 + ids.length * F * 8)
    new DataView(bufOut).setUint32(0, ids.length, true)
    const out = new Float64Array(bufOut, 8)

    ids.forEach((id, i) => {
      const n = byId.get(id)
      let o = i * F
      out[o++] = id
      out[o++] = n.parent?.ID ?? -1
      for (let k = 0; k < 16; k++) out[o++] = n.matrix.elements[k]
      for (let k = 0; k < 16; k++) out[o++] = n.matrixWorld.elements[k]
    })

    const dir = resolve(__dirname, '..', '..', 'crates/m2m-io/tests/fixtures')
    mkdirSync(dir, { recursive: true })
    writeFileSync(resolve(dir, 'fbx-models.bin'), Buffer.from(bufOut))
    // three.js strips ':' from names for animation binding, so these read
    // `mixamorigHips` where the FBX says `mixamorig:Hips`.
    writeFileSync(
      resolve(dir, 'fbx-models-names.txt'),
      ids.map((id) => `${id} ${byId.get(id).name}`).join('\n') + '\n'
    )
  })
})
