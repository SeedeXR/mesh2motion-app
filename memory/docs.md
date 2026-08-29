# Documentation Standards

## 1. Principle

Documentation is written for the person who arrives with no context — including
future agents and future you. **Prose that restates the code is worse than no
prose**; it rots and then lies. Document the things code cannot say: why, what
breaks, what was rejected.

## 2. What gets documented, and where

| Content | Location | Trigger to update |
|---|---|---|
| Why a design is the way it is | `memory/architecture.md` ADR table | any architectural decision |
| System map, module relations | `memory/mindmap.md` | any new module or dependency |
| Original implementation | `memory/porting.md` | when a legacy area is first studied |
| How to build/run/test | `README.md` | any change to the toolchain |
| Public API surface | rustdoc / TSDoc **in source** | every public item |
| Algorithms + citations | `docs/algorithms/<name>.md` | every solver change |
| Research findings | `docs/research/<topic>.md` | every research task |
| User-facing help | in-app guidance strip + `docs/user/` | every UX change |
| Session log | `memory/handover_session.md` | every session |
| Changelog | `CHANGELOG.md` | every release |

## 3. Rules

1. **Cite or don't claim.** Any statement about behaviour cites `file:line`, a
   command and its output, or a paper with authors + year + identifier.
2. **Version-aware.** Docs mentioning a dependency name its version. "Uses `glam`"
   is useless; "`glam` 0.29, chosen for SIMD `Mat4`" is not.
3. **No orphan docs.** Every file under `docs/` is linked from `README.md` or
   `mindmap.md`. Unlinked docs are deleted.
4. **Update in the same commit as the code.** A doc-update-later commit is a lie
   with a delay.
5. **Delete stale docs.** A wrong doc costs more than a missing one.
6. **Diagrams for anything with more than three moving parts** — see §5.

## 4. Algorithm documentation template

Every file in `docs/algorithms/` follows this. Non-negotiable for a project whose
value is its algorithms:

```markdown
# <Algorithm name>

## Problem
What it solves, in one paragraph, in artist terms.

## Approach
The method, with the key equations.

## Citation
Authors, title, venue, year, DOI/arXiv. Note deviations from the paper and why.

## Complexity
Time and space, in n = vertices, b = bones, v = voxels.

## Robustness
What malformed input does to it. Non-watertight? Self-intersecting? Zero-area
triangles? Disconnected islands? State each explicitly.

## Parameters
Every tunable: name, range, default, and what it visibly changes.

## Measured performance
Machine, input, run count, result. Numbers without provenance are not results.

## Rejected alternatives
What else was considered and the specific reason it lost.
```

## 5. Diagrams

**Mermaid**, inline in markdown — renders on GitHub, diffs as text, no binary blobs.

| Use | Type |
|---|---|
| System / module structure | `flowchart TB` |
| Data flow through a pipeline | `flowchart LR` |
| Process/step state machine | `stateDiagram-v2` |
| IPC or bridge protocol | `sequenceDiagram` |
| Build/release pipeline | `flowchart LR` |
| Data model | `erDiagram` |

Rules: label every edge that carries data with *what* it carries. Keep one diagram
to one idea — two small diagrams beat one unreadable one. Diagrams live next to the
prose they explain, never in a gallery.

## 6. Code examples

Every public API doc carries a compiling example. Rust examples are doctests
(`cargo test --doc` runs them in CI) so they cannot rot silently. TypeScript
examples come from real code in `app/src/`, not invented.

## 7. Changelog

Keep-a-Changelog format, semver. Every user-visible change gets a line under
`Added` / `Changed` / `Fixed` / `Removed`, written in artist language:

> Fixed: finger bones no longer steal weights from the palm on hands modelled in a
> closed fist.

not

> Fixed: `ExtremityWeightCorrector` threshold off-by-one.

## 8. Research notes

Every research task writes `docs/research/<topic>.md` **before** implementation:

- The question asked
- Sources consulted with full URLs and access date
- What the sources actually say (quoted where precise wording matters)
- What was chosen and why
- What was rejected and why
- Open questions

This is what keeps "deep research" from decaying into "I recall reading". It is
also the primary defence against hallucinated citations.

## 9. Contributor-facing

`README.md` must let a stranger go from clone to running app in under five minutes:
prerequisites with versions, clone, install, run, test, project layout, where to
start reading. If a step fails for a new contributor, that is a `README` bug.
