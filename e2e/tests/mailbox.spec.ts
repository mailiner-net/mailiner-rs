import { test, expect } from '@playwright/test';
import {
  E2E_ACCOUNT_NAME,
  E2E_MESSAGE_FROM,
  E2E_MESSAGE_SUBJECT,
  gotoMail,
  gotoPath,
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

test('folder tree switches mailboxes and list filters hide unmatched rows', async ({
  page,
}) => {
  await seedAccount(page, { cache: true });
  await gotoMail(page);

  const inboxRow = page.getByRole('option', {
    name: `${E2E_MESSAGE_FROM}, ${E2E_MESSAGE_SUBJECT}`,
  });
  await expect(inboxRow).toBeVisible();

  await page.getByRole('treeitem', { name: 'Archive' }).click();
  await expect(page.getByText('No messages')).toBeVisible();
  await expect(inboxRow).toHaveCount(0);

  await page.getByRole('treeitem', { name: /Inbox/ }).click();
  await expect(inboxRow).toBeVisible();

  await page.getByRole('button', { name: 'Show flagged messages' }).click();
  await expect(page.getByText('No matching messages')).toBeVisible();
  await expect(inboxRow).toHaveCount(0);

  await page.getByRole('button', { name: 'Show flagged messages' }).click();
  await expect(inboxRow).toBeVisible();
});

test('theme select writes data-theme on the document', async ({ page }) => {
  await seedAccount(page);
  await gotoMail(page);

  const theme = page.getByLabel('Color theme');
  await expect(theme).toBeVisible();

  await theme.selectOption('dark');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');

  await theme.selectOption('light');
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');

  await theme.selectOption('system');
  await expect(page.locator('html')).not.toHaveAttribute('data-theme', /light|dark/);
});

test('mail chrome links to settings and accounts', async ({ page }) => {
  await seedAccount(page);
  await gotoMail(page);

  await expect(page.getByRole('link', { name: 'Settings' })).toHaveAttribute(
    'href',
    '/settings',
  );
  await expect(page.getByRole('link', { name: E2E_ACCOUNT_NAME })).toHaveAttribute(
    'href',
    '/settings/accounts',
  );

  await page.getByRole('link', { name: 'Settings' }).click();
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
});

test('phone viewport walks folders → list → viewer', async ({ page }) => {
  await seedAccount(page, { cache: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await gotoPath(page, '/');
  await expect(page.locator('#app')).toBeVisible();
  await expect(page.getByRole('treeitem', { name: /Inbox/ })).toBeVisible();

  await page.getByRole('treeitem', { name: /Inbox/ }).click();
  const row = page.getByRole('option', {
    name: `${E2E_MESSAGE_FROM}, ${E2E_MESSAGE_SUBJECT}`,
  });
  await expect(row).toBeVisible();
  await expect(page.getByRole('button', { name: 'Back' })).toBeVisible();

  await row.click();
  await expect(page.getByText(/Failed to load message: Not connected/)).toBeVisible();
  await page.getByRole('button', { name: 'Back' }).click();
  await expect(row).toBeVisible();
});
