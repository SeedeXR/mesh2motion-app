# R-5: Sourcing CC0 / CC-BY rigged reference creatures

**Status: the research half is done here; the sourcing itself needs a human.**
Finding, vetting, and licensing third-party assets cannot be done autonomously —
it needs a person to download, check the licence, and confirm the file is what it
claims. This is the guide that person follows, and the record they keep.

**Date:** 2026-09-01 · **Blocks:** P3-13 (new templates need these assets first)

## What a candidate must satisfy

A reference creature is only useful if it can *become a template* — a typed
skeleton the fitter can place (see `docs/research/creature-rigs.md`). Require:

1. **Licence: CC0 or CC-BY, in writing at the source.** CC0 needs no attribution;
   CC-BY needs the author credited in `LICENSE-CC0.MD`/a credits file. Reject
   anything "free" without an explicit licence, and anything CC-BY-**NC**/**ND** —
   this project ships MIT code and CC0 assets, and non-commercial/no-derivatives
   licences cannot ride along.
2. **A skeleton, not just a mesh.** The point is a *rig* reference. It must carry
   a bone hierarchy that maps to a `ChainKind`/`LimbRole` layout — a spine, limbs
   with a clear role, a sensible root.
3. **A neutral rest pose.** The fitter places a template onto a mesh in a roughly
   neutral pose (limbs apart). A curled or mid-action rest pose is hard to fit and
   hard to reason about.
4. **Manifold-ish geometry, reasonable scale.** Watertight enough for the voxel
   binder, and in a known unit (glTF metres / FBX centimetres).
5. **A body plan the template set does not already cover well**, or a second
   example of one that fits badly today — the aim is to widen coverage, not
   duplicate the human/fox/horse/bird/spider/snake/shark/kaiju/dragon set.

## Where to look (CC0-first)

None of these are endorsements; each asset's own licence is what governs.

- **Sketchfab**, filtered to *Downloadable* + *CC0* (or *CC-BY*). The largest pool
  of rigged creatures; licence is per-model, so check each.
- **Quaternius** (quaternius.com) — CC0 low-poly animal/creature packs, many rigged.
- **Kenney** (kenney.nl) — CC0, more props than creatures but some characters.
- **Poly Pizza** (poly.pizza) — CC0/CC-BY aggregator, filterable.
- **Mixamo** — auto-rigs *humanoids* only, and its terms are Adobe's, **not** CC0;
  usable for personal reference but do **not** commit Mixamo output as a shipped
  template. (This is why the existing human templates are the project's own.)
- **BlenderKit / Blender demo files** — check each asset's licence individually.

## The record to keep (per asset)

Provenance is a shipping requirement (`README.md` licences section, architecture.md
A7). For every asset added, record — next to the file, or in a manifest — :

```
asset:      rig-<creature>.glb
creature:   <body plan, e.g. "bat — winged quadruped">
source_url: <permalink to the exact model page>
author:     <name/handle>
licence:    CC0 | CC-BY 4.0
retrieved:  <date>
notes:      <rest pose, unit, edits made>
```

CC-BY authors also go in the credits. A CC0 asset still records its source, so a
later licence dispute can be traced.

## Handing off to P3-13

Once an asset passes the checklist and its rig `.glb` is in `assets/rigs/`, P3-13
turns it into a template: write the `templates/<creature>.json` manifest — the
typed chains (`ChainKind`, `LimbRole`, `Posture`) and the guidance string — and
add it to the fit/visual-regression coverage. The template machinery is ready
(build.rs globs the manifests; the fitter, binder and pose-matcher are
role-agnostic); only the assets are missing.

## Why this is as far as automation goes

Downloading a file, reading a human-written licence page, and judging whether a
mesh is cleanly rigged are human tasks. The criteria and the record format above
are the automatable part; the sourcing itself is not.
