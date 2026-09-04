import { test, expect } from '@playwright/test';
import { gotoMail, pressShortcut, seedAccount } from './helpers';

function composeOverlay(page: import('@playwright/test').Page) {
  return page.getByRole('dialog', { name: 'New message' }).or(
    page.getByRole('region', { name: 'New message' }),
  );
}

test('compose FAB opens and close restores mail chrome', async ({ page }) => {
  await seedAccount(page);
  await gotoMail(page);

  const fab = page.getByRole('button', { name: 'Compose' });
  await expect(fab).toBeEnabled();
  await fab.click();

  const overlay = composeOverlay(page);
  await expect(overlay).toBeVisible();
  await expect(page.getByRole('button', { name: 'Compose' })).toHaveCount(0);
  await expect(page.locator('#app')).toBeVisible();
  await expect(page.getByRole('link', { name: 'Skip to message' })).toBeVisible();
  await expect(overlay.getByLabel('From')).toBeVisible();
  await overlay.getByRole('button', { name: 'Close', exact: true }).last().click();
  await expect(overlay).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Compose' })).toBeVisible();
  await expect(page.locator('#app')).toBeVisible();
});

test('compose shortcut opens overlay and close restores chrome', async ({ page }) => {
  await seedAccount(page);
  await gotoMail(page);

  await pressShortcut(page, 'c');
  const overlay = composeOverlay(page);
  await expect(overlay).toBeVisible();

  await overlay.getByRole('button', { name: 'Close', exact: true }).last().click();
  await expect(overlay).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Compose' })).toBeVisible();
});
