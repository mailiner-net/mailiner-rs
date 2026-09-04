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

Account credentials are entered via the first-run onboarding form (or loaded
from browser localStorage). No build-time `IMAP_PASSWORD` is required.
Connecting to a real IMAP account needs the usual local setup described in
the top-level `README.md` (including running `ws-tcp-proxy` and completing
onboarding).

## Seeding an account (no live IMAP)

Specs beyond first-run onboarding inject a **plaintext** (no vault) blob into
`localStorage` before the app boots. That is the same document
`BrowserAccountStore` persists under `mailiner.accounts.v1`. A present vault
object would show the unlock screen instead (see the encrypted-store test in
`app.spec.ts`).

`e2e/tests/helpers.ts` writes:

| Key | Purpose |
| --- | --- |
| `mailiner.accounts.v1` | One dummy account so bootstrap is `Ready` (settings, compose, shortcuts). The proxy URL is `ws://127.0.0.1:59999/proxy` so IMAP fails immediately without using a Chrome-blocked port. |
| `mailiner.cache.v1` | Optional folder tree + one Inbox envelope so list / picker tests can hydrate without a server. |
| `mailiner.e2e.skipConnect` | `1` — bootstrap hydrates the cache and paints mail chrome but does not open a WebSocket. Keeps tests off the network and off auto-reconnect. |

A failed connect **keeps** a cache hit, so the tree and list stay on screen.
Opening a cached row without IMAP shows `Failed to load message: Not connected`.
Server-side MOVE is not exercised — the move picker is asserted open/close
only. Do not point these specs at a live IMAP host.

Useful variations:

```bash
# interactive UI mode
npm run test:e2e:ui

# open the HTML report from the last run
npm run test:e2e:report

# point tests at an already-running dev server instead of spawning one
MAILINER_E2E_BASE_URL=http://127.0.0.1:8080 npx playwright test
```
