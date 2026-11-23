// End-to-end tests against the built Electron app (doc 10): npm run build first.
import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: 'e2e',
  timeout: 60_000,
  workers: 1,
  reporter: [['list']],
  use: { trace: 'retain-on-failure' },
});
