/**
 * Pixel baselines for the offline shell. Run with `--project=visual`.
 *
 * List dates go through `format_list_date` and `Utc::now()`, so the clock is
 * frozen. Regenerate the PNGs with the arm64 Playwright image (same Chromium
 * and Ubuntu fonts as CI). See e2e/README.md.
 */
import { test, expect, type Page } from '@playwright/test';
import {
  E2E_MESSAGE_FROM,
  E2E_MESSAGE_SUBJECT,
  composeButton,
  composeOverlay,
  gotoMail,
  gotoPath,
  gotoSettings,
  seedAccount,
} from './helpers';

/** Fixed instant: the seeded envelope (2024-06-15) stays on the day-and-year format. */
const FIXED_NOW = new Date('2026-10-05T12:00:00Z');

const INBOX_ROW = `${E2E_MESSAGE_FROM}, ${E2E_MESSAGE_SUBJECT}`;

async function pinClock(page: Page) {
  // Fixed `Date` with timers still running, so Dioxus can paint.
  await page.clock.setFixedTime(FIXED_NOW);
}

/** Theme and layout prefs are read from localStorage before first paint. */
async function pinPrefs(
  page: Page,
  prefs: { theme: 'light' | 'dark'; layout?: 'stacked' | 'classic' },
) {
  await page.addInitScript((value) => {
    localStorage.setItem('mailiner.ui.theme', value.theme);
    if (value.layout) {
      localStorage.setItem('mailiner.ui.mailLayout', value.layout);
    }
  }, prefs);
}

async function settle(page: Page) {
  await page.evaluate(async () => {
    await document.fonts.ready;
  });
}

async function shot(page: Page, name: string) {
  await settle(page);
  await expect(page).toHaveScreenshot(name);
}

test('onboarding welcome', async ({ page }) => {
  await pinClock(page);
  await pinPrefs(page, { theme: 'light' });
  await gotoPath(page);
  await expect(page.getByRole('heading', { name: 'Welcome to Mailiner' })).toBeVisible();
  await expect(page.getByRole('button', { name: 'Get started' })).toBeVisible();
  await expect(page.locator('html')).toHaveAttribute('data-theme', 'light');
  await shot(page, 'onboarding.png');
});

test('stacked inbox', async ({ page }) => {
  await pinClock(page);
  await pinPrefs(page, { theme: 'light' });
  await seedAccount(page, { cache: true });
  await gotoMail(page);
  await expect(page.locator('#app')).toHaveClass(/layout-stacked/);
  await expect(page.getByRole('treeitem', { name: /Inbox/ })).toBeVisible();
  await expect(page.getByRole('option', { name: INBOX_ROW })).toBeVisible();
  await shot(page, 'inbox.png');
});

test('classic inbox', async ({ page }) => {
  await pinClock(page);
  await pinPrefs(page, { theme: 'light', layout: 'classic' });
  await seedAccount(page, { cache: true });
  await gotoMail(page);
  await expect(page.locator('#app')).toHaveClass(/layout-classic/);
  await expect(page.getByRole('option', { name: INBOX_ROW })).toBeVisible();
  await shot(page, 'inbox-classic.png');
});

test('compose dialog over the inbox', async ({ page }) => {
  await pinClock(page);
  await pinPrefs(page, { theme: 'light' });
  await seedAccount(page, { cache: true });
  await gotoMail(page);
  await composeButton(page).click();
  const overlay = composeOverlay(page);
  await expect(overlay).toBeVisible();
  await expect(overlay.getByLabel('From')).toBeVisible();
  await shot(page, 'compose.png');
});

test('settings home', async ({ page }) => {
  await pinClock(page);
  await pinPrefs(page, { theme: 'light' });
  await seedAccount(page);
  await gotoSettings(page);
  await expect(page.getByRole('heading', { name: 'Keyboard shortcuts' })).toBeVisible();
  // The card is taller than the viewport. Screenshot the card, not the window.
  await settle(page);
  await expect(page.locator('.settings-card')).toHaveScreenshot('settings.png');
});

test.describe('dark theme', () => {
  test.use({ colorScheme: 'dark' });

  test('stacked inbox', async ({ page }) => {
    await pinClock(page);
    await pinPrefs(page, { theme: 'dark' });
    await seedAccount(page, { cache: true });
    await gotoMail(page);
    await expect(page.locator('html')).toHaveAttribute('data-theme', 'dark');
    await expect(page.getByRole('option', { name: INBOX_ROW })).toBeVisible();
    await shot(page, 'inbox-dark.png');
  });
});

test.describe('narrow viewport', () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test('folder pane', async ({ page }) => {
    await pinClock(page);
    await pinPrefs(page, { theme: 'light' });
    await seedAccount(page, { cache: true });
    await gotoPath(page, '/');
    await expect(page.locator('#app')).toBeVisible();
    await expect(page.getByRole('treeitem', { name: /Inbox/ })).toBeVisible();
    // Single-pane chrome: the list/viewer column is hidden at <=1023px.
    await expect(page.locator('#content')).toBeHidden();
    await shot(page, 'inbox-narrow.png');
  });
});
