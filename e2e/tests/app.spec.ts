import { test, expect } from '@playwright/test';

test('loads the main layout', async ({ page }) => {
  await page.goto('/');

  await expect(page.locator('#app')).toBeVisible();
});
