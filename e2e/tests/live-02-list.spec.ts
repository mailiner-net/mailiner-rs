import { test, expect } from '@playwright/test';
import { pressShortcut } from './helpers';
import {
  cleanupMailbox,
  cleanupSubject,
  createFolder,
  deliverRaw,
  gotoLiveMail,
  messageIndex,
  openFolder,
  messageRows,
  searchFolder,
  seedLiveAccount,
  selectedMessage,
  simpleMessage,
  uniqueName,
} from './live-helpers';

test.describe.configure({ timeout: 120_000 });

test.beforeEach(async ({ page }) => {
  await seedLiveAccount(page);
  await gotoLiveMail(page);
});

test('unread hides mail the seed already marked seen', async ({ page }) => {
  const subject = uniqueName('unseen');
  try {
    await deliverRaw(
      'INBOX',
      simpleMessage('Unread Sender <unread-e2e@example.com>', subject, 'still new'),
    );
    // The folder is already open. Select it again so the append is in the
    // fetch, including when older mail fills the list.
    await openFolder(page, 'Sent');
    await openFolder(page, /Inbox/);
    await expect(messageRows(page).filter({ hasText: subject }).first()).toBeVisible({
      timeout: 40_000,
    });
    await page.getByRole('button', { name: 'Show unread messages' }).click();
    await expect(messageRows(page).filter({ hasText: subject }).first()).toBeVisible({
      timeout: 20_000,
    });
    await expect(messageRows(page).filter({ hasText: 'Welcome to Mailiner' })).toHaveCount(0);
    await expect(messageRows(page).filter({ hasText: 'HTML-only announcement' })).toHaveCount(0);
    await expect(messageRows(page).filter({ hasText: 'Meeting notes' })).toHaveCount(0);
    await expect(page.locator('#message-list-scroll .message-list-item:not(.unread)')).toHaveCount(0);
  } finally {
    await cleanupSubject('INBOX', subject);
  }
});

test('search finds a decoded subject and clears back to the list', async ({ page }) => {
  await searchFolder(page, 'Café');
  const cafe = messageRows(page).filter({ hasText: 'Café' });
  await expect(cafe.first()).toBeVisible({ timeout: 20_000 });
  await expect(cafe).toHaveCount(1);

  await searchFolder(page, '');
  // Arrival order puts the old fixtures below the virtual window once newer
  // mail sits in this inbox, so the restored list is "more than the hit".
  await expect.poll(async () => messageRows(page).count(), { timeout: 20_000 }).toBeGreaterThan(1);
  await expect(messageRows(page).filter({ hasNotText: 'Café' }).first()).toBeVisible();
});

test('a search with no hit shows the empty state', async ({ page }) => {
  await searchFolder(page, 'subject:zzz-no-such-mailiner-fixture');
  await expect(page.getByText('No matching messages')).toBeVisible({ timeout: 20_000 });
});

test('unread-first sort lifts an unseen row above the welcome message', async ({ page }) => {
  const subject = uniqueName('oldunread');
  try {
    // Older than the seed, so date order keeps it below Welcome. Unread-first does not.
    const raw = simpleMessage('Old Unread <old-unread@example.com>', subject, 'from 1998').replace(
      'MIME-Version: 1.0',
      'Date: Thu, 01 Jan 1998 00:00:00 +0000\nMIME-Version: 1.0',
    );
    await deliverRaw('INBOX', raw);
    await page.getByLabel('Sort messages').selectOption('unread');
    const unseen = await messageIndex(page, new RegExp(subject));
    const welcome = await messageIndex(page, /Welcome to Mailiner/);
    expect(unseen).toBeLessThan(welcome);
  } finally {
    await cleanupSubject('INBOX', subject);
  }
});

test('jump-to-folder selects a real mailbox', async ({ page }) => {
  await pressShortcut(page, 'j');
  const jump = page.getByRole('dialog', { name: 'Go to folder' });
  await expect(jump).toBeVisible();
  await jump.getByLabel('Filter folders').fill('Archive');
  await jump.getByRole('option', { name: 'Archive' }).click();
  await expect(page.getByRole('treeitem', { name: 'Archive', selected: true })).toBeVisible();

  await pressShortcut(page, 'j');
  const again = page.getByRole('dialog', { name: 'Go to folder' });
  await again.getByLabel('Filter folders').fill('Inbox');
  await again.getByRole('option', { name: 'Inbox' }).click();
  await expect(page.getByRole('treeitem', { name: /Inbox/, selected: true })).toBeVisible();
  await expect(messageRows(page).first()).toBeVisible({ timeout: 20_000 });
});

test('next and previous move the selection', async ({ page }) => {
  const first = messageRows(page).first();
  await expect(first).toBeVisible({ timeout: 20_000 });
  await first.click();
  const before = await first.getAttribute('aria-label');

  await pressShortcut(page, 'ArrowDown');
  await expect.poll(async () => selectedMessage(page).getAttribute('aria-label')).not.toBe(before);

  await pressShortcut(page, 'ArrowUp');
  await expect(selectedMessage(page)).toHaveAttribute('aria-label', before ?? '');

  // Inbox often has a single unseen row, so `n` has nothing to move to.
  // Two fresh messages, filter off: opening the folder marks the newest one
  // read and leaves it in the list. `n` then lands on the older unread and
  // stays there (the unread filter would drop that row as soon as it is read).
  const folder = uniqueName('next');
  const olderSubject = `${folder}-a`;
  const newerSubject = `${folder}-b`;
  try {
    await createFolder(page, folder);
    await deliverRaw(
      folder,
      simpleMessage('Ada Lovelace <ada-next@example.com>', olderSubject, 'one'),
    );
    await deliverRaw(
      folder,
      simpleMessage('Grace Hopper <grace-next@example.com>', newerSubject, 'two'),
    );
    await openFolder(page, folder);
    await expect(messageRows(page)).toHaveCount(2, { timeout: 20_000 });
    const unreadRows = page.locator('#message-list-scroll .message-list-item.unread');
    await expect(unreadRows).toHaveCount(1, { timeout: 20_000 });
    await expect(selectedMessage(page)).toHaveAttribute(
      'aria-label',
      `Grace Hopper, ${newerSubject}`,
    );
    await pressShortcut(page, 'n');
    await expect(selectedMessage(page)).toHaveAttribute(
      'aria-label',
      `Ada Lovelace, ${olderSubject}`,
      { timeout: 20_000 },
    );
  } finally {
    await cleanupMailbox(folder);
  }
});
