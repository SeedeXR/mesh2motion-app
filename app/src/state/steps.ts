/**
 * The six-step rigging flow.
 *
 * Ported from `legacy/src/lib/enums/ProcessStep.ts`, which is sound — the step
 * sequence is the part of the legacy UX that works. Each step declares its own
 * guidance so the guidance strip is data-driven rather than a switch statement
 * (see memory/design.md §6-7).
 */

export const enum StepId {
  LoadModel = 'load-model',
  LoadSkeleton = 'load-skeleton',
  EditSkeleton = 'edit-skeleton',
  BindWeights = 'bind-weights',
  Animate = 'animate',
  Export = 'export'
}

export interface StepDef {
  readonly id: StepId
  readonly label: string
  /** Lucide icon name shown in the guidance strip. */
  readonly icon: string
  /** What the user must do here, in artist language. */
  readonly goal: string
  /** How the user knows this step succeeded. */
  readonly success: string
}

export const STEPS: readonly StepDef[] = [
  {
    id: StepId.LoadModel,
    label: 'Import model',
    icon: 'upload',
    goal: 'Bring in a mesh as GLB, glTF or FBX.',
    success: 'The mesh appears in the viewport and the analysis reports no blocking problems.'
  },
  {
    id: StepId.LoadSkeleton,
    label: 'Choose skeleton',
    icon: 'bone',
    goal: 'Pick the creature template closest to your mesh.',
    success: 'A preview skeleton appears roughly matching your model’s proportions.'
  },
  {
    id: StepId.EditSkeleton,
    label: 'Fit skeleton',
    icon: 'move-3d',
    goal: 'Move the bones so they sit inside the mesh.',
    success: 'No bone is highlighted as sitting outside the mesh surface.'
  },
  {
    id: StepId.BindWeights,
    label: 'Bind weights',
    icon: 'link',
    goal: 'Solve which bones deform which parts of the mesh.',
    success: 'The weight overlay shows smooth transitions with no flagged regions.'
  },
  {
    id: StepId.Animate,
    label: 'Animate',
    icon: 'play',
    goal: 'Preview animation clips retargeted onto your rig.',
    success: 'Clips play back without limbs detaching or collapsing.'
  },
  {
    id: StepId.Export,
    label: 'Export',
    icon: 'download',
    goal: 'Write the rigged, animated model to GLB or FBX.',
    success: 'The exported file opens in your target application with the skeleton intact.'
  }
] as const
