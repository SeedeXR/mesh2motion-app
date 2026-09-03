/**
 * Creature-aware guidance wiring (design.md §7).
 *
 * The content itself lives in the Rust template manifests and is checked there
 * (`every_template_has_guidance`). This pins the frontend half: the tip is read
 * from the chosen template's `.guidance`, not hardcoded in the UI — design.md is
 * explicit that this copy lives with the template, not in a component.
 */

import { readFileSync } from 'node:fs'
import { describe, expect, test } from 'vitest'

const mainTs = readFileSync('app/src/main.ts', 'utf8')
const humanManifest = readFileSync('crates/m2m-rig/templates/human.json', 'utf8')

describe('creature guidance', () => {
  test('the tip is surfaced from the chosen template', () => {
    expect(mainTs).toMatch(/function creatureGuidance\(\)/)
    // It reads the template's own guidance field, keyed to the chosen name.
    expect(mainTs).toMatch(/\.name === chosen\)\?\.guidance/)
    // And it is placed in the skeleton step.
    expect(mainTs).toMatch(/\$\{creatureGuidance\(\)\}/)
  })

  test('the guidance copy is not hardcoded in the UI', () => {
    // A phrase that lives in the human manifest must NOT appear in main.ts — if
    // it did, the content would have leaked out of the template into the UI.
    const manifest = JSON.parse(humanManifest) as { guidance: string }
    const phrase = manifest.guidance.slice(0, 24)
    expect(phrase.length).toBeGreaterThan(10)
    expect(mainTs).not.toContain(phrase)
  })
})
