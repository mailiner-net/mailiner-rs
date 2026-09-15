# End-to-end tests

Playwright drives the app in a real browser. Locally it starts `dx serve` for
`mailiner-app`. In GitHub Actions the **Playwright e2e** job (part of
**Build & Test**) serves the release web artifact from the **Build mailiner-app
(web)** job — that workflow runs on pull requests and on every push / merge to
`main` (via **Build & Deploy**).

All commands below are run from the repository root — `package.json` and
`playwright.config.ts` live there, not inside `e2e/`.

## Prerequisites

- Node.js 20+
- [`dioxus-cli`](https://dioxuslabs.com/learn/0.7/getting_started) (`dx`) on
  `PATH`, matching the version pinned in `.github/workflows/build-test.yml`
- The `wasm32-unknown-unknown` Rust target: `rustup target add wasm32-unknown-unknown`

CI does not need `dx` in the e2e job: it reuses the already-built
`web-public` artifact.

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

To exercise a release bundle the same way CI does (SPA static server, no
`dx serve`):

```bash
dx build -p mailiner-app --release --web --debug-symbols=false
MAILINER_E2E_SERVE_DIR=target/dx/mailiner-app/release/web/public npm run test:e2e
```

`npm run test:e2e` runs the offline **chromium** project only. It does not
start docker-mail.

## Live tests (docker-mail + proxy)

`e2e/tests/live.spec.ts` connects through `ws-tcp-proxy` to the compose
Dovecot/Postfix container. It is a separate Playwright project so the
offline suite stays off the network.

```bash
docker compose up --build --wait
npm run test:e2e:live
```

IMAP/SMTP host is `mail` (compose DNS + certificate SAN). The live helpers
inject `docker/mail/tls/ca.crt` as an extra CA so a **release** WASM build
accepts the test certificate. Debug `dx serve` already trusts that CA.

GitHub Actions job **Playwright e2e (docker-mail)** checks out
`mailiner-net/ws-tcp-proxy`, runs `docker compose up --wait`, and executes
`--project=live` against the release web artifact.

Account credentials are entered via the first-run onboarding form (or loaded
from browser localStorage). No build-time `IMAP_PASSWORD` is required.

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
MAILINER_E2E_BASE_URL=http://127.0.0.1:8080 npx playwright test --project=chromium
```
