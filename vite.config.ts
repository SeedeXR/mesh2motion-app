import { defineConfig } from 'vite'

// Tauri expects a fixed port and needs the dev server reachable from the webview.
export default defineConfig({
  root: '.',
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  // TAURI_ENV_, never bare TAURI_: vite's loadEnv copies every matching
  // process.env key into the client bundle, and TAURI_SIGNING_PRIVATE_KEY /
  // TAURI_SIGNING_PRIVATE_KEY_PASSWORD are real Tauri variables. A bare TAURI_
  // prefix would ship the updater signing key inside the app.
  envPrefix: ['VITE_', 'TAURI_ENV_'],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    // Safari 15 is the floor implied by tauri.conf minimumSystemVersion 12.0.
    target: 'safari15',
    // Tauri v2 sets TAURI_ENV_DEBUG; TAURI_DEBUG was the v1 name.
    sourcemap: !!process.env['TAURI_ENV_DEBUG'],
    minify: process.env['TAURI_ENV_DEBUG'] ? false : 'esbuild'
  }
})
