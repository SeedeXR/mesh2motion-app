// @vitest-environment node
/**
 * Legacy skinning solver baseline (todo P0-10).
 *
 * Captures what the current rigid nearest-bone solver produces for every
 * template, so the geodesic replacement (P1) can be compared against something
 * real rather than against an impression. Records timing, memory, and a weight
 * fingerprint per template.
 *
 * This must run BEFORE any P1 solver work — see memory/test.md §9.
 *
 *   npx vitest run bench/solver-baseline.ts --testTimeout 600000
 */

import { describe, it } from 'vitest'
import { resolve } from 'node:path'
import { writeFileSync, mkdirSync } from 'node:fs'
import type { Mesh, Object3D, BufferGeometry } from 'three'

import { loadGlbHeadless } from './glb-headless.js'
import SkinningAlgorithm from '../src/lib/solvers/SkinningAlgorithm.js'
import { SkeletonType } from '../src/lib/enums/SkeletonType.js'

const ROOT = resolve(__dirname, '..')
const RUNS = 3

/**
 * SkeletonType.Fish ships as rig-shark.glb — the enum and the asset differ.
 *
 * NOT LISTED: human-a-pose. Objective O8 needs an A-pose baseline, but this
 * harness applies the template rig to the mesh with no fitting step, and
 * rig-human.glb is a T-pose rig (hand_l sits at world x=0.75) while
 * human-a-pose.glb has its arms down (mesh x spans ±0.62 against the T-pose
 * mesh's ±0.97). Benchmarking that pair measures a mismatched rig against a
 * mesh, not A-pose rigging, and would read as a valid baseline while being
 * meaningless. Add it once headless skeleton fitting exists — see todo P3-P1.
 */
const TEMPLATES: ReadonlyArray<{
  type: SkeletonType, rig: string, model: string, modelPath?: string, label?: string
}> = [
  { type: SkeletonType.Human, rig: 'rig-human', model: 'model-human' },
  { type: SkeletonType.Fox, rig: 'rig-fox', model: 'model-fox' },
  { type: SkeletonType.Bird, rig: 'rig-bird', model: 'model-bird' },
  { type: SkeletonType.Horse, rig: 'rig-horse', model: 'model-horse' },
  { type: SkeletonType.Fish, rig: 'rig-shark', model: 'model-shark' },
  { type: SkeletonType.Dragon, rig: 'rig-dragon', model: 'model-dragon' },
  { type: SkeletonType.Kaiju, rig: 'rig-kaiju', model: 'model-kaiju' },
  { type: SkeletonType.Snake, rig: 'rig-snake', model: 'model-snake' },
  { type: SkeletonType.Spider, rig: 'rig-spider', model: 'model-spider' }
]

interface Baseline {
  template: string
  vertices: number
  bones: number
  medianMs: number
  minMs: number
  maxMs: number
  heapDeltaMb: number
  /** Meshes in the model; >1 means the solver runs once per mesh. */
  meshes: number
  /** Vertices whose weights do not sum to 1.0 — legacy invariant check. */
  unnormalised: number
  /** Vertices carrying exactly one non-zero influence: the rigid-assignment tell. */
  singleInfluence: number
  /** Vertices the solver left with no influence at all — a defect, not a tell. */
  zeroInfluence: number
  meanInfluences: number
  /** Stable hash of dominant bone per vertex, for detecting A/B drift. */
  dominantBoneHash: string
  error?: string
}

/**
 * Every mesh in the model, not just the first.
 *
 * model-shark ships as two meshes (1948 + 1578 verts); taking only the first
 * would silently benchmark 55% of the geometry and report it as the whole.
 */
function allGeometries(root: Object3D): BufferGeometry[] {
  const geoms: BufferGeometry[] = []
  root.traverse((o) => {
    const m = o as Mesh
    if (m.isMesh) geoms.push(m.geometry)
  })
  return geoms
}

function countBones(root: Object3D): number {
  let n = 0
  root.traverse((o) => { if (o.type === 'Bone') n++ })
  return n
}

/** FNV-1a over the dominant bone index per vertex. Order-stable, cheap. */
function hashDominantBones(indices: number[], weights: number[]): string {
  let h = 0x811c9dc5
  for (let v = 0; v * 4 < weights.length; v++) {
    let best = 0
    let bestW = -1
    for (let k = 0; k < 4; k++) {
      const w = weights[v * 4 + k] ?? 0
      if (w > bestW) { bestW = w; best = indices[v * 4 + k] ?? 0 }
    }
    h ^= best
    h = Math.imul(h, 0x01000193) >>> 0
  }
  return h.toString(16).padStart(8, '0')
}

function analyse(indices: number[], weights: number[]): Pick<Baseline,
  'unnormalised' | 'singleInfluence' | 'zeroInfluence' | 'meanInfluences' | 'dominantBoneHash'> {
  const vertexCount = weights.length / 4
  if (vertexCount === 0) {
    // An empty weight array is a solver failure, not a result. Report it as
    // such rather than letting 0/0 become NaN, which JSON.stringify writes as
    // null and which reads like a real measurement.
    return {
      unnormalised: -1,
      singleInfluence: -1,
      zeroInfluence: -1,
      meanInfluences: -1,
      dominantBoneHash: ''
    }
  }

  let unnormalised = 0
  let singleInfluence = 0
  let zeroInfluence = 0
  let totalInfluences = 0

  for (let v = 0; v < vertexCount; v++) {
    let sum = 0
    let nonZero = 0
    for (let k = 0; k < 4; k++) {
      const w = weights[v * 4 + k] ?? 0
      sum += w
      if (w > 1e-6) nonZero++
    }
    if (Math.abs(sum - 1) > 1e-4) unnormalised++
    // Exactly one, not "at most one": a vertex with zero influences is an
    // unweighted defect, and folding it in here would inflate the very
    // statistic the P1 comparison is keyed on.
    if (nonZero === 1) singleInfluence++
    if (nonZero === 0) zeroInfluence++
    totalInfluences += nonZero
  }

  return {
    unnormalised,
    singleInfluence,
    zeroInfluence,
    meanInfluences: Number((totalInfluences / vertexCount).toFixed(3)),
    dominantBoneHash: hashDominantBones(indices, weights)
  }
}

describe('legacy solver baseline', () => {
  it('captures weights, timing and memory for every template', async () => {
    const results: Baseline[] = []

    for (const t of TEMPLATES) {
      const label = `${t.label ?? t.type} (${t.model})`
      try {
        const rig = await loadGlbHeadless(resolve(ROOT, `../assets/rigs/${t.rig}.glb`))
        const model = await loadGlbHeadless(
          resolve(ROOT, t.modelPath ?? `static/models/${t.model}.glb`)
        )
        const geometries = allGeometries(model)
        if (geometries.length === 0) throw new Error('no mesh geometry found')

        const bones = countBones(rig)
        const vertices = geometries.reduce(
          (n, g) => n + (g.attributes['position']?.count ?? 0), 0
        )

        const timings: number[] = []
        let indices: number[] = []
        let weights: number[] = []
        let heapDelta = 0

        for (let run = 0; run < RUNS; run++) {
          // Needs --expose-gc (set in the npm script). Without a forced
          // collection the figure is cumulative process growth, not the cost of
          // this solve — which is how the first draft of this harness reported
          // a 924-vertex spider as using more memory than a 7399-vertex human.
          global.gc?.()
          const heapBefore = process.memoryUsage().heapUsed

          const runIndices: number[] = []
          const runWeights: number[] = []
          const start = performance.now()
          for (const geometry of geometries) {
            const algo = new SkinningAlgorithm(rig, t.type)
            algo.set_geometry(geometry)
            const out = algo.calculate_indexes_and_weights()
            runIndices.push(...(out[0] ?? []))
            runWeights.push(...(out[1] ?? []))
          }
          timings.push(performance.now() - start)

          heapDelta = Math.max(heapDelta, process.memoryUsage().heapUsed - heapBefore)
          indices = runIndices
          weights = runWeights
        }

        timings.sort((a, b) => a - b)
        results.push({
          template: t.label ?? t.type,
          vertices,
          bones,
          medianMs: Number((timings[Math.floor(timings.length / 2)] ?? 0).toFixed(1)),
          minMs: Number((timings[0] ?? 0).toFixed(1)),
          maxMs: Number((timings[timings.length - 1] ?? 0).toFixed(1)),
          heapDeltaMb: Number((heapDelta / 1024 / 1024).toFixed(1)),
          meshes: geometries.length,
          ...analyse(indices, weights)
        })
        console.log(`  ok   ${label}`)
      } catch (err) {
        results.push({
          template: t.label ?? t.type,
          vertices: 0, bones: 0, meshes: 0, medianMs: 0, minMs: 0, maxMs: 0, heapDeltaMb: 0,
          unnormalised: -1, singleInfluence: -1, zeroInfluence: -1, meanInfluences: -1,
          dominantBoneHash: '', error: err instanceof Error ? err.message : String(err)
        })
        console.log(`  FAIL ${label}: ${err instanceof Error ? err.message : String(err)}`)
      }
    }

    const failed = results.filter((r) => r.error !== undefined)
    if (failed.length > 0) {
      // Write nothing. Overwriting the committed baseline with error rows would
      // destroy the very artifact P1's A/B depends on, and an exit code of 0
      // would hide it.
      throw new Error(
        `${failed.length}/${results.length} templates failed; baseline NOT written:\n` +
        failed.map((f) => `  ${f.template}: ${f.error ?? ''}`).join('\n')
      )
    }

    const outDir = resolve(ROOT, '..', 'bench', 'baselines')
    mkdirSync(outDir, { recursive: true })
    writeFileSync(
      resolve(outDir, 'legacy-solver.json'),
      JSON.stringify({
        capturedAt: new Date().toISOString(),
        machine: 'Apple M4, 10 cores, 16 GB, macOS 26.6.2',
        runsPerTemplate: RUNS,
        solver: 'legacy rigid nearest-bone (WeightCalculator.ts:71)',
        results
      }, null, 2) + '\n'
    )

    console.table(results.map((r) => ({
      template: r.template,
      verts: r.vertices,
      bones: r.bones,
      'median ms': r.medianMs,
      meshes: r.meshes,
      'heap MB': r.heapDeltaMb,
      zero: r.zeroInfluence,
      'unnorm': r.unnormalised,
      '1-bone %': r.vertices > 0
        ? Math.round((r.singleInfluence / r.vertices) * 100)
        : 0,
      'mean infl': r.meanInfluences,
      error: r.error ?? ''
    })))
  }, 600_000)
})
