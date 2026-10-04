import { test, expect } from '@playwright/test';
import {
  bodyHtml,
  bodyText,
  gotoLiveMail,
  openFolder,
  openSubject,
  revealAttachments,
  searchFolder,
  seedLiveAccount,
} from './live-helpers';

test.describe.configure({ timeout: 120_000 });

test.beforeEach(async ({ page }) => {
  await seedLiveAccount(page);
  await gotoLiveMail(page);
});

async function expectBody(page: import('@playwright/test').Page, text: string) {
  await expect.poll(async () => bodyText(page), { timeout: 20_000 }).toContain(text);
}

test('HTML-only announcement renders the HTML part', async ({ page }) => {
  await openSubject(page, 'HTML-only announcement');
  await expectBody(page, 'HTML only');
  await expectBody(page, 'no text/plain part');
  const html = await bodyHtml(page);
  expect(html).toContain('https://mailiner.test');
  expect(html).toMatch(/<h1[\s>]/);
});

test('multipart alternative prefers HTML and can show the plain part', async ({ page }) => {
  await openSubject(page, 'Meeting notes');
  await expectBody(page, 'Meeting notes');
  await expectBody(page, 'Ship the local mail container');
  await page.getByRole('button', { name: 'Plain text' }).click();
  await expectBody(page, 'Ship the local mail container');
  const html = await bodyHtml(page);
  expect(html).not.toContain('<h2>');
});

test('the seeded flagged message is flagged before any click', async ({ page }) => {
  await openSubject(page, 'Please review the attached notes');
  const row = page.getByRole('option', { name: /Please review the attached notes/ }).first();
  await expect(row.getByRole('button', { name: 'Unflag' })).toBeVisible();
});

test('a single attachment downloads as the original file', async ({ page }) => {
  await openSubject(page, 'Please review the attached notes');
  await revealAttachments(page);
  const item = page.locator('.attachment-item', { hasText: 'notes.txt' });
  await expect(item).toBeVisible();
  const [download] = await Promise.all([
    page.waitForEvent('download'),
    item.getByRole('button', { name: 'Download', exact: true }).click(),
  ]);
  const path = await download.path();
  expect(path).toBeTruthy();
  const text = await download.createReadStream().then(
    (stream) =>
      new Promise<string>((resolve, reject) => {
        const chunks: Buffer[] = [];
        stream.on('data', (chunk) => chunks.push(Buffer.from(chunk)));
        stream.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
        stream.on('error', reject);
      }),
  );
  expect(text).toContain('Project notes');
  expect(text).toContain('Dovecot');
});

test('two attachments are listed and the csv downloads', async ({ page }) => {
  await openSubject(page, 'Two attachments (txt + csv)');
  await revealAttachments(page);
  await expect(page.locator('.attachment-item', { hasText: 'readme.txt' })).toBeVisible();
  const csv = page.locator('.attachment-item', { hasText: 'invoice.csv' });
  await expect(csv).toBeVisible();
  const [download] = await Promise.all([
    page.waitForEvent('download'),
    csv.getByRole('button', { name: 'Download', exact: true }).click(),
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
  expect(text).toContain('Widgets,3,12.00');
});

test('an inline CID image is rehydrated', async ({ page }) => {
  await openSubject(page, 'Logo proof (inline image)');
  const html = await bodyHtml(page);
  expect(html).toContain('red pixel');
  expect(html).not.toContain('cid:pixel@mailiner.test');
  expect(html).toMatch(/src="(?:data:|blob:)/);
});

test('remote images stay blocked until Allow', async ({ page }) => {
  const hits: string[] = [];
  page.on('request', (req) => {
    if (req.url().includes('example.net/tracker.png')) {
      hits.push(req.url());
    }
  });
  await openSubject(page, 'Remote image should stay blocked');
  await expect(page.getByText('remote resources (images, styles) were blocked')).toBeVisible();
  expect(hits).toEqual([]);
  const blocked = await bodyHtml(page);
  expect(blocked).not.toContain('https://example.net/tracker.png');

  await page.getByRole('button', { name: 'Allow', exact: true }).click();
  await expect.poll(() => hits.length, { timeout: 15_000 }).toBeGreaterThan(0);
  await expect(page.getByText('Remote resources are shown for this message.')).toBeVisible();
});

test('always-allow is scoped to the sender and persists', async ({ page }) => {
  await openSubject(page, 'Remote image should stay blocked');
  await page.getByRole('button', { name: 'Always allow from this sender' }).click();
  await expect(page.getByText(/allowed for news@example.net/)).toBeVisible();

  await page.reload();
  await expect(page.getByText('Connected', { exact: true })).toBeVisible({ timeout: 45_000 });
  const hits: string[] = [];
  page.on('request', (req) => {
    if (req.url().includes('example.net/tracker.png')) {
      hits.push(req.url());
    }
  });
  await openSubject(page, 'Remote image should stay blocked');
  await expect.poll(() => hits.length, { timeout: 15_000 }).toBeGreaterThan(0);
  await expect(page.getByRole('button', { name: 'Allow', exact: true })).toHaveCount(0);

  await openSubject(page, 'HTML-only announcement');
  await expect(page.getByText(/allowed for news@example.net/)).toHaveCount(0);
});

test('UTF-8 subject and quoted-printable body decode', async ({ page }) => {
  await openSubject(page, 'Café');
  await expect(page.getByRole('option', { name: /Café résumé/ }).first()).toBeVisible();
  await expectBody(page, 'naïve œufs');
});

test('base64 plain text decodes', async ({ page }) => {
  await openSubject(page, 'Base64-encoded plain text');
  await expectBody(page, 'This plain-text body is base64-encoded.');
  await expectBody(page, 'Line two confirms decoding across a wrap.');
});

test('a missing Subject uses the empty fallback', async ({ page }) => {
  await openSubject(page, 'deliberately');
  await expect(page.getByText('(no subject)').first()).toBeVisible();
  await expectBody(page, 'deliberately has no Subject');
});

test('HTML bait is neutralized', async ({ page }) => {
  await openSubject(page, 'Sanitizer bait');
  await expectBody(page, 'Safe text after the bait.');
  const html = await bodyHtml(page);
  expect(html).not.toMatch(/<script[\s>]/i);
  expect(html).not.toMatch(/\sonerror\s*=/i);
  expect(html).not.toMatch(/<iframe[\s>]/i);
  expect(html).not.toMatch(/href\s*=\s*["']javascript:/i);
});

test('conversations collapse the lunch thread', async ({ page }) => {
  await searchFolder(page, 'lunch');
  await page.getByLabel('Message list view').selectOption('conversations');
  const thread = page.getByRole('option', { name: /lunch plans/ }).first();
  await expect(thread).toBeVisible({ timeout: 20_000 });
  await expect(thread).toContainText(/2/);
  await thread.getByRole('button', { name: 'Expand conversation' }).click();
  await expect(page.getByText('Are you free for lunch on Wednesday?').first()).toBeVisible();
  await expect(page.getByText('12:30 at the usual place').first()).toBeVisible();
});

test('the nested newsletter keeps its inner parts', async ({ page }) => {
  await openSubject(page, 'Nested newsletter');
  await expectBody(page, 'Weekly digest');
  const html = await bodyHtml(page);
  expect(html).not.toContain('cid:badge@mailiner.test');
  expect(html).toMatch(/src="(?:data:|blob:)/);
  await revealAttachments(page);
  await expect(page.locator('.attachment-item', { hasText: 'standup.ics' })).toBeVisible();
});

test('Drafts opens in the composer', async ({ page }) => {
  await openFolder(page, 'Drafts');
  await openSubject(page, '(draft) half-written reply');
  await page.getByRole('button', { name: 'Edit draft' }).click();
  const compose = page.getByRole('dialog', { name: 'Draft' });
  await expect(compose).toBeVisible();
  await expect(compose.locator('.recipient-chip[title*="alice@example.com"]')).toBeVisible();
  await expect(compose.getByLabel('Subject')).toHaveValue('(draft) half-written reply');
  await expect(compose.locator('textarea')).toHaveValue(/Just wanted to follow up/);
});

test('Sent and Trash show their seeded bodies', async ({ page }) => {
  await openFolder(page, 'Sent');
  await openSubject(page, 'Sent copy: lunch confirmed');
  await expectBody(page, 'classify this mailbox as Sent');

  await openFolder(page, 'Trash');
  await openSubject(page, 'Already in Trash');
  await expectBody(page, 'seeded directly into Trash');
  await expect(page.getByRole('button', { name: 'Delete', exact: true })).toBeVisible();
});
