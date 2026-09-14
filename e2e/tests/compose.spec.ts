import { test, expect } from '@playwright/test';
import {
  composeButton,
  composeOverlay,
  gotoMail,
  gotoPath,
  pressShortcut,
  seedAccount,
} from './helpers';

test('compose FAB opens and close restores mail chrome', async ({ page }) => {
  await seedAccount(page);
  await gotoMail(page);

  const fab = composeButton(page);
  await expect(fab).toBeEnabled();
  await fab.click();

  const overlay = composeOverlay(page);
  await expect(overlay).toBeVisible();
  await expect(composeButton(page)).toHaveCount(0);
  await expect(page.locator('#app')).toBeVisible();
  await expect(page.getByRole('link', { name: 'Skip to message' })).toBeVisible();
  await expect(overlay.getByLabel('From')).toBeVisible();
  await overlay.getByRole('button', { name: 'Close', exact: true }).last().click();
  await expect(overlay).toHaveCount(0);
  await expect(composeButton(page)).toBeVisible();
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
  await expect(composeButton(page)).toBeVisible();
});

test('compose overlay exposes from, recipients, subject, body, and send', async ({
  page,
}) => {
  await seedAccount(page);
  await gotoMail(page);

  await composeButton(page).click();
  const overlay = composeOverlay(page);
  await expect(overlay).toBeVisible();

  await expect(overlay.getByLabel('From')).toBeVisible();
  await expect(overlay.getByRole('combobox', { name: 'To' })).toBeVisible();
  await expect(overlay.getByLabel('Subject')).toBeVisible();
  await expect(overlay.getByText('Message', { exact: true })).toBeVisible();
  await expect(overlay.getByRole('button', { name: 'Plain' })).toBeVisible();
  await expect(overlay.getByRole('button', { name: 'Rich' })).toBeVisible();
  await expect(overlay.getByRole('button', { name: 'Attach files' })).toBeVisible();
  await expect(overlay.getByRole('button', { name: 'Send' })).toBeVisible();
  await expect(overlay.getByRole('button', { name: 'Discard' })).toBeVisible();

  await overlay.getByRole('button', { name: 'Cc/Bcc' }).click();
  await expect(overlay.getByRole('combobox', { name: 'Cc', exact: true })).toBeVisible();
  await expect(overlay.getByRole('combobox', { name: 'Bcc', exact: true })).toBeVisible();

  await overlay.getByRole('combobox', { name: 'To' }).fill('ada@example.com');
  await overlay.getByRole('combobox', { name: 'To' }).press('Enter');
  await expect(overlay.getByText('ada@example.com')).toBeVisible();
  await overlay.getByLabel('Subject').fill('Hello from e2e');
  await expect(overlay.getByLabel('Subject')).toHaveValue('Hello from e2e');
});

test('send without a recipient stays on the overlay with an error', async ({ page }) => {
  await seedAccount(page);
  await gotoMail(page);

  await composeButton(page).click();
  const overlay = composeOverlay(page);
  await expect(overlay).toBeVisible();
  await overlay.getByRole('button', { name: 'Send' }).click();
  await expect(overlay.getByText(/Cannot send/)).toBeVisible();
  await expect(overlay).toBeVisible();
});

test('settings compose window docked opens a region instead of a dialog', async ({
  page,
}) => {
  await seedAccount(page);
  await gotoPath(page, '/settings');
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  await page.getByLabel('Compose window').selectOption('docked');

  await gotoPath(page, '/');
  await expect(composeButton(page)).toBeVisible();
  await composeButton(page).click();

  await expect(page.getByRole('region', { name: 'New message' })).toBeVisible();
  await expect(page.getByRole('dialog', { name: 'New message' })).toHaveCount(0);
  await expect(page.getByRole('navigation', { name: 'Folders' })).toBeVisible();
});
