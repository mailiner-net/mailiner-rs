"use strict";

// App-shell service worker. Precaches the HTML document, manifest, and icons.
// Hashed Dioxus JS/CSS/WASM under /assets/ and /wasm/ are cached only after a
// successful same-origin GET. Mail is IMAP over WebSocket — never HTTP — so
// message bodies are not stored here.

const CACHE = "mailiner-shell-v1";

const PRECACHE = [
  "/index.html",
  "/manifest.webmanifest",
  "/icons/icon-192.png",
  "/icons/icon-512.png",
  "/icons/icon-maskable-192.png",
  "/icons/icon-maskable-512.png",
  "/icons/apple-touch-icon.png",
];

function isAppShell(url) {
  if (url.origin !== self.location.origin) {
    return false;
  }
  const path = url.pathname;
  if (path === "/" || path === "/index.html") {
    return true;
  }
  if (path === "/manifest.webmanifest" || path === "/sw.js") {
    return true;
  }
  if (path.startsWith("/icons/") || path.startsWith("/assets/") || path.startsWith("/wasm/")) {
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
  if (url.origin !== self.location.origin) {
    return;
  }

  if (request.mode === "navigate") {
    event.respondWith(networkFirst(request, "/index.html"));
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
