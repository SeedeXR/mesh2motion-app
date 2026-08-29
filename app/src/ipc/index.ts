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
