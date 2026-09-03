// @vitest-environment node
import { describe, it, expect } from 'vitest'
import { execFileSync } from 'node:child_process'
import { existsSync, readFileSync, mkdtempSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { FBXLoader } from '../src/lib/io/FBXLoader.js'

/**
 * Checks the Rust encoder's output against a DIFFERENT reader.
 *
 * `crates/m2m-io/tests/fbx_encode.rs` round-trips through our own reader, which
 * proves the writer and the reader agree but not that the file is valid FBX.
 * Four conformance details survive that test because our reader does not check
 * them: the null record ending a child list (our `end_offset` is
 * authoritative), the top-level null record (never read — the footer heuristic
 * stops the loop first), an uncompressed array's declared byte length
 * (ignored), and the footer's 16-byte padding (only the last 16 bytes are
 * checked). three.js's loader checks enough of them to tell.
 *
 * Shells out to cargo rather than reading a committed fixture, so it can never
 * pass against a stale file.
 *
 *   cd legacy && npm run bench
 */
describe('FBX encoder conformance', () => {
  const repo = resolve(__dirname, '..', '..')
  const source = resolve(repo, 'legacy/static/test-files/retarget testing/mixamo-original-rig.fbx')

  const blenderBinary = '/Applications/Blender.app/Contents/MacOS/Blender'

  /**
   * Imports a file in headless Blender and returns its report.
   *
   * Reads the JSON from a file rather than stdout: Blender writes its own
   * progress there without always terminating the line, so scraping stdout
   * once produced `SyntaxError: Unexpected non-whitespace character after
   * JSON` instead of the assertion under test — a gate that fails for the
   * wrong reason is worse than no gate.
   */
  const runBlender = (path: string): Record<string, unknown> => {
    const report = join(mkdtempSync(join(tmpdir(), 'm2m-blender-')), 'report.json')
    execFileSync(
      blenderBinary,
      ['--background', '--factory-startup', '--python',
       resolve(repo, 'tools/blender-fbx-import-check.py'), '--', path, report],
      { cwd: repo, stdio: 'pipe' }
    )
    return JSON.parse(readFileSync(report, 'utf8')) as Record<string, unknown>
  }

  const summarise = (path: string): { bones: number, meshes: number, clips: Array<{ name: string, tracks: number, duration: number }> } => {
    const bytes = readFileSync(path)
    const buffer = bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength)
    const root = new FBXLoader().parse(buffer, '') as unknown as {
      traverse: (f: (n: { isBone?: boolean, type: string }) => void) => void
      animations: Array<{ name: string, duration: number, tracks: unknown[] }>
    }
    let bones = 0
    let meshes = 0
    root.traverse((n) => {
      if (n.isBone === true) bones++
      if (n.type === 'SkinnedMesh') meshes++
    })
    return {
      bones,
      meshes,
      clips: root.animations.map((c) => ({
        name: c.name,
        tracks: c.tracks.length,
        duration: Number(c.duration.toFixed(3))
      }))
    }
  }

  it('output of the Rust encoder loads identically to its input', () => {
    const out = join(mkdtempSync(join(tmpdir(), 'm2m-conformance-')), 'roundtrip.fbx')
    execFileSync(
      'cargo',
      ['run', '-p', 'm2m-io', '--release', '--quiet', '--example', 'encode_roundtrip', '--', source, out],
      { cwd: repo, stdio: 'pipe' }
    )

    const before = summarise(source)
    const after = summarise(out)

    // Not "it parsed" — the same scene, by value. A file three.js accepts but
    // reads as empty would satisfy a parse-succeeded assertion.
    expect(after).toEqual(before)

    // And the input is substantial, so the comparison is not vacuous.
    expect(before.bones).toBe(129)
    expect(before.meshes).toBe(2)
    expect(before.clips).toEqual([
      { name: 'mixamo.com', tracks: 53, duration: 4.9 },
      { name: 'Take 001', tracks: 0, duration: 0 }
    ])

    // three.js and our reader share a design, so agreeing with it proves less
    // than it looks: measured, four conformance details survive BOTH. Blender's
    // importer is the only independent reader here, and it catches two of them
    // — plus it is what caught the object-name truncation that made the first
    // encoder output refuse to open at all.
    const blender = blenderBinary
    if (!existsSync(blender)) {
      // Loud, because what is being skipped is the only part of this test that
      // can catch a conformance error: three.js agreeing proves little, since
      // it shares our reader's design. Measured — four details survive BOTH.
      console.warn(
        'WARNING: Blender not found at ' + blender + '.\n' +
        '  The three.js half of this gate still ran, but it is NOT sufficient on\n' +
        '  its own: it shares our reader\'s design, and four conformance details\n' +
        '  are known to survive it. Install Blender to run the real check.'
      )
      return
    }
    const run = runBlender

    const blenderBefore = run(source)
    const blenderAfter = run(out)
    expect(blenderBefore.imported).toBe(true)
    expect(blenderAfter.imported).toBe(true)
    // Everything but the file name, which differs by construction.
    delete blenderBefore.file
    delete blenderAfter.file
    expect(blenderAfter).toEqual(blenderBefore)
    expect(blenderBefore.bones).toBe(65)
  })

  it('geometry built from scratch imports as the mesh it describes', () => {
    // The round trip above re-encodes a document our READER produced, so it
    // inherits whatever that file already got right. This builds one from bare
    // positions and triangles, where every count, connection, name and
    // polygon-index sign is ours.
    const blender = blenderBinary
    if (!existsSync(blender)) {
      console.warn('WARNING: Blender not found; the builder has no independent check at all here')
      return
    }
    const out = join(mkdtempSync(join(tmpdir(), 'm2m-build-')), 'cube.fbx')
    execFileSync(
      'cargo',
      ['run', '-p', 'm2m-io', '--release', '--quiet', '--example', 'build_cube', '--', out],
      { cwd: repo, stdio: 'pipe' }
    )

    const report = runBlender(out)

    // A unit cube: 8 shared corners, 12 triangles, nothing loose. Asserting the
    // shape rather than "it imported" — a polygon-index sign error yields a
    // file that imports with one giant face or none.
    expect(report.imported).toBe(true)
    expect(report.meshes).toBe(1)
    expect(report.mesh_vertices).toEqual([8])
    expect(report.mesh_polygons).toEqual([12])
    expect(report.polygon_sizes).toEqual([3])
    expect(report.loose_vertices).toBe(0)
  })

  it('an existing rig survives a round trip through our own types (O9)', () => {
    // The acceptance test for O9: importing an already-rigged model must keep
    // its skeleton, bone names, hierarchy and skin weights. Unlike the round
    // trip above, this rebuilds the document from the SEMANTIC layers —
    // model::parse_all, skin::parse_all — so anything they drop shows up here.
    const blender = blenderBinary
    if (!existsSync(blender)) {
      console.warn('WARNING: Blender not found; O9 has no check at all here')
      return
    }
    const out = join(mkdtempSync(join(tmpdir(), 'm2m-rig-')), 'rebuilt.fbx')
    execFileSync(
      'cargo',
      ['run', '-p', 'm2m-io', '--release', '--quiet', '--example', 'rebuild_rig', '--', source, out],
      { cwd: repo, stdio: 'pipe' }
    )

    const run = runBlender

    const before = run(source)
    const after = run(out)
    expect(after.imported).toBe(true)

    // The skeleton, named bone for named bone — not just a count, because a
    // rig with the right number of wrongly-named bones retargets onto nothing.
    expect(after.armatures).toEqual(before.armatures)
    expect(after.bones).toEqual(before.bones)
    expect(after.bone_names).toEqual(before.bone_names)
    // The HIERARCHY, not just the names. Measured: parenting every bone to the
    // root passes a names-and-counts comparison, and a flattened skeleton is
    // not the same rig — nothing retargets onto it correctly.
    expect(after.bone_parents).toEqual(before.bone_parents)
    expect(after.root_bones).toEqual(before.root_bones)
    // WHERE the bones are, not just how they are named and connected.
    // Measured: dropping PreRotation — which 440 of 522 models in the corpus
    // carry — left every other field here identical.
    expect(after.bone_rest).toEqual(before.bone_rest)

    // The mesh, and the weights, which are what "the rig survived" means.
    expect(after.meshes).toEqual(before.meshes)
    expect(after.mesh_vertices).toEqual(before.mesh_vertices)
    expect(after.mesh_polygons).toEqual(before.mesh_polygons)
    expect(after.vertex_groups).toEqual(before.vertex_groups)
    expect(after.weighted_vertices).toEqual(before.weighted_vertices)
    expect(after.weight_total).toEqual(before.weight_total)
    expect(after.influences_per_vertex).toEqual(before.influences_per_vertex)

    // Measured on the reference rig, so a fixture swap that quietly shrinks it
    // cannot make the comparisons above pass on nothing.
    expect(before.bones).toBe(65)
    expect(before.vertex_groups).toBe(52)
    expect(before.weighted_vertices).toBe(24746)
    expect(before.root_bones).toEqual(['mixamorig:Hips'])
    // Quads. Rebuilding from the triangulated form gives [20840, 28272], and
    // an artist notices a quad mesh coming back as triangles.
    expect(before.polygon_sizes).toEqual([3, 4])

    // The animation, including its name, its curve and key counts, and its
    // frame RANGE. The range is what caught a missing TimeMode: the same keys
    // read at 25fps instead of 30 gave 1-123.5 rather than 1-148, so the clip
    // played 20% slow with every other number here identical.
    expect(after.actions).toEqual(before.actions)
    expect(after.action_detail).toEqual(before.action_detail)
    expect(after.animated_paths).toEqual(before.animated_paths)
    expect(before.actions).toEqual(['Armature|mixamo.com|Layer0'])
    expect(before.action_detail).toEqual([
      'Armature|mixamo.com|Layer0:curves=520,keys=76960,range=1.00-148.00'
    ])
  })
})
