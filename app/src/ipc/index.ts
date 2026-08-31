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
