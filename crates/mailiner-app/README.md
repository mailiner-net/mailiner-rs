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

To serve below the host root (`https://example.com/app`), add
`--base-path /app` to `dx serve` / `dx build`. The router, hashed assets,
manifest, service worker, and OAuth callback follow that prefix. Do not set
`base_path` in `Dioxus.toml` — that would change the default root deploy.
See the workspace README, "Serving under a URL prefix".

The crate `public/` directory (manifest, service worker, install icons) is
copied into that output as-is so the site is installable as a standalone PWA.
Those files use paths relative to their own URL, so the same copies work at
`/` and at `/app`.
