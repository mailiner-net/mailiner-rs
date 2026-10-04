import { test, expect, type Page } from '@playwright/test';
import { composeButton } from './helpers';
import {
  LIVE_ACCOUNT_EMAIL,
  bodyHtml,
  bodyText,
  gotoLiveMail,
  messageRows,
  openFolder,
  openSubject,
  revealAttachments,
  seedLiveAccount,
  uniqueName,
} from './live-helpers';

test.describe.configure({ timeout: 120_000 });

test.beforeEach(async ({ page }) => {
  await seedLiveAccount(page);
  await gotoLiveMail(page);
});

function newMessage(page: Page) {
  return page.getByRole('dialog', { name: 'New message' });
}

async function expectBody(page: Page, text: string) {
  await expect.poll(async () => bodyText(page), { timeout: 20_000 }).toContain(text);
}

async function fillTo(page: Page, email: string) {
  const to = newMessage(page).getByRole('combobox', { name: 'To' });
  await to.fill(email);
  await to.press('Enter');
  await expect(newMessage(page).getByText(email)).toBeVisible();
}

test('reply prefills the sender and the quote', async ({ page }) => {
  await openSubject(page, 'Meeting notes');
  await page.getByRole('button', { name: 'Reply', exact: true }).click();
  const reply = page.getByRole('dialog', { name: 'Reply' });
  await expect(reply).toBeVisible();
  await expect(reply.locator('.recipient-chip[title*="alice@example.com"]')).toBeVisible();
  await expect(reply.getByLabel('Subject')).toHaveValue(/Re: Meeting notes/);
  await expect(reply.locator('textarea')).toHaveValue(/Ship the local mail container/);
  await expect(reply.locator('.recipient-chip[title*="bob@example.org"]')).toHaveCount(0);
});

test('reply all keeps the Cc', async ({ page }) => {
  await openSubject(page, 'Meeting notes');
  await page.getByRole('button', { name: 'Reply All', exact: true }).click();
  const reply = page.getByRole('dialog', { name: 'Reply' });
  await expect(reply).toBeVisible();
  await expect(reply.locator('.recipient-chip[title*="bob@example.org"]')).toBeVisible();
});

test('forward starts a new recipient list', async ({ page }) => {
  await openSubject(page, 'Meeting notes');
  await page.getByRole('button', { name: 'Forward', exact: true }).click();
  const forward = page.getByRole('dialog', { name: 'Forward' });
  await expect(forward).toBeVisible();
  await expect(forward.getByLabel('Subject')).toHaveValue(/Fwd: Meeting notes/);
  await expect(forward.locator('textarea')).toHaveValue(/Ship the local mail container/);
  await expect(forward.locator('.recipient-chip[title*="alice@example.com"]')).toHaveCount(0);
});

test('a plain send to yourself is delivered', async ({ page }) => {
  const subject = uniqueName('send');
  await composeButton(page).click();
  const compose = newMessage(page);
  await expect(compose).toBeVisible();
  await fillTo(page, LIVE_ACCOUNT_EMAIL);
  await compose.getByLabel('Subject').fill(subject);
  await compose.locator('textarea').fill('hello from the e2e send');
  await compose.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.locator('.toast-message', { hasText: 'Sent' })).toBeVisible({ timeout: 40_000 });

  await openFolder(page, 'Sent');
  await openSubject(page, subject);
  await expectBody(page, 'hello from the e2e send');

  await openFolder(page, /Inbox/);
  await openSubject(page, subject);
  await expectBody(page, 'hello from the e2e send');
});

test('a rich send survives a round trip', async ({ page }) => {
  const subject = uniqueName('rich');
  await composeButton(page).click();
  const compose = newMessage(page);
  await fillTo(page, LIVE_ACCOUNT_EMAIL);
  await compose.getByLabel('Subject').fill(subject);
  await compose.getByRole('button', { name: 'Rich', exact: true }).click();
  await compose.locator('#mailiner-compose-editor').click();
  await compose.getByRole('button', { name: 'Bold', exact: true }).click();
  await page.keyboard.type('boldword');
  await compose.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.locator('.toast-message', { hasText: 'Sent' })).toBeVisible({ timeout: 40_000 });

  await openFolder(page, /Inbox/);
  await openSubject(page, subject);
  await expectBody(page, 'boldword');
  const html = await bodyHtml(page);
  expect(html).toMatch(/<(?:b|strong)\b|font-weight:\s*(?:bold|[6-9]00)/i);
});

test('an attachment survives a round trip', async ({ page }) => {
  const subject = uniqueName('attach');
  await composeButton(page).click();
  const compose = newMessage(page);
  await fillTo(page, LIVE_ACCOUNT_EMAIL);
  await compose.getByLabel('Subject').fill(subject);
  await compose.locator('textarea').fill('see attachment');
  await compose.getByLabel('Attach files').setInputFiles({
    name: 'note.txt',
    mimeType: 'text/plain',
    buffer: Buffer.from('attachment-bytes-e2e'),
  });
  await expect(compose.getByText('note.txt')).toBeVisible();
  await compose.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(page.locator('.toast-message', { hasText: 'Sent' })).toBeVisible({ timeout: 40_000 });

  await openFolder(page, /Inbox/);
  await openSubject(page, subject);
  await revealAttachments(page);
  const item = page.locator('.attachment-item', { hasText: 'note.txt' });
  await expect(item).toBeVisible();
  const [download] = await Promise.all([
    page.waitForEvent('download'),
    item.getByRole('button', { name: 'Download', exact: true }).click(),
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
  expect(text).toContain('attachment-bytes-e2e');
});

test('closing compose restores the draft and discard removes it', async ({ page }) => {
  const subject = uniqueName('draft');
  await composeButton(page).click();
  const compose = newMessage(page);
  await compose.getByLabel('Subject').fill(subject);
  await compose.locator('textarea').fill('draft body stays');
  await compose.getByRole('button', { name: 'Close', exact: true }).last().click();
  await expect(compose).toHaveCount(0);

  await composeButton(page).click();
  const restored = newMessage(page);
  await expect(restored.getByLabel('Subject')).toHaveValue(subject);
  await expect(restored.locator('textarea')).toHaveValue(/draft body stays/);
  // The modal backdrop covers the folder tree. Dock the draft, then open Drafts.
  await restored.getByRole('button', { name: 'Dock to bottom' }).click();
  // Docking leaves a region, not a dialog. Discard is on the page.
  const docked = page.getByRole('region', { name: 'New message' });
  await expect(docked).toBeVisible();

  await openFolder(page, 'Drafts');
  await expect(messageRows(page).filter({ hasText: subject }).first()).toBeVisible({
    timeout: 30_000,
  });

  await page.getByRole('button', { name: 'Discard', exact: true }).click();
  await expect(docked).toHaveCount(0);
  await openFolder(page, 'Drafts');
  await expect(messageRows(page).filter({ hasText: subject })).toHaveCount(0, {
    timeout: 20_000,
  });
});

test('a non-address in To does not send', async ({ page }) => {
  await composeButton(page).click();
  const compose = newMessage(page);
  await compose.getByRole('combobox', { name: 'To' }).fill('not an email');
  await compose.getByRole('combobox', { name: 'To' }).press('Enter');
  await compose.getByRole('button', { name: 'Send', exact: true }).click();
  await expect(compose.getByText(/Cannot send/)).toBeVisible();
  await expect(compose).toBeVisible();
});
