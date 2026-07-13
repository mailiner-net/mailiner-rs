# End-to-end tests

Playwright drives the app in a real browser against a `dx serve` instance of
`mailiner-app`.

All commands below are run from the repository root — `package.json` and
`playwright.config.ts` live there, not inside `e2e/`.

## Prerequisites

- Node.js 20+
- [`dioxus-cli`](https://dioxuslabs.com/learn/0.7/getting_started) (`dx`) on
  `PATH`, matching the version pinned in `.github/workflows/deploy-pages.yml`
- The `wasm32-unknown-unknown` Rust target: `rustup target add wasm32-unknown-unknown`

## Setup

```bash
npm install
npx playwright install --with-deps chromium
```

## Running the tests

```bash
npm run test:e2e
```

This starts `dx serve -p mailiner-app` automatically (see `playwright.config.ts`)
and waits for it to come up before running the tests. The first run can take a
few minutes since it compiles the whole workspace to WASM.

`mailiner-app`'s `build.rs` bakes an `IMAP_PASSWORD` value in at compile time.
`playwright.config.ts` loads a repo-root `.env` (if present) and forwards
`IMAP_PASSWORD` from it, falling back to a placeholder otherwise, so no
`.env` file is required just to bring the app up. Connecting to a real IMAP
account still needs the usual local setup described in the top-level
`README.md` (including running `ws-tcp-proxy`).

Useful variations:

```bash
# interactive UI mode
npm run test:e2e:ui

# open the HTML report from the last run
npm run test:e2e:report

# point tests at an already-running dev server instead of spawning one
MAILINER_E2E_BASE_URL=http://127.0.0.1:8080 npx playwright test
```
