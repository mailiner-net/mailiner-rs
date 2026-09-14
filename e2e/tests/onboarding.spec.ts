import { test, expect } from '@playwright/test';
import { gotoPath, wizardContinue } from './helpers';

function validationStatus(page: import('@playwright/test').Page) {
  return page.locator('.onboarding-status-error');
}

async function startWizard(page: import('@playwright/test').Page) {
  await gotoPath(page);
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible();
  await page.getByRole('button', { name: 'Get started' }).click();
  await expect(page.getByRole('heading', { name: 'Your email' })).toBeVisible();
}

test('first-run wizard starts on Welcome and hides the old single-page form', async ({
  page,
}) => {
  await gotoPath(page);
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Get started' })).toBeVisible();
  await expect(page.getByRole('group', { name: 'Setup progress' })).toBeVisible();
  await expect(page.getByText('Step 1 of')).toBeVisible();
  await expect(page.locator('#onboarding-display-name')).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Look up servers' })).toHaveCount(0);
  await expect(page.getByLabel('Proxy base URL')).toHaveCount(0);
});

test('email step requires a name and a full address', async ({ page }) => {
  await startWizard(page);

  await wizardContinue(page);
  await expect(validationStatus(page)).toContainText('Validation');
  await expect(validationStatus(page)).toContainText('Enter a display name.');
  await expect(page.getByRole('heading', { name: 'Your email' })).toBeVisible();

  await page.locator('#onboarding-display-name').fill('Ada Lovelace');
  await wizardContinue(page);
  await expect(validationStatus(page)).toContainText(
    'Enter an email address (user@example.com).',
  );
});

test('wizard walks identity → sign-in → servers', async ({ page }) => {
  await startWizard(page);

  await expect(page.locator('#onboarding-display-name')).toBeVisible();
  await expect(page.locator('#onboarding-email')).toBeVisible();
  await page.locator('#onboarding-display-name').fill('Ada Lovelace');
  await page.locator('#onboarding-email').fill('ada@example.com');
  await wizardContinue(page);

  await expect(page.getByRole('heading', { name: 'Sign in' })).toBeVisible();
  await expect(page.locator('#onboarding-imap-password')).toBeVisible();
  await expect(page.locator('#onboarding-auth-kind')).toHaveValue('password');
});

test('OAuth sign-in fields appear when the method is switched', async ({ page }) => {
  await startWizard(page);
  await page.locator('#onboarding-display-name').fill('Ada Lovelace');
  await page.locator('#onboarding-email').fill('ada@example.com');
  await wizardContinue(page);

  await page.locator('#onboarding-auth-kind').selectOption('oauth2');
  await expect(page.locator('#onboarding-oauth-provider')).toBeVisible();
  await expect(page.getByLabel('OAuth client ID')).toBeVisible();
  await expect(page.getByLabel('Redirect URI')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Sign in with Google' })).toBeVisible();

  await page.locator('#onboarding-oauth-provider').selectOption('microsoft');
  await expect(page.getByLabel('Microsoft tenant (optional)')).toBeVisible();
  await expect(page.getByRole('button', { name: 'Sign in with Microsoft' })).toBeVisible();
});

test('mismatched unlock passphrase is rejected before connect', async ({ page }) => {
  await startWizard(page);
  await page.locator('#onboarding-display-name').fill('Ada Lovelace');
  await page.locator('#onboarding-email').fill('ada@example.com');
  await wizardContinue(page);

  await page.locator('#onboarding-imap-password').fill('secret');
  await wizardContinue(page);

  await expect(page.getByRole('heading', { name: 'Mail servers' })).toBeVisible();
  await page.locator('#onboarding-imap-host').fill('imap.example.com');
  await page.locator('#onboarding-imap-user').fill('ada@example.com');
  await wizardContinue(page);

  // Debug prefill may skip the proxy step.
  if (await page.getByRole('heading', { name: 'How Mailiner connects' }).isVisible()) {
    await page.locator('#onboarding-proxy-url').fill('ws://127.0.0.1:59999/proxy');
    await wizardContinue(page);
  }

  await expect(page.getByRole('heading', { name: 'Protect this device' })).toBeVisible();
  await page.getByLabel('Unlock passphrase', { exact: true }).fill('correct-horse');
  await page.getByLabel('Confirm passphrase').fill('wrong-battery');
  await wizardContinue(page);
  await expect(validationStatus(page)).toContainText('Validation');
  await expect(validationStatus(page)).toContainText(
    'Unlock passphrase and confirmation do not match.',
  );
});
