import { describe, it, expect } from 'vitest'
import { humanizeClipName, clipTags, clipDescription, clipMatches } from '../src/state/clip-index'

describe('clip index', () => {
  it('humanises names from underscores, camelCase and digit runs', () => {
    expect(humanizeClipName('Crouch_Walk')).toBe('Crouch Walk')
    expect(humanizeClipName('ClimbUp1m_RM')).toBe('Climb Up 1m RM')
    expect(humanizeClipName('Chest_Open')).toBe('Chest Open')
  })

  it('tags clips by category, and a clip can carry several', () => {
    expect(clipTags('Crouch_Walk')).toEqual(expect.arrayContaining(['traversal', 'locomotion']))
    expect(clipTags('Death_D')).toContain('death')
    expect(clipTags('Dance_Simple')).toContain('expression')
    expect(clipTags('Sword_Attack')).toContain('combat')
  })

  it('search matches name, humanised form and tags; every term must hit', () => {
    expect(clipMatches('Crouch_Walk', 'traversal')).toBe(true) // via the category tag
    expect(clipMatches('Crouch_Walk', 'walk')).toBe(true) // via the name
    expect(clipMatches('Crouch_Walk', 'crouch walk')).toBe(true) // both terms
    expect(clipMatches('Crouch_Walk', 'combat')).toBe(false)
    expect(clipMatches('Anything', '')).toBe(true) // empty query matches all
  })

  it('describes a clip by its categories, falling back to the name', () => {
    expect(clipDescription('Sword_Attack')).toContain('combat')
    // A name with no category keyword falls back to the humanised name.
    expect(clipDescription('Zxq_Foo')).toBe('Zxq Foo')
  })
})
