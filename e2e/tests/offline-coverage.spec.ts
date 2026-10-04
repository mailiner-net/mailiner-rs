import { test, expect } from '@playwright/test';
import {
  E2E_ACCOUNT_ID,
  composeButton,
  composeOverlay,
  gotoMail,
  gotoPath,
  gotoSettings,
  pressShortcut,
  seedAccount,
} from './helpers';

const PASSPHRASE = 'correct-horse';

async function encryptStore(page: import('@playwright/test').Page) {
  await seedAccount(page);
  await gotoPath(page, '/settings/accounts');
  await expect(page.getByRole('heading', { name: 'Accounts', exact: true })).toBeVisible();
  await page.locator('#vault-new-passphrase').fill(PASSPHRASE);
  await page.locator('#vault-confirm-passphrase').fill(PASSPHRASE);
  await page.getByRole('button', { name: 'Encrypt stored secrets' }).click();
  await expect(page.getByRole('button', { name: 'Lock now' })).toBeVisible();
}

test('encrypt, lock, and unlock the local store', async ({ page }) => {
  await encryptStore(page);
  const stored = await page.evaluate(() => localStorage.getItem('mailiner.accounts.v1') ?? '');
  expect(stored).not.toContain('not-a-real-password');
  expect(stored).toContain('"vault"');

  await page.getByRole('button', { name: 'Lock now' }).click();
  await expect(page.getByRole('heading', { name: 'Unlock accounts' })).toBeVisible();
  await page.reload();
  await expect(page.getByRole('heading', { name: 'Unlock accounts' })).toBeVisible();
  await page.getByLabel('Unlock passphrase').fill('wrong-password');
  await page.getByRole('button', { name: 'Unlock', exact: true }).click();
  await expect(page.getByText('That passphrase is incorrect.')).toBeVisible();

  await page.getByLabel('Unlock passphrase').fill(PASSPHRASE);
  await page.getByRole('button', { name: 'Unlock', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Accounts', exact: true })).toBeVisible();
});

test('change passphrase invalidates the old one', async ({ page }) => {
  await encryptStore(page);
  const next = 'battery-staple';
  await page.locator('#vault-current-passphrase').fill(PASSPHRASE);
  await page.locator('#vault-change-passphrase').fill(next);
  await page.locator('#vault-change-confirm').fill(next);
  await page.getByRole('button', { name: 'Change passphrase' }).click();
  await expect(page.locator('#vault-current-passphrase')).toHaveValue('');

  await page.getByRole('button', { name: 'Lock now' }).click();
  await page.getByLabel('Unlock passphrase').fill(PASSPHRASE);
  await page.getByRole('button', { name: 'Unlock', exact: true }).click();
  await expect(page.getByText('That passphrase is incorrect.')).toBeVisible();
  await page.getByLabel('Unlock passphrase').fill(next);
  await page.getByRole('button', { name: 'Unlock', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Accounts', exact: true })).toBeVisible();
});

test('remove encryption drops the unlock screen', async ({ page }) => {
  await encryptStore(page);
  await page.locator('#vault-current-passphrase').fill(PASSPHRASE);
  await page.getByRole('button', { name: 'Remove encryption' }).click();
  await expect(page.getByRole('button', { name: 'Encrypt stored secrets' })).toBeVisible();
  await page.reload();
  await expect(page.getByRole('heading', { name: 'Accounts', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Unlock accounts' })).toHaveCount(0);
  const stored = await page.evaluate(() => localStorage.getItem('mailiner.accounts.v1') ?? '');
  expect(stored).toContain('not-a-real-password');
});

test('a too-short passphrase is rejected', async ({ page }) => {
  await seedAccount(page);
  await gotoPath(page, '/settings/accounts');
  await page.locator('#vault-new-passphrase').fill('short');
  await page.locator('#vault-confirm-passphrase').fill('short');
  await page.getByRole('button', { name: 'Encrypt stored secrets' }).click();
  await expect(page.getByText(/at least 8 characters/)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Lock now' })).toHaveCount(0);
});

test('remapping a shortcut persists and conflicts are refused', async ({ page }) => {
  await seedAccount(page);
  await gotoSettings(page);
  const reply = page.getByRole('button', {
    name: 'Change shortcut for Reply, currently R',
    exact: true,
  });
  await reply.scrollIntoViewIfNeeded();
  await reply.click();
  await page.keyboard.press('x');
  await expect(
    page.getByRole('button', { name: 'Change shortcut for Reply, currently X', exact: true }),
  ).toBeVisible();

  const forward = page.getByRole('button', {
    name: 'Change shortcut for Forward, currently F',
    exact: true,
  });
  await forward.click();
  await page.keyboard.press('x');
  await expect(page.getByRole('alert')).toContainText(/Already used by Reply/);

  await page.reload();
  await expect(
    page.getByRole('button', { name: 'Change shortcut for Reply, currently X', exact: true }),
  ).toBeVisible();

  await page.getByRole('link', { name: 'Back to mail' }).click();
  await expect(composeButton(page)).toBeVisible();
  await pressShortcut(page, '?');
  const help = page.getByRole('dialog', { name: 'Keyboard shortcuts' });
  await expect(help.getByText('Reply', { exact: true })).toBeVisible();
  await expect(help.locator('kbd', { hasText: 'X' }).first()).toBeVisible();
  await pressShortcut(page, 'Escape');

  await gotoSettings(page);
  await page.getByRole('button', { name: 'Restore defaults' }).click();
  await expect(
    page.getByRole('button', { name: 'Change shortcut for Reply, currently R', exact: true }),
  ).toBeVisible();
});

test('classic layout is applied on the mail screen', async ({ page }) => {
  await seedAccount(page);
  await gotoSettings(page);
  await page.getByLabel('Mail layout').selectOption('classic');
  await page.getByRole('link', { name: 'Back to mail' }).click();
  await expect(page.locator('#app')).toHaveClass(/layout-classic/);
  await expect(page.getByRole('navigation', { name: 'Folders' })).toBeVisible();
  await expect(page.getByRole('listbox', { name: 'Messages' })).toBeVisible();
  await expect(page.getByRole('main', { name: 'Message' })).toBeVisible();

  await gotoSettings(page);
  await page.getByLabel('Mail layout').selectOption('stacked');
  await page.getByRole('link', { name: 'Back to mail' }).click();
  await expect(page.locator('#app')).toHaveClass(/layout-stacked/);
  await expect(page.locator('#app')).not.toHaveClass(/layout-classic/);
});

test('conversation grouping persists', async ({ page }) => {
  await seedAccount(page);
  await gotoSettings(page);
  await page.getByLabel('Message list grouping').selectOption('conversations');
  await page.reload();
  await expect(page.getByLabel('Message list grouping')).toHaveValue('conversations');
});

test('the default compose format opens Rich', async ({ page }) => {
  await seedAccount(page);
  await gotoSettings(page);
  await page.getByLabel('Default format').selectOption('rich');
  await page.getByRole('link', { name: 'Back to mail' }).click();
  await composeButton(page).click();
  const compose = composeOverlay(page);
  await expect(compose.getByRole('button', { name: 'Bold', exact: true })).toBeVisible();
  await expect(compose.locator('#mailiner-compose-editor')).toBeVisible();
});

test('an address-book contact is offered in To', async ({ page }) => {
  await seedAccount(page);
  await gotoSettings(page);
  await page.getByLabel('Name', { exact: true }).fill('Ada Lovelace');
  await page.getByLabel('Email', { exact: true }).fill('ada@example.com');
  await page.getByRole('button', { name: 'Add', exact: true }).click();
  await expect(page.getByText('ada@example.com')).toBeVisible();

  await page.getByRole('link', { name: 'Back to mail' }).click();
  await composeButton(page).click();
  const to = composeOverlay(page).getByRole('combobox', { name: 'To' });
  await to.fill('ada');
  await expect(page.getByRole('option', { name: /ada@example.com/ })).toBeVisible();
  await page.getByRole('option', { name: /ada@example.com/ }).click();
  await expect(composeOverlay(page).getByRole('button', { name: 'Remove Ada Lovelace' })).toBeVisible();
});

test('a saved filter survives reload', async ({ page }) => {
  await seedAccount(page);
  await gotoSettings(page);
  await page.getByRole('button', { name: 'Add filter' }).click();
  await page.locator('#settings-filter-name').fill('Encoder star');
  await page.getByLabel('From contains').fill('encoder@');
  await page.getByLabel('Star', { exact: true }).check();
  await page.locator('.settings-filter-form').getByRole('button', { name: 'Add filter' }).click();
  await expect(page.getByText('Encoder star')).toBeVisible();

  await page.reload();
  await expect(page.getByText('Encoder star')).toBeVisible();
  await expect(page.getByLabel('Enable Encoder star')).toBeChecked();
  await page.getByLabel('Enable Encoder star').uncheck();
  await page.reload();
  await expect(page.getByLabel('Enable Encoder star')).not.toBeChecked();
  await page.getByRole('button', { name: 'Delete Encoder star' }).click();
  await expect(page.getByText('No filters yet.')).toBeVisible();
});

test('editing the display name survives reload', async ({ page }) => {
  await seedAccount(page);
  await gotoPath(page, `/settings/accounts/${E2E_ACCOUNT_ID}`);
  await expect(page.getByRole('heading', { name: 'Edit account' })).toBeVisible();
  await page.getByLabel('Display name').fill('Renamed E2E');
  await page.getByRole('button', { name: 'Save changes' }).click();
  await expect(page.getByRole('heading', { name: 'Accounts', exact: true })).toBeVisible();
  await expect(page.getByText('Renamed E2E')).toBeVisible();
  await page.reload();
  await expect(page.getByText('Renamed E2E')).toBeVisible();
  await gotoMail(page);
  await expect(page.locator('a.pane-header-account-link')).toHaveText('Renamed E2E');
});

test('deleting the only offline account returns to onboarding', async ({ page }) => {
  await seedAccount(page);
  await gotoPath(page, '/settings/accounts');
  await page.getByRole('button', { name: 'Delete', exact: true }).click();
  await page.getByRole('button', { name: 'Delete account', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible();
  await gotoPath(page, '/');
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible();
});

test('the phone compose sheet can be filled and closed', async ({ page }) => {
  await seedAccount(page, { cache: true });
  await page.setViewportSize({ width: 390, height: 844 });
  await gotoPath(page, '/');
  await expect(composeButton(page)).toBeVisible();
  await composeButton(page).click();
  const compose = composeOverlay(page);
  await compose.getByRole('combobox', { name: 'To' }).fill('ada@example.com');
  await compose.getByRole('combobox', { name: 'To' }).press('Enter');
  await compose.getByLabel('Subject').fill('Phone note');
  await compose.getByRole('button', { name: 'Close', exact: true }).last().click();
  await expect(compose).toHaveCount(0);
  await expect(page.getByRole('treeitem', { name: /Inbox/ })).toBeVisible();
});

test('going offline still paints the shell', async ({ page }) => {
  await gotoPath(page);
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible();
  await expect
    .poll(async () =>
      page.evaluate(async () => {
        const regs = await navigator.serviceWorker.getRegistrations();
        return regs.some((reg) => reg.active?.scriptURL.includes('/sw.js'));
      }),
    )
    .toBe(true);
  // The first navigation may have finished before the worker claimed the page.
  // Reload once online so the worker caches the shell, then reload offline.
  await page.reload();
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible();
  await page.context().setOffline(true);
  await page.reload();
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible();
});
