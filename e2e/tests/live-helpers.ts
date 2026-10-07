import { spawn } from 'node:child_process';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { expect, type Page } from '@playwright/test';
import {
  ACCOUNTS_STORAGE_KEY,
  E2E_SEEDED_KEY,
  acceptNextDialog,
  completeEmailStep,
  completeProxyStepIfShown,
  composeButton,
  gotoPath,
  wizardContinue,
} from './helpers';

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
    ({ accountsKey, seededKey, accountsJson }) => {
      // Runs again on every navigation. The seeded early-return must stay below
      // this, or a reload brings the devtools rebuild toast back.
      const root = document.documentElement;
      if (root && !root.querySelector('style[data-mailiner-e2e-hide-toast]')) {
        const style = document.createElement('style');
        style.setAttribute('data-mailiner-e2e-hide-toast', '1');
        style.textContent =
          '.dx-toast { display: none !important; pointer-events: none !important; }';
        root.appendChild(style);
      }
      if (localStorage.getItem(seededKey) === '1') {
        return;
      }
      localStorage.setItem(accountsKey, accountsJson);
      localStorage.setItem(seededKey, '1');
    },
    {
      accountsKey: ACCOUNTS_STORAGE_KEY,
      seededKey: E2E_SEEDED_KEY,
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

export function folderItem(page: Page, name: string | RegExp) {
  return page.getByRole('treeitem', { name }).first();
}

export async function openFolder(page: Page, name: string | RegExp) {
  const item = folderItem(page, name);
  await item.click();
  await expect(item).toHaveAttribute('aria-selected', 'true', { timeout: 20_000 });
  // The row click and the following search share one re-render. Wait out the
  // folder fetch so Search is not dropped on a detached button.
  const loading = page.locator('.message-list-empty', { hasText: /Loading…|Searching…/ });
  await loading.waitFor({ state: 'visible', timeout: 400 }).catch(() => undefined);
  await expect(loading).toHaveCount(0, { timeout: 20_000 });
}

/** Message listbox. Page-level `option` also matches hidden `<select>` options. */
export function messageList(page: Page) {
  return page.getByRole('listbox', { name: 'Messages' });
}

export function messageRows(page: Page) {
  return messageList(page).getByRole('option');
}

export function selectedMessage(page: Page) {
  return messageList(page).locator('[role="option"][aria-selected="true"]');
}

/**
 * Scroll the virtual list until a row matching `pattern` is mounted.
 * Returns its `data-index` (position in the sorted list, not the viewport).
 */
export async function messageIndex(page: Page, pattern: RegExp): Promise<number> {
  const scroller = page.locator('#message-list-scroll');
  let found = -1;
  await expect
    .poll(
      async () => {
        const row = messageRows(page).filter({ hasText: pattern }).first();
        if ((await row.count()) > 0) {
          const raw = await row.getAttribute('data-index');
          if (raw != null && raw !== '') {
            found = Number(raw);
            return found;
          }
        }
        await scroller.evaluate((el) => {
          const node = el as HTMLElement;
          const step = Math.max(node.clientHeight, 1);
          const next = node.scrollTop + step;
          node.scrollTop = next >= node.scrollHeight - step ? 0 : next;
        });
        return -1;
      },
      { timeout: 20_000 },
    )
    .toBeGreaterThanOrEqual(0);
  return found;
}

/** Attachment rows live in a closed `<details>` until the summary is opened. */
export async function revealAttachments(page: Page) {
  const details = page.locator('details.message-attachments-details');
  await expect(details).toBeVisible({ timeout: 20_000 });
  await details.evaluate((el) => {
    (el as HTMLDetailsElement).open = true;
  });
  await expect(page.locator('.attachment-item').first()).toBeVisible();
}

export async function searchFolder(page: Page, query: string) {
  const box = page.getByRole('searchbox', { name: 'Search this folder' });
  await expect(box).toBeVisible();
  await box.fill(query);
  await page.getByRole('button', { name: 'Search', exact: true }).click();
}

/**
 * Search the open folder and open the first row whose text contains `subject`.
 * The row's accessible name is "From, Subject", so a snippet-only query
 * (an empty Subject, for example) is not in that name.
 */
export async function openSubject(page: Page, subject: string) {
  await searchFolder(page, subject);
  const row = messageRows(page).filter({ hasText: subject }).first();
  await expect(row).toBeVisible({ timeout: 30_000 });
  await row.click();
  await expect(page.locator('#mailiner-message-content')).toBeVisible({ timeout: 30_000 });
  return row;
}

export async function bodyText(page: Page) {
  return page.locator('#mailiner-message-content').evaluate((el) => {
    const root = (el as HTMLElement).shadowRoot;
    return (root?.textContent ?? el.textContent ?? '').replace(/\s+/g, ' ').trim();
  });
}

export async function bodyHtml(page: Page) {
  return page.locator('#mailiner-message-content').evaluate((el) => {
    return (el as HTMLElement).shadowRoot?.innerHTML ?? '';
  });
}

export async function createFolder(page: Page, name: string) {
  acceptNextDialog(page, name);
  await page.getByRole('button', { name: 'New folder', exact: true }).click();
  await expect(folderItem(page, name)).toBeVisible({ timeout: 20_000 });
}

export async function deleteFolder(page: Page, name: string) {
  const item = folderItem(page, name);
  if ((await item.count()) === 0) {
    return;
  }
  await item.click({ button: 'right' });
  const del = page.getByRole('menuitem', { name: 'Delete', exact: true });
  await expect(del).toBeVisible();
  acceptNextDialog(page);
  await del.click();
  await expect(folderItem(page, name)).toHaveCount(0, { timeout: 20_000 });
}

export async function copySelectionTo(page: Page, folder: string) {
  await page.getByRole('button', { name: 'Copy to…' }).click();
  const dialog = page.getByRole('dialog', { name: 'Copy to folder' });
  await expect(dialog).toBeVisible();
  await dialog.getByLabel('Filter folders').fill(folder);
  await dialog.getByRole('option', { name: folder }).click();
  await expect(page.getByText(`Copied to ${folder}`)).toBeVisible({ timeout: 20_000 });
}

export async function moveSelectionTo(page: Page, folder: string) {
  await page.locator('select[aria-label="Move to folder"]').selectOption({ label: folder });
}

/**
 * Copy `sourceSubject` from Inbox into a new folder and open the copy.
 * The caller deletes `folder` afterward. Do not use this when the copy is
 * moved into a shared mailbox: the copy keeps the seed subject.
 */
export async function seedPrivateCopy(page: Page, folder: string, sourceSubject: string) {
  await createFolder(page, folder);
  await openFolder(page, /Inbox/);
  await openSubject(page, sourceSubject);
  await copySelectionTo(page, folder);
  await openFolder(page, folder);
  await openSubject(page, sourceSubject);
}

/**
 * Append a new message into a new folder and open it.
 * `subject` stays unique so a later move into Archive, Junk, or Trash can be
 * expunged without matching a seed.
 */
export async function seedPrivateMessage(page: Page, folder: string, subject: string, body = 'e2e') {
  await createFolder(page, folder);
  await deliverRaw(
    folder,
    simpleMessage('Ada Lovelace <ada-e2e@example.com>', subject, body),
  );
  await openFolder(page, folder);
  await openSubject(page, subject);
}

let folderSeq = 0;

export function uniqueName(prefix: string) {
  folderSeq += 1;
  return `${prefix}-${Date.now().toString(36)}-${folderSeq}`;
}

/** Container publishing IMAP 993 (this compose project, or one already running). */
function mailContainerId(): Promise<string> {
  return new Promise((resolve, reject) => {
    const child = spawn('docker', ['ps', '-q', '--filter', 'publish=993'], {
      stdio: ['ignore', 'pipe', 'pipe'],
    });
    let out = '';
    let err = '';
    child.stdout.on('data', (chunk) => {
      out += String(chunk);
    });
    child.stderr.on('data', (chunk) => {
      err += String(chunk);
    });
    child.on('error', reject);
    child.on('close', (code) => {
      const id = out.trim().split('\n').find((line) => line.trim())?.trim();
      if (code === 0 && id) {
        resolve(id);
      } else {
        reject(new Error(`no mail container publishing 993: ${err || out}`));
      }
    });
  });
}

/** Deliver one RFC 5322 message through the running mail container. */
export function deliverRaw(mailbox: string, raw: string): Promise<void> {
  const crlf = raw.includes('\r\n') ? raw : raw.replace(/\n/g, '\r\n');
  return mailContainerId().then(
    (id) =>
      new Promise((resolve, reject) => {
    const child = spawn(
      'docker',
      [
        'exec',
        '-i',
        id,
        '/usr/libexec/dovecot/dovecot-lda',
        '-d',
        LIVE_ACCOUNT_EMAIL,
        '-m',
        mailbox,
      ],
      { stdio: ['pipe', 'pipe', 'pipe'] },
    );
    let err = '';
    child.stderr.on('data', (chunk) => {
      err += String(chunk);
    });
    child.on('error', reject);
    child.on('close', (code) => {
      if (code === 0) {
        resolve();
      } else {
        reject(new Error(`dovecot-lda exited ${code}: ${err}`));
      }
    });
    child.stdin.end(crlf);
      }),
  );
}

export function simpleMessage(from: string, subject: string, body: string) {
  return [
    `From: ${from}`,
    `To: Dev User <${LIVE_ACCOUNT_EMAIL}>`,
    `Subject: ${subject}`,
    `Message-ID: <${subject.replace(/[^a-z0-9]/gi, '')}@e2e.mailiner.test>`,
    'MIME-Version: 1.0',
    'Content-Type: text/plain; charset=utf-8',
    '',
    body,
    '',
  ].join('\n');
}

function doveadm(args: string[]): Promise<string> {
  return mailContainerId().then(
    (id) =>
      new Promise((resolve, reject) => {
        const child = spawn('docker', ['exec', id, 'doveadm', ...args], {
          stdio: ['ignore', 'pipe', 'pipe'],
        });
        let out = '';
        let err = '';
        child.stdout.on('data', (chunk) => {
          out += String(chunk);
        });
        child.stderr.on('data', (chunk) => {
          err += String(chunk);
        });
        child.on('error', reject);
        child.on('close', (code) => {
          if (code === 0) {
            resolve(out);
          } else {
            reject(new Error(`doveadm ${args[0]} exited ${code}: ${err || out}`));
          }
        });
      }),
  );
}

function ignoredCleanupError(err: unknown): boolean {
  return String(err).includes("doesn't exist");
}

const SHARED_MAILBOXES = new Set([
  'INBOX',
  'Inbox',
  'Sent',
  'Drafts',
  'Trash',
  'Junk',
  'Archive',
]);

/**
 * Delete a private folder created by a test, including one the UI cannot see
 * (unsubscribed, or the page is no longer on mail). Already-gone is success.
 * Shared special-use mailboxes are refused.
 */
export async function cleanupMailbox(name: string): Promise<void> {
  if (SHARED_MAILBOXES.has(name)) {
    console.warn(`refusing to delete shared mailbox ${name}`);
    return;
  }
  try {
    await doveadm(['mailbox', 'delete', '-u', LIVE_ACCOUNT_EMAIL, '-s', name]);
  } catch (err) {
    if (!ignoredCleanupError(err)) {
      console.warn(String(err));
    }
  }
}

/**
 * Expunge messages whose Subject contains `subject` from one mailbox.
 * doveadm's SUBJECT query is a substring match, so `subject` must not be a
 * substring of a seed subject in that mailbox. A missing message is success.
 */
export async function cleanupSubject(mailbox: string, subject: string): Promise<void> {
  try {
    await doveadm([
      'expunge',
      '-u',
      LIVE_ACCOUNT_EMAIL,
      'mailbox',
      mailbox,
      'subject',
      subject,
    ]);
  } catch (err) {
    if (!ignoredCleanupError(err)) {
      console.warn(String(err));
    }
  }
}

/** True when the mailbox has a message whose Subject contains `subject`. */
export async function mailboxHasSubject(mailbox: string, subject: string): Promise<boolean> {
  const out = await doveadm([
    'search',
    '-u',
    LIVE_ACCOUNT_EMAIL,
    'mailbox',
    mailbox,
    'subject',
    subject,
  ]);
  return out.trim().length > 0;
}

export type WizardOptions = {
  password?: string;
  /** When set, open Change connection on Review and replace the proxy URL. */
  proxyUrl?: string;
  /** Upload the test CA on the servers step. Debug builds also trust it built-in. */
  uploadCa?: boolean;
  passphrase?: string;
};

/** Walk first-run onboarding through Connect. Leaves the wizard on screen if Connect fails. */
export async function connectThroughWizard(page: Page, options: WizardOptions = {}) {
  const password = options.password ?? LIVE_ACCOUNT_PASSWORD;
  await gotoPath(page);
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible();
  await page.getByRole('button', { name: 'Get started' }).click();
  await completeEmailStep(page, LIVE_ACCOUNT_NAME, LIVE_ACCOUNT_EMAIL);
  await page.locator('#onboarding-imap-password').fill(password);
  await wizardContinue(page);

  await expect(page.getByRole('heading', { name: 'Mail servers' })).toBeVisible();
  await page.locator('#onboarding-imap-host').fill(LIVE_IMAP_HOST);
  await page.locator('#onboarding-imap-port').fill('993');
  await page.locator('#onboarding-imap-user').fill(LIVE_ACCOUNT_EMAIL);
  if (options.uploadCa !== false) {
    await page.getByText('Extra CA certificates (optional)').click();
    await page.locator('#onboarding-extra-ca-file').setInputFiles('docker/mail/tls/ca.crt');
  }
  await wizardContinue(page);
  await completeProxyStepIfShown(page, LIVE_PROXY_URL);

  const unlock = page.getByRole('heading', { name: 'Protect this device' });
  const review = page.getByRole('heading', { name: 'Review and connect' });
  await expect(unlock.or(review)).toBeVisible();
  if (await unlock.isVisible()) {
    if (options.passphrase) {
      await page.getByLabel('Unlock passphrase', { exact: true }).fill(options.passphrase);
      await page.getByLabel('Confirm passphrase').fill(options.passphrase);
    }
    await wizardContinue(page);
  }

  await expect(page.getByRole('heading', { name: 'Review and connect' })).toBeVisible();
  if (options.proxyUrl) {
    await page.getByRole('button', { name: 'Change connection' }).click();
    await expect(page.getByRole('heading', { name: 'How Mailiner connects' })).toBeVisible();
    await page.locator('#onboarding-proxy-url').fill(options.proxyUrl);
    await wizardContinue(page);
    await expect(page.getByRole('heading', { name: 'Review and connect' })).toBeVisible();
  }
  await wizardContinue(page, 'Connect');
}
