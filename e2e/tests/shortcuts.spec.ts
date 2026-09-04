import { test, expect } from '@playwright/test';
import { gotoMail, pressShortcut, seedAccount } from './helpers';

test('question mark opens keyboard shortcuts help', async ({ page }) => {
  await seedAccount(page);
  await gotoMail(page);

  await pressShortcut(page, '?');
  const dialog = page.getByRole('dialog', { name: 'Keyboard shortcuts' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('heading', { name: 'Keyboard shortcuts' })).toBeVisible();
  await expect(dialog.getByRole('heading', { name: 'Mail' })).toBeVisible();
  await expect(dialog.getByRole('heading', { name: 'Reading' })).toBeVisible();
  await expect(dialog.getByRole('heading', { name: 'Help' })).toBeVisible();
  await expect(dialog.getByText('New message')).toBeVisible();
  await expect(dialog.getByText('Go to folder')).toBeVisible();
  await expect(dialog.getByText('Show keyboard shortcuts')).toBeVisible();

  await pressShortcut(page, 'Escape');
  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Compose' })).toBeVisible();
});

test('shortcuts help close button dismisses the dialog', async ({ page }) => {
  await seedAccount(page);
  await gotoMail(page);

  await pressShortcut(page, '?');
  const dialog = page.getByRole('dialog', { name: 'Keyboard shortcuts' });
  await expect(dialog).toBeVisible();
  await dialog.getByRole('button', { name: 'Close' }).click();
  await expect(dialog).toHaveCount(0);
});
