import { test, expect } from '@playwright/test';

test('empty store shows first-run onboarding', async ({ page }) => {
  await page.goto('/');

  // Empty localStorage → NeedsOnboarding → /onboarding (no mail chrome #app).
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible();
  await expect(page.locator('.onboarding-shell')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Save & continue' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Test connection' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Look up servers' })).toBeVisible();
  await expect(page.locator('#app')).toHaveCount(0);
});
