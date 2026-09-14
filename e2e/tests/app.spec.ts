import { test, expect } from '@playwright/test';
import { gotoPath } from './helpers';

test('empty store shows first-run onboarding', async ({ page }) => {
  await gotoPath(page);

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
    '/manifest.webmanifest',
  );
  await expect(page.locator('meta[name="theme-color"]')).toHaveAttribute(
    'content',
    '#121212',
  );

  const res = await request.get('/manifest.webmanifest');
  expect(res.ok()).toBe(true);
  const manifest = await res.json();
  expect(manifest.name).toBe('Mailiner');
  expect(manifest.short_name).toBe('Mailiner');
  expect(manifest.display).toBe('standalone');
  expect(manifest.start_url).toBe('/');
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
    const res = await request.get(path);
    expect(res.ok(), path).toBe(true);
    expect(res.headers()['content-type'] ?? '').toMatch(/image\/png/);
  }
});

test('app-shell service worker is registered', async ({ page, request }) => {
  const sw = await request.get('/sw.js');
  expect(sw.ok()).toBe(true);
  const body = await sw.text();
  expect(body).toContain('mailiner-shell-v1');
  expect(body).toContain('isAppShell');

  await gotoPath(page);
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible();
  await expect
    .poll(async () =>
      page.evaluate(async () => {
        if (!('serviceWorker' in navigator)) {
          return false;
        }
        const regs = await navigator.serviceWorker.getRegistrations();
        return regs.some((reg) => {
          const script = reg.active ?? reg.waiting ?? reg.installing;
          return script?.scriptURL.includes('/sw.js') ?? false;
        });
      }),
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
