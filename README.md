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
server account. IMAP/SMTP passwords and proxy tokens can be encrypted at rest
with an optional unlock passphrase (WebCrypto AES-GCM, PBKDF2-SHA-256). Without
a passphrase they remain plaintext in origin storage. Anyone with this browser
profile can use Mailiner while a session is unlocked; clear site data to remove
the vault.

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
| `script-src` | `'self' 'wasm-unsafe-eval'` | App WASM only; no third-party JS. `wasm-unsafe-eval` is required to instantiate WASM (not full `unsafe-eval`) |
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

Step 1: run the ws-tcp-proxy (from the separate `mailiner/ws-tcp-proxy` repo):

```
cd ws-tcp-proxy && cargo run
```

By default the proxy listens on `ws://localhost:9400/proxy`. Use a token that
matches what you enter in the onboarding form (e.g. `testtoken` for a local
dev proxy that expects that value).

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
**Onboarding**:

1. Enter display name and email — Mailiner looks up IMAP/SMTP (Mozilla ISPDB,
   then domain `.well-known` autoconfig, then common `imap.` / `smtp.` host
   guesses). You can edit the result.
2. Enter IMAP password, proxy base URL and token (and optional remote overrides)
3. Optionally click **Test connection**
4. Click **Save & continue** — Mailiner connects and authenticates first; only
   on success are settings saved and the main mail UI opened

No build-time `IMAP_PASSWORD` is required.
