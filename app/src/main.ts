import './ui/tokens.css'
import './ui/shell.css'

import {
  createIcons,
  AlertTriangle,
  Bone,
  Upload,
  Link,
  Play,
  Download,
  Move3d,
  Rotate3d,
  Maximize,
  ZoomIn,
  ZoomOut,
  HelpCircle,
  Pause,
  Rewind,
  FastForward,
  Square
} from 'lucide'
import { History } from './state/history'
import { STEPS, StepId, type StepDef } from './state/steps'
import {
  animationClips,
  animationLibrary,
  bindWeights,
  buildInfo,
  devAutoload,
  devAutofit,
  onRigProgress,
  forwardConsoleToTerminal,
  exportModel,
  weightOverlay,
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
import { createClipPreview, type ClipPreview } from './viewport/preview'
import { type ViewPreset, frameOfTime, timeOfFrame, totalFrames } from './viewport/model'

// Mirror all webview console output to the Rust terminal for debugging.
forwardConsoleToTerminal()

// Surface long-running-command progress (fit/bind/export/preview) in the status
// bar; it clears when the command finishes.
void onRigProgress((p) => {
  const el = document.querySelector<HTMLSpanElement>('#diag')
  if (el === null) return
  el.textContent = p.fraction >= 1 ? '' : `${p.command}: ${p.phase} ${Math.round(p.fraction * 100)}%`
})

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
/** True while a clip preview is loaded in the viewport (playing or paused). */
let playing = false
/** Playback transport for the Animate timeline. */
let paused = false
let direction: 1 | -1 = 1
let fps: 24 | 30 = 30
let clipDuration = 0
/** The rAF handle for the loop that walks the timeline slider during playback. */
let playhead = 0

let viewport: Viewport | null = null

/** Which model-transform tool is active (rotate/move the model), or none. */
let transformMode: 'none' | 'rotate' | 'translate' = 'none'

/** The clip chooser's moving preview, created on first use, and which creature's
 *  library it has loaded. */
let clipPreview: ClipPreview | null = null
let libraryFor: string | null = null

function ensureClipPreview(): ClipPreview {
  clipPreview ??= createClipPreview()
  return clipPreview
}

/** Loads a creature's animation library into the preview once, then shows the
 *  selected clip (or the first). */
async function ensureLibrary(template: string): Promise<void> {
  if (libraryFor === template || !isDesktop()) return
  libraryFor = template
  try {
    await ensureClipPreview().load(await animationLibrary(template))
    const first = clips?.[0]?.name
    const show = clip ?? first
    if (show !== undefined) ensureClipPreview().play(show)
  } catch (err) {
    libraryFor = null
    console.error('clip preview library failed to load', err)
  }
}

function ensureViewport(): Viewport {
  viewport ??= createViewport()
  return viewport
}

/**
 * The rig state undo/redo covers (design.md §11, "undo/redo at every step").
 *
 * Only what the user edits: the template chosen, where its skeleton landed
 * (including joints dragged by hand), whether it is bound, the selected clip,
 * and where in the flow they are. The imported model is not here — undo winds
 * back the rigging, it does not un-import the file.
 */
interface Snapshot {
  chosen: string | null
  fitted: FittedSkeleton | null
  bound: BindReport | null
  clip: string | null
  activeStep: number
  furthestStep: number
}

const history = new History<Snapshot>()

/** Captures the current rig state. Values are reassigned immutably elsewhere, so
 * holding references is safe — a snapshot never sees a later mutation. */
function snapshot(): Snapshot {
  return { chosen, fitted, bound, clip, activeStep, furthestStep }
}

/** Records the current state as an undo point. */
function record(): void {
  history.push(snapshot())
}

/** Applies a snapshot and redraws, including the viewport's fitted skeleton. */
function restore(state: Snapshot): void {
  chosen = state.chosen
  fitted = state.fitted
  bound = state.bound
  clip = state.clip
  activeStep = state.activeStep
  furthestStep = state.furthestStep
  if (fitted !== null) {
    ensureViewport().showFittedSkeleton(fitted.positions, fitted.parents, onJointEdited)
  }
  render()
}

function undo(): void {
  const state = history.undo()
  if (state !== null) restore(state)
}

function redo(): void {
  const state = history.redo()
  if (state !== null) restore(state)
}

/** The fitted-skeleton edit callback: a dragged joint replaces the placement and
 * makes any earlier weights stale, then becomes its own undo point. */
function onJointEdited(positions: ReadonlyArray<readonly [number, number, number]>): void {
  if (fitted === null) return
  fitted = { ...fitted, positions }
  bound = null
  record()
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
      ? `<p class="warn"><i data-lucide="alert-triangle" width="14" height="14" aria-hidden="true"></i>
           ${model.over_influence_limit} of them carry more than four bone influences;
           only the strongest four are kept.</p>`
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
           ${poseRow()}
         </dl>
         <p style="color:var(--fg-2)">The skeleton is drawn over the mesh. Fit it by hand next \u2014 drag any joint that sits outside the body.</p>`

  return `${list}${creatureGuidance()}${fitting ? '<p style="color:var(--fg-2)">Fitting\u2026</p>' : outcome}`
}

/** The chosen creature's placement tip (design.md \u00a77), shown once one is picked.
 * The copy is the template's own, carried from its manifest \u2014 not written here. */
function creatureGuidance(): string {
  const guide = templates?.find((template) => template.name === chosen)?.guidance
  if (guide === undefined || guide === '') return ''
  return `<p class="tip"><i data-lucide="bone" width="14" height="14" aria-hidden="true"></i> ${escape(guide)}</p>`
}

/** A human-readable pose name, or `null` when there is nothing worth showing
 * (a non-human template, or a pose the detector could not place). */
function poseLabel(pose: string): string | null {
  switch (pose) {
    case 't-pose':
      return 'T-pose'
    case 'a-pose':
      return 'A-pose'
    case 'arms-down':
      return 'Arms down'
    default:
      return null
  }
}

/** A `Pose` fact row for the fit summaries, or empty when there is none. */
function poseRow(): string {
  const label = fitted === null ? null : poseLabel(fitted.pose)
  return label === null ? '' : `<dt>Pose</dt><dd>${label}</dd>`
}

/** The Fit Skeleton step: reports the automatic fit and detected pose, and the
 * viewport lets the user drag joints to adjust it. */
function renderEditStep(): string {
  if (fitted === null) {
    return '<p style="color:var(--fg-2)">Choose a skeleton first.</p>'
  }
  return `<dl class="facts">
      <dt>Bones</dt><dd>${fitted.bones.length}</dd>
      <dt>Scale</dt><dd>${fitted.scale.toFixed(3)}\u00d7</dd>
      ${poseRow()}
    </dl>
    <p style="color:var(--fg-2)">The skeleton is placed automatically. Drag a joint handle in the viewport to adjust any bone that sits outside the mesh, then bind.</p>`
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
    ${problems.join('') || '<p style="color:var(--ok)">Every vertex is attached.</p>'}
    <button id="paint" class="action">Show weight paint</button>
    <p style="color:var(--fg-2)">Each vertex is coloured by the bone that moves it; guessed regions are flagged red.</p>`
}

/** Bakes the weight-paint overlay and shows it in the viewport. */
async function runPaint(): Promise<void> {
  if (loaded === null || fitted === null) return
  const glb = await weightOverlay(loaded.path, fitted, 2.0)
  await ensureViewport().showOverlay(glb)
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
    record()
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

  // A moving preview of the clip under the cursor (or the selected one), played
  // on the library character — see what a motion looks like before committing.
  const preview = '<div class="clip-preview" id="clip-preview" aria-label="Clip preview"></div>'
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
    return `${preview}${list}<p style="color:var(--fg-2)">Hover a clip to preview it, then pick one. It is retargeted onto your rig — the library and the template do not share a rest pose, so the motion is moved, not copied.</p>`
  }

  if (!playing) {
    return `${preview}${list}
      <button id="preview" class="action">Preview ${escape(clip)}</button>
      <p style="color:var(--fg-2)">${escape(clip)} will be written into the export.</p>`
  }
  return `${preview}${list}${renderTransport()}
    <p style="color:var(--fg-2)">${escape(clip)} will be written into the export.</p>`
}

/** The playback transport for the Animate step: fps, direction, pause, stop, and
 *  a scrubbable frame timeline. */
function renderTransport(): string {
  const frames = totalFrames(clipDuration, fps)
  const dirOn = (d: 1 | -1): string => (!paused && direction === d ? 'on' : '')
  return `
    <div class="transport" role="group" aria-label="Playback">
      <button class="chip ${fps === 24 ? 'on' : ''}" data-fps="24" aria-pressed="${fps === 24}">24</button>
      <button class="chip ${fps === 30 ? 'on' : ''}" data-fps="30" aria-pressed="${fps === 30}">30</button>
      <span class="tp-fps">fps</span>
      <span class="tp-sp"></span>
      <button class="tp-btn ${dirOn(-1)}" id="play-back" title="Play backward" aria-label="Play backward"><i data-lucide="rewind" width="16" height="16" aria-hidden="true"></i></button>
      <button class="tp-btn" id="play-pause" title="${paused ? 'Resume' : 'Pause'}" aria-label="${paused ? 'Resume' : 'Pause'}"><i data-lucide="${paused ? 'play' : 'pause'}" width="16" height="16" aria-hidden="true"></i></button>
      <button class="tp-btn ${dirOn(1)}" id="play-fwd" title="Play forward" aria-label="Play forward"><i data-lucide="fast-forward" width="16" height="16" aria-hidden="true"></i></button>
      <button class="tp-btn" id="stop" title="Stop" aria-label="Stop"><i data-lucide="square" width="16" height="16" aria-hidden="true"></i></button>
    </div>
    <input id="timeline" class="timeline" type="range" min="0" max="${frames}" step="1" value="0" aria-label="Timeline (frame)"/>
    <div class="timecode"><span id="frame">0</span> / ${frames} frames</div>`
}

/** Plays the chosen clip in the viewport, or stops it. */
async function runPreview(): Promise<void> {
  if (loaded === null || fitted === null || chosen === null || clip === null || playing) return
  playing = true
  paused = false
  direction = 1
  render()
  try {
    const glb = await previewAnimation(loaded.path, fitted, 2.0, chosen, clip)
    const duration = await ensureViewport().playAnimated(glb, clip)
    if (duration === null) {
      playing = false
      render()
      return
    }
    clipDuration = duration
    render() // the timeline max needs the real duration
    startPlayhead()
  } catch (err) {
    playing = false
    render()
    const message = err instanceof Error ? err.message : String(err)
    console.error('preview failed', message)
  }
}

function stopPreview(): void {
  if (!playing) return
  stopPlayhead()
  ensureViewport().stop()
  playing = false
  render()
}

/** Sets the play direction and resumes. */
function setDirection(value: 1 | -1): void {
  direction = value
  paused = false
  const vp = ensureViewport()
  vp.setPlaybackDirection(value)
  vp.setPaused(false)
  render()
}

/** Toggles pause/resume of the running clip. */
function togglePause(): void {
  paused = !paused
  ensureViewport().setPaused(paused)
  render()
}

/** Walks the timeline slider and frame label along with playback, leaving them
 *  alone while the user is scrubbing (the slider is focused). */
function startPlayhead(): void {
  cancelAnimationFrame(playhead)
  const tick = (): void => {
    const frames = totalFrames(clipDuration, fps)
    const frame = Math.min(frameOfTime(ensureViewport().playbackTime(), fps), frames)
    const slider = document.querySelector<HTMLInputElement>('#timeline')
    if (slider !== null && document.activeElement !== slider) slider.value = String(frame)
    const label = document.querySelector<HTMLSpanElement>('#frame')
    if (label !== null) label.textContent = String(frame)
    playhead = requestAnimationFrame(tick)
  }
  playhead = requestAnimationFrame(tick)
}

function stopPlayhead(): void {
  cancelAnimationFrame(playhead)
  playhead = 0
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
    ensureViewport().showFittedSkeleton(fitted.positions, fitted.parents, onJointEdited)
    // A placed skeleton is what binding needs. The Fit step in between has
    // nothing to complete yet, so it does not gate the one after it.
    furthestStep = Math.max(furthestStep, 3)
    record()
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
      // The imported-but-unrigged state is the baseline undo returns to.
      record()
      // Rendered before drawing, so the canvas is in the DOM and has a size to
      // frame the model against.
      render()
      const viewport = ensureViewport()
      // A new model arrives untransformed; the gizmo is cleared inside show().
      transformMode = 'none'
      await viewport.show(geometry)
      // Surface the render diagnostics for "the viewport is blank" reports.
      const diag = viewport.info()
      console.log('viewport:', diag)
      void reportStartup(`viewport ${diag}`)
      const el = document.querySelector<HTMLSpanElement>('#diag')
      if (el !== null) el.textContent = diag
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
    <i data-lucide="${step.icon}" width="18" height="18" aria-hidden="true"></i>
    <div>
      <strong>${step.label}</strong>
      <p>${step.goal} <span style="color:var(--fg-2)">${step.success}</span></p>
    </div>`
}

/** The floating viewport controls: frame, zoom, and the front/side/top presets.
 *  A mouse-free way to do what orbit/scroll do, and a discoverable one. */
function renderViewportNav(): string {
  return `
    <div class="viewport-nav" role="toolbar" aria-label="Viewport controls">
      <button class="nav-btn" data-view-action="frame" title="Frame the model" aria-label="Frame the model">
        <i data-lucide="maximize" width="16" height="16" aria-hidden="true"></i>
      </button>
      <button class="nav-btn" data-view-action="zoom-in" title="Zoom in" aria-label="Zoom in">
        <i data-lucide="zoom-in" width="16" height="16" aria-hidden="true"></i>
      </button>
      <button class="nav-btn" data-view-action="zoom-out" title="Zoom out" aria-label="Zoom out">
        <i data-lucide="zoom-out" width="16" height="16" aria-hidden="true"></i>
      </button>
      <span class="nav-sep"></span>
      <button class="nav-btn" data-transform="translate" aria-pressed="false" title="Move the model (independent of the grid)" aria-label="Move the model">
        <i data-lucide="move-3d" width="16" height="16" aria-hidden="true"></i>
      </button>
      <button class="nav-btn" data-transform="rotate" aria-pressed="false" title="Rotate the model (independent of the grid)" aria-label="Rotate the model">
        <i data-lucide="rotate-3d" width="16" height="16" aria-hidden="true"></i>
      </button>
      <span class="nav-sep"></span>
      <button class="nav-btn nav-txt" data-view-preset="front" title="Front view (numpad 1)" aria-label="Front view">Front</button>
      <button class="nav-btn nav-txt" data-view-preset="right" title="Side view (numpad 3)" aria-label="Side view">Side</button>
      <button class="nav-btn nav-txt" data-view-preset="top" title="Top view (numpad 7)" aria-label="Top view">Top</button>
      <span class="nav-sep"></span>
      <button class="nav-btn" aria-label="Navigation help"
        title="Blender-style navigation&#10;Orbit: middle-drag, ⌥+drag, or two-finger swipe&#10;Pan: ⇧+middle-drag, right-drag, or ⇧+two-finger&#10;Zoom: scroll, pinch, or ⌃/⌘+two-finger&#10;Views: numpad 1/3/7 front/side/top (⌃ opposite), 4/6/8/2 orbit, . frame&#10;Model: the move / rotate tools reorient the mesh on the grid&#10;Fit step: click a joint, drag or arrow-keys to nudge (⇧ finer)">
        <i data-lucide="help-circle" width="16" height="16" aria-hidden="true"></i>
      </button>
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
      <h1 class="visually-hidden">Mesh2Motion</h1>

      <nav class="rail" aria-label="Rigging steps">
        <h2>Steps</h2>
        ${renderRail()}
      </nav>

      <main class="viewport">
        <div class="viewport-empty">
          <i data-lucide="bone" width="40" height="40" stroke-width="1" aria-hidden="true"></i>
          <div>
            <div style="color:var(--fg-1);font-size:var(--fs-lg)">${loaded === null ? 'No model loaded' : escape(loaded.name)}</div>
            <div>${loaded === null ? 'Import a mesh to begin' : 'Viewport rendering arrives with the model preview step'}</div>
          </div>
        </div>
        ${loaded === null ? '' : renderViewportNav()}
        <div class="guidance">${renderGuidance(step)}</div>
      </main>

      <aside class="inspector" aria-label="Properties">
        <h2>${step.label}</h2>
        ${renderInspector(step)}
      </aside>

      <div class="status" role="status">
        <span id="env">—</span>
        <span class="spacer"></span>
        <span id="diag">—</span>
      </div>
    </div>`

  // The canvas is moved into the freshly built shell rather than recreated.
  if (loaded !== null) {
    const stage = app.querySelector<HTMLElement>('.viewport')
    stage?.querySelector('.viewport-empty')?.remove()
    stage?.prepend(ensureViewport().canvas)
  }

  createIcons({
    icons: { AlertTriangle, Bone, Upload, Link, Play, Download, Move3d, Rotate3d, Maximize, ZoomIn, ZoomOut, HelpCircle, Pause, Rewind, FastForward, Square }
  })

  // Viewport navigation toolbar: frame / zoom / preset views.
  app.querySelectorAll<HTMLButtonElement>('[data-view-action]').forEach((button) => {
    const action = button.dataset['viewAction']
    button.addEventListener('click', () => {
      const vp = ensureViewport()
      if (action === 'frame') vp.reframe()
      else if (action === 'zoom-in') vp.zoom(0.8)
      else if (action === 'zoom-out') vp.zoom(1.25)
    })
  })
  app.querySelectorAll<HTMLButtonElement>('[data-view-preset]').forEach((button) => {
    const preset = button.dataset['viewPreset'] as ViewPreset | undefined
    if (preset === undefined) return
    button.addEventListener('click', () => ensureViewport().setView(preset))
  })

  // Rotate / move the model. Clicking a tool toggles it; the two are mutually
  // exclusive, and clicking the active one returns to plain navigation.
  const transformButtons = app.querySelectorAll<HTMLButtonElement>('[data-transform]')
  transformButtons.forEach((button) => {
    const mode = button.dataset['transform']
    if (mode !== 'rotate' && mode !== 'translate') return
    button.setAttribute('aria-pressed', String(transformMode === mode))
    button.addEventListener('click', () => {
      transformMode = transformMode === mode ? 'none' : mode
      ensureViewport().setTransformMode(transformMode)
      transformButtons.forEach((b) =>
        b.setAttribute('aria-pressed', String(b.dataset['transform'] === transformMode))
      )
    })
  })

  const importButton = app.querySelector<HTMLButtonElement>('#import')
  if (importButton !== null) {
    importButton.addEventListener('click', () => void runImport(importButton))
  }

  const bindButton = app.querySelector<HTMLButtonElement>('#bind')
  bindButton?.addEventListener('click', () => void runBind())

  app.querySelector<HTMLButtonElement>('#paint')?.addEventListener('click', () => void runPaint())

  app.querySelectorAll<HTMLButtonElement>('.export').forEach((button) => {
    const format = button.dataset['format']
    if (format !== 'glb' && format !== 'fbx') return
    button.addEventListener('click', () => void runExport(format))
  })

  app.querySelector<HTMLButtonElement>('#preview')?.addEventListener('click', () => void runPreview())
  app.querySelector<HTMLButtonElement>('#stop')?.addEventListener('click', () => stopPreview())
  app.querySelector<HTMLButtonElement>('#play-back')?.addEventListener('click', () => setDirection(-1))
  app.querySelector<HTMLButtonElement>('#play-fwd')?.addEventListener('click', () => setDirection(1))
  app.querySelector<HTMLButtonElement>('#play-pause')?.addEventListener('click', () => togglePause())
  app.querySelectorAll<HTMLButtonElement>('[data-fps]').forEach((button) => {
    const value = Number(button.dataset['fps'])
    if (value !== 24 && value !== 30) return
    button.addEventListener('click', () => {
      fps = value
      render()
    })
  })
  const timeline = app.querySelector<HTMLInputElement>('#timeline')
  timeline?.addEventListener('input', () => {
    ensureViewport().seek(timeOfFrame(Number(timeline.value), fps))
  })

  app.querySelectorAll<HTMLButtonElement>('.template').forEach((button) => {
    const name = button.dataset['template']
    const clipName = button.dataset['clip']
    if (clipName !== undefined) {
      // Hovering plays the clip in the chooser preview; clicking selects it.
      button.addEventListener('mouseenter', () => clipPreview?.play(clipName))
      button.addEventListener('focus', () => clipPreview?.play(clipName))
      button.addEventListener('click', () => {
        // Switching clips stops any playback of the old one.
        stopPreview()
        clip = clipName
        exported = null
        clipPreview?.play(clipName)
        record()
        render()
      })
      return
    }
    if (name === undefined) return
    button.addEventListener('click', () => void runFit(name))
  })

  if (step.id === StepId.Animate) {
    void ensureClips()
    // Mount the persistent preview canvas and load the creature's library once.
    const slot = app.querySelector<HTMLElement>('#clip-preview')
    if (slot !== null && chosen !== null) {
      slot.appendChild(ensureClipPreview().canvas)
      void ensureLibrary(chosen)
    }
  }

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

// Undo/redo across the whole flow (design.md §11). Global, not per-render:
// render() replaces the shell's markup, so a listener bound inside it would be
// torn down and rebound every draw. Cmd/Ctrl+Z undoes, add Shift to redo.
window.addEventListener('keydown', (event) => {
  const meta = event.metaKey || event.ctrlKey
  if (!meta || event.key.toLowerCase() !== 'z') return
  event.preventDefault()
  if (event.shiftKey) redo()
  else undo()
})

render()

/** Dev/screenshot harness: when `M2M_AUTOLOAD` is set, import that model at
 *  startup without the native picker, so the viewport can be screenshotted. */
async function maybeAutoload(): Promise<void> {
  if (!isDesktop()) return
  let path: string | null = null
  try {
    path = await devAutoload()
  } catch {
    return
  }
  if (path === null) return
  const button = document.querySelector<HTMLButtonElement>('#import')
  if (button !== null) await runImport(button)
  // Optionally auto-fit a template so the Fit step (and its auto-placement) can
  // be screenshotted without clicking through the workflow.
  let template: string | null = null
  try {
    template = await devAutofit()
  } catch {
    return
  }
  if (template === null) return
  await runFit(template)
  activeStep = STEPS.findIndex((s) => s.id === StepId.EditSkeleton)
  render()
}
// Fire-and-forget at the entry module's end; nothing runs after it.
await maybeAutoload()
