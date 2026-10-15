import { defineConfig, devices } from '@playwright/test';
import dotenv from 'dotenv';

// Optional repo-root .env for MAILINER_DEV_* form prefill / other tooling.
dotenv.config();

const PORT = process.env.MAILINER_E2E_PORT ?? '8080';
const BASE_URL = process.env.MAILINER_E2E_BASE_URL ?? `http://127.0.0.1:${PORT}`;
const SERVE_DIR = process.env.MAILINER_E2E_SERVE_DIR;
const SKIP_WEBSERVER = process.env.MAILINER_E2E_SKIP_WEBSERVER === '1';
/** Deploy prefix without slashes. Empty means the host root. */
const SERVE_PREFIX = (process.env.MAILINER_E2E_BASE_PATH ?? '').replace(/^\/+|\/+$/g, '');

function webServerCommand(): string {
  if (!SERVE_DIR) {
    return `dx serve -p mailiner-app --port ${PORT}`;
  }
  // `serve -s` rewrites every miss to `/index.html`. A prefixed artifact has
  // that file at `/app/index.html`, so the prefix server does the fallback.
  if (!SERVE_PREFIX) {
    return `npx --no-install serve -s ${SERVE_DIR} -l tcp://127.0.0.1:${PORT}`;
  }
  return 'node e2e/prefix-static-server.mjs';
}

/**
 * Mailiner is a Dioxus/WASM app. Locally `dx serve` compiles the workspace
 * to wasm32-unknown-unknown (first run can take several minutes). CI serves
 * the already-built release bundle from MAILINER_E2E_SERVE_DIR instead.
 */
export default defineConfig({
  testDir: './e2e/tests',
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  // One worker: the WASM bundle is large; two cold navigations starve `load`.
  workers: 1,
  reporter: process.env.CI ? [['github'], ['html', { open: 'never' }]] : 'html',
  timeout: 60_000,
  expect: {
    timeout: 15_000,
  },

  use: {
    baseURL: BASE_URL,
    navigationTimeout: 60_000,
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
  },

  projects: [
    {
      name: 'chromium',
      // `live.spec.ts` and `live-*.spec.ts` need docker-mail.
      testIgnore: /live.*\.spec\.ts/,
      use: { ...devices['Desktop Chrome'] },
    },
    {
      name: 'live',
      testMatch: /live.*\.spec\.ts/,
      use: { ...devices['Desktop Chrome'] },
    },
  ],

  webServer: SKIP_WEBSERVER
    ? undefined
    : {
        command: webServerCommand(),
        url:
          SERVE_DIR && SERVE_PREFIX
            ? `${BASE_URL.replace(/\/+$/, '')}/${SERVE_PREFIX}/`
            : BASE_URL,
        reuseExistingServer: !process.env.CI,
        timeout: SERVE_DIR ? 60_000 : 5 * 60_000,
      },
});
