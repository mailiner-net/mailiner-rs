import { test, expect } from '@playwright/test';
import { composeButton, gotoMail, pressShortcut, seedAccount } from './helpers';

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
  await expect(composeButton(page)).toBeVisible();
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

test('c opens compose and j opens the folder picker', async ({ page }) => {
  await seedAccount(page, { cache: true });
  await gotoMail(page);

  await pressShortcut(page, 'c');
  const compose = page.getByRole('dialog', { name: 'New message' }).or(
    page.getByRole('region', { name: 'New message' }),
  );
  await expect(compose).toBeVisible();
  await compose.getByRole('button', { name: 'Close', exact: true }).last().click();
  await expect(compose).toHaveCount(0);

  await pressShortcut(page, 'j');
  const jump = page.getByRole('dialog', { name: 'Go to folder' });
  await expect(jump).toBeVisible();
  await expect(jump.getByRole('option', { name: 'Inbox' })).toBeVisible();
  await jump.getByRole('button', { name: 'Close' }).click();
  await expect(jump).toHaveCount(0);
});
