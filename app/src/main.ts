import './ui/tokens.css'
import './ui/shell.css'

import { createIcons, Bone, Upload, Link, Play, Download, Move3d } from 'lucide'
import { STEPS, StepId, type StepDef } from './state/steps'
import {
  buildInfo,
  importModel,
  isDesktop,
  loadModel,
  reportStartup,
  type ImportedFile
} from './ipc'
import { detectBackend } from './viewport/backend'
import { createViewport, type Viewport } from './viewport/scene'

/** Index of the step the user is currently on. */
// ponytail: nothing advances activeStep yet — each step gains its own completion
// gate in P3-6. Backwards navigation works; forwards is deliberately absent
// rather than fake.
let activeStep = 0

/** The model the user imported, or `null` before they have. */
let loaded: ImportedFile | null = null

/**
 * Size of the model's geometry payload, once it has crossed the bulk channel.
 *
 * Fetched but not yet drawn — the viewport arrives in P3-7. Reporting the size
 * is what proves the binary channel works end to end, and it is the number that
 * matters: FBX geometry is unshared per corner, so a 10.5k-vertex mesh crosses
 * as 62.5k vertices until welding lands.
 */
let geometryBytes: number | null = null

/**
 * The Three.js viewport, created on first use.
 *
 * Held across renders rather than rebuilt with the DOM: `render()` replaces the
 * shell's markup, and a new canvas each time would mean a new WebGL context
 * each time — browsers cap those and start dropping the oldest.
 */
let viewport: Viewport | null = null

function ensureViewport(): Viewport {
  viewport ??= createViewport()
  return viewport
}

/**
 * Escapes text bound for `innerHTML`.
 *
 * A filename and an error message both carry text this app did not write — a
 * model saved as `<img onerror=...>.glb` would otherwise run as markup.
 */
function escape(text: string): string {
  const node = document.createElement('span')
  node.textContent = text
  return node.innerHTML
}

/**
 * What the inspector shows for the current step.
 *
 * The import step is the only one with anything to say yet, and what it says is
 * objective O9: a file that arrives with a skeleton keeps it. The legacy app
 * warned that it was about to drop your rig; this reports that it kept it.
 */
function renderInspector(step: StepDef): string {
  if (step.id !== StepId.LoadModel) {
    return '<p style="color:var(--fg-2)">Properties for this step appear here.</p>'
  }

  const button = '<button id="import" class="action">Import model\u2026</button>'
  if (loaded === null) {
    return `${button}
      <p style="color:var(--fg-2)">GLB or FBX. An existing rig is kept, not replaced.</p>`
  }

  const model = loaded.import
  const rigged = model.bones.length > 0
  const truncated =
    model.over_influence_limit > 0
      ? `<p style="color:var(--warn)">${model.over_influence_limit} of them carry more
           than four bone influences; only the strongest four are kept.</p>`
      : ''

  return `${button}
    <p style="color:var(--fg-1)">${escape(loaded.name)} \u00b7 ${model.format === 'Fbx' ? 'FBX' : 'glTF'}</p>
    <dl class="facts">
      <dt>Meshes</dt><dd>${model.meshes}${model.skinned_meshes > 0 ? ` (${model.skinned_meshes} skinned)` : ''}</dd>
      <dt>Bones</dt><dd>${rigged ? model.bones.length : 'none'}</dd>
      <dt>Clips</dt><dd>${model.clips.length}</dd>
      <dt>Geometry</dt><dd>${geometryBytes === null ? '—' : `${(geometryBytes / 1_000_000).toFixed(1)} MB`}</dd>
    </dl>
    ${
      rigged
        ? `<p style="color:var(--fg-1)">This model is already rigged. Its skeleton, weights and
             ${model.clips.length === 1 ? 'clip are' : 'clips are'} kept \u2014 re-rigging is
             yours to choose, not the default.</p>`
        : '<p style="color:var(--fg-2)">No skeleton found. Choose a template in the next step.</p>'
    }
    ${truncated}`
}

/** Runs the import step: pick a file, report what came back. */
async function runImport(button: HTMLButtonElement): Promise<void> {
  if (!isDesktop()) {
    button.textContent = 'Importing needs the desktop app'
    return
  }
  button.disabled = true
  button.textContent = 'Reading\u2026'
  try {
    const picked = await importModel()
    // A cancelled picker leaves the previous import alone rather than clearing it.
    if (picked !== null) {
      loaded = picked
      const geometry = await loadModel(picked.path)
      geometryBytes = geometry.byteLength
      // Rendered before drawing, so the canvas is in the DOM and has a size to
      // frame the model against.
      render()
      await ensureViewport().show(geometry)
      return
    }
    render()
  } catch (err) {
    button.disabled = false
    button.textContent = 'Import model\u2026'
    const message = err instanceof Error ? err.message : String(err)
    button.insertAdjacentHTML('afterend', `<p style="color:var(--err)">${escape(message)}</p>`)
  }
}

function renderRail(): string {
  return STEPS.map((step, i) => {
    const current = i === activeStep
    // Steps ahead of the current one are unreachable until this one completes.
    const locked = i > activeStep
    return `
      <button class="step" data-step="${i}"
              ${current ? 'aria-current="step"' : ''}
              ${locked ? 'disabled aria-disabled="true"' : ''}
              title="${locked ? 'Complete the previous step first' : step.goal}">
        <span class="step-num">${i + 1}</span>
        <span>${step.label}</span>
      </button>`
  }).join('')
}

function renderGuidance(step: StepDef): string {
  return `
    <i data-lucide="${step.icon}" width="18" height="18"></i>
    <div>
      <strong>${step.label}</strong>
      <p>${step.goal} <span style="color:var(--fg-2)">${step.success}</span></p>
    </div>`
}

function render(): void {
  const step = STEPS[activeStep]
  if (step === undefined) throw new Error(`no step at index ${activeStep}`)

  const app = document.querySelector<HTMLDivElement>('#app')
  if (app === null) throw new Error('#app mount point missing from index.html')

  app.innerHTML = `
    <div class="shell">
      <div class="titlebar"></div>

      <nav class="rail" aria-label="Rigging steps">
        <h2>Steps</h2>
        ${renderRail()}
      </nav>

      <main class="viewport">
        <div class="viewport-empty">
          <i data-lucide="bone" width="40" height="40" stroke-width="1"></i>
          <div>
            <div style="color:var(--fg-1);font-size:var(--fs-lg)">${loaded === null ? 'No model loaded' : escape(loaded.name)}</div>
            <div>${loaded === null ? 'Import a mesh to begin' : 'Viewport rendering arrives with the model preview step'}</div>
          </div>
        </div>
        <div class="guidance">${renderGuidance(step)}</div>
      </main>

      <aside class="inspector" aria-label="Properties">
        <h2>${step.label}</h2>
        ${renderInspector(step)}
      </aside>

      <div class="status" role="status">
        <span id="env">—</span>
        <span class="spacer"></span>
        <span>0 verts</span>
        <span>— ms</span>
      </div>
    </div>`

  // The canvas is moved into the freshly built shell rather than recreated.
  if (loaded !== null) {
    const stage = app.querySelector<HTMLElement>('.viewport')
    stage?.querySelector('.viewport-empty')?.remove()
    stage?.prepend(ensureViewport().canvas)
  }

  createIcons({ icons: { Bone, Upload, Link, Play, Download, Move3d } })

  const importButton = app.querySelector<HTMLButtonElement>('#import')
  if (importButton !== null) {
    importButton.addEventListener('click', () => void runImport(importButton))
  }

  app.querySelectorAll<HTMLButtonElement>('.step').forEach((btn) => {
    btn.addEventListener('click', () => {
      const next = Number(btn.dataset['step'])
      if (Number.isInteger(next)) {
        activeStep = next
        render()
      }
    })
  })

  void showEnvironment()
}

/** Reports what we are running inside, proving the IPC round-trip works. */
async function showEnvironment(): Promise<void> {
  const el = document.querySelector<HTMLSpanElement>('#env')
  if (el === null) return

  const backend = await detectBackend()

  // document.fonts.check() is only meaningful once loading has settled.
  await document.fonts.ready
  const fontLoaded = document.fonts.check('400 13px "Asta Sans"')

  if (!isDesktop()) {
    el.textContent = `browser · ${backend} · no native core`
    return
  }
  try {
    const info = await buildInfo()
    await reportStartup(`render ${backend}, font ${fontLoaded ? 'ok' : 'MISSING'}`)
    el.textContent = `v${info.version} · ${info.target} · ${backend} · native core ready`
  } catch (err) {
    el.textContent = 'native core unavailable'
    console.error('ipc failed', err)
  }
}

render()
