import { defineConfig } from 'vitest/config'

// Separate from vitest.config.ts on purpose: benchmarks load large GLB assets
// and take minutes, so they must not run in the legacy CI test job. Invoked
// explicitly via `npm run bench`.
export default defineConfig({
  test: {
    globals: true,
    environment: 'node',
    include: ['bench/**/*.ts'],
    exclude: ['bench/glb-headless.ts'],
    testTimeout: 600_000,
    // The solver is single-threaded and memory-heavy; parallel files would make
    // the RSS figures meaningless.
    fileParallelism: false
  }
})
