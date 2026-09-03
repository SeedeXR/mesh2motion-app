/**
 * Accessibility guards (design.md §10, "non-negotiable").
 *
 * These do not exercise a browser — they pin the requirements that are easy to
 * drop in a refactor and expensive to notice missing: a reduced-motion escape
 * hatch, a visible focus ring, a document landmark, and warnings that never lean
 * on colour alone. The shell markup and CSS are read as text, which is enough to
 * catch a regression without standing up a DOM.
 */

import { readFileSync } from 'node:fs'
import { describe, expect, test } from 'vitest'

const shellCss = readFileSync('app/src/ui/shell.css', 'utf8')
const mainTs = readFileSync('app/src/main.ts', 'utf8')

describe('reduced motion', () => {
  test('a prefers-reduced-motion block takes transitions to instant', () => {
    expect(shellCss).toMatch(/@media\s*\(prefers-reduced-motion:\s*reduce\)/)
    // The block must actually neutralise motion, not just exist.
    const block = shellCss.slice(shellCss.indexOf('prefers-reduced-motion'))
    expect(block).toMatch(/transition-duration:\s*0/)
    expect(block).toMatch(/animation-duration:\s*0/)
  })
})

describe('focus is always visible', () => {
  test('interactive controls have a focus ring', () => {
    expect(shellCss).toMatch(/\.step:focus-visible/)
    expect(shellCss).toMatch(/\.action:focus-visible/)
    expect(shellCss).toMatch(/outline:\s*2px solid var\(--focus-ring\)/)
  })

  test('nothing removes the outline without replacing it', () => {
    // `outline: none` on its own is the classic focus-killer.
    expect(shellCss).not.toMatch(/outline:\s*none\s*;/)
  })
})

describe('landmarks and labels', () => {
  test('the document has one heading landmark', () => {
    expect(mainTs).toMatch(/<h1[^>]*>Mesh2Motion<\/h1>/)
  })

  test('the step rail and inspector are labelled regions', () => {
    expect(mainTs).toMatch(/<nav class="rail" aria-label="Rigging steps">/)
    expect(mainTs).toMatch(/<aside class="inspector" aria-label="Properties">/)
    expect(mainTs).toMatch(/role="status"/)
  })

  test('decorative icons are hidden from assistive tech', () => {
    // Every Lucide `<i data-lucide>` is decorative — the text beside it carries
    // the meaning — so each must be aria-hidden.
    const icons = mainTs.match(/<i data-lucide=[^>]*>/g) ?? []
    expect(icons.length).toBeGreaterThan(0)
    for (const icon of icons) {
      expect(icon).toContain('aria-hidden="true"')
    }
  })
})

describe('never colour alone', () => {
  test('a warning pairs its colour with an icon and text', () => {
    expect(mainTs).toMatch(/class="warn"><i data-lucide="alert-triangle"/)
    expect(shellCss).toMatch(/\.warn\s*\{/)
  })

  test('hit targets meet the 32px minimum', () => {
    // --s-6 is 32px; both the step and action controls set it as a floor.
    const stepBlock = shellCss.slice(shellCss.indexOf('.step {'))
    expect(stepBlock).toMatch(/min-height:\s*var\(--s-6\)/)
    const actionBlock = shellCss.slice(shellCss.indexOf('.action {'))
    expect(actionBlock).toMatch(/min-height:\s*var\(--s-6\)/)
  })
})
