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
  devAutoclip,
  devAutopaint,
  devAutomark,
  devAutomarkSolve,
  devAutomarkHover,
  devAutomarkCapture,
  devCaptureSelftest,
  devSaveFixture,
  devAnimateView,
  devAnimateMirror,
  devAnimateArmSpace,
  devAutoexport,
  devAutoOrbit,
  devAutoproceed,
  onRigProgress,
  forwardConsoleToTerminal,
  exportModel,
  weightOverlay,
  fitSkeleton,
  skeletonFromImport,
  fitFromMarkers,
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
  type Marker,
  type SkeletonTemplate
} from './ipc'
import { detectBackend } from './viewport/backend'
import { createViewport, type Viewport } from './viewport/scene'
import { createClipPreview, type ClipPreview } from './viewport/preview'
import { type ViewPreset, frameOfTime, timeOfFrame, totalFrames } from './viewport/model'
import { markerSetFor, slotForClickedSide } from './state/markers'
import { clipDescription, clipMatches, humanizeClipName } from './state/clip-index'
import markerGuideHuman from './assets/marker-guide-human.png'

/** A rendered guide image per template, showing where the markers go on a real
 *  model — the reference's placement thumbnail. Only templates with one show it. */
const MARKER_GUIDES: Readonly<Record<string, string>> = { human: markerGuideHuman }

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

/** Marker-placement flow: on while a person is placing markers, before the
 *  skeleton is solved. Off for the auto-fit flow and once solved. */
let markerMode = false
/** Placed marker positions, by slot id (see state/markers.ts). */
const markerPositions = new Map<string, [number, number, number]>()
/** The slot the next viewport click fills, or null. */
let activeSlot: string | null = null
/** Whether placing one side's marker mirrors it to the other. */
let useSymmetry = true
/** Dev/testing: reveal a "Save markers" button so a hand placement can be
 *  captured as a fixture (set by the M2M_AUTOMARK_CAPTURE harness). */
let markerCapture = false
/** Confirmation shown under the "Save markers" button after a save. */
let markerSaveStatus: string | null = null

/** What binding the mesh to the skeleton produced. */
let bound: BindReport | null = null
let binding = false
/** True while "Proceed to Animate" reads an already-rigged import's own skeleton. */
let riggingImport = false
/** The imported model's own glb bytes, kept so its OWN clips can be played back
 *  directly (no retarget) when it arrives already rigged and animated. */
let modelBytes: ArrayBuffer | null = null
/** True when Animate plays the imported model's OWN embedded clips rather than
 *  retargeting a template library onto a fitted skeleton. */
let ownClips = false

/** The file the rigged model was last written to. */
let exported: string | null = null
let exporting = false

/** Download-modal settings (Mixamo's "Download Settings"). Format and the clip
 *  options are chosen here; Trim comes from the Animate step's range. */
let exportFormat: 'glb' | 'fbx' = 'glb'
let exportSkin = true
let exportFps: 24 | 30 | 60 = 30
let keyframeReduction: 'none' | 'low' | 'high' = 'none'
/** Reduction level → tolerance (radians) the Rust reducer bounds error to. */
const REDUCTION_TOL: Record<'none' | 'low' | 'high', number> = { none: 0, low: 0.01, high: 0.05 }

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
/** Playback speed (Mixamo's "Overdrive"): 0–100, 50 = 1× (so 0–2× of real time). */
let overdrive = 50
/** Trim: the fraction of the clip to keep, [start, end] in 0–1. Playback loops
 *  within it and the export is trimmed to it. Kept as fractions so it survives
 *  clip and fps changes. */
let trimStart = 0
let trimEnd = 1
/** Mirror the animation left↔right (re-retargets the clip on the Rust side). */
let mirrored = false
/** Character Arm-Space: 0–100, 50 neutral. Re-retargets the clip when changed. */
let armSpace = 50
/** The rAF handle for the loop that walks the timeline slider during playback. */
let playhead = 0
/** The Animate step's 3-way view over the playing clip. Mesh by default, like
 *  Mixamo (its skull toggle switches to the skeleton). */
let animateView: 'mesh' | 'skeleton' | 'both' = 'mesh'
/** Current animation-search query (filters the clip list by name/description/tags). */
let clipQuery = ''

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
        ? `<p style="color:var(--fg-1)">This model is already rigged${
            model.clips.length > 0
              ? ` and carries ${model.clips.length} ${
                  model.clips.length === 1 ? 'clip' : 'clips'
                } \u2014 play them directly, or`
              : ' \u2014'
          } re-rig from a template, or retarget our clips onto its own skeleton.</p>
           ${
             model.clips.length > 0
               ? '<button id="play-own" class="action primary">Animate its own clips</button>'
               : ''
           }
           <button id="proceed-rigged" class="action${model.clips.length > 0 ? '' : ' primary'}" ${
             riggingImport ? 'disabled' : ''
           }>${riggingImport ? 'Reading rig\u2026' : 'Retarget our clips'}</button>
           <button id="rerig" class="action" ${riggingImport ? 'disabled' : ''}>Re-rig from template</button>`
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

/** The Fit Skeleton step: marker placement (its default) or, once solved, the
 * fit report with the viewport letting the user drag joints to adjust it. */
function renderEditStep(): string {
  if (markerMode) return renderMarkerPanel()
  if (fitted === null) {
    return '<p style="color:var(--fg-2)">Choose a skeleton first.</p>'
  }
  const refit = `<button id="refit" class="action" ${fitting ? 'disabled' : ''}>${
    fitting ? 'Fitting\u2026' : 'Auto-fit again'
  }</button>`
  const replace =
    chosen !== null && markerSetFor(chosen) !== null
      ? '<button id="replace-markers" class="action">Re-place markers</button>'
      : ''
  return `<dl class="facts">
      <dt>Bones</dt><dd>${fitted.bones.length}</dd>
      <dt>Scale</dt><dd>${fitted.scale.toFixed(3)}\u00d7</dd>
      ${poseRow()}
    </dl>
    ${refit}${replace}
    <p style="color:var(--fg-2)">Drag a joint handle to nudge any bone that sits outside the mesh, then bind. Re-place markers to solve again from scratch, or auto-fit to let the mesh place it.</p>`
}

/** The marker-placement panel (the Mixamo Place-markers step): a guide diagram,
 * a grouped set of ring markers, the symmetry toggle and the solve / auto-fit
 * choice. Clicking a ring arms it for the next viewport click. */
function renderMarkerPanel(): string {
  const set = chosen === null ? null : markerSetFor(chosen)
  if (set === null) return ''

  // Group the slots the way the reference does: single markers on their own row,
  // left/right pairs on one row side by side.
  const groups: { label: string; slots: typeof set }[] = []
  const done = new Set<string>()
  for (const slot of set) {
    if (done.has(slot.id)) continue
    if (slot.pair !== undefined) {
      const pair = set.find((s) => s.id === slot.pair)
      const slots = pair === undefined ? [slot] : [slot, pair]
      slots.forEach((s) => done.add(s.id))
      groups.push({ label: `${slot.label.replace(/ [LR]$/, '')}s`, slots })
    } else {
      done.add(slot.id)
      groups.push({ label: slot.label, slots: [slot] })
    }
  }

  const rows = groups
    .map((group) => {
      const rings = group.slots
        .map((slot) => {
          const hex = `#${slot.color.toString(16).padStart(6, '0')}`
          const state = markerPositions.has(slot.id) ? 'placed' : ''
          const active = slot.id === activeSlot ? 'aria-current="true"' : ''
          return `<button class="marker-ring ${state}" data-slot="${slot.id}" ${active}
                     style="--ring:${hex}" title="${escape(slot.label)}" aria-label="${escape(slot.label)}"></button>`
        })
        .join('')
      return `<div class="marker-row"><span class="marker-label">${escape(group.label)}</span><span class="marker-rings">${rings}</span></div>`
    })
    .join('')

  const placed = set.filter((s) => markerPositions.has(s.id)).length
  const total = set.length
  const canSolve = placed >= 2 && !fitting
  const active = set.find((s) => s.id === activeSlot)
  const hint = active?.hint === undefined ? '' : ` \u2014 ${escape(active.hint)}`
  const prompt =
    active === undefined
      ? 'All markers placed \u2014 press Solve to build the rig.'
      : `Click the <b>${escape(active.label)}</b> on your model${hint}.${useSymmetry ? ' The other side mirrors.' : ''}`

  return `
    <p style="color:var(--fg-2)">Face the model forward in a T-pose (use the move / rotate tools), then place a marker on each joint and Solve.</p>
    ${guideImage(chosen)}
    <p class="marker-prompt">${prompt}</p>
    <div class="marker-grid">${rows}</div>
    <label class="toggle"><input type="checkbox" id="symmetry" ${useSymmetry ? 'checked' : ''}/> Use symmetry</label>
    <button id="solve" class="action primary" ${canSolve ? '' : 'disabled'}>${
      fitting ? 'Solving\u2026' : `Solve rig (${placed}/${total})`
    }</button>
    <button id="autofit" class="action">Auto-fit instead</button>
    ${markerCapture ? `<button id="save-markers" class="action" ${placed > 0 ? '' : 'disabled'}>Save markers (test)</button>` : ''}
    ${markerCapture && markerSaveStatus !== null ? `<p class="marker-save-status" style="color:var(--fg-2);word-break:break-all">${escape(markerSaveStatus)}</p>` : ''}`
}

/** The rendered guide image for a template, or nothing when it has none. */
function guideImage(template: string | null): string {
  const src = template === null ? undefined : MARKER_GUIDES[template]
  if (src === undefined) return ''
  return `<img class="marker-guide" src="${src}" alt="Where the markers go on a T-posed model" />`
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

/** "Proceed to Animate" for an already-rigged import: read its OWN skeleton,
 *  bind weights to it, and jump to Animate to retarget our clips onto it —
 *  skipping the template Choose/Fit steps entirely. */
async function proceedRigged(): Promise<void> {
  if (loaded === null || riggingImport || binding) return
  riggingImport = true
  render()
  try {
    fitted = await skeletonFromImport(loaded.path)
    // No template; humanoid imports (the common already-rigged case) draw from
    // the human clip library, and the retarget auto-maps foreign bone names.
    chosen = 'human'
    clips = null
    clip = null
    // Bind is template-independent (weights solve straight from the skeleton),
    // so this yields a complete rig with no marker/fit pass. It also unlocks
    // Animate + Export via furthestStep.
    await runBind()
    activeStep = STEPS.findIndex((s) => s.id === StepId.Animate)
    record()
  } finally {
    riggingImport = false
    render()
  }
}

/** Animate an already-rigged import with its OWN embedded clips — no fitting,
 *  no retarget. The mesh is already bound to its own skeleton, so its authored
 *  clips deform it directly (the correct path for a rigged, animated model). */
function proceedOwnClips(): void {
  if (loaded === null) return
  ownClips = true
  clip = loaded.import.clips[0] ?? null
  furthestStep = Math.max(furthestStep, STEPS.findIndex((s) => s.id === StepId.Animate))
  activeStep = STEPS.findIndex((s) => s.id === StepId.Animate)
  render()
  void playOwnClip()
}

/** Plays the selected own-clip on the imported model's own bytes. */
async function playOwnClip(): Promise<void> {
  if (modelBytes === null || clip === null) return
  const duration = await ensureViewport().playAnimated(modelBytes, clip)
  if (duration === null) return
  playing = true
  clipDuration = duration
  render()
  startPlayhead()
  ensureViewport().setPlaybackRate(playbackRate())
}

/** The Animate step: pick a clip to retarget onto the rig. */
function renderAnimateStep(): string {
  // Own-clips mode: list the imported model's OWN clips (played directly).
  if (ownClips && loaded !== null) {
    const names = loaded.import.clips.filter((n) => !n.includes('_source_'))
    if (names.length === 0) return '<p style="color:var(--fg-2)">This model has no clips.</p>'
    const rows = names
      .map(
        (name) =>
          `<button class="action template clip-item" data-ownclip="${escape(name)}" ${
            name === clip ? 'aria-current="true"' : ''
          }><span class="clip-text"><span class="clip-name">${escape(
            humanizeClipName(name)
          )}</span></span></button>`
      )
      .join('')
    return `<p style="color:var(--fg-2)">Playing this model's own clips.</p>${
      clip === null ? '' : renderTransport()
    }<div class="clip-list">${rows}</div>`
  }
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
  // Search across the name, its humanised form, and category tags.
  const matches = clips.filter((c) => clipMatches(c.name, clipQuery))
  const search = `<input id="clip-search" class="clip-search" type="search" placeholder="Search animations…" value="${escape(
    clipQuery
  )}" aria-label="Search animations"/>`
  const list =
    matches.length === 0
      ? `<p style="color:var(--fg-2)">No animation matches “${escape(clipQuery)}”.</p>`
      : matches
          .map(
            (c) => `
        <button class="action template clip-item" data-clip="${escape(c.name)}"
                ${c.name === clip ? 'aria-current="true"' : ''}>
          <span class="clip-text"><span class="clip-name">${escape(c.name)}</span>
            <span class="clip-desc">${escape(clipDescription(c.name))}</span></span>
          <span class="clip-dur">${c.duration.toFixed(2)}s</span>
        </button>`
          )
          .join('')

  // 3-way view over the playing clip: mesh, skeleton, or both.
  const seg = (v: 'mesh' | 'skeleton' | 'both', label: string): string =>
    `<button class="seg-btn${animateView === v ? ' on' : ''}" data-view="${v}" aria-pressed="${
      animateView === v
    }">${label}</button>`
  const bones = `<div class="seg" role="group" aria-label="View">${seg('mesh', 'Mesh')}${seg(
    'skeleton',
    'Skeleton'
  )}${seg('both', 'Both')}</div>`

  // Playback controls sit directly under the preview, above the (long, scrolling)
  // clip list, so stop / forward / reverse stay reachable without scrolling.
  const controls = playing
    ? renderTransport()
    : clip !== null
      ? `<button id="preview" class="action">Play ${escape(clip)}</button>`
      : ''
  const caption =
    clip === null
      ? 'Click a clip to play it on your rig — it is retargeted, so the motion is moved, not copied (the library and the template do not share a rest pose).'
      : `${escape(clip)} will be written into the export.`
  return `${preview}${controls}${bones}${search}${list}<p style="color:var(--fg-2)">${caption}</p>`
}

/** The playback transport for the Animate step: fps, direction, pause, stop, and
 *  a scrubbable frame timeline. */
function renderTransport(): string {
  const frames = totalFrames(clipDuration, fps)
  const startF = Math.round(trimStart * frames)
  const endF = Math.round(trimEnd * frames)
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
    <div class="timecode"><span id="frame">0</span> / ${frames} frames</div>
    <div class="anim-ctl">
      <div class="anim-head"><span>Overdrive</span><span id="overdrive-val" class="anim-val">${overdrive}</span></div>
      <input id="overdrive" class="anim-slider" type="range" min="0" max="100" step="1" value="${overdrive}" aria-label="Overdrive (playback speed)"/>
    </div>
    <div class="anim-ctl">
      <div class="anim-head"><span>Character Arm-Space</span><span id="arm-space-val" class="anim-val">${armSpace}</span></div>
      <input id="arm-space" class="anim-slider" type="range" min="0" max="100" step="1" value="${armSpace}" aria-label="Character Arm-Space"/>
    </div>
    <div class="anim-ctl">
      <div class="anim-head"><span>Trim</span><span class="anim-sub">${frames} total frames</span></div>
      <div class="trim-range">
        <input id="trim-start" type="range" min="0" max="${frames}" step="1" value="${startF}" aria-label="Trim start (frame)"/>
        <input id="trim-end" type="range" min="0" max="${frames}" step="1" value="${endF}" aria-label="Trim end (frame)"/>
      </div>
      <div class="anim-ends"><span id="trim-start-val">${startF}</span><span id="trim-end-val">${endF}</span></div>
    </div>
    <label class="toggle"><input type="checkbox" id="mirror" ${
      mirrored ? 'checked' : ''
    }/> Mirror</label>`
}

/** Plays the chosen clip in the viewport, or stops it. */
async function runPreview(): Promise<void> {
  if (loaded === null || fitted === null || chosen === null || clip === null || playing) return
  playing = true
  paused = false
  direction = 1
  render()
  try {
    const glb = await previewAnimation(loaded.path, fitted, 2.0, chosen, clip, mirrored, armSpace)
    const duration = await ensureViewport().playAnimated(glb, clip)
    if (duration === null) {
      playing = false
      render()
      return
    }
    clipDuration = duration
    render() // the timeline max needs the real duration
    startPlayhead()
    ensureViewport().setPlaybackRate(playbackRate()) // carry Overdrive across clips
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

/** The signed playback rate: direction × the Overdrive speed (50 → 1×). */
function playbackRate(): number {
  return direction * (overdrive / 50)
}

/** Sets the play direction and resumes. */
function setDirection(value: 1 | -1): void {
  direction = value
  paused = false
  const vp = ensureViewport()
  vp.setPlaybackRate(playbackRate())
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
    // Keep playback inside the trim range, in whichever direction it runs.
    const startF = Math.round(trimStart * frames)
    const endF = Math.round(trimEnd * frames)
    if (endF > startF && !paused) {
      if (direction >= 0 && (frame >= endF || frame < startF)) {
        ensureViewport().seek(timeOfFrame(startF, fps))
      } else if (direction < 0 && (frame <= startF || frame > endF)) {
        ensureViewport().seek(timeOfFrame(endF, fps))
      }
    }
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

  const done =
    exported === null
      ? `<p style="color:var(--fg-2)">Mesh, skeleton and weights, in one file${
          clip === null ? '' : `, with ${escape(clip)}`
        }.</p>`
      : `<p style="color:var(--ok)">Wrote ${escape(exported)}.</p>`

  return `<button class="action primary" id="open-export" ${exporting ? 'disabled' : ''}>${
    exporting ? 'Writing\u2026' : 'Export\u2026'
  }</button>${done}${renderExportModal()}`
}

/** The Mixamo-style export-settings modal (a native <dialog>). */
function renderExportModal(): string {
  const opt = (value: string, label: string, on: boolean): string =>
    `<option value="${value}"${on ? ' selected' : ''}>${label}</option>`
  // "Without Skin" (skeleton + animation only) needs an animation to be useful.
  const noClip = clip === null
  const trimmed = trimStart > 0 || trimEnd < 1
  const trimNote =
    clip !== null && trimmed
      ? `<p class="export-note">Trim: frames ${Math.round(
          trimStart * totalFrames(clipDuration, exportFps)
        )}\u2013${Math.round(trimEnd * totalFrames(clipDuration, exportFps))} (from Animate) will be exported.</p>`
      : ''

  return `<dialog class="export-modal" id="export-modal">
    <h2>Export Settings</h2>
    <div class="export-grid">
      <div class="export-field">
        <label for="ex-format">Format</label>
        <select id="ex-format">
          ${opt('fbx', 'FBX Binary (.fbx)', exportFormat === 'fbx')}
          ${opt('glb', 'glTF Binary (.glb)', exportFormat === 'glb')}
        </select>
      </div>
      <div class="export-field">
        <label for="ex-skin">Skin</label>
        <select id="ex-skin" ${noClip ? 'disabled title="Pick an animation to export without skin"' : ''}>
          ${opt('with', 'With Skin', exportSkin)}
          ${opt('without', 'Without Skin', !exportSkin)}
        </select>
      </div>
      <div class="export-field">
        <label for="ex-fps">Frames per Second</label>
        <select id="ex-fps">
          ${opt('24', '24', exportFps === 24)}
          ${opt('30', '30', exportFps === 30)}
          ${opt('60', '60', exportFps === 60)}
        </select>
      </div>
      <div class="export-field">
        <label for="ex-keyframe">Keyframe Reduction</label>
        <select id="ex-keyframe">
          ${opt('none', 'none', keyframeReduction === 'none')}
          ${opt('low', 'low', keyframeReduction === 'low')}
          ${opt('high', 'high', keyframeReduction === 'high')}
        </select>
      </div>
    </div>
    ${trimNote}
    <div class="export-actions">
      <button class="action" id="ex-cancel">Cancel</button>
      <button class="action primary" id="ex-download">Export</button>
    </div>
  </dialog>`
}

/** Writes the rigged model to a file the user picks, with the modal's options. */
async function runExport(): Promise<void> {
  if (loaded === null || fitted === null || exporting) return
  exporting = true
  render()
  try {
    // "Without Skin" only applies with a clip; a bare-mesh export keeps its skin.
    const skin = clip === null ? true : exportSkin
    const saved = await exportModel(loaded.path, fitted, 2.0, exportFormat, chosen ?? '', clip, mirrored, armSpace, {
      skin,
      fps: exportFps,
      keyframe: REDUCTION_TOL[keyframeReduction],
      trimStart,
      trimEnd
    })
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

/** A template was chosen: place markers (its default flow) or, for a template
 *  with no marker set, fall straight through to automatic fitting. */
function chooseTemplate(name: string): void {
  if (markerSetFor(name) !== null) void enterMarkerMode(name)
  // Animals autofit with no markers; jump to the Fit step so the joint handles
  // (adjust for precision) and Bind (resolve) are surfaced, not hidden.
  else void runFit(name, true)
}

/** Enters the marker-placement flow for a template: clears any prior rig, shows
 *  the bare model, and arms the viewport to place markers on it. */
async function enterMarkerMode(name: string): Promise<void> {
  chosen = name
  markerMode = true
  markerPositions.clear()
  activeSlot = markerSetFor(name)?.[0]?.id ?? null
  // Drop any skeleton from a previous solve so only the bare mesh and the
  // markers show while placing (the "Re-place markers" case; a no-op on first
  // entry, when nothing is drawn yet).
  ensureViewport().clearFittedSkeleton()
  fitted = null
  bound = null
  activeStep = STEPS.findIndex((s) => s.id === StepId.EditSkeleton)
  furthestStep = Math.max(furthestStep, activeStep)
  render()
}

/** Sets a slot's marker to a point, mirroring to its pair when symmetry is on. */
function setMarkerAt(slotId: string, point: [number, number, number]): void {
  const set = markerSetFor(chosen ?? '')
  const slot = set?.find((s) => s.id === slotId)
  if (set === null || slot === undefined) return
  markerPositions.set(slot.id, point)
  if (useSymmetry && slot.pair !== undefined) {
    const mirror = 2 * ensureViewport().symmetryX() - point[0]
    markerPositions.set(slot.pair, [mirror, point[1], point[2]])
  }
}

/** Places the active marker from a click on the model, then advances to the
 *  next empty slot. A paired marker is routed to the L or R slot by which side
 *  of the model the click landed on, so the sides can't be placed swapped. */
function onMarkerPick(point: [number, number, number]): void {
  if (activeSlot === null) return
  const set = markerSetFor(chosen ?? '')
  const slot = set?.find((s) => s.id === activeSlot)
  const target =
    slot !== undefined
      ? slotForClickedSide(slot, point[0], ensureViewport().symmetryX())
      : activeSlot
  setMarkerAt(target, point)
  activeSlot = set?.find((s) => !markerPositions.has(s.id))?.id ?? null
  drawMarkers()
  render()
}

/** Moves an already-placed marker as it is dragged on the model. */
function onMarkerMove(id: string, point: [number, number, number]): void {
  setMarkerAt(id, point)
  drawMarkers()
}

/** Draws the placed markers in the viewport. */
function drawMarkers(): void {
  const set = markerSetFor(chosen ?? '')
  if (set === null) return
  ensureViewport().setMarkers(
    set
      .filter((s) => markerPositions.has(s.id))
      .map((s) => ({
        id: s.id,
        position: markerPositions.get(s.id) as [number, number, number],
        color: s.color
      }))
  )
}

/** Saves the placed markers, and which model they were placed on, as a JSON
 *  fixture in the repo's `e2e/` directory for regression testing. Shows a
 *  confirmation (or the error) under the button so a save is never in doubt. */
async function saveMarkers(): Promise<void> {
  const set = markerSetFor(chosen ?? '')
  if (set === null || chosen === null) return
  const byBone: Record<string, [number, number, number]> = {}
  for (const slot of set) {
    const p = markerPositions.get(slot.id)
    if (p !== undefined) byBone[slot.bone] = p
  }
  const state = {
    model: loaded?.name ?? null,
    modelPath: loaded?.path ?? null,
    template: chosen,
    markers: byBone
  }
  // Console too, mirrored to the dev terminal, as a redundant record.
  console.log(`[markers:${chosen}] ${JSON.stringify(byBone)}`)
  const base = (loaded?.name ?? chosen)
    .replace(/\.[^.]+$/, '')
    .replace(/[^a-zA-Z0-9_-]/g, '-')
  try {
    const path = await devSaveFixture(`${base}-markers`, JSON.stringify(state, null, 2))
    markerSaveStatus = `Saved ${Object.keys(byBone).length} markers → ${path}`
  } catch (err) {
    markerSaveStatus = `Save failed: ${err instanceof Error ? err.message : String(err)}`
  }
  render()
}

/** Solves the rig from the placed markers and draws the fitted skeleton. */
async function runMarkerFit(): Promise<void> {
  const set = chosen === null ? null : markerSetFor(chosen)
  if (chosen === null || set === null || loaded === null || fitting) return
  const path = loaded.path
  const markers: Marker[] = set
    .filter((s) => markerPositions.has(s.id))
    .map((s) => ({ bone: s.bone, position: markerPositions.get(s.id) as [number, number, number] }))
  if (markers.length < 2) return
  fitting = true
  render()
  try {
    const viewport = ensureViewport()
    viewport.endMarkerPlacement()
    markerMode = false
    fitted = await fitFromMarkers(chosen, markers, path)
    bound = null
    clips = null
    clip = null
    viewport.showFittedSkeleton(fitted.positions, fitted.parents, onJointEdited)
    furthestStep = Math.max(furthestStep, 3)
    record()
  } finally {
    fitting = false
    render()
  }
}

/** Places the chosen template's skeleton and draws it. */
async function runFit(name: string, advance = false): Promise<void> {
  if (loaded === null || fitting) return
  if (markerMode) {
    ensureViewport().endMarkerPlacement()
    markerMode = false
  }
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
    // Autofit from Choose-skeleton lands on the Fit step so its bone handles show.
    if (advance) activeStep = STEPS.findIndex((s) => s.id === StepId.EditSkeleton)
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
      // Kept so an already-rigged import can play its OWN clips without a retarget.
      modelBytes = geometry
      ownClips = false
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
    // Plain left-drag orbits in Animate (nothing to select there); elsewhere left
    // stays free for joint/marker picking.
    ensureViewport().setLeftOrbit(step.id === StepId.Animate)
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

  // Already-rigged import: skip fitting and animate its own rig, or re-rig.
  app.querySelector<HTMLButtonElement>('#play-own')?.addEventListener('click', () => proceedOwnClips())
  app.querySelector<HTMLButtonElement>('#proceed-rigged')?.addEventListener('click', () => void proceedRigged())
  app.querySelector<HTMLButtonElement>('#rerig')?.addEventListener('click', () => {
    activeStep = STEPS.findIndex((s) => s.id === StepId.LoadSkeleton)
    render()
  })

  const bindButton = app.querySelector<HTMLButtonElement>('#bind')
  bindButton?.addEventListener('click', () => void runBind())

  app.querySelector<HTMLButtonElement>('#paint')?.addEventListener('click', () => void runPaint())

  // Auto-fit again re-runs the automatic placement — handy after orienting or
  // moving the model on the grid.
  app
    .querySelector<HTMLButtonElement>('#refit')
    ?.addEventListener('click', () => {
      if (chosen !== null) void runFit(chosen)
    })

  // Marker-placement flow: ring selection, symmetry, solve / auto-fit.
  app.querySelectorAll<HTMLButtonElement>('.marker-ring').forEach((button) => {
    button.addEventListener('click', () => {
      activeSlot = button.dataset['slot'] ?? null
      render()
    })
  })
  app.querySelector<HTMLInputElement>('#symmetry')?.addEventListener('change', (event) => {
    useSymmetry = (event.target as HTMLInputElement).checked
  })
  app.querySelector<HTMLButtonElement>('#solve')?.addEventListener('click', () => void runMarkerFit())
  app.querySelector<HTMLButtonElement>('#save-markers')?.addEventListener('click', () => void saveMarkers())
  app.querySelector<HTMLButtonElement>('#autofit')?.addEventListener('click', () => {
    if (chosen !== null) void runFit(chosen)
  })
  app
    .querySelector<HTMLButtonElement>('#replace-markers')
    ?.addEventListener('click', () => {
      if (chosen !== null) void enterMarkerMode(chosen)
    })

  // While placing markers, keep the viewport armed to pick and the placed
  // markers drawn — render() re-runs this binding, and both calls are idempotent.
  if (step.id === StepId.EditSkeleton && markerMode) {
    ensureViewport().beginMarkerPlacement({ onPlace: onMarkerPick, onMove: onMarkerMove })
    drawMarkers()
  }
  // Editing the rig needs its bones visible again after the Animate step hid them.
  if (step.id === StepId.EditSkeleton && !markerMode) {
    ensureViewport().setSkeletonVisible(true)
  }

  // Download modal: open it, keep its selects in state, cancel or download.
  const bindSelect = (selector: string, set: (value: string) => void): void => {
    const el = app.querySelector<HTMLSelectElement>(selector)
    el?.addEventListener('change', () => set(el.value))
  }
  const exportModal = app.querySelector<HTMLDialogElement>('#export-modal')
  app.querySelector<HTMLButtonElement>('#open-export')?.addEventListener('click', () => exportModal?.showModal())
  app.querySelector<HTMLButtonElement>('#ex-cancel')?.addEventListener('click', () => exportModal?.close())
  bindSelect('#ex-format', (v) => { exportFormat = v === 'fbx' ? 'fbx' : 'glb' })
  bindSelect('#ex-skin', (v) => { exportSkin = v !== 'without' })
  bindSelect('#ex-fps', (v) => { exportFps = v === '24' ? 24 : v === '60' ? 60 : 30 })
  bindSelect('#ex-keyframe', (v) => {
    keyframeReduction = v === 'low' ? 'low' : v === 'high' ? 'high' : 'none'
  })
  app.querySelector<HTMLButtonElement>('#ex-download')?.addEventListener('click', () => {
    exportModal?.close()
    void runExport()
  })

  app.querySelector<HTMLButtonElement>('#preview')?.addEventListener('click', () => void runPreview())
  app.querySelectorAll<HTMLButtonElement>('[data-view]').forEach((button) => {
    const v = button.dataset['view']
    if (v !== 'mesh' && v !== 'skeleton' && v !== 'both') return
    button.addEventListener('click', () => {
      animateView = v
      ensureViewport().setAnimateView(v)
      render()
    })
  })
  app.querySelector<HTMLButtonElement>('#stop')?.addEventListener('click', () => stopPreview())
  app.querySelector<HTMLInputElement>('#overdrive')?.addEventListener('input', (event) => {
    overdrive = Number((event.target as HTMLInputElement).value)
    ensureViewport().setPlaybackRate(playbackRate())
    const val = document.querySelector<HTMLElement>('#overdrive-val')
    if (val !== null) val.textContent = String(overdrive)
  })
  const armInput = app.querySelector<HTMLInputElement>('#arm-space')
  armInput?.addEventListener('input', (event) => {
    armSpace = Number((event.target as HTMLInputElement).value)
    const val = document.querySelector<HTMLElement>('#arm-space-val')
    if (val !== null) val.textContent = String(armSpace)
  })
  // Baked on the Rust side, so re-retarget only when the drag settles.
  armInput?.addEventListener('change', () => {
    if (playing) {
      stopPreview()
      void runPreview()
    }
  })
  // Trim works in frames (like Mixamo's "N total frames"), stored as fractions.
  const trimFrames = (): number => Math.max(1, totalFrames(clipDuration, fps))
  const showTrim = (): void => {
    const f = trimFrames()
    const s = document.querySelector<HTMLElement>('#trim-start-val')
    const e = document.querySelector<HTMLElement>('#trim-end-val')
    if (s !== null) s.textContent = String(Math.round(trimStart * f))
    if (e !== null) e.textContent = String(Math.round(trimEnd * f))
  }
  app.querySelector<HTMLInputElement>('#trim-start')?.addEventListener('input', (event) => {
    const input = event.target as HTMLInputElement
    const f = trimFrames()
    trimStart = Number(input.value) / f
    if (trimStart > trimEnd) {
      trimStart = trimEnd
      input.value = String(Math.round(trimStart * f))
    }
    showTrim()
  })
  app.querySelector<HTMLInputElement>('#trim-end')?.addEventListener('input', (event) => {
    const input = event.target as HTMLInputElement
    const f = trimFrames()
    trimEnd = Number(input.value) / f
    if (trimEnd < trimStart) {
      trimEnd = trimStart
      input.value = String(Math.round(trimEnd * f))
    }
    showTrim()
  })
  // Live-filter the clip list as the query changes, without a re-render (which
  // would drop focus). A full re-render re-applies the same filter from clipQuery.
  app.querySelector<HTMLInputElement>('#clip-search')?.addEventListener('input', (event) => {
    clipQuery = (event.target as HTMLInputElement).value
    app.querySelectorAll<HTMLButtonElement>('.clip-item').forEach((btn) => {
      btn.hidden = !clipMatches(btn.dataset['clip'] ?? '', clipQuery)
    })
  })
  app.querySelector<HTMLInputElement>('#mirror')?.addEventListener('change', (event) => {
    mirrored = (event.target as HTMLInputElement).checked
    // Mirroring is baked on the Rust side, so re-retarget the clip to apply it.
    if (playing) {
      stopPreview()
      void runPreview()
    }
  })
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
    // Own-clips mode: play the imported model's OWN clip directly.
    const ownClipName = button.dataset['ownclip']
    if (ownClipName !== undefined) {
      button.addEventListener('click', () => {
        stopPreview()
        clip = ownClipName
        render()
        void playOwnClip()
      })
      return
    }
    if (clipName !== undefined) {
      // Hovering plays the clip in the chooser preview; clicking selects it.
      button.addEventListener('mouseenter', () => clipPreview?.play(clipName))
      button.addEventListener('focus', () => clipPreview?.play(clipName))
      button.addEventListener('click', () => {
        // Clicking a clip loads and plays it right away (Mixamo-style), stopping
        // any playback of the old one first.
        stopPreview()
        clip = clipName
        exported = null
        clipPreview?.play(clipName)
        record()
        void runPreview()
      })
      return
    }
    if (name === undefined) return
    button.addEventListener('click', () => chooseTemplate(name))
  })

  if (step.id === StepId.Animate) {
    ensureViewport().setSkeletonVisible(false)
    ensureViewport().setAnimateView(animateView)
    // Own-clips mode plays the model's own bytes; there is no template library.
    if (!ownClips) {
      void ensureClips()
      // Mount the persistent preview canvas and load the creature's library once.
      const slot = app.querySelector<HTMLElement>('#clip-preview')
      if (slot !== null && chosen !== null) {
        slot.appendChild(ensureClipPreview().canvas)
        void ensureLibrary(chosen)
      }
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

  // Testing: "Proceed to Animate" for an already-rigged import — read its own
  // rig, bind, jump to Animate, then optionally play a clip retargeted onto it.
  if (await devAutoproceed().catch(() => false)) {
    const clipName = await devAutoclip().catch(() => null)
    // A rigged, animated import plays its OWN clips; otherwise retarget our lib.
    if (loaded !== null && loaded.import.clips.length > 0) {
      ownClips = true
      clip = loaded.import.clips.find((n) => n === clipName) ?? loaded.import.clips[0] ?? null
      furthestStep = Math.max(furthestStep, STEPS.findIndex((s) => s.id === StepId.Animate))
      activeStep = STEPS.findIndex((s) => s.id === StepId.Animate)
      render()
      await playOwnClip()
      render()
    } else {
      await proceedRigged()
      if (clipName !== null && chosen !== null) {
        await ensureClips()
        clip = clips?.find((c) => c.name === clipName)?.name ?? clips?.[0]?.name ?? null
        await ensureLibrary(chosen)
        render()
        await runPreview()
        render()
      }
    }
    return
  }

  // Testing capture: open a template's marker step EMPTY so a person can place
  // the markers by hand, with a "Save markers" button to log them as a fixture.
  const captureTemplate = await devAutomarkCapture().catch(() => null)
  if (captureTemplate !== null) {
    markerCapture = true
    chooseTemplate(captureTemplate)
    // Self-test: place a couple of markers by synthetic clicks on the model and
    // save, to prove the place→save→fixture path end-to-end before a person is
    // asked to place them for real.
    if (await devCaptureSelftest().catch(() => false)) {
      const canvas = ensureViewport().canvas
      const rect = canvas.getBoundingClientRect()
      const drop = (fx: number, fy: number): void =>
        void canvas.dispatchEvent(
          new PointerEvent('pointerdown', {
            clientX: rect.left + rect.width * fx,
            clientY: rect.top + rect.height * fy,
            button: 0,
            bubbles: true
          })
        )
      drop(0.5, 0.35) // upper body — the first (chin) slot
      drop(0.5, 0.55) // lower body — the next slot advances automatically
      await saveMarkers()
    }
    return
  }

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

  // Optionally drive the marker-placement flow: seed the markers from the
  // auto-fit joints (a stand-in for clicking each one) and rest in placement
  // mode, so the marker Fit step can be screenshotted.
  if (await devAutomark().catch(() => false)) {
    const truth = new Map<string, [number, number, number]>()
    if (fitted !== null) {
      const positions = fitted.positions
      fitted.bones.forEach((bone, i) => {
        const p = positions[i]
        if (p !== undefined) truth.set(bone, [p[0], p[1], p[2]])
      })
    }
    await enterMarkerMode(template)
    for (const slot of markerSetFor(template) ?? []) {
      let at = truth.get(slot.bone)
      // The chin slot maps to the `head` bone, which sits at the forehead; the
      // chin/jaw is lower. For the guide render, drop it to jaw level (midway
      // between the head and neck bones) so the guide matches where to place it.
      if (slot.id === 'chin') {
        const head = truth.get('head')
        const neck = truth.get('neck_01')
        if (head !== undefined && neck !== undefined) {
          at = [head[0], (head[1] + neck[1]) / 2, head[2]]
        }
      }
      if (at !== undefined) markerPositions.set(slot.id, at)
    }
    activeSlot = null
    render()
    drawMarkers()
    // Optionally hover the model so the precision-preview loupe shows, for a
    // screenshot. A real OS hover can't be delivered into a WKWebView, so drive
    // it with a synthetic pointer-move at the model's chest.
    if (await devAutomarkHover().catch(() => false)) {
      const canvas = ensureViewport().canvas
      const rect = canvas.getBoundingClientRect()
      // Dead-centre lands on the torso for any full-body framing (a fixed
      // fraction lower down can miss the mesh when the model frames smaller).
      canvas.dispatchEvent(
        new PointerEvent('pointermove', {
          clientX: rect.left + rect.width / 2,
          clientY: rect.top + rect.height / 2,
          bubbles: true
        })
      )
    }
    // Optionally run the solve too, so the fitted skeleton can be screenshotted.
    if (await devAutomarkSolve().catch(() => false)) await runMarkerFit()
    return
  }

  // Optionally bind and show the weight-paint overlay, for the Bind step.
  if (await devAutopaint().catch(() => false)) {
    await runBind()
    activeStep = STEPS.findIndex((s) => s.id === StepId.BindWeights)
    render()
    await runPaint()
    render()
    return
  }

  // Optionally bind and preview a clip so the Animate step (retargeted preview +
  // clip thumbnails) can be screenshotted without clicking through.
  let autoclip: string | null = null
  try {
    autoclip = await devAutoclip()
  } catch {
    return
  }
  if (autoclip === null) return
  await runBind()
  await ensureClips()
  clip = clips?.find((c) => c.name === autoclip)?.name ?? clips?.[0]?.name ?? null
  activeStep = STEPS.findIndex((s) => s.id === StepId.Animate)
  // Dev/screenshot: preselect the 3-way view so the animated skeleton can be shot.
  const view = await devAnimateView().catch(() => null)
  if (view === 'mesh' || view === 'skeleton' || view === 'both') {
    animateView = view
    ensureViewport().setAnimateView(view)
  }
  mirrored = await devAnimateMirror().catch(() => false)
  const arm = await devAnimateArmSpace().catch(() => null)
  if (arm !== null && arm !== '') armSpace = Number(arm)
  render()
  await ensureLibrary(template)
  await runPreview()
  render()

  // Dev/testing: dispatch a synthetic left-drag so camera orbit in Animate can be
  // verified in a screenshot (a real OS drag can't reach the webview).
  if (await devAutoOrbit().catch(() => false)) {
    const canvas = ensureViewport().canvas
    const rect = canvas.getBoundingClientRect()
    const cx = rect.left + rect.width / 2
    const cy = rect.top + rect.height / 2
    const fire = (type: string, x: number, y: number, buttons: number): void => {
      const ev = new PointerEvent(type, {
        pointerId: 1,
        pointerType: 'mouse',
        button: 0,
        buttons,
        clientX: x,
        clientY: y,
        bubbles: true,
        cancelable: true
      })
      canvas.dispatchEvent(ev)
      window.dispatchEvent(ev)
    }
    fire('pointerdown', cx, cy, 1)
    for (let i = 1; i <= 8; i++) fire('pointermove', cx + i * 35, cy, 1)
    fire('pointerup', cx + 280, cy, 0)
    render()
  }

  // Dev/screenshot: jump to Export and open the Download modal so it can be shot.
  const autoexport = await devAutoexport().catch(() => null)
  if (autoexport !== null) {
    if (autoexport === 'without' || autoexport === 'with') exportSkin = autoexport === 'with'
    stopPreview() // settle the viewport so no playback re-render closes the modal
    activeStep = STEPS.findIndex((s) => s.id === StepId.Export)
    render()
    document.querySelector<HTMLDialogElement>('#export-modal')?.showModal()
  }
}
// Fire-and-forget at the entry module's end; nothing runs after it.
await maybeAutoload()
