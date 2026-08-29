# Design System & UX Methodology

Single source of truth for the interface. Implementation-ready.

## 1. Principle

**The user is an artist, not a rigger.** Every screen answers three questions
without being asked: *where am I*, *what do I do now*, *what happens if I'm wrong*.

Rigging is intimidating because tools show a skeleton and a mesh and expect you to
know. We show one task at a time, with the creature-specific guidance for the
template actually chosen.

## 2. Design tokens

```css
:root {
  /* ── surface ─────────────────────────── */
  --bg-0:        #0d0f12;   /* app chrome, deepest */
  --bg-1:        #14171c;   /* panels */
  --bg-2:        #1c2027;   /* raised cards, inputs */
  --bg-3:        #262b34;   /* hover */
  --viewport:    #0a0c0f;   /* 3D background — darkest, mesh must pop */

  /* ── line ────────────────────────────── */
  --border:      #2a3038;
  --border-soft: #21262d;
  --focus-ring:  #4a9eff;

  /* ── text ────────────────────────────── */
  --fg-0:        #e8eaed;   /* primary */
  --fg-1:        #a8b0ba;   /* secondary */
  --fg-2:        #6b7684;   /* tertiary, hints */
  --fg-dis:      #454d59;

  /* ── accent ──────────────────────────── */
  --accent:      #4a9eff;   /* primary action, selection */
  --accent-hi:   #6bb0ff;
  --accent-dim:  #1e3a5f;

  /* ── semantic ────────────────────────── */
  --ok:          #3fb950;
  --warn:        #d29922;
  --err:         #f85149;
  --info:        #58a6ff;

  /* ── rig-domain (consistent everywhere) ─ */
  --bone:        #ffb454;   /* bone display */
  --bone-sel:    #ffd899;
  --joint:       #ff8c42;
  --weight-lo:   #2b3a67;   /* weight paint ramp, low */
  --weight-hi:   #ff4757;   /* weight paint ramp, high */
  --mirror-axis: #b57edc;

  /* ── type ────────────────────────────── */
  --font: "Asta Sans", "42dot Sans", -apple-system, system-ui, sans-serif;
  --font-mono: ui-monospace, "SF Mono", monospace;
  --fs-xs: 11px;  --fs-sm: 12px;  --fs-md: 13px;
  --fs-lg: 15px;  --fs-xl: 19px;  --fs-2xl: 24px;
  --lh-tight: 1.25; --lh-body: 1.5;

  /* ── space (4px base) ────────────────── */
  --s-1: 4px;  --s-2: 8px;  --s-3: 12px; --s-4: 16px;
  --s-5: 24px; --s-6: 32px; --s-7: 48px;

  --radius: 6px;  --radius-lg: 10px;
  --shadow: 0 4px 16px rgb(0 0 0 / 0.4);
  --dur-fast: 120ms; --dur: 200ms; --ease: cubic-bezier(0.4, 0, 0.2, 1);
}
```

**Dark theme only** — matches Blender/Maya/Substance, and a bright UI beside a dark
viewport causes iris fatigue over a long session. Tokens are defined once on
`:root`; no component hardcodes a hex value.

## 3. Typography

**Asta Sans** (formerly 42dot Sans), variable, weight 300–800, SIL OFL 1.1.
Verified 2026-08-29: renamed Feb 2026 and **removed from Google Fonts** — the
`.woff2` files are vendored in `assets/fonts/`, **no CDN link**. An offline desktop
app must not depend on a font CDN.

| Role | Size | Weight |
|---|---|---|
| Step title | `--fs-xl` | 600 |
| Section header | `--fs-md` | 600, +0.02em tracking, uppercase |
| Body / label | `--fs-md` | 400 |
| Hint / helper | `--fs-sm` | 400, `--fg-2` |
| Numeric field | `--fs-sm` | 500, `--font-mono` (tabular alignment) |

## 4. Icons

**Lucide** exclusively, via `lucide` npm package, tree-shaken per-icon imports.
Never the full sprite sheet.

- Default 16px, stroke 1.5px, `currentColor`
- 20px for primary toolbar, 14px inline with text
- Every icon-only control has an `aria-label` **and** a tooltip. No exceptions.

Canonical mappings (keep stable — muscle memory is a feature):

| Action | Icon | Action | Icon |
|---|---|---|---|
| Import model | `upload` | Move tool | `move-3d` |
| Export | `download` | Rotate tool | `rotate-3d` |
| Skeleton | `bone` | Mirror | `flip-horizontal-2` |
| Bind / weight | `link` | Undo / redo | `undo-2` / `redo-2` |
| Play / pause | `play` / `pause` | Reset | `rotate-ccw` |
| Wireframe | `box` | Weight paint | `paintbrush` |
| Settings | `settings-2` | Help | `circle-help` |
| Blender bridge | `blend` | Warning | `triangle-alert` |

## 5. Layout

```
┌──────────────────────────────────────────────────────────────┐
│ Title bar (native, transparent)                              │
├────────────┬─────────────────────────────────┬───────────────┤
│            │                                 │               │
│  STEP RAIL │          VIEWPORT               │  INSPECTOR    │
│   200px    │           flex                  │    280px      │
│            │                                 │               │
│ ① Model  ✓ │   [3D scene]                    │  contextual   │
│ ② Skeleton✓│                                 │  properties   │
│ ③ Fit    ◉ │   ┌───────────────────────┐     │  for current  │
│ ④ Bind     │   │ floating tool bar     │     │  step         │
│ ⑤ Animate  │   └───────────────────────┘     │               │
│ ⑥ Export   │                                 │               │
│            ├─────────────────────────────────┤               │
│ ─────────  │  GUIDANCE STRIP                 │               │
│ [guidance] │  what to do now + why           │               │
├────────────┴─────────────────────────────────┴───────────────┤
│ Status bar · vertex count · solve time · memory · bridge     │
└──────────────────────────────────────────────────────────────┘
```

Inspector and step rail are collapsible. Viewport never shrinks below 640px wide.

## 6. Information architecture — the six steps

Ported from the legacy `ProcessStep` flow, which is sound. Each step declares:
**goal · required input · success condition · what "wrong" looks like.**

| # | Step | Success condition | Failure signal shown |
|---|---|---|---|
| 1 | Import model | mesh loaded, analysis clean | scale/normals/manifold warnings, each with a fix |
| 2 | Choose skeleton | template chosen | creature-shape hint if mesh proportions mismatch template |
| 3 | Fit skeleton | bones inside mesh | live out-of-mesh bone highlight in `--err` |
| 4 | Bind weights | weights solved | per-vertex weight paint overlay + auto-flagged bad regions |
| 5 | Animate | clips previewing | retarget mismatch report |
| 6 | Export | file written | format compatibility notes per target DCC |

**Steps 3 and 4 are where users fail.** They get the most guidance, the most
visual feedback, and full undo.

## 7. Creature-aware guidance

The differentiator versus Mixamo. Guidance copy, landmark names, and reference
imagery are **per template**, not generic.

- Human → "place the hips at the pelvis, roughly at navel height"
- Bird → "the wing chain runs shoulder → elbow → wrist → primaries; keep the keel bone forward of the hips"
- Fish → "the spine chain should follow the lateral line; the tail needs at least 4 segments to swim convincingly"
- Quadruped → "the front leg attaches at the scapula, higher than it looks; the hock bends backwards"

Each ships with a labelled diagram in the guidance strip. This content lives with
the template definition (`instruction.md` §6), not hardcoded in UI components.

## 8. States — every interactive element defines all seven

`default · hover · active · focus-visible · disabled · loading · error`

- Focus ring `--focus-ring`, 2px offset, **never** `outline: none`
- Disabled: `--fg-dis`, `cursor: not-allowed`, tooltip explaining *why*
- Loading: skeleton shimmer for content, inline spinner for actions
- Error: `--err` border + message below, never a bare red field

## 9. Feedback and system status

| Duration | Pattern |
|---|---|
| < 100 ms | nothing — instant |
| 100 ms – 1 s | inline spinner on the trigger |
| 1 s – 10 s | determinate progress bar + stage label ("voxelising… 40%") |
| > 10 s | progress + cancel + time estimate |

Solve time and peak memory are always visible in the status bar. Users trust a tool
that tells them what it costs.

## 10. Accessibility (non-negotiable — see `philosophy.md`)

- Full keyboard operation; visible focus order follows the step flow
- Text contrast ≥ 4.5:1, UI chrome ≥ 3:1 (tokens above are checked)
- **Never colour alone** — weight-paint issues get an icon and a count, not just red
- `prefers-reduced-motion` disables all transitions
- Semantic HTML; ARIA only where semantics run out
- Minimum hit target 32×32 px

## 11. HCI heuristics applied

| Heuristic | Applied as |
|---|---|
| Visibility of status | step rail progress, status bar, live solve timing |
| Match to real world | anatomical bone names, not `Bone.017` |
| User control | undo/redo at every step; every step is re-enterable |
| Consistency | one icon per concept, one colour per rig concept, everywhere |
| Error prevention | validate on import; warn before destructive re-bind |
| Recognition over recall | template thumbnails, not a dropdown of names |
| Flexibility | keyboard shortcuts mirroring Blender where sensible |
| Minimalist design | inspector shows only the current step's properties |
| Recover from errors | plain-language errors naming the fix, never a stack trace |
| Help | contextual guidance strip, no separate manual required |
