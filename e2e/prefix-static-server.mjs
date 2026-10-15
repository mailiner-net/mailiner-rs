/**
 * Static server for a release artifact nested under a URL prefix.
 *
 * `serve -s` rewrites every miss to `/index.html`. A prefixed Pages artifact
 * keeps that file at `/app/index.html` and leaves `/` empty. This server
 * serves real files from `{MAILINER_E2E_SERVE_DIR}/{prefix}` only, redirects
 * the bare prefix to the trailing-slash URL, and falls back to
 * `{prefix}/index.html` for other paths under the prefix. Upload-root files
 * such as `_headers` and paths outside the prefix are 404.
 *
 * Env: `MAILINER_E2E_SERVE_DIR`, `MAILINER_E2E_PORT`, `MAILINER_E2E_BASE_PATH`.
 */
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';

const port = Number(process.env.MAILINER_E2E_PORT ?? '8080');
const root = path.resolve(process.env.MAILINER_E2E_SERVE_DIR ?? '');
const prefixName = (process.env.MAILINER_E2E_BASE_PATH ?? '').replace(/^\/+|\/+$/g, '');
const prefix = prefixName ? `/${prefixName}` : '';
const prefixRoot = path.resolve(root, prefixName);

const TYPES = {
  '.html': 'text/html; charset=utf-8',
  '.js': 'text/javascript; charset=utf-8',
  '.css': 'text/css; charset=utf-8',
  '.wasm': 'application/wasm',
  '.json': 'application/json',
  '.webmanifest': 'application/manifest+json',
  '.png': 'image/png',
  '.svg': 'image/svg+xml',
  '.ico': 'image/x-icon',
  '.woff': 'font/woff',
  '.woff2': 'font/woff2',
};

if (!prefix || !fs.existsSync(root)) {
  console.error('prefix-static-server: set MAILINER_E2E_BASE_PATH and MAILINER_E2E_SERVE_DIR');
  process.exit(1);
}

function underPrefix(file) {
  const resolved = path.resolve(file);
  return resolved === prefixRoot || resolved.startsWith(prefixRoot + path.sep);
}

function sendFile(res, file) {
  const type = TYPES[path.extname(file)] ?? 'application/octet-stream';
  res.writeHead(200, { 'content-type': type, 'cache-control': 'no-cache' });
  fs.createReadStream(file).pipe(res);
}

const server = http.createServer((req, res) => {
  // Use the URL parser's pathname only. A second decode would turn
  // `/app/..%2f_headers` into a path outside the prefix.
  let pathname;
  try {
    pathname = new URL(req.url ?? '/', 'http://127.0.0.1').pathname;
  } catch {
    res.writeHead(400).end('bad path');
    return;
  }
  if (pathname.includes('\0') || pathname.includes('\\') || pathname.includes('%')) {
    res.writeHead(400).end('bad path');
    return;
  }
  if (pathname === prefix) {
    res.writeHead(301, { location: `${prefix}/` }).end();
    return;
  }
  const inPrefix = pathname === `${prefix}/` || pathname.startsWith(`${prefix}/`);
  if (!inPrefix) {
    res.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' }).end('not found');
    return;
  }

  const rel = pathname.slice(prefix.length + 1);
  const file = path.resolve(prefixRoot, rel);
  if (!underPrefix(file)) {
    res.writeHead(400).end('bad path');
    return;
  }
  if (rel && fs.existsSync(file) && fs.statSync(file).isFile()) {
    sendFile(res, file);
    return;
  }

  const index = path.resolve(prefixRoot, 'index.html');
  if (underPrefix(index) && fs.existsSync(index)) {
    sendFile(res, index);
    return;
  }
  res.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' }).end('not found');
});

server.listen(port, '127.0.0.1', () => {
  console.log(`prefix static server http://127.0.0.1:${port}${prefix}/`);
});
