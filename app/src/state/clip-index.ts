/**
 * A searchable index over animation clips.
 *
 * The libraries only carry a clip's NAME (e.g. `Crouch_Walk`, `Chop_Tree`). To
 * make them searchable by more than an exact label — and readable at a glance —
 * this derives a human-readable description and a set of category tags from the
 * name alone, so it works for the human library and every animal library without
 * a hand-authored table per clip.
 */

/** Category keyword → tag, matched against the humanised name. Order-independent;
 *  a clip can carry several tags (a "Crouch_Walk" is both traversal and
 *  locomotion), which is what makes a broad search like "walk" or "sneak" work. */
const CATEGORIES: ReadonlyArray<readonly [RegExp, string]> = [
  [/\b(idle|stand|breath|wait)\b/i, 'idle'],
  [/\b(walk|run|jog|sprint|march|strafe|step|locomot)/i, 'locomotion'],
  [/\b(jump|leap|hop|vault)/i, 'jump'],
  [/\b(attack|punch|kick|hit|slash|stab|combat|fight|shoot|gun|sword|bow|throw|block|dodge)/i, 'combat'],
  [/\b(death|die|dead|defeat|fall|ko)\b/i, 'death'],
  [/\b(dance|celebrat|cheer|wave|clap|salute|taunt|bow|laugh|gesture)/i, 'expression'],
  [/\b(crouch|sneak|climb|crawl|roll|swim|hang|balance)/i, 'traversal'],
  [/\b(sit|kneel|lie|rest|sleep|pose|t.?pose)/i, 'pose'],
  [/\b(eat|drink|consume|farm|chop|dig|mine|fish|plant|water|harvest|carry|pick|work|build)/i, 'action'],
  [/\b(react|hurt|flinch|stun|impact|stagger)/i, 'reaction'],
  [/\b(turn|look|aim|point|inspect)/i, 'look']
]

/** `Crouch_Walk_RM` → `Crouch Walk RM`; `ClimbUp1m` → `Climb Up 1m`. */
export function humanizeClipName(name: string): string {
  return name
    .replace(/[_-]+/g, ' ')
    .replace(/([a-z])([A-Z])/g, '$1 $2')
    .replace(/([A-Za-z])(\d)/g, '$1 $2')
    .replace(/\s+/g, ' ')
    .trim()
}

/** The category tags a clip's name matches (may be empty). */
export function clipTags(name: string): string[] {
  const humanized = humanizeClipName(name)
  const tags: string[] = []
  for (const [pattern, tag] of CATEGORIES) {
    if (pattern.test(humanized) && !tags.includes(tag)) tags.push(tag)
  }
  return tags
}

/** A short description shown beside the label: the humanised name, plus any
 *  categories so the clip's kind reads without playing it. */
export function clipDescription(name: string): string {
  const tags = clipTags(name)
  return tags.length > 0 ? tags.join(' · ') : humanizeClipName(name)
}

/** Whether a clip matches a search query — every whitespace-separated term must
 *  appear in the name, its humanised form, or its tags (so "sneak", "crouch" and
 *  "traversal" all find a crouch-walk). Empty query matches everything. */
export function clipMatches(name: string, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (q === '') return true
  const haystack = `${name} ${humanizeClipName(name)} ${clipTags(name).join(' ')}`.toLowerCase()
  return q.split(/\s+/).every((term) => haystack.includes(term))
}
