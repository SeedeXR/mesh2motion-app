// Mounts the real viewport (scene.ts) inside the real app shell (.viewport grid
// cell, styled by shell.css), loads a glb from /model.glb, and exposes what a
// regression test needs: whether the canvas backing store stays bounded to its
// grid cell, and whether the model actually rasterised.
//
// The plain viewport-harness renders the same scene under trivial CSS and so
// cannot see a layout-driven bug. This one can: the `.viewport` grid item used
// to default to `min-height:auto`, letting the canvas backing store inflate the
// cell, which grew the measured size, which grew the buffer on the next resize —
// a runaway that pushed the model off-screen while only the clear colour showed.
import { createViewport } from '../app/src/viewport/scene'

const win = window as unknown as Record<string, unknown>

const viewport = createViewport()
const stage = document.querySelector<HTMLElement>('.viewport')!
stage.prepend(viewport.canvas)

// Backing-store size of the one canvas, plus its on-screen rect. A canvas
// readback is deliberately NOT used to check visibility here: WebGLRenderer
// defaults to preserveDrawingBuffer:false, so drawImage after present reads an
// empty buffer. The spec proves the model is visible with a Playwright
// screenshot (the composited display) instead.
function measure(): { bufW: number; bufH: number; rectW: number; rectH: number } {
  const c = viewport.canvas
  const r = c.getBoundingClientRect()
  return { bufW: c.width, bufH: c.height, rectW: Math.round(r.width), rectH: Math.round(r.height) }
}

async function run(): Promise<void> {
  const res = await fetch('/model.glb')
  await viewport.show(await res.arrayBuffer())
  // Let the standing loop draw enough frames that any resize feedback loop would
  // have run away by now, then sample.
  await new Promise((r) => setTimeout(r, 800))
  win.__measure = measure()
  win.__ready = true
}

run().catch((e) => {
  win.__error = String(e)
  win.__ready = true
})

// Exposed so the test can re-sample after its own wait and prove the buffer is
// stable, not merely small at one instant.
win.__sample = measure
