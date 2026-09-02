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

  await page.waitForTimeout(600) // let the event-driven renderer draw a frame

  // Count pixels that differ from the #14161a background — a black/invisible
  // model leaves this near zero; a lit model paints thousands.
  const litPixels = await page.evaluate(() => {
    const webgl = document.querySelector('canvas') as HTMLCanvasElement
    const c = document.createElement('canvas')
    c.width = webgl.width
    c.height = webgl.height
    const ctx = c.getContext('2d')!
    ctx.drawImage(webgl, 0, 0)
    const { data } = ctx.getImageData(0, 0, c.width, c.height)
    let count = 0
    for (let i = 0; i < data.length; i += 4) {
      const dr = Math.abs(data[i] - 20)
      const dg = Math.abs(data[i + 1] - 22)
      const db = Math.abs(data[i + 2] - 26)
      if (dr + dg + db > 30) count++
    }
    return { count, total: c.width * c.height }
  })
  console.log('LIT PIXELS:', JSON.stringify(litPixels), '| logs:', logs.join(' | '))

  await page.locator('canvas').screenshot({ path: 'e2e/shot.png' })
  expect(error).toBeUndefined()
})
