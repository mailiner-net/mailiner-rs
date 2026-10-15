"use strict";

// App-shell service worker. Precaches the HTML document, manifest, and icons.
//
// The script is served next to index.html. Its directory is the deploy prefix:
// `/sw.js` controls `/`, and `/app/sw.js` controls `/app/`. Cache URLs are
// resolved against that directory so one file works at the host root and
// under a path. Hashed Dioxus JS/CSS/WASM under assets/ and wasm/ are cached
// only after a successful same-origin GET. Mail is IMAP over WebSocket —
// never HTTP — so message bodies are not stored here.

const CACHE = "mailiner-shell-v3";

// Directory of this script (`/` or `/app/`), including the trailing slash.
const BASE = new URL("./", self.location);
const INDEX_HTML = new URL("index.html", BASE).href;

const PRECACHE = [
  "index.html",
  "manifest.webmanifest",
  "icons/icon-192.png",
  "icons/icon-512.png",
  "icons/icon-maskable-192.png",
  "icons/icon-maskable-512.png",
  "icons/apple-touch-icon.png",
].map((path) => new URL(path, BASE).href);

function inScope(url) {
  if (url.origin !== self.location.origin) {
    return false;
  }
  const base = BASE.pathname;
  const path = url.pathname;
  // The registration scope of `/app/sw.js` is `/app/`. A request for bare
  // `/app` is outside that scope, so this worker never sees it. Hosting
  // redirects `/app` to `/app/` (Pages `_redirects`) and that URL is in scope.
  return path === base || path + "/" === base || path.startsWith(base);
}

function relativePath(url) {
  const base = BASE.pathname;
  const path = url.pathname + "/" === base ? base : url.pathname;
  return path.startsWith(base) ? path.slice(base.length) : path;
}

function isAppShell(url) {
  if (!inScope(url)) {
    return false;
  }
  const path = relativePath(url);
  if (path === "" || path === "index.html") {
    return true;
  }
  if (path === "manifest.webmanifest" || path === "sw.js") {
    return true;
  }
  if (path.startsWith("icons/") || path.startsWith("assets/") || path.startsWith("wasm/")) {
    return true;
  }
  return /\.(?:js|wasm|css|ico|png|svg|webmanifest)$/.test(path);
}

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE)
      .then((cache) => cache.addAll(PRECACHE))
      .then(() => self.skipWaiting()),
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) =>
        Promise.all(keys.filter((key) => key !== CACHE).map((key) => caches.delete(key))),
      )
      .then(() => self.clients.claim()),
  );
});

self.addEventListener("fetch", (event) => {
  const request = event.request;
  if (request.method !== "GET") {
    return;
  }

  const url = new URL(request.url);
  if (!inScope(url)) {
    return;
  }

  if (request.mode === "navigate") {
    event.respondWith(networkFirst(request, INDEX_HTML));
    return;
  }

  if (!isAppShell(url)) {
    return;
  }

  event.respondWith(networkFirst(request, request));
});

function networkFirst(request, fallbackKey) {
  return fetch(request)
    .then((response) => {
      if (response.ok) {
        const copy = response.clone();
        caches.open(CACHE).then((cache) => {
          cache.put(fallbackKey, copy);
        });
      }
      return response;
    })
    .catch(() => caches.match(fallbackKey));
}
