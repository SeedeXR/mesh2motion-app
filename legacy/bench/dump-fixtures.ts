// @vitest-environment node
import { describe, it } from 'vitest'
import { resolve } from 'node:path'
import { writeFileSync, mkdirSync } from 'node:fs'
import type { Mesh } from 'three'
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
  it('writes human-small.bin', async () => {
    const root = await loadGlbHeadless(
      resolve(__dirname, '..', 'static/test-files/human-small.glb')
    )
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
    const out = Buffer.concat([
      Buffer.from(header.buffer), Buffer.from(positions.buffer), Buffer.from(indices.buffer)
    ])
    const dir = resolve(__dirname, '..', '..', 'crates/m2m-core/tests/fixtures')
    mkdirSync(dir, { recursive: true })
    writeFileSync(resolve(dir, 'human-small.bin'), out)
    console.log(`DUMP meshes=${meshes.length} verts=${pos.count} tris=${indices.length / 3} bytes=${out.byteLength}`)
  }, 120000)
})
