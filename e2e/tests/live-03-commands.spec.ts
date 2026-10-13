import { readFileSync } from 'node:fs';
import { test, expect } from '@playwright/test';
import { acceptNextDialog, pressShortcut } from './helpers';
import {
  LIVE_WELCOME_SUBJECT,
  cleanupMailbox,
  cleanupSubject,
  copySelectionTo,
  createFolder,
  deliverRaw,
  gotoLiveMail,
  mailboxHasSubject,
  messageRows,
  moveSelectionTo,
  openFolder,
  openSubject,
  searchFolder,
  seedLiveAccount,
  seedPrivateCopy,
  seedPrivateMessage,
  simpleMessage,
  uniqueName,
} from './live-helpers';

test.describe.configure({ timeout: 120_000 });

test.beforeEach(async ({ page }) => {
  await seedLiveAccount(page);
  await gotoLiveMail(page);
});

test('copy leaves the original in place', async ({ page }) => {
  const folder = uniqueName('copy');
  try {
    await createFolder(page, folder);
    await openFolder(page, /Inbox/);
    await openSubject(page, LIVE_WELCOME_SUBJECT);
    await copySelectionTo(page, folder);

    await openFolder(page, /Inbox/);
    await searchFolder(page, LIVE_WELCOME_SUBJECT);
    await expect(messageRows(page).filter({ hasText: 'Welcome to Mailiner' }).first()).toBeVisible({
      timeout: 20_000,
    });

    await openFolder(page, folder);
    await openSubject(page, LIVE_WELCOME_SUBJECT);
  } finally {
    await cleanupMailbox(folder);
  }
});

test('move removes the message from the source folder', async ({ page }) => {
  const source = uniqueName('mvsrc');
  const dest = uniqueName('mvdst');
  try {
    await seedPrivateCopy(page, source, LIVE_WELCOME_SUBJECT);
    await createFolder(page, dest);
    await moveSelectionTo(page, dest);
    await expect(page.getByText(`Moved to ${dest}`)).toBeVisible({ timeout: 20_000 });

    // The copy was opened by search. Staying on this folder keeps that query,
    // so the empty hit is "No matching messages" until the query is cleared.
    await openFolder(page, source);
    await searchFolder(page, '');
    await expect(page.getByText('No messages', { exact: true })).toBeVisible({ timeout: 20_000 });

    await openFolder(page, dest);
    await openSubject(page, LIVE_WELCOME_SUBJECT);
  } finally {
    await cleanupMailbox(dest);
    await cleanupMailbox(source);
  }
});

test('undo puts a moved message back', async ({ page }) => {
  const source = uniqueName('undosrc');
  const dest = uniqueName('undodst');
  try {
    await seedPrivateCopy(page, source, LIVE_WELCOME_SUBJECT);
    await createFolder(page, dest);
    await moveSelectionTo(page, dest);
    await page.getByRole('button', { name: 'Undo' }).click();
    await expect(page.getByText('Undone', { exact: true })).toBeVisible({ timeout: 20_000 });

    await openFolder(page, source);
    await openSubject(page, LIVE_WELCOME_SUBJECT);
    await openFolder(page, dest);
    await expect(page.getByText('No messages', { exact: true })).toBeVisible({ timeout: 20_000 });
  } finally {
    await cleanupMailbox(dest);
    await cleanupMailbox(source);
  }
});

test('archive files the copy in Archive', async ({ page }) => {
  const folder = uniqueName('arch');
  const subject = uniqueName('archmsg');
  try {
    await seedPrivateMessage(page, folder, subject, 'archive me');
    await page.getByRole('button', { name: 'Archive', exact: true }).click();
    await expect(page.getByText('Moved to Archive', { exact: true })).toBeVisible({
      timeout: 20_000,
    });
    await openFolder(page, 'Archive');
    await openSubject(page, subject);
  } finally {
    await cleanupSubject('Archive', subject);
    await cleanupMailbox(folder);
  }
});

test('junk files the copy in Junk', async ({ page }) => {
  const folder = uniqueName('junk');
  const subject = uniqueName('junkmsg');
  try {
    await seedPrivateMessage(page, folder, subject, 'junk me');
    await page.getByRole('button', { name: 'Junk', exact: true }).click();
    await expect(page.getByText('Moved to Junk', { exact: true })).toBeVisible({
      timeout: 20_000,
    });
    await openFolder(page, 'Junk');
    await openSubject(page, subject);
  } finally {
    await cleanupSubject('Junk', subject);
    await cleanupMailbox(folder);
  }
});

test('trash files the copy in Trash and leaves the inbox seed', async ({ page }) => {
  const folder = uniqueName('trash');
  const subject = uniqueName('trashmsg');
  try {
    await seedPrivateMessage(page, folder, subject, 'trash me');
    await page.getByRole('button', { name: 'Trash', exact: true }).click();
    await expect(page.getByText('Moved to Trash')).toBeVisible({ timeout: 20_000 });
    await openFolder(page, /Inbox/);
    await searchFolder(page, LIVE_WELCOME_SUBJECT);
    await expect(messageRows(page).filter({ hasText: 'Welcome to Mailiner' }).first()).toBeVisible({
      timeout: 20_000,
    });
    await openFolder(page, 'Trash');
    await openSubject(page, subject);
  } finally {
    await cleanupSubject('Trash', subject);
    await cleanupMailbox(folder);
  }
});

test('permanent delete removes the copy from Trash', async ({ page }) => {
  const folder = uniqueName('perm');
  const subject = uniqueName('permmsg');
  try {
    await seedPrivateMessage(page, folder, subject, 'delete me');
    await page.getByRole('button', { name: 'Trash', exact: true }).click();
    await expect(page.getByText('Moved to Trash')).toBeVisible({ timeout: 20_000 });
    await openFolder(page, 'Trash');
    await openSubject(page, subject);
    acceptNextDialog(page);
    await page.getByRole('button', { name: 'Delete', exact: true }).click();
    await expect(page.getByText('Deleted')).toBeVisible({ timeout: 20_000 });
    await searchFolder(page, subject);
    await expect(page.getByRole('option', { name: new RegExp(subject) })).toHaveCount(0);
  } finally {
    await cleanupSubject('Trash', subject);
    await cleanupMailbox(folder);
  }
});

test('empty Trash deletes everything in Trash, including the seed', async ({ page }) => {
  const subject = uniqueName('empty');
  try {
    await deliverRaw(
      'INBOX',
      [
        'From: Ada Lovelace <ada@example.com>',
        `To: Dev User <dev@mailiner.test>`,
        `Subject: ${subject}`,
        'Content-Type: text/plain; charset=utf-8',
        '',
        'empty-trash fixture',
        '',
      ].join('\n'),
    );
    await openFolder(page, /Inbox/);
    await openSubject(page, subject);
    await page.getByRole('button', { name: 'Trash', exact: true }).click();
    await openFolder(page, 'Trash');
    await expect(page.getByRole('option', { name: /Already in Trash/ }).first()).toBeVisible({
      timeout: 20_000,
    });
    acceptNextDialog(page);
    await page.getByRole('button', { name: 'Empty Trash' }).click();
    await expect(page.getByText('No messages')).toBeVisible({ timeout: 30_000 });
    await expect(page.getByRole('option', { name: /Already in Trash/ })).toHaveCount(0);
  } finally {
    // Empty Trash clears the shared mailbox, including the seed. Put the seed
    // back only when it is gone, so a failure before Empty does not duplicate it.
    await cleanupSubject('INBOX', subject);
    await cleanupSubject('Trash', subject);
    const seeded = await mailboxHasSubject('Trash', 'Already in Trash').catch(() => false);
    if (!seeded) {
      await deliverRaw('Trash', readFileSync('docker/mail/seed/trashed.eml', 'utf8')).catch(
        () => undefined,
      );
    }
  }
});

test('star, flag, and pin survive a folder change and a reload', async ({ page }) => {
  const folder = uniqueName('marks');
  try {
    await seedPrivateCopy(page, folder, LIVE_WELCOME_SUBJECT);
    const row = page.getByRole('option', { name: /Welcome to Mailiner/ }).first();
    await row.getByRole('button', { name: 'Star', exact: true }).click();
    await row.getByRole('button', { name: 'Flag', exact: true }).click();
    await row.getByRole('button', { name: 'Pin', exact: true }).click();
    await expect(row.getByRole('button', { name: 'Unstar' })).toBeVisible();
    await expect(row.getByRole('button', { name: 'Unflag' })).toBeVisible();
    await expect(row.getByRole('button', { name: 'Unpin' })).toBeVisible();

    await openFolder(page, /Inbox/);
    await openFolder(page, folder);
    await openSubject(page, LIVE_WELCOME_SUBJECT);
    const again = page.getByRole('option', { name: /Welcome to Mailiner/ }).first();
    await expect(again.getByRole('button', { name: 'Unstar' })).toBeVisible();
    await expect(again.getByRole('button', { name: 'Unflag' })).toBeVisible();
    await expect(again.getByRole('button', { name: 'Unpin' })).toBeVisible();

    await page.reload();
    await expect(page.getByText('Connected', { exact: true })).toBeVisible({ timeout: 45_000 });
    await openFolder(page, folder);
    await openSubject(page, LIVE_WELCOME_SUBJECT);
    const after = page.getByRole('option', { name: /Welcome to Mailiner/ }).first();
    await expect(after.getByRole('button', { name: 'Unstar' })).toBeVisible();
    await expect(after.getByRole('button', { name: 'Unflag' })).toBeVisible();
    await expect(after.getByRole('button', { name: 'Unpin' })).toBeVisible();
  } finally {
    await cleanupMailbox(folder);
  }
});

test('mark unread sets the row back to unread', async ({ page }) => {
  const folder = uniqueName('unread');
  try {
    await seedPrivateCopy(page, folder, 'HTML-only announcement');
    await page.getByRole('button', { name: 'Mark unread' }).click();
    await expect(page.locator('.message-list-item.unread').first()).toBeVisible();
    await expect(page.getByRole('treeitem', { name: new RegExp(`${folder}, \\d+ unread`) })).toBeVisible();
  } finally {
    await cleanupMailbox(folder);
  }
});

test('snooze hides the copy', async ({ page }) => {
  const folder = uniqueName('snooze');
  try {
    await seedPrivateCopy(page, folder, LIVE_WELCOME_SUBJECT);
    await page.getByRole('button', { name: 'Snooze', exact: true }).click();
    await page.getByRole('menuitem', { name: '1 hour', exact: true }).click();
    await expect(page.getByText(/Snoozed until/)).toBeVisible();
    await expect(page.getByRole('option', { name: /Welcome to Mailiner/ })).toHaveCount(0);
  } finally {
    await cleanupMailbox(folder);
  }
});

test('star, flag, archive, and trash fire from the keyboard', async ({ page }) => {
  const folder = uniqueName('keys');
  const archived = uniqueName('keys-arch');
  const trashed = uniqueName('keys-trash');
  try {
    await seedPrivateMessage(page, folder, archived, 'archive from the keyboard');
    await pressShortcut(page, 's');
    await expect(
      page.getByRole('option', { name: new RegExp(archived) }).first().getByRole('button', { name: 'Unstar' }),
    ).toBeVisible();
    await pressShortcut(page, 'i');
    await expect(
      page.getByRole('option', { name: new RegExp(archived) }).first().getByRole('button', { name: 'Unflag' }),
    ).toBeVisible();
    await pressShortcut(page, 'e');
    await expect(page.getByText('Moved to Archive', { exact: true })).toBeVisible({
      timeout: 20_000,
    });

    await seedPrivateMessage(page, `${folder}-del`, trashed, 'trash from the keyboard');
    await pressShortcut(page, 'Delete');
    await expect(page.getByText('Moved to Trash')).toBeVisible({ timeout: 20_000 });
  } finally {
    await cleanupSubject('Archive', archived);
    await cleanupSubject('Trash', trashed);
    await cleanupMailbox(folder);
    await cleanupMailbox(`${folder}-del`);
  }
});

test('a multi-selection moves every selected row', async ({ page }) => {
  const source = uniqueName('multi');
  const dest = uniqueName('multidst');
  try {
    await createFolder(page, source);
    await createFolder(page, dest);
    await openFolder(page, /Inbox/);
    await openSubject(page, 'Base64-encoded plain text');
    await copySelectionTo(page, source);
    await openFolder(page, /Inbox/);
    await openSubject(page, 'Sanitizer bait');
    await copySelectionTo(page, source);
    await openFolder(page, source);
    await searchFolder(page, '');
    const first = messageRows(page).first();
    await expect(first).toBeVisible({ timeout: 20_000 });
    await first.click();
    await pressShortcut(page, 'ArrowDown', true);
    await expect(page.getByText('2 selected')).toBeVisible();
    await moveSelectionTo(page, dest);
    await openFolder(page, dest);
    await expect(page.getByRole('option', { name: /Base64-encoded plain text/ })).toBeVisible({
      timeout: 20_000,
    });
    await expect(page.getByRole('option', { name: /Sanitizer bait/ })).toBeVisible();
  } finally {
    await cleanupMailbox(dest);
    await cleanupMailbox(source);
  }
});

test('select-all plus mark read clears the unread count', async ({ page }) => {
  const folder = uniqueName('all');
  try {
    await createFolder(page, folder);
    // Copies of opened seed mail are already \Seen, so select-all offered
    // "Mark unread". Two fresh messages leave one unread after the folder's
    // auto-mark, and the button marks the whole selection read.
    await deliverRaw(folder, simpleMessage('A Sender <a-all@example.com>', `${folder}-a`, 'one'));
    await deliverRaw(folder, simpleMessage('B Sender <b-all@example.com>', `${folder}-b`, 'two'));
    await openFolder(page, folder);
    await expect(messageRows(page)).toHaveCount(2, { timeout: 20_000 });
    await pressShortcut(page, 'a', false, true);
    await expect(page.getByText('2 selected')).toBeVisible();
    await page.getByRole('button', { name: 'Mark read', exact: true }).click();
    await expect(page.locator('#message-list-scroll .message-list-item.unread')).toHaveCount(0);
    await expect(page.locator('#message-list-scroll .message-list-item')).toHaveCount(2);
  } finally {
    await cleanupMailbox(folder);
  }
});
