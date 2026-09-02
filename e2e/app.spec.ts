import { test, expect } from '@playwright/test'

// The real app (index.html -> main.ts) with the Tauri backend mocked. load_model
// returns the ORIGINAL glb bytes (what read_as_glb now sends for a glb), so this
// proves the real UI shows the imported model, textured.
const IMPORTED = {
  name: 'rhino.glb',
  path: '/virtual/rhino.glb',
  import: {
    format: 'Glb',
    meshes: 1,
    bones: Array.from({ length: 35 }, (_, i) => `bone_${i}`),
    skinned_meshes: 1,
    clips: [],
    over_influence_limit: 0
  }
}

test('the real app shows the imported model, textured', async ({ page }) => {
  const logs: string[] = []
  page.on('pageerror', (e) => logs.push(`[pageerror] ${e.message}`))

  await page.route('**/model.glb', (r) =>
    r.fulfill({ path: 'assets/characters/rhino.glb', contentType: 'model/gltf-binary' })
  )

  await page.addInitScript((imported) => {
    const w = window as unknown as Record<string, unknown>
    w.__TAURI_INTERNALS__ = {
      transformCallback: (cb: unknown) => cb,
      invoke: async (cmd: string) => {
        switch (cmd) {
          case 'build_info':
            return { version: '0.1.0', target: 'e2e' }
          case 'report_startup':
            return null
          case 'import_model':
            return imported
          case 'load_model':
            return await (await fetch('/model.glb')).arrayBuffer()
          case 'skeleton_templates':
            return []
          default:
            return null
        }
      }
    }
  }, IMPORTED)

  await page.setViewportSize({ width: 1200, height: 800 })
  await page.goto('/')
  await page.locator('#import').click()
  await page.waitForSelector('.viewport canvas', { timeout: 15_000 })
  await page.waitForTimeout(1200) // let the model parse + a frame draw

  console.log('page errors:', logs.join(' | ') || 'none')
  await page.screenshot({ path: 'e2e/app-import.png' })
  await expect(page.locator('.viewport canvas')).toBeVisible()
})
