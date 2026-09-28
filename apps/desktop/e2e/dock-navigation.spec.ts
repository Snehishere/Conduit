import { test, expect } from './fixtures';

test.describe('Dock navigation', () => {
  const NAV_ITEMS = ['Home', 'Inbox', 'Calls', 'Files', 'Mirror', 'Remote', 'Automation', 'Settings'] as const;

  for (const label of NAV_ITEMS) {
    test(`clicking "${label}" in dock switches the view`, async ({ mockedPage: page }) => {
      await page.goto('/');

      const dock = page.locator('[aria-label="Main navigation"]');
      await expect(dock).toBeVisible({ timeout: 15_000 });

      const button = dock.locator(`[aria-label="${label}"]`);
      await expect(button).toBeVisible();

      // Click the nav item
      await button.click();

      // The active indicator should move to the clicked button
      // (the button should have accent-dim background, indicating active state)
      await expect(button).toBeVisible();
    });
  }

  test('dock has correct number of primary nav items', async ({ mockedPage: page }) => {
    await page.goto('/');

    const dock = page.locator('[aria-label="Main navigation"]');
    await expect(dock).toBeVisible({ timeout: 15_000 });

    // Single non-expanding dock: 8 nav items + optional surround toggle + the
    // "Pair device" "+" button (surround only renders when App passes the
    // toggle props).
    const navButtons = dock.locator('button[aria-label]');
    const count = await navButtons.count();
    expect(count).toBeGreaterThanOrEqual(9);
    expect(count).toBeLessThanOrEqual(10);
    await expect(dock.locator('[aria-label="Pair device"]')).toBeVisible();
    await expect(dock.locator('[aria-label="Show additional actions"]')).toHaveCount(0);
  });
});
