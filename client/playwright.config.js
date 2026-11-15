import { defineConfig } from '@playwright/test'

// Runs against the production build: `vite preview` serves client/build, and
// the specs answer every /api and third-party request themselves.
export default defineConfig({
  testDir: './e2e',
  // Not *.spec.js / *.test.js: those names belong to vitest.
  testMatch: '**/*.e2e.js',
  fullyParallel: true,
  use: { browserName: 'chromium' },
  webServer: {
    command: 'npm run build && npx vite preview --port 4173 --strictPort --host 127.0.0.1',
    url: 'http://127.0.0.1:4173',
    reuseExistingServer: false,
  },
})
