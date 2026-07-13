import { defineConfig, devices } from '@playwright/test';
import dotenv from 'dotenv';

// Load repo-root .env (the same file build.rs reads via the `dotenv` crate)
// so a real IMAP_PASSWORD set there is used instead of the placeholder below.
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
    env: {
      // build.rs bakes IMAP_PASSWORD in at compile time; a real value is
      // only needed for actually logging into an IMAP account. Falls back
      // to whatever a local .env already provides.
      IMAP_PASSWORD: process.env.IMAP_PASSWORD ?? 'e2e-placeholder',
    },
  },
});
