import { describe, it, expect } from 'vitest'
import { MARKER_SETS, markerSetFor } from '../src/state/markers'

describe('marker sets', () => {
  it('offers the human Mixamo set and nothing for an unknown creature', () => {
    expect(markerSetFor('human')).not.toBeNull()
    expect(markerSetFor('nonesuch')).toBeNull()
    expect(markerSetFor('human')).toHaveLength(8)
  })

  it('pairs sided markers reciprocally and leaves midline ones unpaired', () => {
    for (const slots of Object.values(MARKER_SETS)) {
      const byId = new Map(slots.map((s) => [s.id, s]))
      for (const slot of slots) {
        if (slot.side === undefined) {
          expect(slot.pair, `${slot.id} is midline`).toBeUndefined()
          continue
        }
        // A sided marker names a pair, on the opposite side, that names it back.
        const pair = slot.pair === undefined ? undefined : byId.get(slot.pair)
        expect(pair, `${slot.id} pair`).toBeDefined()
        expect(pair?.side).not.toBe(slot.side)
        expect(pair?.pair).toBe(slot.id)
      }
    }
  })

  it('gives every slot a bone and a colour', () => {
    for (const slots of Object.values(MARKER_SETS)) {
      for (const slot of slots) {
        expect(slot.bone.length).toBeGreaterThan(0)
        expect(slot.color).toBeGreaterThanOrEqual(0)
      }
    }
  })
})
