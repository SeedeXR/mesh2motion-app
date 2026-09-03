/**
 * A linear undo/redo history over immutable snapshots.
 *
 * The app records a snapshot of its meaningful state after each change the user
 * would expect to undo — a template chosen, a joint dragged, weights bound, a
 * clip picked. `undo`/`redo` walk a pointer over the recorded snapshots; a fresh
 * `push` after an undo drops the redo future, which is what every editor does.
 *
 * Snapshots are held by reference and never mutated, so the app must reassign
 * its state immutably (build a new object rather than edit in place) — which it
 * already does.
 */
export class History<T> {
  // The reference never changes — the array is mutated in place (push, length) —
  // so it is readonly even though its contents are not.
  private readonly states: T[] = []
  /** Index of the current state in `states`, or -1 when empty. */
  private index = -1

  /** Records a new current state, discarding any redo future beyond it. */
  push(state: T): void {
    // Drop anything the user had redone past — a new action forks the timeline.
    this.states.length = this.index + 1
    this.states.push(state)
    this.index = this.states.length - 1
  }

  /** Steps back and returns the previous state, or `null` at the beginning. */
  undo(): T | null {
    if (this.index <= 0) return null
    this.index -= 1
    return this.states[this.index] ?? null
  }

  /** Steps forward and returns the next state, or `null` at the end. */
  redo(): T | null {
    if (this.index >= this.states.length - 1) return null
    this.index += 1
    return this.states[this.index] ?? null
  }

  canUndo(): boolean {
    return this.index > 0
  }

  canRedo(): boolean {
    return this.index < this.states.length - 1
  }
}
