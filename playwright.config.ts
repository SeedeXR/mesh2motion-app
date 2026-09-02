import { defineConfig } from '@playwright/test'

// Drives the vite frontend in a real browser so the viewport's GPU render can
// be screenshotted — the one part no unit test can see.
export default defineConfig({
  testDir: './e2e',
  timeout: 60_000,
  use: {
    baseURL: 'http://localhost:1420',
    // SwiftShader gives headless Chromium a working WebGL context.
    launchOptions: { args: ['--use-gl=angle', '--use-angle=swiftshader', '--ignore-gpu-blocklist'] }
  },
  webServer: {
    command: 'npm run dev',
    url: 'http://localhost:1420',
    reuseExistingServer: true,
    timeout: 60_000
  }
})
