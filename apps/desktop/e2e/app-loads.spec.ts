import { test, expect } from './fixtures';

test.describe('App smoke tests', () => {
  test('app loads and renders the main UI', async ({ mockedPage: page }) => {
    await page.goto('/');

    // The app should render a root element
    const root = page.locator('#root');
    await expect(root).toBeVisible();

    // Wait for React to hydrate — the FloatingDock is always present
    const dock = page.locator('[aria-label="Main navigation"]');
    await expect(dock).toBeVisible({ timeout: 15_000 });

    // No custom title bar: the window uses native decorations
    // (tauri.conf.json → decorations: true), so TitleBar is intentionally
    // not mounted. Assert the app shell instead.
    await expect(page.locator('main')).toBeVisible();
    await expect(page.locator('footer')).toBeVisible();
  });

  test('status bar shows connection state', async ({ mockedPage: page }) => {
    await page.goto('/');

    // The offline banner or status bar should be present
    const statusText = page.locator('text=/offline|connected|disconnect/i');
    await expect(statusText.first()).toBeVisible({ timeout: 15_000 });
  });
});
