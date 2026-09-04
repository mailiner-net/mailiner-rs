import { test, expect } from '@playwright/test';
import { E2E_ACCOUNT_EMAIL, E2E_ACCOUNT_NAME, seedAccount } from './helpers';

test('empty store redirects settings routes to onboarding', async ({ page }) => {
  await page.goto('/settings');
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible({
    timeout: 15_000,
  });
  await expect(page).toHaveURL(/\/onboarding$/);
  await expect(page.getByRole('heading', { name: 'Settings' })).toHaveCount(0);
});

test('settings and account pages render after an account exists', async ({ page }) => {
  await seedAccount(page);
  await page.goto('/settings');

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

  await page.goto('/settings/accounts');
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

  await page.goto('/settings/accounts/new');
  await expect(page.getByRole('heading', { name: 'Add account' })).toBeVisible();
  await expect(page.getByLabel('Display name')).toBeVisible();
  await expect(page.locator('#account-new-email')).toBeVisible();

  await page.goto('/settings/accounts/e2e-account-1');
  await expect(page.getByRole('heading', { name: 'Edit account' })).toBeVisible();
  await expect(page.getByLabel('Display name')).toHaveValue(E2E_ACCOUNT_NAME);
  await expect(page.locator('#account-edit-email')).toHaveValue(E2E_ACCOUNT_EMAIL);
  await expect(page.getByRole('link', { name: 'Back to accounts' })).toBeVisible();

  await page.goto('/');
  await expect(page.locator('#app')).toBeVisible();
  await expect(page.getByRole('link', { name: 'Skip to message' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Settings' })).toHaveAttribute('href', '/settings');

  await page.goto('/settings');
  await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
});

test('deep-link to /settings/accounts stays on the accounts list', async ({ page }) => {
  await seedAccount(page);
  await page.goto('/settings/accounts');
  await expect(page.getByRole('heading', { name: 'Accounts', exact: true })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Settings' })).toBeVisible();
  await expect(page.getByRole('link', { name: 'Add account' })).toBeVisible();
});
