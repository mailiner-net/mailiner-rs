import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, type Page } from '@playwright/test';
import { ACCOUNTS_STORAGE_KEY, composeButton, gotoPath, wizardContinue } from './helpers';

export const LIVE_ACCOUNT_ID = 'e2e-live-1';
export const LIVE_ACCOUNT_NAME = 'Dev';
export const LIVE_ACCOUNT_EMAIL = 'dev@mailiner.test';
export const LIVE_ACCOUNT_PASSWORD = 'dev';
export const LIVE_IMAP_HOST = 'mail';
export const LIVE_PROXY_URL = 'ws://127.0.0.1:9400/proxy';
export const LIVE_WELCOME_SUBJECT = 'Welcome to Mailiner';
export const LIVE_WELCOME_FROM = 'Mailiner Fixtures';

/** Test-only CA that signs the compose `mail` service certificate. */
export function liveCaPem(): string {
  return readFileSync(join(__dirname, '../../docker/mail/tls/ca.crt'), 'utf8').trim();
}

/** Plaintext account that connects through the compose proxy to docker-mail. */
export function liveAccountBlob() {
  return {
    schema_version: 1,
    active_account_id: LIVE_ACCOUNT_ID,
    accounts: [
      {
        id: LIVE_ACCOUNT_ID,
        display_name: LIVE_ACCOUNT_NAME,
        email: LIVE_ACCOUNT_EMAIL,
        imap: {
          host: LIVE_IMAP_HOST,
          port: 993,
          username: LIVE_ACCOUNT_EMAIL,
          password: LIVE_ACCOUNT_PASSWORD,
          tls_mode: 'implicit',
          use_tls: true,
        },
        smtp: {
          host: LIVE_IMAP_HOST,
          port: 465,
          username: LIVE_ACCOUNT_EMAIL,
          password: LIVE_ACCOUNT_PASSWORD,
          tls_mode: 'implicit',
          use_tls: true,
        },
        proxy: {
          base_url: LIVE_PROXY_URL,
          token: '',
          remote_host: null,
          remote_port: null,
        },
        extra_ca_pems: [liveCaPem()],
        created_at: '2024-06-15T12:00:00Z',
        updated_at: '2024-06-15T12:00:00Z',
      },
    ],
  };
}

/** Inject the live account. Does **not** set skipConnect. */
export async function seedLiveAccount(page: Page) {
  const accounts = JSON.stringify(liveAccountBlob());
  await page.addInitScript(
    ({ accountsKey, accountsJson }) => {
      localStorage.setItem(accountsKey, accountsJson);
    },
    {
      accountsKey: ACCOUNTS_STORAGE_KEY,
      accountsJson: accounts,
    },
  );
}

/** Mail chrome after a successful IMAP session against docker-mail. */
export async function gotoLiveMail(page: Page) {
  await page.setViewportSize({ width: 1280, height: 800 });
  await gotoPath(page, '/');
  await expect(page.locator('#app')).toBeVisible();
  await expect(composeButton(page)).toBeVisible();
  await expect(page.getByText('Connected', { exact: true })).toBeVisible({
    timeout: 45_000,
  });
  await expect(page.getByRole('treeitem', { name: /Inbox/ })).toBeVisible({
    timeout: 45_000,
  });
}
