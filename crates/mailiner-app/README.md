# mailiner-app

The Dioxus/WASM UI crate for [Mailiner](../../README.md).

For local development (ws-tcp-proxy, docker mail, onboarding), see the
[workspace README](../../README.md).

## Serve / build

From the workspace root:

```bash
dx serve -p mailiner-app
```

Optional form prefill for local development (does **not** auto-connect):

```bash
dx serve -p mailiner-app --features dev-defaults
```

Release (size-optimized, what CI deploys):

```bash
dx build -p mailiner-app --release --web --debug-symbols=false
```

The crate `public/` directory (manifest, service worker, install icons) is
copied into that output as-is so the site is installable as a standalone PWA.
