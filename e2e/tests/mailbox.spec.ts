import { test, expect } from '@playwright/test';
import {
  E2E_MESSAGE_FROM,
  E2E_MESSAGE_SUBJECT,
  gotoMail,
  pressShortcut,
  seedAccount,
} from './helpers';

test('empty cache shows mailbox chrome and jump picker', async ({ page }) => {
  await seedAccount(page);
  await gotoMail(page);

  await expect(page.getByRole('navigation', { name: 'Folders' })).toBeVisible();
  await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible();
  await expect(page.getByText('Select a mailbox')).toBeVisible();
  await expect(page.getByRole('main', { name: 'Message' })).toBeVisible();
  await expect(page.getByText('Select a message')).toBeVisible();

  await pressShortcut(page, 'j');
  const picker = page.getByRole('dialog', { name: 'Go to folder' });
  await expect(picker).toBeVisible();
  await expect(picker.getByLabel('Filter folders')).toBeVisible();
  await expect(picker.getByText('No matching folders')).toBeVisible();

  await picker.getByRole('button', { name: 'Close' }).click();
  await expect(picker).toHaveCount(0);
  await expect(page.getByText('Select a mailbox')).toBeVisible();
});

test('move without a selection shows a toast and does not open the picker', async ({
  page,
}) => {
  await seedAccount(page);
  await gotoMail(page);

  await pressShortcut(page, 'm');
  await expect(page.getByText('Select a message first')).toBeVisible();
  await expect(page.getByRole('dialog', { name: 'Move to folder' })).toHaveCount(0);
});

test('seeded cache lists a message, opens it, and drives the folder picker', async ({
  page,
}) => {
  await seedAccount(page, { cache: true });
  await gotoMail(page);

  await expect(page.getByRole('treeitem', { name: /Inbox/ })).toBeVisible();
  await expect(page.getByRole('treeitem', { name: 'Archive' })).toBeVisible();
  await expect(page.getByRole('treeitem', { name: 'Sent' })).toBeVisible();

  const row = page.getByRole('option', {
    name: `${E2E_MESSAGE_FROM}, ${E2E_MESSAGE_SUBJECT}`,
  });
  await expect(row).toBeVisible();
  await expect(page.getByText(E2E_MESSAGE_SUBJECT)).toBeVisible();
  await expect(page.getByText('Hello from the seeded cache.')).toBeVisible();

  await row.click();
  await expect(row).toHaveAttribute('aria-selected', 'true', { timeout: 15_000 });
  await expect(page.getByText(/Failed to load message: Not connected/)).toBeVisible();

  await pressShortcut(page, 'm');
  const movePicker = page.getByRole('dialog', { name: 'Move to folder' });
  await expect(movePicker).toBeVisible();
  await expect(movePicker.getByRole('option', { name: 'Archive' })).toBeVisible();
  await expect(movePicker.getByRole('option', { name: 'Sent' })).toBeVisible();
  await expect(movePicker.getByRole('option', { name: 'Inbox' })).toHaveCount(0);

  await movePicker.getByRole('button', { name: 'Close' }).click();
  await expect(movePicker).toHaveCount(0);

  await pressShortcut(page, 'j');
  const jumpPicker = page.getByRole('dialog', { name: 'Go to folder' });
  await expect(jumpPicker).toBeVisible();
  await expect(jumpPicker.getByRole('option', { name: 'Inbox' })).toBeVisible();
  await expect(jumpPicker.getByRole('option', { name: 'Archive' })).toBeVisible();
  await jumpPicker.getByRole('button', { name: 'Close' }).click();
  await expect(jumpPicker).toHaveCount(0);
});
