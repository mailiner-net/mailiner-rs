# Mailiner

Read @README.md to learn about Mailiner and local development instructions.

## Coding Guidelines

This project targets the browser through the `Dioxus` crate, so it gets cross-compiled
to WASM. This has several implications:

- not every crate supports WASM target, either find an alternative, or, if the scope is
  small-enough, or only part of the crate's functionality is needed, implement it directly
  in Mailiner
- performance and memory efficiency are paramount - lazy loading, streaming, pre-fetch,
  LRU caching are effective techniques to ensure the app remains snappy while also keeping
  memory usage under control

