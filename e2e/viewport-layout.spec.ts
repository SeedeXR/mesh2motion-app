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

// Integration: the camera controls (preset views, zoom) actually change what is
// drawn. Driven through the public viewport API and verified by the composited
// image differing — no access to internal camera state.
test('preset views and zoom change the rendered image', async ({ page }) => {
  await page.route('**/model.glb', (route) =>
    route.fulfill({ path: 'assets/models/model-human.glb', contentType: 'model/gltf-binary' })
  )
  await page.setViewportSize({ width: 1200, height: 800 })
  await page.goto('/e2e/viewport-layout-harness.html')
  await page.waitForFunction(() => (window as unknown as Record<string, unknown>).__ready === true, {
    timeout: 30_000
  })
  const canvas = page.locator('.viewport canvas')

  const drive = async (fn: string, arg?: unknown): Promise<Buffer> => {
    await page.evaluate(
      ([f, a]) => {
        const vp = (window as unknown as { __viewport: Record<string, (x?: unknown) => void> })
          .__viewport
        vp[f as string](a)
      },
      [fn, arg] as const
    )
    await page.waitForTimeout(250)
    return await canvas.screenshot()
  }

  // A default (front-ish) framing, then straight down: a person seen from the
  // top is a very different image, so the bytes must differ.
  const front = await drive('reframe')
  const top = await drive('setView', 'top')
  expect(Buffer.compare(front, top)).not.toBe(0)

  // Zooming in from the front also changes the image.
  await drive('setView', 'front')
  const before = await canvas.screenshot()
  const zoomed = await drive('zoom', 0.5)
  expect(Buffer.compare(before, zoomed)).not.toBe(0)
})

// Blender-style navigation: the middle button (and Option+left on a
// middle-button-less Mac mouse) orbits; plain left is reserved for selection
// and must NOT orbit.
test('Blender navigation: middle-drag and Alt+left orbit, plain left does not', async ({ page }) => {
  await page.route('**/model.glb', (route) =>
    route.fulfill({ path: 'assets/models/model-human.glb', contentType: 'model/gltf-binary' })
  )
  await page.setViewportSize({ width: 1200, height: 800 })
  await page.goto('/e2e/viewport-layout-harness.html')
  await page.waitForFunction(() => (window as unknown as Record<string, unknown>).__ready === true, {
    timeout: 30_000
  })
  const canvas = page.locator('.viewport canvas')
  const box = await canvas.boundingBox()
  if (box === null) throw new Error('no canvas box')
  const cx = box.x + box.width / 2
  const cy = box.y + box.height / 2

  const reframe = async (): Promise<Buffer> => {
    await page.evaluate(() => (window as unknown as { __viewport: { reframe: () => void } }).__viewport.reframe())
    await page.waitForTimeout(400)
    return await canvas.screenshot()
  }
  const drag = async (button: 'left' | 'middle', alt: boolean): Promise<Buffer> => {
    if (alt) await page.keyboard.down('Alt')
    await page.mouse.move(cx, cy)
    await page.mouse.down({ button })
    await page.mouse.move(cx + 140, cy + 30, { steps: 8 })
    await page.mouse.up({ button })
    if (alt) await page.keyboard.up('Alt')
    await page.waitForTimeout(400)
    return await canvas.screenshot()
  }

  // Plain left drag: reserved for selection, must leave the camera untouched.
  const base1 = await reframe()
  expect(Buffer.compare(base1, await drag('left', false))).toBe(0)

  // Middle drag: orbits.
  const base2 = await reframe()
  expect(Buffer.compare(base2, await drag('middle', false))).not.toBe(0)

  // Option+left drag: emulates the middle button, orbits.
  const base3 = await reframe()
  expect(Buffer.compare(base3, await drag('left', true))).not.toBe(0)
})

// A wheel gesture (the trackpad two-finger swipe, or a mouse wheel) moves the
// camera. The handler intercepts every wheel itself so a trackpad swipe orbits
// Blender-style instead of zooming; either way the rendered image must change.
test('wheel gestures move the camera', async ({ page }) => {
  await page.route('**/model.glb', (route) =>
    route.fulfill({ path: 'assets/models/model-human.glb', contentType: 'model/gltf-binary' })
  )
  await page.setViewportSize({ width: 1200, height: 800 })
  await page.goto('/e2e/viewport-layout-harness.html')
  await page.waitForFunction(() => (window as unknown as Record<string, unknown>).__ready === true, {
    timeout: 30_000
  })
  const canvas = page.locator('.viewport canvas')
  const box = await canvas.boundingBox()
  if (box === null) throw new Error('no canvas box')
  await page.evaluate(() => (window as unknown as { __viewport: { reframe: () => void } }).__viewport.reframe())
  await page.waitForTimeout(400)
  const before = await canvas.screenshot()

  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2)
  await page.mouse.wheel(0, 80)
  await page.waitForTimeout(400)
  const after = await canvas.screenshot()
  expect(Buffer.compare(before, after)).not.toBe(0)
})
