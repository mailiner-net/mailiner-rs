import { test, expect } from '@playwright/test';
import { appPath, appPrefix, gotoPath, seedAccount } from './helpers';

/**
 * Mount-prefix checks. The rest of the suite also runs under
 * `MAILINER_E2E_BASE_PATH` (see `appPath`). These cases are the ones that
 * only fail when the app is served below the host root.
 */
test.describe('non-root deploy', () => {
  test.skip(!appPrefix(), 'set MAILINER_E2E_BASE_PATH (for example /app)');

  test('shell assets, routes, and the service worker stay under the prefix', async ({
    page,
  }) => {
    const prefix = appPrefix();
    const assetUrls: string[] = [];
    page.on('request', (request) => {
      const type = request.resourceType();
      if (type === 'script' || type === 'stylesheet' || type === 'image') {
        assetUrls.push(request.url());
      }
    });

    await seedAccount(page);
    await gotoPath(page, '/settings/accounts/e2e-account-1');
    await expect(page.getByRole('heading', { name: 'Edit account' })).toBeVisible();
    await expect(page).toHaveURL(
      (url) => url.pathname === appPath('/settings/accounts/e2e-account-1'),
    );

    await page.locator('#account-edit-auth-kind').selectOption('oauth2');
    const origin = new URL(page.url()).origin;
    await expect(page.getByLabel('Redirect URI')).toHaveValue(
      `${origin}${appPath('/oauth/callback')}`,
    );

    const scriptSrcs = await page.locator('script[src]').evaluateAll((nodes) =>
      nodes.map((node) => (node as HTMLScriptElement).src),
    );
    expect(scriptSrcs.length).toBeGreaterThan(0);
    for (const src of scriptSrcs) {
      const path = new URL(src).pathname;
      expect(path === prefix || path.startsWith(`${prefix}/`), src).toBe(true);
    }

    const sameOriginAssets = assetUrls.filter((url) => {
      const parsed = new URL(url);
      return parsed.origin === new URL(page.url()).origin;
    });
    expect(sameOriginAssets.length).toBeGreaterThan(0);
    for (const url of sameOriginAssets) {
      const path = new URL(url).pathname;
      expect(path === prefix || path.startsWith(`${prefix}/`), url).toBe(true);
    }

    await page.getByRole('link', { name: 'Back to accounts' }).click();
    await expect(page.getByRole('heading', { name: 'Accounts', exact: true })).toBeVisible();
    await expect(page).toHaveURL((url) => url.pathname === appPath('/settings/accounts'));

    await expect
      .poll(async () =>
        page.evaluate(async (scopePrefix) => {
          const regs = await navigator.serviceWorker.getRegistrations();
          return regs.some((reg) => {
            const scopePath = new URL(reg.scope).pathname;
            return scopePath === `${scopePrefix}/` || scopePath === scopePrefix;
          });
        }, prefix),
      )
      .toBe(true);

    // The callback document is the SPA shell. With no opener and no stored
    // session the page assigns the app root, and that return stays under the
    // prefix. A bare `/` would be the host root.
    const callback = await page.goto(appPath('/oauth/callback'), {
      waitUntil: 'domcontentloaded',
    });
    expect(callback?.status()).toBe(200);
    expect(new URL(callback?.url() ?? page.url()).pathname).toBe(appPath('/oauth/callback'));
    await expect(page).toHaveURL((url) => url.pathname.replace(/\/$/, '') === prefix);
    await expect(page.getByRole('link', { name: 'Skip to message' })).toBeVisible();
  });

  test('the host root is not the app', async ({ page }) => {
    const response = await page.goto('/', { waitUntil: 'domcontentloaded' });
    const status = response?.status() ?? 0;
    const splash = await page.locator('#mailiner-splash').count();
    const main = await page.locator('#main').count();
    expect(status === 404 || (splash === 0 && main === 0)).toBe(true);
  });
});
