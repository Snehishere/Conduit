import { test, expect } from './fixtures';

test.describe('Settings page', () => {
  test('settings page is accessible via dock navigation', async ({ mockedPage: page }) => {
    await page.goto('/');

    // Wait for the floating dock to appear
    const dock = page.locator('[aria-label="Main navigation"]');
    await expect(dock).toBeVisible({ timeout: 15_000 });

    // Settings lives directly in the single non-expanding dock.
    const settingsButton = page.locator('[aria-label="Settings"]');
    await expect(settingsButton).toBeVisible();
    await settingsButton.click();

    // Settings page should render with a heading
    const heading = page.locator('text=Settings').first();
    await expect(heading).toBeVisible({ timeout: 10_000 });

    // Verify sidebar categories are present (sidebar button, not the h2)
    const generalTab = page.getByRole('button', { name: 'General' });
    await expect(generalTab).toBeVisible();

    // Verify the "Save Settings" button exists
    const saveButton = page.locator('text=Save Settings');
    await expect(saveButton).toBeVisible();
  });

  test('settings has device name input', async ({ mockedPage: page }) => {
    await page.goto('/');

    // Navigate to settings
    const dock = page.locator('[aria-label="Main navigation"]');
    await expect(dock).toBeVisible({ timeout: 15_000 });
    await page.locator('[aria-label="Settings"]').click();

    // The device name input should be present in General settings
    const deviceNameInput = page.locator('input[type="text"]').first();
    await expect(deviceNameInput).toBeVisible({ timeout: 10_000 });
  });

  test('settings persist across page reload after Save', async ({ mockedPage: page }) => {
    await page.goto('/');
    await expect(page.locator('[aria-label="Main navigation"]')).toBeVisible({ timeout: 15_000 });

    // Ctrl+, navigates to Settings (defined in App.tsx)
    await page.keyboard.press('Control+,');
    const deviceName = page.locator('#settings-device-name');
    await expect(deviceName).toBeVisible({ timeout: 10_000 });

    // Change name and save
    await deviceName.fill('Persisted Desktop');
    await page.locator('button:has-text("Save Settings")').click();
    await expect(page.locator('button:has-text("Saved!")')).toBeVisible({ timeout: 5_000 });

    // Assert save_settings was invoked with our value
    const saveCalls = await page.evaluate(() =>
      (window as any).__invokeLog?.filter((e: any) => e.cmd === 'save_settings') ?? []
    );
    expect(saveCalls.length).toBeGreaterThan(0);
    expect(saveCalls[0].args?.settings?.device_name).toBe('Persisted Desktop');

    // Reload — settings must round-trip via localStorage-backed mock
    await page.reload();
    await expect(page.locator('[aria-label="Main navigation"]')).toBeVisible({ timeout: 15_000 });
    await page.keyboard.press('Control+,');
    await expect(page.locator('#settings-device-name')).toBeVisible({ timeout: 10_000 });
    await expect(page.locator('#settings-device-name')).toHaveValue('Persisted Desktop');
  });
});
