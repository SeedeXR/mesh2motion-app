# Session Handover Log

Newest entries at the top. Every session appends one entry before exiting.
Timestamps are local (macOS, `date "+%Y-%m-%d %H:%M:%S"`).

---

## Session 001 — 2026-08-29

**Started:** 2026-08-29 15:52:11
**Ended:** 2026-08-29 16:48:38
**Focus:** Grounding, architecture decisions, memory bootstrap, port foundation

### Completed
- **P0-1** Grounding pass over the legacy codebase and toolchain
- **P0-2** All 13 `memory/` documents created
- **P0-3** Legacy app moved to `legacy/` by `git mv` — history preserved
- **P0-4** Tauri scaffold; `tauri-cli` 2.11.4 installed via npm
- **P0-5** Rust workspace: 4 crates, all `#![forbid(unsafe_code)]`
- **P0-7** CI created — none existed before this session
- **P0-8** `.cargo/config.toml`, `.gitignore`
- **P0-6** *(partial)* `sonar-scanner` 8.1.0.6389 installed, config written
- **R-1** SOTA survey → `docs/research/skinning-sota.md`

### Observed facts (verified this session, not assumed)
- Legacy app: 30,780 LOC TS across 148 files at commit `1271226`
- **Core defect located:** `legacy/src/lib/solvers/WeightCalculator.ts:71-80` — rigid nearest-bone, one bone per vertex, Euclidean distance to `bone_midpoint_to_child`. The three weight correctors exist to patch this one weakness.
- 9 templates in `SkeletonType`; `Fish` maps to `rig-shark.glb`
- FBX parser is ~4,100 LOC hand-written, handles ASCII **and** binary
- Toolchain present: Rust 1.96.0, Node 22.16.0, Xcode 26.5 + Metal, Blender.app
- Toolchain **missing**: Tauri CLI, SonarQube (→ P0-4, P0-6)
- **No CI exists** — `.github/` contains only `FUNDING.yml` (→ P0-7)
- Machine: Apple M4, 10 cores, 16 GB RAM, macOS 26.6.2, **~34 GB free disk**

### Corrections to stated premises
- **Font:** "42dot Sans" was renamed **Asta Sans** (Feb 2026) and **removed from Google Fonts**. Still SIL OFL. Must be vendored — a CDN link would not work. Confirmed with user.
- **Rust FBX crates:** `fbxcel` is binary-only, read-only, no ASCII, no export; `fbxcel-dom` is v0.0.6. Porting the legacy parser is the lower-risk path (ADR A3).

### Decisions (user-confirmed)
- **A1** Rust compute core + Three.js viewport, not full wgpu — preserves ~3k LOC of working interaction code
- **A2** Geodesic voxel binding as the default solver
- **A5** Neural rigging (UniRig) deferred to opt-in P4-6

### Research findings
- Dionne & de Lasa, *Geodesic Voxel Binding*, SCA 2013 — Maya's method, robust on non-watertight meshes
- Dodik/Sitzmann/Solomon/Stein, *Robust Biharmonic Skinning Using Geometric Fields*, TOG 2025 (arXiv:2406.00238) — mesh-free, no tetrahedralisation
- UniRig (SIGGRAPH 2025) / RigAnything — skeleton prediction across bipeds, quadrupeds, birds, insects, marine

### Verification performed (all observed, none assumed)
- `legacy/`: **107/107 vitest tests pass** and `vite build` succeeds after the move
- `cargo test --workspace`: **9 passed**; `clippy -D warnings` clean; `cargo fmt --check` clean
- `npx tsc --noEmit` clean; `vite build` produces 8.85 kB JS
- App bundles (**6624 KB**, budget 40960 KB) and launches: **98 MB idle RSS**, **0.1% idle CPU**
- IPC round-trip confirmed live — the status bar rendered `v0.1.0 · native core ready`, sourced from the Rust `build_info` command
- Target triple `aarch64-apple-darwin` confirmed embedded in the release binary
- Dialog capability confirmed compiled: `gen/schemas/capabilities.json` → `['default']`

### Code review findings — all 9 fixed in commit 2
One was a genuine security bug, verified independently before fixing:
- **HIGH** `vite.config.ts` had `envPrefix: ['VITE_', 'TAURI_']`. Vite's `loadEnv` copies **every** matching `process.env` key into `import.meta.env` (`dep-Dm0c1Wj2.js:16967`, read directly), and `TAURI_SIGNING_PRIVATE_KEY` / `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` are real Tauri variables (confirmed by `strings` on the CLI binary). A CI build with updater signing would have shipped the private key inside the app. → narrowed to `TAURI_ENV_`.
- **MED** `TAURI_DEBUG` is the v1 name; v2 uses `TAURI_ENV_DEBUG` — sourcemaps were permanently off.
- **MED** `tauri-plugin-dialog` registered with no capability → every dialog call would be ACL-rejected at runtime.
- **MED** `BuildInfo.target` returned `ARCH` ("aarch64"), not a triple as documented.
- **MED** window `minWidth` 1024 < shell's real 1120 minimum → inspector clipped, no scrollbar.
- **LOW** ×4: arch-gate grepped use-statements but a dep arrives via `Cargo.toml`; bundle budget floored 40.9 MB to a passing "40"; dead `lint`/`test` npm scripts; unmarked missing forward navigation.

**Lesson for future sessions:** the review caught things local green checks could not. `cargo test` + `clippy` + `tsc` all passed with the signing-key leak in place. Run `/code-review` before every commit, as `session_start.md` §6 requires.

### Blockers
- **P0-6** SonarQube *server* needs Docker Desktop running (daemon verified not running) or SonarCloud credentials.
- **`references/`** — the 7 Mixamo FBX files are gitignored pending a licensing decision: royalty-free to use but not CC0, and this repo licenses all art as CC0.

### Git
- Branch `port/tauri-rust-foundation`, 2 commits, pushed to `SeedeXR/mesh2motion-app`
- PR #1 opened **against the fork's own main**, deliberately not against upstream `Mesh2Motion/mesh2motion-app`
- First CI run: 5/6 green (arch-gate, frontend, legacy suite, rust test, rust lint); bundle job still running at handover

### Next session starts at
**P0-10** — capture legacy solver benchmarks for all 9 templates. This must
happen **before** any P1 solver work or the A/B comparison has no baseline.

Then **P0-9** (vendor Asta Sans), **P0-11** (verify WebGPU availability in this
WKWebView — currently assumed WebGL2 fallback, unverified), and **R-2**
(geodesic voxel binding algorithm doc) before P1-1.

### Notes
- `legacy/` is a **test dependency**, not dead code — it is the A/B baseline for P1-8. Do not delete it.
- Capture legacy benchmarks (**P0-10**) *before* touching the solver, or the A/B comparison has no baseline.
