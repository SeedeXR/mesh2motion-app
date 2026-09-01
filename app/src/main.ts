import './ui/tokens.css'
import './ui/shell.css'

import { createIcons, Bone, Upload, Link, Play, Download, Move3d } from 'lucide'
import { STEPS, StepId, type StepDef } from './state/steps'
import {
  animationClips,
  bindWeights,
  buildInfo,
  exportModel,
  fitSkeleton,
  previewAnimation,
  importModel,
  isDesktop,
  loadModel,
  reportStartup,
  skeletonTemplates,
  type BindReport,
  type ClipSummary,
  type FittedSkeleton,
  type ImportedFile,
  type SkeletonTemplate
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
/** The furthest step unlocked so far. Steps are gated on real progress. */
let furthestStep = 0

/** The creature templates, once fetched. */
let templates: SkeletonTemplate[] | null = null

/** The template the user picked, and where its skeleton landed. */
let chosen: string | null = null
let fitted: FittedSkeleton | null = null
/** Set while a fit is running — voxelising takes a moment. */
let fitting = false

/** What binding the mesh to the skeleton produced. */
let bound: BindReport | null = null
let binding = false

/** The file the rigged model was last written to. */
let exported: string | null = null
let exporting = false

/** The chosen creature's clips, once fetched, and the one selected. */
let clips: ClipSummary[] | null = null
let clip: string | null = null
/** True while a clip is playing in the viewport. */
let playing = false

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
  if (step.id === StepId.LoadSkeleton) return renderSkeletonStep()
  if (step.id === StepId.EditSkeleton) return renderEditStep()
  if (step.id === StepId.BindWeights) return renderBindStep()
  if (step.id === StepId.Animate) return renderAnimateStep()
  if (step.id === StepId.Export) return renderExportStep()
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

/** The Choose Skeleton step: pick a creature, place its rig on the mesh. */
function renderSkeletonStep(): string {
  if (loaded === null) {
    return '<p style="color:var(--fg-2)">Import a model first — a skeleton is fitted to a mesh, so there has to be one.</p>'
  }
  if (templates === null) {
    return '<p style="color:var(--fg-2)">Loading templates\u2026</p>'
  }

  const list = templates
    .map((template) => {
      const current = template.name === chosen
      return `
        <button class="action template" data-template="${escape(template.name)}"
                ${template.available ? '' : 'disabled'}
                ${current ? 'aria-current="true"' : ''}>
          <span>${escape(template.name)}</span>
          <span style="color:var(--fg-2)">${template.bones} bones</span>
        </button>`
    })
    .join('')

  const outcome =
    fitted === null
      ? '<p style="color:var(--fg-2)">Pick the creature closest to your mesh.</p>'
      : `<dl class="facts">
           <dt>Placed</dt><dd>${fitted.bones.length} bones</dd>
           <dt>Scale</dt><dd>${fitted.scale.toFixed(3)}\u00d7</dd>
         </dl>
         <p style="color:var(--fg-2)">The skeleton is drawn over the mesh. Fitting it by hand comes next.</p>`

  return `${list}${fitting ? '<p style="color:var(--fg-2)">Fitting\u2026</p>' : outcome}`
}

/** The Fit Skeleton step, which so far only reports what the automatic fit did. */
function renderEditStep(): string {
  if (fitted === null) {
    return '<p style="color:var(--fg-2)">Choose a skeleton first.</p>'
  }
  return `<dl class="facts">
      <dt>Bones</dt><dd>${fitted.bones.length}</dd>
      <dt>Scale</dt><dd>${fitted.scale.toFixed(3)}\u00d7</dd>
    </dl>
    <p style="color:var(--fg-2)">Moving bones by hand is not built yet, so the automatic placement is used as it stands. Check it in the viewport before binding.</p>`
}

/** The Bind Weights step: solve which bones deform which vertices. */
function renderBindStep(): string {
  if (fitted === null || loaded === null) {
    return '<p style="color:var(--fg-2)">Fit a skeleton first — weights are solved against one.</p>'
  }

  const button = `<button id="bind" class="action" ${binding ? 'disabled' : ''}>${
    binding ? 'Solving\u2026' : bound === null ? 'Bind weights' : 'Bind again'
  }</button>`
  const report = bound
  if (report === null) return button

  const [one, two, three, four] = report.influence_histogram
  const share = (n: number): string => `${((n / Math.max(report.vertices, 1)) * 100).toFixed(1)}%`
  const problems: string[] = []
  if (report.unweighted_vertices > 0) {
    problems.push(
      `<p style="color:var(--err)">${report.unweighted_vertices} vertices got no weight at all and will detach when the rig moves.</p>`
    )
  }
  if (report.fallback_vertices > 0) {
    problems.push(
      `<p style="color:var(--warn)">${report.fallback_vertices} vertices are on a disconnected island; their nearest bone was guessed in a straight line.</p>`
    )
  }

  return `${button}
    <dl class="facts">
      <dt>Vertices</dt><dd>${report.vertices}</dd>
      <dt>Bones used</dt><dd>${report.weighted_bones} <span style="color:var(--fg-2)">(${report.excluded_bones} root and leaf)</span></dd>
      <dt>1 influence</dt><dd>${one} <span style="color:var(--fg-2)">${share(one)}</span></dd>
      <dt>2</dt><dd>${two} <span style="color:var(--fg-2)">${share(two)}</span></dd>
      <dt>3</dt><dd>${three} <span style="color:var(--fg-2)">${share(three)}</span></dd>
      <dt>4</dt><dd>${four} <span style="color:var(--fg-2)">${share(four)}</span></dd>
    </dl>
    ${problems.join('') || '<p style="color:var(--ok)">Every vertex is attached.</p>'}`
}

/** Solves the weights for the fitted skeleton. */
async function runBind(): Promise<void> {
  if (loaded === null || fitted === null || binding) return
  binding = true
  render()
  try {
    bound = await bindWeights(loaded.path, fitted, 2.0)
    exported = null
    // Bound weights are what an export carries, so this unlocks the last step.
    // Animate sits in between and has nothing to complete yet, so it does not
    // gate the one after it.
    furthestStep = Math.max(furthestStep, 5)
  } finally {
    binding = false
    render()
  }
}

/** The Animate step: pick a clip to retarget onto the rig. */
function renderAnimateStep(): string {
  if (fitted === null || chosen === null) {
    return '<p style="color:var(--fg-2)">Choose and fit a skeleton first — a clip is retargeted onto one.</p>'
  }
  if (clips === null) {
    return '<p style="color:var(--fg-2)">Loading clips\u2026</p>'
  }
  if (clips.length === 0) {
    return `<p style="color:var(--fg-2)">No animation library ships for ${escape(chosen)} yet.</p>`
  }

  const list = clips
    .map(
      (c) => `
        <button class="action template" data-clip="${escape(c.name)}"
                ${c.name === clip ? 'aria-current="true"' : ''}>
          <span>${escape(c.name)}</span>
          <span style="color:var(--fg-2)">${c.duration.toFixed(2)}s</span>
        </button>`
    )
    .join('')

  if (clip === null) {
    return `${list}<p style="color:var(--fg-2)">Pick a clip. It is retargeted onto your rig — the library and the template do not share a rest pose, so the motion is moved, not copied.</p>`
  }

  const label = playing ? 'Playing\u2026' : `Preview ${escape(clip)}`
  const stopButton = playing ? '<button id="stop" class="action">Stop</button>' : ''
  return `${list}
    <button id="preview" class="action" ${playing ? 'disabled' : ''}>${label}</button>
    ${stopButton}
    <p style="color:var(--fg-2)">${escape(clip)} will be written into the export.</p>`
}

/** Plays the chosen clip in the viewport, or stops it. */
async function runPreview(): Promise<void> {
  if (loaded === null || fitted === null || chosen === null || clip === null || playing) return
  playing = true
  render()
  try {
    const glb = await previewAnimation(loaded.path, fitted, 2.0, chosen, clip)
    const duration = await ensureViewport().playAnimated(glb, clip)
    if (duration === null) {
      playing = false
      render()
    }
  } catch (err) {
    playing = false
    render()
    const message = err instanceof Error ? err.message : String(err)
    console.error('preview failed', message)
  }
}

function stopPreview(): void {
  if (!playing) return
  ensureViewport().stop()
  playing = false
  render()
}

/** Fetches the chosen creature's clips once, then re-renders with them. */
async function ensureClips(): Promise<void> {
  if (clips !== null || chosen === null || !isDesktop()) return
  try {
    clips = await animationClips(chosen)
  } catch {
    // A creature with no library is a fact to show, not an error to throw.
    clips = []
  }
  render()
}

/** The Export step: write the rigged model out. */
function renderExportStep(): string {
  if (bound === null || fitted === null || loaded === null) {
    return '<p style="color:var(--fg-2)">Bind the weights first — an export carries the skeleton and the weights, not just the mesh.</p>'
  }

  const buttons = (['glb', 'fbx'] as const)
    .map(
      (format) =>
        `<button class="action export" data-format="${format}" ${exporting ? 'disabled' : ''}>${
          exporting ? 'Writing\u2026' : `Export as .${format}`
        }</button>`
    )
    .join('')
  const done =
    exported === null
      ? `<p style="color:var(--fg-2)">Mesh, skeleton and weights, in one file${
          clip === null ? '' : `, with ${escape(clip)}`
        }. Both formats carry the same rig.</p>`
      : `<p style="color:var(--ok)">Wrote ${escape(exported)}.</p>`

  return `${buttons}${done}`
}

/** Writes the rigged model to a file the user picks. */
async function runExport(format: 'glb' | 'fbx'): Promise<void> {
  if (loaded === null || fitted === null || exporting) return
  exporting = true
  render()
  try {
    const saved = await exportModel(loaded.path, fitted, 2.0, format, chosen ?? '', clip)
    // A cancelled dialog leaves the previous result alone rather than clearing it.
    if (saved !== null) exported = saved
  } finally {
    exporting = false
    render()
  }
}

/** Fetches the template list once, then re-renders with it. */
async function ensureTemplates(): Promise<void> {
  if (templates !== null || !isDesktop()) return
  templates = await skeletonTemplates()
  render()
}

/** Places the chosen template's skeleton and draws it. */
async function runFit(name: string): Promise<void> {
  if (loaded === null || fitting) return
  chosen = name
  fitting = true
  render()
  try {
    fitted = await fitSkeleton(name, loaded.path)
    bound = null
    // A different creature has a different library.
    clips = null
    clip = null
    ensureViewport().showFittedSkeleton(fitted.positions, fitted.parents)
    // A placed skeleton is what binding needs. The Fit step in between has
    // nothing to complete yet, so it does not gate the one after it.
    furthestStep = Math.max(furthestStep, 3)
  } finally {
    fitting = false
    render()
  }
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
      // A model is what the skeleton step needs, so earning it unlocks that step.
      furthestStep = Math.max(furthestStep, 1)
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
    // Locked on real progress, not on where the user happens to be standing:
    // going back a step must not lock the ones already earned.
    const locked = i > furthestStep
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

  const bindButton = app.querySelector<HTMLButtonElement>('#bind')
  bindButton?.addEventListener('click', () => void runBind())

  app.querySelectorAll<HTMLButtonElement>('.export').forEach((button) => {
    const format = button.dataset['format']
    if (format !== 'glb' && format !== 'fbx') return
    button.addEventListener('click', () => void runExport(format))
  })

  app.querySelector<HTMLButtonElement>('#preview')?.addEventListener('click', () => void runPreview())
  app.querySelector<HTMLButtonElement>('#stop')?.addEventListener('click', () => stopPreview())

  app.querySelectorAll<HTMLButtonElement>('.template').forEach((button) => {
    const name = button.dataset['template']
    const clipName = button.dataset['clip']
    if (clipName !== undefined) {
      button.addEventListener('click', () => {
        // Switching clips stops any playback of the old one.
        stopPreview()
        clip = clipName
        exported = null
        render()
      })
      return
    }
    if (name === undefined) return
    button.addEventListener('click', () => void runFit(name))
  })

  if (step.id === StepId.Animate) void ensureClips()

  if (step.id === StepId.LoadSkeleton && loaded !== null) void ensureTemplates()

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
