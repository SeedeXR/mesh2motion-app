import { describe, it, expect } from 'vitest'
import { MARKER_SETS, markerSetFor, slotForClickedSide } from '../src/state/markers'

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

  it('routes a paired click to the slot on the side it was clicked', () => {
    const human = markerSetFor('human')!
    const wristL = human.find((s) => s.id === 'wrist_l')!
    const wristR = human.find((s) => s.id === 'wrist_r')!
    const chin = human.find((s) => s.id === 'chin')!
    const centre = 0

    // The model's left is +X: a left-side click fills wrist_l whichever slot
    // (L or R) was active, and a right-side click fills wrist_r either way.
    expect(slotForClickedSide(wristL, 0.5, centre)).toBe('wrist_l')
    expect(slotForClickedSide(wristR, 0.5, centre)).toBe('wrist_l')
    expect(slotForClickedSide(wristR, -0.5, centre)).toBe('wrist_r')
    expect(slotForClickedSide(wristL, -0.5, centre)).toBe('wrist_r')
    // Off-origin centre: side is relative to the model, not world zero.
    expect(slotForClickedSide(wristR, 9.5, 10)).toBe('wrist_r')
    // A midline marker always fills itself.
    expect(slotForClickedSide(chin, 0.5, centre)).toBe('chin')
    expect(slotForClickedSide(chin, -0.5, centre)).toBe('chin')
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
