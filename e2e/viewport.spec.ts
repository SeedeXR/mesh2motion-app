import { test, expect } from '@playwright/test'

test('a loaded model is visible in the viewport', async ({ page }) => {
  const logs: string[] = []
  page.on('console', (m) => logs.push(`[${m.type()}] ${m.text()}`))
  page.on('pageerror', (e) => logs.push(`[pageerror] ${e.message}`))

  // Serve the exact bytes the app hands the frontend (import::load -> glb::write).
  await page.route('**/model.glb', (route) =>
    route.fulfill({ path: 'assets/models/model-human.glb', contentType: 'model/gltf-binary' })
  )

  await page.setViewportSize({ width: 900, height: 700 })
  await page.goto('/e2e/viewport-harness.html')
  await page.waitForFunction(() => (window as unknown as Record<string, unknown>).__ready === true, {
    timeout: 30_000
  })

  const error = await page.evaluate(() => (window as unknown as Record<string, unknown>).__error)
  const contents = await page.evaluate(() => (window as unknown as Record<string, unknown>).__contents)
  console.log('HARNESS error:', error, 'contents:', JSON.stringify(contents))

  await page.waitForTimeout(600) // let the standing render loop draw some frames

  expect(error).toBeUndefined()

  // Visibility from a Playwright screenshot (the composited display), not a
  // canvas readback: WebGLRenderer defaults to preserveDrawingBuffer:false, so
  // drawImage/readback after present sees an empty buffer. A blank uniform
  // viewport compresses to a tiny PNG; a lit, textured model is far larger.
  const shot = await page.locator('canvas').screenshot({ path: 'e2e/shot.png' })
  console.log('SCREENSHOT bytes:', shot.length, '| logs:', logs.join(' | '))
  expect(shot.length).toBeGreaterThan(12_000)
})
