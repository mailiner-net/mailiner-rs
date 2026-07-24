# Mailiner

Mailiner is a browser-based IMAP client. It focuses on privacy, efficiency and
flexibility.

## Technology

Mailiner is written in Rust using the Dioxus library. It gets cross-compiled to
WASM in order to run in the browser.

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

Account settings (including the IMAP password and proxy token) are stored only in
this browser on this device (localStorage). Mailiner has no server account. Anyone
with access to this browser profile can read those secrets; clear site data to
remove them.

Mailiner also prevents malicious emails from executing JavaScript code or loading
remote references that could reveal details about the user to the sender.

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

1. Enter display name, email, IMAP host / port / username / password
2. Enter proxy base URL and token (and optional remote host/port overrides)
3. Optionally click **Test connection**
4. Click **Save & continue** — Mailiner connects and authenticates first; only
   on success are settings saved and the main mail UI opened

No build-time `IMAP_PASSWORD` is required.
