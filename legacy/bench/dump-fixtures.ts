// @vitest-environment node
import { describe, it } from 'vitest'
import { resolve } from 'node:path'
import { writeFileSync, mkdirSync } from 'node:fs'
import { Vector3 } from 'three'
import type { Mesh, Object3D } from 'three'
import { loadGlbHeadless } from './glb-headless.js'

/**
 * Exports real mesh geometry as compact binary fixtures for the Rust tests.
 *
 * m2m-core has no GLB loader by design — it does no I/O (memory/architecture.md
 * §2) — and m2m-io does not exist until P2. Without this, the solver would be
 * tested only against synthetic tetrahedra, which is precisely the kind of
 * fixture that hides real-mesh defects like seam-split vertices.
 *
 * Format: [u32 vertexCount][u32 indexCount][f32 positions...][u32 indices...]
 * Little-endian, matching x86 and aarch64.
 *
 *   npm run bench   (runs as part of the bench config)
 */
describe('dump fixtures', () => {
  const dumpMesh = async (source: string, out: string): Promise<void> => {
    const root = await loadGlbHeadless(resolve(__dirname, '..', source))
    // ALL meshes, with world transforms baked, merged into one buffer.
    //
    // human-small.glb ships as 3 meshes; taking the first would export a third
    // of the model while looking like the whole thing — the same trap
    // model-shark set for the solver baseline. Exporters also routinely put a
    // scale on the root, and the Rust side derives its weld epsilon from the
    // bounding-box diagonal, which would then be computed on untransformed
    // geometry.
    root.updateMatrixWorld(true)
    const meshes: Mesh[] = []
    root.traverse((o) => { const m = o as Mesh; if (m.isMesh) meshes.push(m) })
    if (meshes.length === 0) throw new Error('no meshes found')

    const allPositions: number[] = []
    const allIndices: number[] = []
    for (const mesh of meshes) {
      const g = mesh.geometry.clone()
      g.applyMatrix4(mesh.matrixWorld)
      const pos = g.attributes['position']
      if (pos === undefined) continue

      // Indices are per-geometry; shift them by the vertices already emitted.
      const base = allPositions.length / 3
      for (let i = 0; i < pos.count; i++) {
        allPositions.push(pos.getX(i), pos.getY(i), pos.getZ(i))
      }
      if (g.index !== null) {
        for (const i of g.index.array) allIndices.push(base + i)
      } else {
        for (let i = 0; i < pos.count; i++) allIndices.push(base + i)
      }
    }

    const positions = new Float32Array(allPositions)
    const indices = new Uint32Array(allIndices)
    const pos = { count: positions.length / 3 }

    const header = new Uint32Array([pos.count, indices.length])
    const fileBytes = Buffer.concat([
      Buffer.from(header.buffer), Buffer.from(positions.buffer), Buffer.from(indices.buffer)
    ])
    const dir = resolve(__dirname, '..', '..', 'crates/m2m-core/tests/fixtures')
    mkdirSync(dir, { recursive: true })
    writeFileSync(resolve(dir, out), fileBytes)
    console.log(`DUMP ${out} meshes=${meshes.length} verts=${pos.count} tris=${indices.length / 3} bytes=${fileBytes.byteLength}`)
  }

  it('writes human-small.bin', async () => {
    await dumpMesh('static/test-files/human-small.glb', 'human-small.bin')
  }, 120000)

  it('writes human-template.bin', async () => {
    // The model rig-human.glb was authored for. human-small.glb is a
    // scaled-down test asset (rig is 2.19x its size), so pairing the rig with
    // it puts every bone outside the mesh.
    await dumpMesh('static/models/model-human.glb', 'human-template.bin')
  }, 120000)

  it('writes human-rig.bin', async () => {
    // Bone segments for the human template, so the Rust solver can be measured
    // against a real 66-bone skeleton rather than synthetic segments.
    //
    // Format: [u32 boneCount][f32 head.xyz, tail.xyz per bone], little-endian.
    // A bone's tail is its first child's head; leaf bones extrapolate along
    // the parent direction, matching how the legacy solver treats them.
    const rig = await loadGlbHeadless(
      resolve(__dirname, '..', 'static/rigs/rig-human.glb')
    )
    rig.updateMatrixWorld(true)

    const bones: Object3D[] = []
    rig.traverse((o) => { if (o.type === 'Bone') bones.push(o) })
    if (bones.length === 0) throw new Error('no bones found')

    const out: number[] = []
    for (const bone of bones) {
      const head = new Vector3()
      bone.getWorldPosition(head)

      const child = bone.children.find((c) => c.type === 'Bone')
      const tail = new Vector3()
      if (child !== undefined) {
        child.getWorldPosition(tail)
      } else {
        // Leaf: extend a short way along the direction from the parent.
        const parent = bone.parent
        if (parent !== null && parent.type === 'Bone') {
          const p = new Vector3()
          parent.getWorldPosition(p)
          tail.copy(head).add(head.clone().sub(p).multiplyScalar(0.5))
        } else {
          tail.copy(head).addScalar(1e-3)
        }
      }
      out.push(head.x, head.y, head.z, tail.x, tail.y, tail.z)
    }

    const header = new Uint32Array([bones.length])
    const body = new Float32Array(out)
    const buf = Buffer.concat([Buffer.from(header.buffer), Buffer.from(body.buffer)])
    const dir = resolve(__dirname, '..', '..', 'crates/m2m-core/tests/fixtures')
    mkdirSync(dir, { recursive: true })
    writeFileSync(resolve(dir, 'human-rig.bin'), buf)
    console.log(`DUMP bones=${bones.length} bytes=${buf.byteLength}`)
  }, 120000)
})
