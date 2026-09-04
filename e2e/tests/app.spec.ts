import { test, expect } from '@playwright/test';

test('empty store shows first-run onboarding', async ({ page }) => {
  await page.goto('/');

  // Empty localStorage → NeedsOnboarding → /onboarding (no mail chrome #app).
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible();
  await expect(page.locator('.onboarding-shell')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Save & continue' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Test connection' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Look up servers' })).toBeVisible();
  await expect(page.getByLabel('Unlock passphrase')).toBeVisible();
  await expect(page.locator('#app')).toHaveCount(0);
});

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

  await page.goto('/');
  await expect(page.getByRole('heading', { name: 'Unlock accounts' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Unlock' })).toBeVisible();
  await expect(page.locator('#app')).toHaveCount(0);
});
