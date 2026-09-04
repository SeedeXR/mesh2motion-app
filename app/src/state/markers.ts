/**
 * The marker sets for the marker-placement rig flow.
 *
 * A marker is a joint a person drops on their model; the solver
 * (`m2m_rig::fit::fit_from_markers`) fits the template to those joints. The set
 * is the Mixamo one — chin, wrists, elbows, knees, groin — mapped to the
 * template's own bone names, so the app carries no anatomy knowledge beyond this
 * table.
 *
 * Only creatures with a set here offer marker placement; the rest fall back to
 * automatic fitting. Human is the one that ships today.
 */

export interface MarkerSlot {
  /** Stable id, e.g. `wrist_l`. */
  readonly id: string
  /** Shown in the panel, e.g. `Wrist L`. */
  readonly label: string
  /** The template bone this marker pins. */
  readonly bone: string
  /** Which side, for the symmetry mirror. Absent for midline markers. */
  readonly side?: 'l' | 'r'
  /** The paired slot on the other side, for symmetry. */
  readonly pair?: string
  /** Marker colour (Mixamo's palette: chin cyan … groin pink). */
  readonly color: number
}

export const MARKER_SETS: Readonly<Record<string, readonly MarkerSlot[]>> = {
  human: [
    { id: 'chin', label: 'Chin', bone: 'head', color: 0x38bdf8 },
    { id: 'wrist_l', label: 'Wrist L', bone: 'hand_l', side: 'l', pair: 'wrist_r', color: 0xa3e635 },
    { id: 'wrist_r', label: 'Wrist R', bone: 'hand_r', side: 'r', pair: 'wrist_l', color: 0xa3e635 },
    { id: 'elbow_l', label: 'Elbow L', bone: 'lowerarm_l', side: 'l', pair: 'elbow_r', color: 0xfacc15 },
    { id: 'elbow_r', label: 'Elbow R', bone: 'lowerarm_r', side: 'r', pair: 'elbow_l', color: 0xfacc15 },
    { id: 'knee_l', label: 'Knee L', bone: 'calf_l', side: 'l', pair: 'knee_r', color: 0xfb923c },
    { id: 'knee_r', label: 'Knee R', bone: 'calf_r', side: 'r', pair: 'knee_l', color: 0xfb923c },
    { id: 'groin', label: 'Groin', bone: 'pelvis', color: 0xf472b6 }
  ]
}

/** The marker slots for a template, or `null` when it has no set (auto-fit only). */
export function markerSetFor(template: string): readonly MarkerSlot[] | null {
  return MARKER_SETS[template] ?? null
}
