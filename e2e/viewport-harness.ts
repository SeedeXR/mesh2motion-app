// Renders the real viewport (scene.ts) with a glb served at /model.glb, so a
// Playwright test can screenshot exactly what the app draws after a load.
import { createViewport } from '../app/src/viewport/scene'

const viewport = createViewport()
const app = document.getElementById('app')!
viewport.canvas.style.width = '100%'
viewport.canvas.style.height = '100%'
viewport.canvas.style.display = 'block'
app.appendChild(viewport.canvas)

async function run(): Promise<void> {
  const res = await fetch('/model.glb')
  const bytes = await res.arrayBuffer()
  const contents = await viewport.show(bytes)
  ;(window as unknown as Record<string, unknown>).__contents = {
    bones: contents.bones,
    skinnedMeshes: contents.skinnedMeshes,
    min: contents.bounds.min.toArray(),
    max: contents.bounds.max.toArray()
  }
  ;(window as unknown as Record<string, unknown>).__ready = true
}

run().catch((e) => {
  ;(window as unknown as Record<string, unknown>).__error = String(e)
  ;(window as unknown as Record<string, unknown>).__ready = true
})
