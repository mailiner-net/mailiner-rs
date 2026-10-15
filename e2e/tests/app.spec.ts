import { test, expect } from '@playwright/test';
import { appPath, gotoPath } from './helpers';

test('boot splash paints before wasm and follows the saved theme', async ({
  page,
}) => {
  // Production sends this as an HTTP header (see CONTENT_SECURITY_POLICY).
  // The inline theme script must keep running under that policy.
  const csp = [
    "default-src 'self'",
    "script-src 'self' 'unsafe-eval' 'wasm-unsafe-eval' 'sha256-KaoW6l5hTJxx84D7Fm09Vjr67WhRAgdifCVfrs4oYNk='",
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data: blob: http: https:",
    "font-src 'self'",
    "connect-src 'self' ws: wss: http: https:",
    "frame-src 'self' blob:",
    "object-src 'none'",
    "base-uri 'self'",
    "form-action 'self'",
  ].join('; ');

  await page.route('**/*', async (route) => {
    if (route.request().resourceType() !== 'document') {
      await route.continue();
      return;
    }
    const response = await route.fetch();
    const headers = response.headers();
    headers['content-security-policy'] = csp;
    await route.fulfill({
      status: response.status(),
      headers,
      body: await response.body(),
    });
  });
  await page.route('**/*.wasm', (route) => route.abort());
  await page.emulateMedia({ colorScheme: 'dark' });
  await page.goto(appPath('/'), { waitUntil: 'domcontentloaded' });

  const splash = page.locator('#mailiner-splash');
  await expect(splash).toBeVisible();
  await expect(splash).toContainText('Mailiner');
  await expect(splash).toContainText('Loading…');
  await expect(page.locator('#main')).toBeEmpty();
  await expect(page.locator('html')).not.toHaveAttribute('data-theme', /.+/);
  expect(
    await page.evaluate(() => getComputedStyle(document.body).backgroundColor),
  ).toBe('rgb(18, 18, 18)');

  await page.addInitScript(() => {
    // Matches `THEME_KEY` in ui_prefs.rs (`mailiner.ui.theme`).
    localStorage.setItem('mailiner.ui.theme', 'light');
  });
  await page.reload({ waitUntil: 'domcontentloaded' });
  await expect(splash).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  expect(
    await page.evaluate(() => getComputedStyle(document.body).backgroundColor),
  ).toBe('rgb(255, 255, 255)');
});

test('empty store shows first-run onboarding', async ({ page }) => {
  await gotoPath(page);
  await expect(page.locator('#mailiner-splash')).toHaveCount(0);

  // Empty localStorage → NeedsOnboarding → /onboarding (no mail chrome #app).
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible();
  await expect(page.getByRole('main')).toBeVisible();
  await expect(page.locator('.onboarding-shell')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Get started' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Save & continue' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Look up servers' })).toHaveCount(0);
  await expect(page.getByLabel('Unlock passphrase')).toHaveCount(0);
  await expect(page.getByLabel('Proxy base URL')).toHaveCount(0);
  await expect(page.locator('#app')).toHaveCount(0);
  await expect(page.getByRole('link', { name: 'Skip to message' })).toHaveCount(0);
});

test('viewport meta enables device-width media queries', async ({ page }) => {
  await gotoPath(page);
  // Dioxus may inject a second viewport tag after mount; either is sufficient.
  await expect(page.locator('meta[name="viewport"]').first()).toHaveAttribute(
    'content',
    /width=device-width/,
  );
});

test('web app manifest is linked and installable', async ({ page, request }) => {
  await gotoPath(page);
  await expect(page.locator('link[rel="manifest"]')).toHaveAttribute(
    'href',
    appPath('/manifest.webmanifest'),
  );
  await expect(page.locator('meta[name="theme-color"]')).toHaveAttribute(
    'content',
    '#121212',
  );

  const manifestPath = appPath('/manifest.webmanifest');
  const res = await request.get(manifestPath);
  expect(res.ok()).toBe(true);
  const manifest = await res.json();
  expect(manifest.name).toBe('Mailiner');
  expect(manifest.short_name).toBe('Mailiner');
  expect(manifest.display).toBe('standalone');
  // Relative to the manifest URL so the same file works at `/` and `/app/`.
  expect(manifest.start_url).toBe('./');
  expect(manifest.scope).toBe('./');
  expect(manifest.id).toBe('./');
  const manifestUrl = new URL(manifestPath, 'http://mailiner.test');
  const mountDir = appPath('/').endsWith('/') ? appPath('/') : `${appPath('/')}/`;
  expect(new URL(manifest.start_url, manifestUrl).pathname).toBe(mountDir);
  expect(new URL(manifest.scope, manifestUrl).pathname).toBe(mountDir);
  for (const icon of manifest.icons as { src: string; sizes: string }[]) {
    const iconPath = new URL(icon.src, manifestUrl).pathname;
    expect(iconPath.startsWith(`${mountDir}icons/`)).toBe(true);
  }
  expect(manifest.icons.some((icon: { sizes: string }) => icon.sizes === '192x192')).toBe(
    true,
  );
  expect(manifest.icons.some((icon: { sizes: string }) => icon.sizes === '512x512')).toBe(
    true,
  );
});

test('PWA icons are served', async ({ request }) => {
  for (const path of [
    '/icons/icon-192.png',
    '/icons/icon-512.png',
    '/icons/icon-maskable-192.png',
    '/icons/icon-maskable-512.png',
    '/icons/apple-touch-icon.png',
  ]) {
    const res = await request.get(appPath(path));
    expect(res.ok(), path).toBe(true);
    expect(res.headers()['content-type'] ?? '').toMatch(/image\/png/);
  }
});

test('app-shell service worker is registered', async ({ page, request }) => {
  const sw = await request.get(appPath('/sw.js'));
  expect(sw.ok()).toBe(true);
  const body = await sw.text();
  expect(body).toContain('mailiner-shell-v3');
  expect(body).toContain('isAppShell');

  await gotoPath(page);
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible();
  await expect
    .poll(async () =>
      page.evaluate(async (scriptPath) => {
        if (!('serviceWorker' in navigator)) {
          return false;
        }
        const regs = await navigator.serviceWorker.getRegistrations();
        return regs.some((reg) => {
          const script = reg.active ?? reg.waiting ?? reg.installing;
          return script?.scriptURL.endsWith(scriptPath) ?? false;
        });
      }, appPath('/sw.js')),
    )
    .toBe(true);
});

for (const viewport of [
  { name: 'phone', width: 390, height: 844 },
  { name: 'tablet', width: 768, height: 1024 },
] as const) {
  test(`${viewport.name} viewport keeps first-run onboarding usable`, async ({
    page,
  }) => {
    await page.setViewportSize({ width: viewport.width, height: viewport.height });
    await gotoPath(page);
    await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible();
    await expect(page.getByRole('button', { name: 'Get started' })).toBeVisible();
    const noHorizontalOverflow = await page.evaluate(
      () => document.documentElement.scrollWidth <= window.innerWidth + 1,
    );
    expect(noHorizontalOverflow).toBe(true);
  });
}

test('encrypted store shows unlock instead of mail chrome', async ({ page }) => {
  await page.addInitScript(() => {
    localStorage.setItem(
      'mailiner.accounts.v1',
      JSON.stringify({
        schema_version: 1,
        active_account_id: null,
        accounts: [],
        vault: {
          kdf: 'pbkdf2-sha256',
          iterations: 210000,
          salt_b64: 'AAAAAAAAAAAAAAAAAAAAAA==',
          cipher: 'aes-256-gcm',
          nonce_b64: 'AAAAAAAAAAAAAAAA',
          ciphertext_b64: 'AAAAAAAAAAAAAAAAAAAAAA==',
        },
      }),
    );
  });

  await gotoPath(page);
  await expect(page.getByRole('heading', { name: 'Unlock accounts' })).toBeVisible();
  await expect(page.getByRole('main')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Unlock' })).toBeVisible();
  await expect(page.locator('#app')).toHaveCount(0);
});
