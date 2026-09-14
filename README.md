# Mailiner

Mailiner is a browser-based IMAP client. It focuses on privacy, efficiency and
flexibility.

## Technology

Mailiner is written in Rust using the Dioxus library. It gets cross-compiled to
WASM in order to run in the browser. The web build ships a web app manifest and
a minimal app-shell service worker so the site can be installed as a standalone
window. The worker caches only same-origin shell files (HTML, hashed JS/CSS/WASM,
icons); mail bodies travel over IMAP/WebSocket and are not stored there.

Since browsers do not yet support creating plain TCP connections, Mailiner cannot
establish a direct connection to the target IMAP server. To work around this, we
use a WebSocket connection routed through `ws-tcp-proxy` — a simple Rust server
that accepts WebSocket connections and forwards their payload to a TCP connection
to the destination server. Responses from the TCP connection are routed back to
the WebSocket.

## Security & Privacy

Mailiner is not like other webmail clients — it doesn't require an intermediate
PHP/Java/NodeJS server to do the actual communication with the IMAP server. It uses
a tiny websocket-to-TCP proxy to work around a limitation of current browsers, but
otherwise it acts like full desktop email clients such as Outlook, Thunderbird or
KMail.

The connection to the email server is encrypted on the browser side, meaning that
only the client and the server can see the communication — no intermediaries,
including the websocket-to-TCP proxy, can see the content of the communication.

You can use our proxy, or run your own, to prevent the email server operator from
tracking your location based on where you are connecting from — all traffic will
look like it comes from the proxy.

Account settings, the local address book, and recent compose recipients are
stored only in this browser on this device (localStorage). Mailiner has no
server account. IMAP/SMTP passwords, OAuth tokens, and proxy tokens can be
encrypted at rest with an optional unlock passphrase (WebCrypto AES-GCM,
PBKDF2-SHA-256). Without a passphrase they remain plaintext in origin storage.
Anyone with this browser profile can use Mailiner while a session is unlocked;
clear site data to remove the vault.

Incoming-mail filters (Settings → Filters) also live only in this browser. They
run locally when a folder is opened or new mail arrives (IDLE / NOOP). Vacation
/ out-of-office (Settings → Vacation) is local too: this browser sends the
replies over SMTP until ManageSieve exists. Mailiner does not speak ManageSieve
yet.

Mailiner also prevents malicious emails from executing JavaScript code or loading
remote references that could reveal details about the user to the sender.

### Content-Security-Policy (baseline)

The app ships a baseline CSP via a document `<meta>` tag (see `mailiner-app`
`CONTENT_SECURITY_POLICY`) so local `dx serve` is covered after mount. The meta
tag is injected after WASM/`App` mounts, so the **initial** HTML/script/WASM
load is not constrained by it. Cloudflare Pages deploy writes the same policy
as a `Content-Security-Policy` HTTP header (`_headers` in the built public dir,
next to the SPA `_redirects`) for first-paint coverage. `dx serve` does not
send the header; HMR may also inject scripts after mount.

| Directive | Policy | Why |
|-----------|--------|-----|
| `default-src` | `'self'` | Deny unexpected origins by default |
| `script-src` | `'self' 'unsafe-eval' 'wasm-unsafe-eval'` | App WASM only; no third-party JS. `wasm-unsafe-eval` instantiates WASM; `unsafe-eval` is required for wasm-bindgen `new Function` closures (click/shortcut handlers) |
| `style-src` | `'self' 'unsafe-inline'` | Dioxus uses inline `style=` (virtual list, layout). Strict style-src would break the UI. Remote stylesheets are stripped by the sanitizer |
| `img-src` | `'self' data: blob: http: https:` | Inline message images (`data:` from cid rehydration); download / image-attachment previews (`blob:`); remote images when the user clicks **Allow remote resources** (privacy is gated in the HTML formatter first; CSP must not veto that path) |
| `connect-src` | `'self' ws: wss: http: https:` | User-configured proxies can be any host; a strict host allowlist is not feasible without dynamic CSP. IMAP remains TLS-wrapped in the client |
| `frame-src` | `'self' blob:` | PDF attachment preview (`<iframe src="blob:…">`). HTML/SVG attachments are not previewed |
| `object-src` | `'none'` | No plugins; PDFs use `iframe`, not `<embed>`/`<object>` |
| `base-uri` / `form-action` | `'self'` | Limit base URL and form targets |

**Tradeoffs:** CSP is primarily XSS hardening for secrets stored in the origin.
It does **not** pin proxy destinations or remote image hosts — privacy for mail
images is enforced in the formatter (block by default; Allow opts in). Deploy
sends this same baseline as an HTTP header — do not tighten `connect-src` or
`img-src`, and retain `wasm-unsafe-eval` + `style-src 'unsafe-inline'` for the
Dioxus runtime.

## Running Mailiner locally

Step 1: start the local mail server and WebSocket proxy:

```
docker compose up --build --wait
```

That runs Dovecot/Postfix (`mailiner-mail`) and `ws-tcp-proxy` (`mailiner-proxy`).
Default account:

| Field | Value |
|---|---|
| Email / username | `dev@mailiner.test` |
| Password | `dev` |
| IMAP | `mail:993` (implicit TLS) |
| SMTP | `mail:465` (implicit TLS) |
| Proxy | `ws://localhost:9400/proxy` (no token) |

The browser talks to the proxy on localhost; the proxy dials the `mail`
service on the compose network. Use host **`mail`** (it is on the test
certificate SAN), not `localhost`. TLS uses the test CA at
`docker/mail/tls/ca.crt`. Debug `dx serve` builds (and `--features
dev-defaults`) trust that CA automatically; a stock release build needs
the PEM pasted under Extra CA certificates.

Re-seed an existing volume with `FORCE_SEED=1 docker compose up`. The
proxy is built from the sibling `../ws-tcp-proxy` checkout by default.
Override with `WS_TCP_PROXY_CONTEXT=/path/to/ws-tcp-proxy`.

Alternatively, run only the mail container and start the proxy from the
`ws-tcp-proxy` repo (`cargo run`). In that case IMAP/SMTP host is
`localhost` and a debug proxy accepts token `testtoken`.

Step 2: run Mailiner

```
cd mailiner-rs && dx serve -p mailiner-app
```

That is a debug WASM build (`wasm-dev`, ~100 MB). For a size-optimized release
bundle (what CI deploys):

```
dx build -p mailiner-app --release --web --debug-symbols=false
```

Output lands in `target/dx/mailiner-app/release/web/public/`. Release uses the
workspace `wasm-release` profile (`opt-level = "s"`, LTO) and runs `wasm-opt`
without DWARF. Pass `--debug-symbols=false` so the CLI default does not
re-enable debug info and skip optimization.

Optional form prefill for local development (does **not** auto-connect):

```
dx serve -p mailiner-app --features dev-defaults
```

Under debug builds, or with `--features dev-defaults`, the first-run form is
prefilled with a local proxy URL (`ws://localhost:9400/proxy`) and optional
compile-time `MAILINER_DEV_*` values if set in the environment at build time
(`MAILINER_DEV_IMAP_HOST`, `MAILINER_DEV_IMAP_USER`, `MAILINER_DEV_IMAP_PASSWORD`,
`MAILINER_DEV_EMAIL`, `MAILINER_DEV_DISPLAY_NAME`, `MAILINER_DEV_PROXY_URL`,
`MAILINER_DEV_PROXY_TOKEN`, etc.).

Step 3: open the app in the browser. With an empty account store you will see
a short **setup wizard**:

1. **Get started** — what Mailiner is (a client in this browser; no Mailiner account)
2. **Your email** — display name and address. Mailiner looks up IMAP/SMTP
   (Mozilla ISPDB, then domain `.well-known` autoconfig, then common `imap.` /
   `smtp.` host guesses). Pick a provider preset if you know it.
3. **Sign in** — IMAP password / app password, or **OAuth 2.0** for Gmail / Outlook
4. **Mail servers** — confirm the looked-up hosts (edit if needed)
5. **How Mailiner connects** — proxy URL and token. Skipped when a URL is
   already known (debug / `dev-defaults` prefill, or the last-used proxy when
   adding another account)
6. **Protect this device** — optional unlock passphrase
7. **Review and connect** — Mailiner signs in first; only on success are
   settings saved and the main mail UI opened

Add account (`Settings → Accounts → Add account`) uses the same wizard without
the welcome and passphrase steps.

No build-time `IMAP_PASSWORD` is required.

### OAuth 2.0 (Gmail and Outlook)

Mailiner does **not** ship Google or Microsoft client IDs. Operators bring a
**public** OAuth client (authorization-code + PKCE; no client secret).

1. Create a Web / SPA OAuth client in Google Cloud Console or Microsoft Entra.
2. Add the exact redirect URI shown on the account form:
   `https://<your-origin>/oauth/callback`
   Local `dx serve` uses `http://localhost:<port>/oauth/callback` (Google
   allows `http://localhost`; Microsoft does too for loopback).
3. On the account form, choose **OAuth 2.0**, pick Google or Microsoft, paste
   the client ID (and a Microsoft tenant if you are not using `common`).
4. Click **Sign in with Google/Microsoft**. A popup (or same-tab redirect)
   completes consent; Mailiner stores the access and refresh tokens with the
   account (encrypted if you set an unlock passphrase).
5. IMAP and SMTP then authenticate with SASL **XOAUTH2**. Tokens are refreshed
   automatically before AUTH when expired.

Password / app-password login remains the default.

## End-to-end tests

Playwright covers first-run onboarding, unlock, mail chrome, compose, settings,
and keyboard shortcuts without a live IMAP server. From the repo root:

```
npm install
npx playwright install --with-deps chromium
npm run test:e2e
```

That starts `dx serve` locally. The same suite runs in GitHub Actions on pull
requests and on every push / merge to `main` (it serves the release web
artifact).

A second **live** suite talks to docker-mail through the compose proxy:

```
docker compose up --build --wait
npm run test:e2e:live
```

CI runs that as **Playwright e2e (docker-mail)** after the web build. Details
are in [`e2e/README.md`](e2e/README.md).
