import { test, expect } from './fixtures';

test.describe('Keyboard shortcuts', () => {
  test('Ctrl+N opens pairing, Ctrl+W closes it', async ({ mockedPage: page }) => {
    await page.goto('/');

    const dock = page.locator('[aria-label="Main navigation"]');
    await expect(dock).toBeVisible({ timeout: 15_000 });

    // Open pairing with Ctrl+N
    await page.keyboard.press('Control+n');
    await expect(page.locator('text=Pair Device')).toBeVisible({ timeout: 10_000 });

    // Close with Ctrl+W
    await page.keyboard.press('Control+w');
    await expect(page.locator('text=Pair Device')).not.toBeVisible({ timeout: 5_000 });
  });

  test('Ctrl+, navigates to settings', async ({ mockedPage: page }) => {
    await page.goto('/');

    const dock = page.locator('[aria-label="Main navigation"]');
    await expect(dock).toBeVisible({ timeout: 15_000 });

    // Navigate to settings via shortcut; the h2 shows the active category
    await page.keyboard.press('Control+,');

    // Settings view should render (sidebar heading + save button + category h2)
    await expect(page.locator('text=Save Settings')).toBeVisible({ timeout: 10_000 });
    await expect(page.locator('h2:has-text("General")')).toBeVisible();
  });
});
