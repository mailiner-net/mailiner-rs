# Mailiner

Mailiner is a browser-based IMAP client. It focuses on privacy, efficiency and
flexibility.

## Technology

Mailiner is written in Rust using the Dioxus library. It gets cross-compiled to
WASM in order to run in the browser.

Since browsers do not yet support creating plain TCP connections, Mailiner cannot
established a direct connection to the target IMAP server. To workaround this, we
use a WebSocket connection that we route through our own `ws-tcp-proxy` - a simple
Rust server that accepts WebSocket connections and forwards their payload to a TCP
connection to the destination server. Responses from the TCP connection are routed
back to the WebSocket.


## Security & Privacy

Mailiner is not like other webmail clients - it doesn't require an intermediate
PHP/Java/NodeJS server to do the actual communication with the IMAP server. It uses
a tiny websocket-to-TCP proxy to work around a limitation of current browsers, but
otherwise it acts like full desktop email clients like Outlook, Thunderbird or KMail.

The connection to the email server is encrypted on the browser side, meaning that
only the client and the server can see the communication - no intermediaries, including
the websocket-to-TCP proxy, can see the content of the communication.

You can use our proxy, or run your own, to prevent the email server operator from
tracking your location based on where you are connecting from - all traffic will
look like it comes from the proxy.

Mailiner also prevents malicious emails from executing JavaScript code or loading
remote references that could reveal details about the user to the sender. 

## Account storage (browser)

Account connection settings (including IMAP passwords and the proxy token) are
persisted **only in this browser** under the origin’s `localStorage` key
`mailiner.accounts.v1` (single JSON blob: schema version, active account id,
account list). Mailiner has no server-side account vault.

- Clearing site data removes saved accounts.
- Private / strict modes may block `localStorage`; the app treats that as
  storage unavailable (session-only fallback is planned with onboarding UI).
- Anyone with access to this browser profile (or XSS on this origin) can read
  stored credentials — use a private device and app passwords where possible.
- IndexedDB persistence is deferred; configs are small enough for `localStorage`.

## Running Mailiner locally

Step 1: run the ws-tcp-proxy (from `mailiner/ws-tcp-proxy` repo):

```
cd ws-tcp-proxy && cargo run
```

Step 2: run Mailiner in dev mode

```
cd mailiner-rs && dx serve -p mailiner-app
```


