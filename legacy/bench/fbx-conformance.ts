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
    const blender = '/Applications/Blender.app/Contents/MacOS/Blender'
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
    const run = (path: string): Record<string, unknown> => {
      const stdout = execFileSync(
        blender,
        ['--background', '--factory-startup', '--python',
         resolve(repo, 'tools/blender-fbx-import-check.py'), '--', path],
        { cwd: repo, encoding: 'utf8' }
      )
      const line = stdout.split('\n').find((l) => l.startsWith('BLENDER_JSON '))
      if (line === undefined) throw new Error(`no BLENDER_JSON in:\n${stdout}`)
      return JSON.parse(line.slice('BLENDER_JSON '.length)) as Record<string, unknown>
    }

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
})
