/**
 * Typed wrappers over Tauri commands.
 *
 * This module is the **only** place `invoke` is called (memory/architecture.md
 * §5). Everything else imports these functions, which is what lets the frontend
 * be tested with the Rust side mocked.
 */

import { invoke } from '@tauri-apps/api/core'

export interface BuildInfo {
  readonly version: string
  readonly target: string
}

/** Returns the running binary's version and target architecture. */
export async function buildInfo(): Promise<BuildInfo> {
  return await invoke<BuildInfo>('build_info')
}

/** Records startup diagnostics on the Rust side so they reach the app log. */
export async function reportStartup(diagnostics: string): Promise<void> {
  await invoke('report_startup', { diagnostics })
}

/** True when running inside the Tauri webview rather than a plain browser. */
export function isDesktop(): boolean {
  return '__TAURI_INTERNALS__' in window
}

/** What reading a model file found. Mirrors `m2m_io::import::Import`. */
export interface Import {
  readonly format: 'Fbx' | 'Glb'
  readonly meshes: number
  /** Bone names, parents before children. Empty means the file has no skeleton. */
  readonly bones: readonly string[]
  readonly skinned_meshes: number
  readonly clips: readonly string[]
  /** Vertices (FBX) or primitives (glTF) whose influences past the fourth are dropped. */
  readonly over_influence_limit: number
}

export interface ImportedFile {
  readonly name: string
  /** Where the file came from, so its geometry can be fetched without re-picking. */
  readonly path: string
  readonly import: Import
}

/**
 * Opens the native file picker and reports what the chosen model contains.
 *
 * Resolves to `null` when the user cancels. Nothing is stripped — see O9 in
 * `memory/todo.md`: an existing skeleton is kept and reported, never silently
 * discarded the way the legacy app's cleanup step did.
 */
export async function importModel(): Promise<ImportedFile | null> {
  return await invoke<ImportedFile | null>('import_model')
}

/**
 * Fetches a model's geometry as a `.glb`, over the **bulk channel**.
 *
 * The body is raw bytes, never JSON — `memory/architecture.md` §4. A 50k-vertex
 * mesh is ~1.2 MB binary and ~9 MB as a JSON number array, and parsing that
 * array in the webview would cost more than the whole Rust-side solve.
 *
 * glTF is the wire format because it already is a JSON header plus a binary
 * chunk, so a loader that already exists can read it. An FBX is converted on
 * the Rust side.
 */
export async function loadModel(path: string): Promise<ArrayBuffer> {
  return await invoke<ArrayBuffer>('load_model', { path })
}

/** A creature template offered in the Choose Skeleton step. */
export interface SkeletonTemplate {
  readonly name: string
  readonly bones: number
  readonly chains: readonly string[]
  /** False when the manifest names a rig that is not bundled. */
  readonly available: boolean
}

/** A template skeleton placed on the imported mesh. */
export interface FittedSkeleton {
  readonly bones: readonly string[]
  /** Each bone's parent as an index into `bones`; `null` for a root. */
  readonly parents: readonly (number | null)[]
  readonly positions: readonly (readonly [number, number, number])[]
  /** Each bone's local rest rotation, xyzw, carried from the template. */
  readonly rotations: readonly (readonly [number, number, number, number])[]
  readonly scale: number
}

/** The creature templates that ship with the app. */
export async function skeletonTemplates(): Promise<SkeletonTemplate[]> {
  return await invoke<SkeletonTemplate[]>('skeleton_templates')
}

/**
 * Fits a template's skeleton to the imported model.
 *
 * Travels as JSON, not over the bulk channel: a skeleton is a few hundred
 * bones, and architecture.md §4 draws its line at bulk geometry.
 */
export async function fitSkeleton(template: string, path: string): Promise<FittedSkeleton> {
  return await invoke<FittedSkeleton>('fit_skeleton', { template, path })
}

/** What binding produced, without the weights themselves. */
export interface BindReport {
  readonly vertices: number
  readonly weighted_bones: number
  readonly excluded_bones: number
  /** Vertices no bone reached through the mesh — a disconnected island. */
  readonly fallback_vertices: number
  /** Vertices with 1, 2, 3 and 4 influences. */
  readonly influence_histogram: readonly [number, number, number, number]
  /** Vertices with no influence at all. Non-zero is a solver bug. */
  readonly unweighted_vertices: number
}

/**
 * Binds the mesh to the fitted skeleton.
 *
 * The weights themselves stay on the Rust side: they are vertices x 4 indices
 * and vertices x 4 floats, which belongs on the bulk channel beside the
 * geometry, not in a JSON reply. What comes back is what a person can act on.
 */
export async function bindWeights(
  path: string,
  skeleton: FittedSkeleton,
  falloff: number
): Promise<BindReport> {
  return await invoke<BindReport>('bind_weights', { path, skeleton, falloff })
}

/**
 * Writes the rigged model to a file the user chooses.
 *
 * Resolves to the saved file's name, or `null` when the user cancels. The
 * weights are recomputed on the Rust side rather than sent back and forth.
 */
export async function exportModel(
  path: string,
  skeleton: FittedSkeleton,
  falloff: number,
  format: 'glb' | 'fbx',
  template: string,
  clip: string | null
): Promise<string | null> {
  return await invoke<string | null>('export_model', {
    path,
    skeleton,
    falloff,
    format,
    template,
    clip
  })
}

/** One clip in a creature's animation library. */
export interface ClipSummary {
  readonly name: string
  /** Longest key time, in seconds. */
  readonly duration: number
  readonly tracks: number
}

/** The clips a creature's animation library offers. */
export async function animationClips(template: string): Promise<ClipSummary[]> {
  return await invoke<ClipSummary[]>('animation_clips', { template })
}

/**
 * Returns the rigged model with a clip retargeted onto it, as a `.glb`.
 *
 * The **bulk channel**: a whole animated model as raw bytes, the same the
 * viewport loads for the imported mesh. These are the bytes a `.glb` export
 * writes to disk, so preview and export cannot drift.
 */
export async function previewAnimation(
  path: string,
  skeleton: FittedSkeleton,
  falloff: number,
  template: string,
  clip: string
): Promise<ArrayBuffer> {
  return await invoke<ArrayBuffer>('preview_animation', {
    path,
    skeleton,
    falloff,
    template,
    clip
  })
}
