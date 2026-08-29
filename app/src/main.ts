import './ui/tokens.css'
import './ui/shell.css'

import { createIcons, Bone, Upload, Link, Play, Download, Move3d } from 'lucide'
import { STEPS, type StepDef } from './state/steps'
import { buildInfo, isDesktop, reportStartup } from './ipc'
import { detectBackend } from './viewport/backend'

/** Index of the step the user is currently on. */
// ponytail: nothing advances activeStep yet — each step gains its own completion
// gate in P3-6. Backwards navigation works; forwards is deliberately absent
// rather than fake.
let activeStep = 0

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
            <div style="color:var(--fg-1);font-size:var(--fs-lg)">No model loaded</div>
            <div>Import a mesh to begin</div>
          </div>
        </div>
        <div class="guidance">${renderGuidance(step)}</div>
      </main>

      <aside class="inspector" aria-label="Properties">
        <h2>${step.label}</h2>
        <p style="color:var(--fg-2)">Properties for this step appear here.</p>
      </aside>

      <div class="status" role="status">
        <span id="env">—</span>
        <span class="spacer"></span>
        <span>0 verts</span>
        <span>— ms</span>
      </div>
    </div>`

  createIcons({ icons: { Bone, Upload, Link, Play, Download, Move3d } })

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
