import { test, expect } from '@playwright/test';
import { completeEmailStep, completeProxyStepIfShown, composeButton, gotoPath, wizardContinue } from './helpers';
import {
  LIVE_ACCOUNT_EMAIL,
  LIVE_ACCOUNT_NAME,
  LIVE_ACCOUNT_PASSWORD,
  LIVE_IMAP_HOST,
  LIVE_PROXY_URL,
  LIVE_WELCOME_SUBJECT,
  gotoLiveMail,
  seedLiveAccount,
} from './live-helpers';

test.describe.configure({ timeout: 90_000 });

test('seeded account connects through the proxy and opens a real message', async ({
  page,
}) => {
  await seedLiveAccount(page);
  await gotoLiveMail(page);

  await expect(page.getByRole('treeitem', { name: 'Drafts' })).toBeVisible();
  await expect(page.getByRole('treeitem', { name: 'Sent' })).toBeVisible();
  await expect(page.getByRole('treeitem', { name: 'Trash' })).toBeVisible();

  // Arrival order puts the oldest seed (Welcome) below the virtual-list window.
  await page.getByRole('searchbox', { name: 'Search this folder' }).fill(LIVE_WELCOME_SUBJECT);
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  const welcome = page.getByRole('option', { name: new RegExp(LIVE_WELCOME_SUBJECT) });
  await expect(welcome).toBeVisible({ timeout: 30_000 });
  await welcome.click();
  await expect(page.getByLabel('Message body')).toContainText(
    'This is a plain-text fixture used by the local mail container.',
  );

  await page.getByRole('treeitem', { name: 'Trash' }).click();
  await expect(page.getByRole('option', { name: /Already in Trash/ })).toBeVisible();
});

test('setup wizard Connect authenticates against docker-mail', async ({ page }) => {
  await gotoPath(page);
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible();
  await page.getByRole('button', { name: 'Get started' }).click();

  await completeEmailStep(page, LIVE_ACCOUNT_NAME, LIVE_ACCOUNT_EMAIL);
  await page.locator('#onboarding-imap-password').fill(LIVE_ACCOUNT_PASSWORD);
  await wizardContinue(page);

  await expect(page.getByRole('heading', { name: 'Mail servers' })).toBeVisible();
  await page.locator('#onboarding-imap-host').fill(LIVE_IMAP_HOST);
  await page.locator('#onboarding-imap-port').fill('993');
  await page.locator('#onboarding-imap-user').fill(LIVE_ACCOUNT_EMAIL);
  await page.getByText('Extra CA certificates (optional)').click();
  await page.locator('#onboarding-extra-ca-file').setInputFiles('docker/mail/tls/ca.crt');
  await wizardContinue(page);

  await completeProxyStepIfShown(page, LIVE_PROXY_URL);

  if (await page.getByRole('heading', { name: 'Protect this device' }).isVisible()) {
    await wizardContinue(page);
  }

  await expect(page.getByRole('heading', { name: 'Review and connect' })).toBeVisible();
  await wizardContinue(page, 'Connect');

  await expect(composeButton(page)).toBeVisible({ timeout: 60_000 });
  await expect(page.getByText('Connected', { exact: true })).toBeVisible({
    timeout: 45_000,
  });
  await expect(page.getByRole('treeitem', { name: /Inbox/ })).toBeVisible();
  await page.getByRole('searchbox', { name: 'Search this folder' }).fill(LIVE_WELCOME_SUBJECT);
  await page.getByRole('button', { name: 'Search', exact: true }).click();
  await expect(page.getByRole('option', { name: new RegExp(LIVE_WELCOME_SUBJECT) })).toBeVisible({
    timeout: 30_000,
  });
});
