import { test, expect } from '@playwright/test';
import {
  E2E_ACCOUNT_EMAIL,
  E2E_ACCOUNT_NAME,
  composeButton,
  gotoPath,
  gotoSettings,
  seedAccount,
} from './helpers';

test('empty store redirects settings routes to onboarding', async ({ page }) => {
  await gotoPath(page, '/settings');
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible({
    timeout: 15_000,
  });
  await expect(page).toHaveURL(/\/onboarding$/);
  await expect(page.getByRole('heading', { name: 'Settings' })).toHaveCount(0);
});

test('settings and account pages render after an account exists', async ({ page }) => {
  await seedAccount(page);
  await gotoPath(page, '/settings');

  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Appearance' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Composer' })).toBeVisible();
  await expect(page.getByLabel('Message list density')).toBeVisible();
  await expect(page.getByLabel('Compose window')).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Keyboard shortcuts' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Accounts' })).toHaveAttribute(
    'href',
    '/settings/accounts',
  );

  await gotoPath(page, '/settings/accounts');
  await expect(page.getByRole('heading', { name: 'Accounts', exact: true })).toBeVisible();
  await expect(page.getByText(E2E_ACCOUNT_NAME)).toBeVisible();
  await expect(page.getByText(E2E_ACCOUNT_EMAIL)).toBeVisible();
  await expect(page.getByText('Active', { exact: true })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Add account' })).toHaveAttribute(
    'href',
    '/settings/accounts/new',
  );
  await expect(page.getByRole('link', { name: 'Edit' })).toHaveAttribute(
    'href',
    '/settings/accounts/e2e-account-1',
  );

  await gotoPath(page, '/settings/accounts/new');
  await expect(page.getByText('Add account', { exact: true }).first()).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Your email' })).toBeVisible();
  await expect(page.getByLabel('Display name')).toBeVisible();
  await expect(page.locator('#account-new-email')).toBeVisible();
  await expect(page.getByLabel('Proxy base URL')).toHaveCount(0);

  await gotoPath(page, '/settings/accounts/e2e-account-1');
  await expect(page.getByRole('heading', { name: 'Edit account' })).toBeVisible();
  await expect(page.getByLabel('Display name')).toHaveValue(E2E_ACCOUNT_NAME);
  await expect(page.locator('#account-edit-email')).toHaveValue(E2E_ACCOUNT_EMAIL);
  await expect(page.getByRole('link', { name: 'Back to accounts' })).toBeVisible();

  await gotoPath(page, '/');
  await expect(page.locator('#app')).toBeVisible();
  await expect(page.getByRole('link', { name: 'Skip to message' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Settings' })).toHaveAttribute('href', '/settings');

  await gotoPath(page, '/settings');
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
});

test('deep-link to /settings/accounts stays on the accounts list', async ({ page }) => {
  await seedAccount(page);
  await gotoPath(page, '/settings/accounts');
  await expect(page.getByRole('heading', { name: 'Accounts', exact: true })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Settings' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Add account' })).toBeVisible();
});

test('settings home lists appearance, composer, filters, vacation, and privacy', async ({
  page,
}) => {
  await seedAccount(page);
  await gotoSettings(page);

  await expect(page.getByRole('heading', { name: 'Appearance' })).toBeVisible();
  await expect(page.getByLabel('Language')).toBeVisible();
  await expect(page.getByLabel('Message list density')).toBeVisible();
  await expect(page.getByLabel('Message list grouping')).toBeVisible();
  await expect(page.getByLabel('Mail layout')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Reset pane sizes' })).toBeVisible();

  await expect(page.getByRole('heading', { name: 'Composer' })).toBeVisible();
  await expect(page.getByLabel('Compose window')).toBeVisible();
  await expect(page.getByLabel('Default format')).toBeVisible();
  await expect(page.getByLabel('Default From')).toBeVisible();

  await expect(page.getByRole('heading', { name: 'Filters' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Add filter' })).toBeVisible();
  await expect(page.getByText('No filters yet.')).toBeVisible();

  await expect(page.getByRole('heading', { name: 'Vacation' })).toBeVisible();
  await expect(page.getByLabel('Subject')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Save vacation' })).toBeVisible();

  await expect(page.getByRole('heading', { name: 'Address book' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Privacy' })).toBeVisible();
  await expect(page.getByLabel('Remote images')).toBeVisible();
  await expect(page.getByRole('heading', { name: 'OpenPGP' })).toBeVisible();
  await expect(page.getByRole('heading', { name: 'Keyboard shortcuts' })).toBeVisible();
});

test('appearance density persists onto the message list', async ({ page }) => {
  await seedAccount(page, { cache: true });
  await gotoSettings(page);

  await page.getByLabel('Message list density').selectOption('compact');
  await page.getByRole('link', { name: 'Back to mail' }).click();
  await expect(composeButton(page)).toBeVisible();
  await expect(page.locator('.density-compact')).toBeVisible();
});

test('address book can add and remove a contact', async ({ page }) => {
  await seedAccount(page);
  await gotoSettings(page);

  await expect(page.getByText('No contacts yet.')).toBeVisible();
  await page.getByLabel('Name', { exact: true }).fill('Ada Lovelace');
  await page.getByLabel('Email', { exact: true }).fill('ada@example.com');
  await page.getByRole('button', { name: 'Add', exact: true }).click();

  await expect(page.getByText('Ada Lovelace')).toBeVisible();
  await expect(page.getByText('ada@example.com')).toBeVisible();
  await expect(page.getByText('No contacts yet.')).toHaveCount(0);

  await page.getByRole('button', { name: 'Remove ada@example.com' }).click();
  await expect(page.getByText('No contacts yet.')).toBeVisible();
});

test('vacation form saves locally', async ({ page }) => {
  await seedAccount(page);
  await gotoSettings(page);

  await page.locator('#settings-vacation-subject').fill('Out of office');
  await page.locator('#settings-vacation-body').fill('I am away.');
  await page.getByRole('button', { name: 'Save vacation' }).click();
  await expect(page.getByText('Vacation settings saved.')).toBeVisible();
  await expect(page.locator('#settings-vacation-subject')).toHaveValue('Out of office');
});

test('add filter opens the local rule editor', async ({ page }) => {
  await seedAccount(page);
  await gotoSettings(page);

  await page.getByRole('button', { name: 'Add filter' }).click();
  await expect(page.getByRole('heading', { name: 'New filter' })).toBeVisible();
  await expect(page.getByLabel('From contains')).toBeVisible();
  await expect(page.getByLabel('Subject contains')).toBeVisible();
  await expect(page.getByLabel('Move to folder')).toBeVisible();
  await page.getByRole('button', { name: 'Cancel' }).click();
  await expect(page.getByRole('heading', { name: 'New filter' })).toHaveCount(0);
  await expect(page.getByText('No filters yet.')).toBeVisible();
});

test('privacy remote-images pref persists across reload', async ({ page }) => {
  await seedAccount(page);
  await gotoSettings(page);

  await page.getByLabel('Remote images').selectOption('allow');
  await expect(page.getByLabel('Remote images')).toHaveValue('allow');

  await page.reload();
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
  await expect(page.getByLabel('Remote images')).toHaveValue('allow');
});
