# Engineering Philosophy

## Prime directive

Build a rigging tool a 3D artist reaches for *because it is faster and better than
the alternative*, not because it is free. Every decision serves that.

## The ladder — choosing a solution

After comprehension, stop at the first rung that holds:

1. **Does this need to exist at all?** Speculative need = skip it, say so in one line.
2. **Already in this codebase?** A helper, type, or pattern that already lives here — reuse it. Re-implementing what sits three files over is the most common form of slop. `legacy/` counts as this codebase.
3. **Stdlib does it?** Use it.
4. **Native platform feature covers it?** Metal/Accelerate over hand-rolled SIMD. CSS over JS. `<input type=range>` over a custom slider.
5. **Already-installed dependency solves it?** Use it. Never add a crate for what twenty lines do.
6. **Can it be one line?** One line.
7. **Only then:** the minimum code that works.

Two rungs work → take the higher one and move on.

## Never lazy about understanding

The ladder shortens the *writing*, never the *reading*. Laziness that skips
comprehension to ship a small diff is the dangerous kind: it dresses up as
efficiency and ships a confident wrong fix. Read fully, then be lazy.

## Root cause, not symptom

A report names a symptom. Before editing, find every caller of what you are about
to change. The fix belongs in the shared path all callers route through — which is
also usually the smaller diff. Patching only the path the ticket names leaves every
sibling caller broken.

## Native optimisation, honestly

Apple Silicon is a **unified-memory** machine. The wins, in order of actual payoff:

1. **Don't copy.** Zero-copy buffer handoff between Rust and the webview beats any
   micro-optimisation inside either side.
2. **Don't allocate in the hot loop.** Pre-size, reuse, arena where it helps.
3. **Parallelise wide before optimising narrow.** `rayon` across vertices first;
   SIMD inside the kernel second.
4. **Measure before and after, on the target machine.** M4, 10 cores, 16 GB.
5. Only then reach for Metal compute or `Accelerate`.

An optimisation without a before/after measurement on real input is a guess wearing
a lab coat.

## Resource budgets are features

The app competes with Blender and Maya on a laptop. Budgets (see `test.md` for
enforcement):

- Idle RSS ≤ 250 MB
- Rigging a 50k-vertex mesh: ≤ 1.5 GB peak, ≤ 3 s wall
- Idle CPU ≈ 0% (no polling render loop when nothing moves)
- Binary ≤ 40 MB

Exceeding a budget is a bug, not a tradeoff, unless explicitly renegotiated in
`todo.md` with a reason.

## Hardware and the physical world

Real meshes are not ideal meshes. They are non-watertight, have inverted normals,
duplicate vertices, degenerate triangles, disconnected islands, and scale in
centimetres when you assumed metres. **Every algorithm ships with its robustness
story stated**, and the guard is written before the happy path is celebrated.

Leave the calibration knob. An artist needs to nudge a result the model cannot see.

## Clean architecture

- Core algorithms are **pure functions over plain data**. No Three.js types, no
  Tauri types, no I/O inside `m2m-core`. This is what makes them testable and
  benchmarkable.
- I/O lives at the edges (`m2m-io`).
- The UI is a view over state, never the owner of algorithmic truth.
- A module you cannot unit-test without a window on screen is designed wrong.

## Deliberate shortcuts

Marked in-code with a `ponytail:` comment naming the ceiling and the upgrade path:

```rust
// ponytail: O(n²) nearest-bone scan; fine to 100k verts, swap in the BVH if profiling says so
```

Untracked shortcuts rot into "later means never". `/ponytail:ponytail-debt`
harvests them into a ledger.

## Never simplify away

Input validation at trust boundaries. Error handling that prevents data loss.
Security measures. Accessibility basics. Anything explicitly requested.
