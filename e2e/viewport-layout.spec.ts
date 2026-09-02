import { test, expect } from '@playwright/test'

// Integration + regression test for the viewport inside the REAL app shell —
// the layout the plain viewport-harness (trivial CSS) can't exercise.
//
// The bug this guards: `.viewport` (a grid item) defaulted to min-height:auto,
// so the canvas backing store inflated its grid cell, which grew the measured
// size, which grew the buffer next resize — a runaway (~5636px observed) that
// pushed the model off-screen while only the clear colour showed. The runaway
// is WKWebView-specific and does NOT reproduce in headless Chromium, so the
// behavioural checks below are integration coverage; the true regression guard
// is the `min-height: 0` CSS invariant, asserted directly and engine-agnostic.
test('the viewport canvas stays bounded to its cell and shows the model', async ({ page }) => {
  const logs: string[] = []
  page.on('console', (m) => logs.push(`[${m.type()}] ${m.text()}`))
  page.on('pageerror', (e) => logs.push(`[pageerror] ${e.message}`))

  await page.route('**/model.glb', (route) =>
    route.fulfill({ path: 'assets/models/model-human.glb', contentType: 'model/gltf-binary' })
  )

  const H = 800
  await page.setViewportSize({ width: 1200, height: H })
  await page.goto('/e2e/viewport-layout-harness.html')
  await page.waitForFunction(() => (window as unknown as Record<string, unknown>).__ready === true, {
    timeout: 30_000
  })

  const error = await page.evaluate(() => (window as unknown as Record<string, unknown>).__error)
  const m = (await page.evaluate(() => (window as unknown as Record<string, unknown>).__measure)) as {
    bufW: number
    bufH: number
    rectW: number
    rectH: number
  }
  const dpr = await page.evaluate(() => window.devicePixelRatio)
  console.log('LAYOUT measure:', JSON.stringify(m), 'dpr:', dpr, '| logs:', logs.join(' | '))

  expect(error).toBeUndefined()

  // Regression guard for the fix itself: since the runaway does not reproduce in
  // Chromium, assert the CSS invariant that prevents it. If someone drops
  // `min-height: 0` from `.viewport`, this fails in any engine.
  const minHeight = await page.evaluate(
    () => getComputedStyle(document.querySelector('.viewport')!).minHeight
  )
  expect(minHeight).toBe('0px')

  // Bounded: the canvas can never be taller than the window it lives in. Before
  // the min-height:0 fix, the backing store ran to several times this while the
  // model was pushed off-screen.
  expect(m.rectH).toBeLessThanOrEqual(H)
  expect(m.bufH).toBeLessThanOrEqual(Math.ceil(H * dpr) + 4)
  // ...and actually filling its cell, not collapsed to nothing.
  expect(m.rectH).toBeGreaterThan(200)

  // Stable: re-sample after more frames — a runaway would keep climbing.
  await page.waitForTimeout(600)
  const again = (await page.evaluate(() =>
    (window as unknown as { __sample: () => { bufH: number; rectH: number } }).__sample()
  )) as { bufH: number; rectH: number }
  console.log('LAYOUT resample:', JSON.stringify(again))
  expect(again.bufH).toBe(m.bufH)

  // Visible: screenshot the composited canvas (a real display grab, so it works
  // despite preserveDrawingBuffer:false). A blank uniform viewport compresses to
  // a tiny PNG; a lit, textured model is an order of magnitude larger.
  const shot = await page.locator('.viewport canvas').screenshot({ path: 'e2e/layout-shot.png' })
  console.log('LAYOUT screenshot bytes:', shot.length)
  expect(shot.length).toBeGreaterThan(12_000)
})
