import { test, expect } from '@playwright/test'

// The clip chooser's moving preview draws the library character playing a clip.
// A blank preview leaves a tiny PNG; a rendered character is far larger.
test('the clip preview renders the library character', async ({ page }) => {
  const logs: string[] = []
  page.on('console', (m) => logs.push(`[${m.type()}] ${m.text()}`))
  page.on('pageerror', (e) => logs.push(`[pageerror] ${e.message}`))

  await page.route('**/anim.glb', (route) =>
    route.fulfill({
      path: 'assets/animations/human-base-animations.glb',
      contentType: 'model/gltf-binary'
    })
  )

  await page.goto('/e2e/clip-preview-harness.html')
  await page.waitForFunction(() => (window as unknown as Record<string, unknown>).__ready === true, {
    timeout: 30_000
  })
  const error = await page.evaluate(() => (window as unknown as Record<string, unknown>).__error)
  expect(error, logs.join(' | ')).toBeUndefined()

  await page.waitForTimeout(500) // let it frame and draw a few animated frames
  const shot = await page.locator('canvas').screenshot({ path: 'e2e/preview-shot.png' })
  console.log('PREVIEW screenshot bytes:', shot.length, '| logs:', logs.join(' | '))
  expect(shot.length).toBeGreaterThan(6000)
})
