import { defineConfig, devices } from '@playwright/test';
import dotenv from 'dotenv';

// Optional repo-root .env for MAILINER_DEV_* form prefill / other tooling.
dotenv.config();

const PORT = process.env.MAILINER_E2E_PORT ?? '8080';
const BASE_URL = process.env.MAILINER_E2E_BASE_URL ?? `http://127.0.0.1:${PORT}`;
const SERVE_DIR = process.env.MAILINER_E2E_SERVE_DIR;
const SKIP_WEBSERVER = process.env.MAILINER_E2E_SKIP_WEBSERVER === '1';

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
    toHaveScreenshot: {
      animations: 'disabled',
      caret: 'hide',
      scale: 'css',
      stylePath: './e2e/screenshot.css',
    },
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
      testIgnore: [/live\.spec\.ts/, /visual\.spec\.ts/],
      use: { ...devices['Desktop Chrome'] },
    },
    {
      name: 'live',
      testMatch: /live\.spec\.ts/,
      use: { ...devices['Desktop Chrome'] },
    },
    {
      // Pixel baselines. Locale, timezone, and color scheme are pinned because
      // list dates use Intl and the theme follows prefers-color-scheme.
      // Update snapshots in the arm64 variant of
      // mcr.microsoft.com/playwright:v1.63.0-noble. CI runs the amd64 variant
      // of that tag. See e2e/README.md.
      name: 'visual',
      testMatch: /visual\.spec\.ts/,
      retries: 0,
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1280, height: 800 },
        deviceScaleFactor: 1,
        colorScheme: 'light',
        locale: 'en-US',
        timezoneId: 'UTC',
        reducedMotion: 'reduce',
        // Optional local browser when Playwright's Chromium build is unavailable.
        // Baselines still have to come from the CI image. See e2e/README.md.
        ...(process.env.MAILINER_CHROME_PATH
          ? { launchOptions: { executablePath: process.env.MAILINER_CHROME_PATH } }
          : {}),
      },
    },
  ],

  webServer: SKIP_WEBSERVER
    ? undefined
    : {
        command: SERVE_DIR
          ? `npx --no-install serve -s ${SERVE_DIR} -l tcp://127.0.0.1:${PORT}`
          : `dx serve -p mailiner-app --port ${PORT}`,
        url: BASE_URL,
        reuseExistingServer: !process.env.CI,
        timeout: SERVE_DIR ? 60_000 : 5 * 60_000,
      },
});
