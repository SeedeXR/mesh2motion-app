import { describe, expect, test } from 'vitest'
import { History } from '../src/state/history'

describe('History', () => {
  test('a fresh history has nothing to undo or redo', () => {
    const h = new History<number>()
    expect(h.canUndo()).toBe(false)
    expect(h.canRedo()).toBe(false)
    expect(h.undo()).toBeNull()
    expect(h.redo()).toBeNull()
  })

  test('undo walks back through pushed states and redo walks forward', () => {
    const h = new History<number>()
    h.push(1)
    h.push(2)
    h.push(3)
    expect(h.canRedo()).toBe(false)
    expect(h.undo()).toBe(2)
    expect(h.undo()).toBe(1)
    expect(h.canUndo()).toBe(false)
    expect(h.redo()).toBe(2)
    expect(h.redo()).toBe(3)
    expect(h.canRedo()).toBe(false)
  })

  test('the first state cannot be undone past', () => {
    const h = new History<number>()
    h.push(1)
    expect(h.canUndo()).toBe(false)
    expect(h.undo()).toBeNull()
  })

  test('a push after an undo forks the timeline, dropping the redo future', () => {
    const h = new History<string>()
    h.push('a')
    h.push('b')
    h.push('c')
    expect(h.undo()).toBe('b') // now sitting on b, with c ahead
    h.push('d') // forks: c is gone
    expect(h.canRedo()).toBe(false)
    expect(h.undo()).toBe('b')
    expect(h.redo()).toBe('d')
  })

  test('snapshots are returned by identity, not copied', () => {
    const h = new History<{ v: number }>()
    const a = { v: 1 }
    const b = { v: 2 }
    h.push(a)
    h.push(b)
    expect(h.undo()).toBe(a)
  })
})
