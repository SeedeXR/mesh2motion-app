import { defineConfig } from 'vitest/config'

// Only the app's own tests. `legacy/` has its own vitest project and is run
// from that directory in CI, so including it here would run it twice under
// different config.
export default defineConfig({
  test: {
    include: ['app/tests/**/*.test.ts'],
    setupFiles: ['app/tests/setup.ts']
  }
})
