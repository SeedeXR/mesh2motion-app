import { defineConfig } from 'vite'

// Tauri expects a fixed port and needs the dev server reachable from the webview.
export default defineConfig({
  root: '.',
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  envPrefix: ['VITE_', 'TAURI_'],
  build: {
    outDir: 'dist',
    emptyOutDir: true,
    // Safari 15 is the floor implied by tauri.conf minimumSystemVersion 12.0.
    target: 'safari15',
    sourcemap: !!process.env.TAURI_DEBUG,
    minify: process.env.TAURI_DEBUG ? false : 'esbuild'
  }
})
