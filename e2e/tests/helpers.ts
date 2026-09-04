import { expect, type Page } from '@playwright/test';

/** Matches `ACCOUNTS_LOCAL_STORAGE_KEY` in `account_store.rs`. */
export const ACCOUNTS_STORAGE_KEY = 'mailiner.accounts.v1';
/** Matches `MAIL_CACHE_LOCAL_STORAGE_KEY` in `mail_cache.rs`. */
export const MAIL_CACHE_STORAGE_KEY = 'mailiner.cache.v1';
/** Matches `E2E_SKIP_CONNECT_KEY` in `local_data.rs`. */
export const E2E_SKIP_CONNECT_KEY = 'mailiner.e2e.skipConnect';

export const E2E_ACCOUNT_ID = 'e2e-account-1';
export const E2E_ACCOUNT_NAME = 'E2E Work';
export const E2E_ACCOUNT_EMAIL = 'e2e@example.com';
export const E2E_MESSAGE_SUBJECT = 'Cached welcome message';
export const E2E_MESSAGE_FROM = 'Ada Lovelace';

/**
 * Dead proxy so bootstrap IMAP never hits a live server.
 * Avoid Chrome's blocked-port list (e.g. :1 / :7) — those abort WASM with
 * `ERR_UNSAFE_PORT` and freeze click/shortcut handlers.
 */
export const E2E_PROXY_URL = 'ws://127.0.0.1:59999/proxy';

/** Plaintext (no vault) account blob — bootstrap goes to Ready without unlock. */
export function plaintextAccountBlob() {
  return {
    schema_version: 1,
    active_account_id: E2E_ACCOUNT_ID,
    accounts: [
      {
        id: E2E_ACCOUNT_ID,
        display_name: E2E_ACCOUNT_NAME,
        email: E2E_ACCOUNT_EMAIL,
        imap: {
          host: 'imap.example.com',
          port: 993,
          username: E2E_ACCOUNT_EMAIL,
          password: 'not-a-real-password',
          tls_mode: 'implicit',
          use_tls: true,
        },
        smtp: null,
        proxy: {
          base_url: E2E_PROXY_URL,
          token: '',
          remote_host: null,
          remote_port: null,
        },
        created_at: '2024-06-15T12:00:00Z',
        updated_at: '2024-06-15T12:00:00Z',
      },
    ],
  };
}

/**
 * Folder tree + one Inbox envelope. Hydrate paints this before IMAP;
 * a failed connect keeps the cache hit visible.
 */
export function mailCacheBlob() {
  return {
    schema_version: 1,
    accounts: {
      [E2E_ACCOUNT_ID]: {
        folders: {
          folders: [
            folder('INBOX', 'INBOX', 'inbox'),
            folder('Archive', 'Archive', 'archive'),
            folder('Sent', 'Sent', 'sent'),
          ],
          counts: {
            INBOX: { total_messages: 1, unread_messages: 1 },
            Archive: { total_messages: 0, unread_messages: 0 },
            Sent: { total_messages: 0, unread_messages: 0 },
          },
        },
        messages: {
          INBOX: {
            mailbox_id: 'INBOX',
            sort: 'arrival',
            total: 1,
            unread: 1,
            envelopes: [cachedEnvelope()],
            accessed_at: '2024-06-15T12:00:00Z',
          },
        },
      },
    },
  };
}

function folder(id: string, name: string, role: string) {
  return {
    id,
    account_id: E2E_ACCOUNT_ID,
    name,
    parent_id: null,
    role,
    selectable: true,
    subscribed: true,
  };
}

function cachedEnvelope() {
  return {
    id: { folder_id: 'INBOX', uid: '1' },
    account_id: E2E_ACCOUNT_ID,
    folder_id: 'INBOX',
    subject: E2E_MESSAGE_SUBJECT,
    from: {
      List: [{ name: E2E_MESSAGE_FROM, email: 'ada@example.com' }],
    },
    to: {
      List: [{ name: null, email: E2E_ACCOUNT_EMAIL }],
    },
    cc: null,
    bcc: null,
    date: '2024-06-15T12:00:00Z',
    is_read: false,
    is_starred: false,
    is_flagged: false,
    is_draft: false,
    is_deleted: false,
    has_attachments: false,
    snippet: 'Hello from the seeded cache.',
  };
}

export type SeedOptions = {
  /** When true, also write `mailiner.cache.v1` (folders + one message). */
  cache?: boolean;
};

/** Inject the account (and optional mail cache) before the first document script. */
export async function seedAccount(page: Page, options: SeedOptions = {}) {
  const accounts = JSON.stringify(plaintextAccountBlob());
  const cache = options.cache ? JSON.stringify(mailCacheBlob()) : null;
  await page.addInitScript(
    ({ accountsKey, cacheKey, skipKey, accountsJson, cacheJson }) => {
      localStorage.setItem(accountsKey, accountsJson);
      localStorage.setItem(skipKey, '1');
      if (cacheJson) {
        localStorage.setItem(cacheKey, cacheJson);
      }
    },
    {
      accountsKey: ACCOUNTS_STORAGE_KEY,
      cacheKey: MAIL_CACHE_STORAGE_KEY,
      skipKey: E2E_SKIP_CONNECT_KEY,
      accountsJson: accounts,
      cacheJson: cache,
    },
  );
}

/** Main mail chrome after a seeded Ready bootstrap. */
export async function gotoMail(page: Page) {
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto('/');
  await expect(page.locator('#app')).toBeVisible();
  await expect(page.getByRole('link', { name: 'Skip to message' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Compose' })).toBeVisible();
}

/**
 * Dispatch a window-level keydown that matches `shortcuts.rs`.
 * Playwright's page.keyboard can land in the folder-filter input; the app
 * listener also ignores events whose target is an input.
 */
export async function pressShortcut(page: Page, key: string, shift = false) {
  await page.evaluate(
    ({ key, shift }) => {
      const active = document.activeElement;
      if (
        active instanceof HTMLElement &&
        (active.isContentEditable || /^(INPUT|TEXTAREA|SELECT)$/i.test(active.tagName))
      ) {
        active.blur();
      }
      window.dispatchEvent(
        new KeyboardEvent('keydown', {
          key,
          shiftKey: shift,
          bubbles: true,
          cancelable: true,
        }),
      );
    },
    { key, shift },
  );
}
