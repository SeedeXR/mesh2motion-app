# Session Start Protocol

**MANDATORY. Read this file first, every session, before any other action.**

## 1. Required reading order

| # | File | Why |
|---|---|---|
| 1 | `memory/session_start.md` | this file |
| 2 | `memory/agent_profile.md` | who you are operating as |
| 3 | `memory/philosophy.md` | the engineering values that break ties |
| 4 | `memory/project_context.md` | what we are building and for whom |
| 5 | `memory/handover_session.md` | **last session's tail — read the last 2 entries minimum** |
| 6 | `memory/todo.md` | pick the next unchecked task from here |
| 7 | `memory/architecture.md` | before touching any module boundary |
| 8 | `memory/instruction.md` | before writing code |
| 9 | `memory/test.md` | before claiming anything works |
| 10 | `memory/design.md` | before touching UI |

`mindmap.md`, `porting.md`, `docs.md` are consulted on demand, not every session.

## 2. Reconcile memory against live state

Memory is point-in-time. State is truth. Before trusting any claim in these files:

```bash
git log --oneline -10          # what actually landed
git status --porcelain         # uncommitted drift
cargo metadata --no-deps 2>/dev/null | head -1   # workspace still parses
ls -d target 2>/dev/null && du -sh target        # disk guard (see §5)
```

If `handover_session.md` says something merged and `git log` disagrees, **git wins** — correct the memory file in the same session.

## 3. Operating mindset

- **Zero hallucination.** Every claim about code, an API, a crate version, or system state must be observed this session (file read, command run) or explicitly labelled unverified. Cite `file:line` or the command.
- **Three registers, never blurred:** observed / inferred / assumed. Say which.
- Never quote a crate or Three.js API from memory. Check `Cargo.lock`, `node_modules`, or `--help`.
- "I don't know yet — checking" beats a plausible guess, every time.
- A wrong answer delivered confidently costs more than the three tool calls that would have prevented it.

## 4. Token efficiency

- Read the specific range you need (`sed -n '40,90p'`), not whole files.
- Prefer `rg` over reading directories.
- Do not re-derive facts already recorded in `handover_session.md` this session.
- Do not restate the plan back to the user; act on it.
- Batch independent tool calls into one message.

## 5. Disk guard (hard constraint)

The dev machine has **~34 GB free**. Rust `target/` grows without bound across incremental builds.

```bash
# run at session start AND before any full rebuild
SZ=$(du -sm target 2>/dev/null | cut -f1); [ "${SZ:-0}" -gt 8000 ] && cargo clean
```

Threshold: **8 GB**. Over it, `cargo clean` and recompile. Never let `target/` push free space below 10 GB.

## 6. Session exit checklist

Nothing is "done" until all of these pass:

1. `cargo test --workspace` green
2. `cargo clippy --workspace -- -D warnings` clean
3. `npm test` green
4. `/code-review` run and findings addressed or explicitly deferred with reason
5. `/ponytail:ponytail-review` run — over-engineering removed
6. SonarQube scan run (see `memory/test.md` §7)
7. `memory/todo.md` updated — completed items checked, changed items struck through with reason
8. `memory/handover_session.md` appended with a timestamped entry
9. Committed and pushed; CI observed green (not assumed green) — **match the run to HEAD by SHA**, never `gh run list --limit 1`:

```bash
SHA=$(git rev-parse HEAD)
RUN=$(gh run list --limit 10 --json databaseId,headSha \
      -q ".[] | select(.headSha==\"$SHA\") | .databaseId" | head -1)
gh run watch "$RUN" --exit-status
```

A fresh push takes a few seconds to register, so `--limit 1` returns the
*previous* commit's run — which is already green and reads as confirmation.
This nearly shipped a false green in session 004.
