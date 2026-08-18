import { defineConfig, devices } from '@playwright/test';
import dotenv from 'dotenv';

// Optional repo-root .env for MAILINER_DEV_* form prefill / other tooling.
dotenv.config();

const PORT = process.env.MAILINER_E2E_PORT ?? '8080';
const BASE_URL = process.env.MAILINER_E2E_BASE_URL ?? `http://127.0.0.1:${PORT}`;

/**
 * Mailiner is a Dioxus/WASM app served via `dx serve`. The first build
 * compiles the whole workspace to wasm32-unknown-unknown, which can take
 * several minutes, so timeouts here are generous compared to a typical
 * JS dev server.
 */
export default defineConfig({
  testDir: './e2e/tests',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  workers: process.env.CI ? 1 : undefined,
  reporter: process.env.CI ? 'github' : 'html',
  timeout: 30_000,

  use: {
    baseURL: BASE_URL,
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
  },

  projects: [
    {
      name: 'chromium',
      use: { ...devices['Desktop Chrome'] },
    },
  ],

  webServer: {
    command: `dx serve -p mailiner-app --port ${PORT}`,
    url: BASE_URL,
    reuseExistingServer: !process.env.CI,
    timeout: 5 * 60_000,
  },
});
