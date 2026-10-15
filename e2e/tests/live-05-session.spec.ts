import { test, expect, type Page } from '@playwright/test';
import {
  acceptNextDialog,
  composeButton,
  gotoPath,
  wizardContinue,
} from './helpers';
import {
  LIVE_ACCOUNT_EMAIL,
  LIVE_ACCOUNT_ID,
  LIVE_ACCOUNT_PASSWORD,
  LIVE_IMAP_HOST,
  LIVE_PROXY_URL,
  LIVE_WELCOME_SUBJECT,
  bodyText,
  cleanupMailbox,
  cleanupSubject,
  connectThroughWizard,
  createFolder,
  deleteFolder,
  deliverRaw,
  folderItem,
  gotoLiveMail,
  messageList,
  openFolder,
  openSubject,
  searchFolder,
  seedLiveAccount,
  simpleMessage,
  uniqueName,
} from './live-helpers';

test.describe.configure({ timeout: 120_000 });

test('a wrong IMAP password does not open mail', async ({ page }) => {
  await connectThroughWizard(page, { password: 'wrong' });
  await expect(page.getByText('Sign-in failed')).toBeVisible({ timeout: 40_000 });
  await expect(composeButton(page)).toHaveCount(0);
});

test('a release build rejects the test server without the extra CA', async ({ page }) => {
  test.skip(
    !process.env.MAILINER_E2E_SERVE_DIR,
    'debug dx serve trusts docker/mail/tls/ca.crt on its own',
  );
  await connectThroughWizard(page, { uploadCa: false });
  await expect(page.getByText('TLS / certificate')).toBeVisible({ timeout: 40_000 });
  await expect(composeButton(page)).toHaveCount(0);
});

test('a dead proxy does not open mail', async ({ page }) => {
  await connectThroughWizard(page, { proxyUrl: 'ws://127.0.0.1:59999/proxy' });
  await expect(page.getByText('Network / proxy')).toBeVisible({ timeout: 40_000 });
  await expect(composeButton(page)).toHaveCount(0);
});

test('the wizard passphrase locks the store', async ({ page }) => {
  const passphrase = 'correct-horse';
  await connectThroughWizard(page, { passphrase });
  await expect(composeButton(page)).toBeVisible({ timeout: 60_000 });

  const stored = await page.evaluate(() => localStorage.getItem('mailiner.accounts.v1') ?? '');
  // The address stays in the clear so the unlock screen can name the account.
  // The passphrase vault holds the password.
  expect(stored).not.toContain(`"password":"${LIVE_ACCOUNT_PASSWORD}"`);
  expect(stored).toContain('"vault"');

  await page.reload();
  await expect(page.getByRole('heading', { name: 'Unlock accounts' })).toBeVisible();
  await page.getByLabel('Unlock passphrase').fill('wrong-password');
  await page.getByRole('button', { name: 'Unlock', exact: true }).click();
  await expect(page.getByText('That passphrase is incorrect.')).toBeVisible();

  await page.getByLabel('Unlock passphrase').fill(passphrase);
  await page.getByRole('button', { name: 'Unlock', exact: true }).click();
  await expect(composeButton(page)).toBeVisible({ timeout: 45_000 });
});

test('test connection reports both outcomes', async ({ page }) => {
  await seedLiveAccount(page);
  await gotoLiveMail(page);
  await gotoPath(page, `/settings/accounts/${LIVE_ACCOUNT_ID}`);
  await expect(page.getByRole('heading', { name: 'Edit account' })).toBeVisible();
  await page.getByRole('button', { name: 'Test connection' }).click();
  await expect(page.getByText('Connection successful. You can save the account.')).toBeVisible({
    timeout: 40_000,
  });

  await page.locator('#account-edit-imap-password').fill('wrong');
  await page.getByRole('button', { name: 'Test connection' }).click();
  await expect(page.getByText('Sign-in failed')).toBeVisible({ timeout: 40_000 });
});

test('a second account can be added and selected', async ({ page }) => {
  await seedLiveAccount(page);
  await gotoLiveMail(page);
  await gotoPath(page, '/settings/accounts/new');
  await expect(page.getByRole('heading', { name: 'Your email' })).toBeVisible();
  await page.locator('#account-new-display-name').fill('Dev Two');
  await page.locator('#account-new-email').fill(LIVE_ACCOUNT_EMAIL);
  await page.getByRole('heading', { name: 'Your email' }).click();
  await expect(page.getByText(/Looking up IMAP/i)).toHaveCount(0, { timeout: 30_000 });
  await wizardContinue(page);
  await expect(page.getByRole('heading', { name: 'Sign in' })).toBeVisible();
  await page.locator('#account-new-imap-password').fill(LIVE_ACCOUNT_PASSWORD);
  await wizardContinue(page);
  await expect(page.getByRole('heading', { name: 'Mail servers' })).toBeVisible();
  await page.locator('#account-new-imap-host').fill(LIVE_IMAP_HOST);
  await page.locator('#account-new-imap-port').fill('993');
  await page.locator('#account-new-imap-user').fill(LIVE_ACCOUNT_EMAIL);
  await page.getByText('Extra CA certificates (optional)').click();
  await page.locator('#account-new-extra-ca-file').setInputFiles('docker/mail/tls/ca.crt');
  await wizardContinue(page);
  const review = page.getByRole('heading', { name: 'Review and connect' });
  const proxy = page.getByRole('heading', { name: 'How Mailiner connects' });
  await expect(review.or(proxy)).toBeVisible();
  if (await proxy.isVisible()) {
    await page.locator('#account-new-proxy-url').fill(LIVE_PROXY_URL);
    await wizardContinue(page);
  }
  await expect(review).toBeVisible();
  await wizardContinue(page, 'Connect');

  await expect(page.getByRole('heading', { name: 'Accounts', exact: true })).toBeVisible({
    timeout: 60_000,
  });
  const added = page.locator('li.accounts-list-item', { hasText: 'Dev Two' });
  await expect(added).toBeVisible();
  // Adding an account selects it. Switch is only on the account that is not active.
  await expect(added.getByText('Active', { exact: true })).toBeVisible();
  await gotoPath(page, '/');
  await expect(page.getByText('Connected', { exact: true })).toBeVisible({ timeout: 45_000 });
  await expect(page.getByRole('link', { name: 'Dev Two' })).toBeVisible();
});

test('delete account removes it from this browser', async ({ page }) => {
  await seedLiveAccount(page);
  await gotoPath(page, '/settings/accounts');
  await expect(page.getByRole('heading', { name: 'Accounts', exact: true })).toBeVisible();
  await page.getByRole('button', { name: 'Delete', exact: true }).click();
  await page.getByRole('button', { name: 'Delete account', exact: true }).click();
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible({
    timeout: 20_000,
  });
});

test('a new folder can be created, renamed, and removed', async ({ page }) => {
  await seedLiveAccount(page);
  await gotoLiveMail(page);
  const name = uniqueName('folder');
  const renamed = `${name}-renamed`;
  try {
    await createFolder(page, name);

    await folderItem(page, name).click({ button: 'right' });
    acceptNextDialog(page, renamed);
    await page.getByRole('menuitem', { name: 'Rename', exact: true }).click();
    await expect(folderItem(page, renamed)).toBeVisible({ timeout: 20_000 });

    await folderItem(page, renamed).click({ button: 'right' });
    await page.getByRole('menuitem', { name: 'Unsubscribe', exact: true }).click();
    await expect(folderItem(page, renamed)).toHaveCount(0);

    await page.getByRole('button', { name: 'Folder subscriptions' }).click();
    const dialog = page.getByRole('dialog', { name: 'Folder subscriptions' });
    await dialog.getByLabel('Filter folders').fill(renamed);
    await dialog.getByLabel(`Subscribe to ${renamed}`).check();
    await dialog.getByRole('button', { name: 'Close', exact: true }).click();
    await expect(folderItem(page, renamed)).toBeVisible({ timeout: 20_000 });

    await deleteFolder(page, renamed);
  } finally {
    // Unsubscribe hides the folder from the tree, so a failure there would
    // leave it on the server. Delete both names even when the UI cannot see them.
    await cleanupMailbox(renamed);
    await cleanupMailbox(name);
  }
});

test('import .eml appends to the current folder', async ({ page }) => {
  await seedLiveAccount(page);
  await gotoLiveMail(page);
  const folder = uniqueName('import');
  try {
    await createFolder(page, folder);
    await openFolder(page, folder);
    await page
      .getByLabel('Import .eml or mbox into this folder')
      .setInputFiles('docker/mail/seed/12-no-subject.eml');
    await expect(page.getByText('(no subject)').first()).toBeVisible({ timeout: 30_000 });
    await page.getByRole('option', { name: /no subject|Quiet Sender/ }).first().click();
    await expect.poll(async () => bodyText(page)).toContain('deliberately has no Subject');
  } finally {
    await cleanupMailbox(folder);
  }
});

test('export writes the message back out', async ({ page }) => {
  await seedLiveAccount(page);
  await gotoLiveMail(page);
  await openSubject(page, LIVE_WELCOME_SUBJECT);
  const [download] = await Promise.all([
    page.waitForEvent('download'),
    page.getByRole('button', { name: 'Save as .eml' }).click(),
  ]);
  const text = await download.createReadStream().then(
    (stream) =>
      new Promise<string>((resolve, reject) => {
        const chunks: Buffer[] = [];
        stream.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
        stream.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
        stream.on('error', reject);
      }),
  );
  expect(text).toContain('Subject: Welcome to Mailiner');
  expect(text).toContain('This is a plain-text fixture used by the local mail container.');
});

test('view source and headers show the raw fixture', async ({ page }) => {
  await seedLiveAccount(page);
  await gotoLiveMail(page);
  await openSubject(page, LIVE_WELCOME_SUBJECT);
  await page.getByRole('button', { name: 'View source' }).click();
  const source = page.getByRole('dialog', { name: 'Message source' });
  await expect(source).toBeVisible();
  await expect(source).toContainText('welcome-plain@mailiner.test');
  await source.getByRole('button', { name: 'Close', exact: true }).last().click();

  await page.getByRole('button', { name: 'Show headers' }).click();
  const headers = page.getByRole('dialog', { name: 'Message headers' });
  await expect(headers).toBeVisible();
  await expect(headers).toContainText('Mailiner Fixtures');
  await expect(headers).toContainText('From');
});

test('print opens a document with the subject and body', async ({ page }) => {
  await seedLiveAccount(page);
  await gotoLiveMail(page);
  await openSubject(page, LIVE_WELCOME_SUBJECT);
  const [popup] = await Promise.all([
    page.waitForEvent('popup'),
    page.getByRole('button', { name: 'Print', exact: true }).click(),
  ]);
  await expect(popup.getByText('Welcome to Mailiner').first()).toBeVisible();
  await expect(popup.getByText('plain-text fixture').first()).toBeVisible();
});

test('a saved filter moves a matching message when the folder is opened', async ({ page }) => {
  await seedLiveAccount(page);
  await gotoLiveMail(page);
  const folder = uniqueName('filt');
  const subject = uniqueName('filter');
  const other = uniqueName('keep');
  try {
    await createFolder(page, folder);
    await deliverRaw('INBOX', simpleMessage('Ada Lovelace <ada@example.com>', subject, 'move me'));
    await deliverRaw('INBOX', simpleMessage('Ada Lovelace <ada@example.com>', other, 'leave me'));

    await page.getByRole('link', { name: 'Settings' }).click();
    await expect(page.getByRole('heading', { name: 'Settings', exact: true })).toBeVisible();
    await page.getByRole('button', { name: 'Add filter' }).click();
    await page.locator('#settings-filter-name').fill(subject);
    await page.getByLabel('Subject contains').fill(subject);
    await page.getByLabel('Move to folder').selectOption({ label: folder });
    await page.getByLabel('Mark as read').check();
    await page.getByRole('button', { name: 'Add filter', exact: true }).last().click();
    await expect(page.getByText(subject).first()).toBeVisible();

    await page.getByRole('link', { name: 'Back to mail' }).click();
    await expect(composeButton(page)).toBeVisible();
    await openFolder(page, /Inbox/);
    await expect(page.getByText(/Filed \d+ message/)).toBeVisible({ timeout: 30_000 });
    await openFolder(page, folder);
    await openSubject(page, subject);
    await expect(page.getByRole('button', { name: 'Mark unread' })).toBeVisible();

    await openFolder(page, /Inbox/);
    await openSubject(page, other);
    await expect.poll(async () => bodyText(page)).toContain('leave me');
  } finally {
    await cleanupSubject('INBOX', subject);
    await cleanupSubject('INBOX', other);
    await cleanupMailbox(folder);
  }
});

/** Search hit count from the "N results" label. Zero when that label is absent. */
async function searchResultCount(page: Page): Promise<number> {
  const label = page.locator('.message-list-filter-count');
  if ((await label.count()) === 0) {
    return 0;
  }
  const match = /^(\d+) results$/.exec((await label.innerText()).trim());
  return match ? Number(match[1]) : 0;
}

/**
 * Search the open folder and wait until the result length stops growing.
 * Dateless mail is stamped at fetch time, so opening Inbox replies once per
 * new sender on the first page, and those copies arrive after Sent is visible.
 */
async function searchAndSettle(page: Page, query: string): Promise<number> {
  await page.evaluate(() => {
    const w = window as unknown as {
      __mlnrSearchSawSearching?: boolean;
      __mlnrSearchObs?: MutationObserver;
    };
    w.__mlnrSearchObs?.disconnect();
    w.__mlnrSearchSawSearching = false;
    const list = document.querySelector('.message-list-body');
    const obs = new MutationObserver(() => {
      const empty = list?.querySelector('.message-list-empty');
      if ((empty?.textContent ?? '').includes('Searching')) {
        w.__mlnrSearchSawSearching = true;
      }
    });
    if (list) {
      obs.observe(list, { childList: true, subtree: true, characterData: true });
    }
    w.__mlnrSearchObs = obs;
  });
  await searchFolder(page, query);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          (window as unknown as { __mlnrSearchSawSearching?: boolean }).__mlnrSearchSawSearching ===
          true,
      ),
    )
    .toBe(true);
  await expect(page.locator('.message-list-empty', { hasText: 'Searching…' })).toHaveCount(0, {
    timeout: 20_000,
  });

  let previous = -1;
  let settled = 0;
  await expect
    .poll(
      async () => {
        const count = await searchResultCount(page);
        const stable = count > 0 && count === previous;
        previous = count;
        if (stable) {
          settled = count;
        }
        return stable;
      },
      { timeout: 40_000, intervals: [2_000] },
    )
    .toBe(true);
  return settled;
}

function toMeta(page: Page) {
  return page.locator('.message-view-meta-item').filter({
    has: page.locator('.message-view-meta-k', { hasText: /^To$/ }),
  });
}

/** Scroll a virtual row into view, open it, and return the To header text. */
async function openRowRecipient(page: Page, index: number, subject: string): Promise<string> {
  const scroller = page.locator('#message-list-scroll');
  await scroller.evaluate((el, rowIndex) => {
    const row = el.querySelector(
      '.virtual-scroll-item, .virtual-scroll-placeholder',
    ) as HTMLElement | null;
    const itemHeight = row ? Number.parseFloat(row.style.height) : 52;
    (el as HTMLElement).scrollTop = (rowIndex as number) * itemHeight;
  }, index);
  // `data-index` is on the option itself. A nested locator would miss it.
  const row = messageList(page).locator(`[role="option"][data-index="${index}"]`);
  await expect(row).toBeVisible({ timeout: 20_000 });
  const alreadyOpen = (await row.getAttribute('aria-selected')) === 'true';
  if (!alreadyOpen) {
    await page.evaluate(() => {
      const w = window as unknown as {
        __mlnrViewTick?: number;
        __mlnrViewObs?: MutationObserver;
      };
      w.__mlnrViewObs?.disconnect();
      w.__mlnrViewTick = 0;
      const obs = new MutationObserver(() => {
        w.__mlnrViewTick = (w.__mlnrViewTick ?? 0) + 1;
      });
      const view = document.querySelector('#messageview');
      if (view) {
        obs.observe(view, { childList: true, subtree: true, characterData: true });
      }
      const host = document.querySelector('#mailiner-message-content');
      if (host?.shadowRoot) {
        obs.observe(host.shadowRoot, { childList: true, subtree: true, characterData: true });
      }
      w.__mlnrViewObs = obs;
    });
    await row.click();
    await expect(row).toHaveAttribute('aria-selected', 'true');
    // The header updates with the selection. A second reply to the same
    // address has the same To text, so wait for the viewer DOM to change.
    await expect
      .poll(() =>
        page.evaluate(() => (window as unknown as { __mlnrViewTick?: number }).__mlnrViewTick ?? 0),
      )
      .toBeGreaterThan(0);
  }
  await expect(page.locator('.message-view-loading, .message-view-error')).toHaveCount(0);
  await expect(page.locator('.message-view-subject')).toContainText(subject);
  const to = toMeta(page);
  await expect(to).toBeVisible();
  const title = (await to.getAttribute('title')) ?? '';
  return `${await to.innerText()}\n${title}`;
}

/**
 * Open every Sent row for `subject` and count those addressed to `email`.
 * The list label is the From name plus the subject, never the recipient.
 */
async function countRepliesTo(
  page: Page,
  subject: string,
  email: string,
): Promise<{ hits: number; total: number }> {
  let total = await searchAndSettle(page, subject);
  const countOnce = async (rows: number) => {
    let hits = 0;
    for (let index = 0; index < rows; index += 1) {
      const blob = (await openRowRecipient(page, index, subject)).toLowerCase();
      if (blob.includes(email.toLowerCase())) {
        hits += 1;
      }
    }
    return hits;
  };
  let hits = await countOnce(total);
  // A reply can land while the rows are being opened. The same query does not
  // start a second search, so wait for the live list and walk it again.
  if ((await searchResultCount(page)) !== total) {
    let previous = -1;
    await expect
      .poll(
        async () => {
          const count = await searchResultCount(page);
          const stable = count > 0 && count === previous;
          previous = count;
          if (stable) {
            total = count;
          }
          return stable;
        },
        { timeout: 20_000, intervals: [2_000] },
      )
      .toBe(true);
    hits = await countOnce(total);
  }
  return { hits, total };
}

test('vacation sends one reply and does not send a second', async ({ page }) => {
  test.setTimeout(180_000);
  await seedLiveAccount(page);
  await gotoLiveMail(page);
  const subject = uniqueName('vacation');
  const firstInbound = uniqueName('from-vac');
  const secondInbound = uniqueName('from-vac-2');
  const recipient = `vacation-${subject}@example.com`;
  try {
    await gotoPath(page, '/settings');
    const vacation = page.locator('.settings-section', { hasText: 'Vacation' });
    await vacation.locator('#settings-vacation-subject').fill(subject);
    await vacation.locator('#settings-vacation-body').fill('Away for e2e.');
    await vacation.getByRole('checkbox', { name: 'Enabled' }).check();
    await expect(page.getByText('Vacation settings saved.')).toBeVisible();

    await gotoPath(page, '/');
    await expect(page.getByText('Connected', { exact: true })).toBeVisible({ timeout: 45_000 });
    const sender = `Vacation Sender <${recipient}>`;
    await deliverRaw('INBOX', simpleMessage(sender, firstInbound, 'ping'));
    await openFolder(page, /Inbox/);
    await openFolder(page, 'Sent');
    const first = await countRepliesTo(page, subject, recipient);
    expect(first.hits, `opened ${first.total} Sent messages`).toBe(1);

    await deliverRaw('INBOX', simpleMessage(sender, secondInbound, 'ping again'));
    await openFolder(page, /Inbox/);
    await openFolder(page, 'Sent');
    const second = await countRepliesTo(page, subject, recipient);
    expect(second.hits, `opened ${second.total} Sent messages`).toBe(1);
  } finally {
    await cleanupSubject('INBOX', firstInbound);
    await cleanupSubject('INBOX', secondInbound);
    // Postfix rewrites every recipient to this account, so each auto-reply
    // is delivered to Inbox as well as appended to Sent.
    await cleanupSubject('INBOX', subject);
    await cleanupSubject('Sent', subject);
  }
});

test('mail delivered while the folder is open shows up on its own', async ({ page }) => {
  await seedLiveAccount(page);
  await gotoLiveMail(page);
  const folder = uniqueName('idlebox');
  const subject = uniqueName('idle');
  try {
    await createFolder(page, folder);
    await openFolder(page, folder);
    await deliverRaw(folder, simpleMessage('Ada Lovelace <ada@example.com>', subject, 'arrived live'));
    await expect(page.getByRole('option', { name: new RegExp(subject) }).first()).toBeVisible({
      timeout: 40_000,
    });
  } finally {
    await cleanupMailbox(folder);
  }
});

test('going offline and Retry recover the session', async ({ page }) => {
  await seedLiveAccount(page);
  await gotoLiveMail(page);
  await expect(page.locator('.message-list-empty', { hasText: /Loading…|Searching…/ })).toHaveCount(0, {
    timeout: 20_000,
  });
  await page.context().setOffline(true);
  await expect(page.getByText('Connected', { exact: true })).toHaveCount(0, { timeout: 30_000 });
  await page.context().setOffline(false);
  const retry = page.getByRole('button', { name: 'Retry', exact: true });
  if (await retry.isVisible()) {
    await retry.click();
  }
  await expect(page.getByText('Connected', { exact: true })).toBeVisible({ timeout: 45_000 });
  await openSubject(page, LIVE_WELCOME_SUBJECT);
  await expect.poll(async () => bodyText(page)).toContain('plain-text fixture');
});
