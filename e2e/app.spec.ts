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
          case 'load_model': {
            // Reproduce the real bug: some IPC transports deliver raw bytes as a
            // plain number array (not an ArrayBuffer). The frontend's bulk()
            // normaliser must recover from this, or the model never draws.
            const buf = await (await fetch('/model.glb')).arrayBuffer()
            return Array.from(new Uint8Array(buf))
          }
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

  // The bug showed "Geometry NaN MB" and an empty viewport; the fix must give a
  // real size and a mesh on screen.
  const geometry = await page.getByText(/MB$/).innerText()
  console.log('geometry cell:', geometry)
  expect(geometry).not.toContain('NaN')
})
