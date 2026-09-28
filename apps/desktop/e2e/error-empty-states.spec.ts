import { test, expect } from './fixtures';

test.describe('Error and empty states', () => {
  test('pairing token failure shows Pairing Failed, Try Again recovers', async ({ mockedPage: page }) => {
    // Fail generate_pairing_token on the first two calls (StrictMode
    // double-invokes the PairingFlow mount effect), succeed on retry.
    await page.addInitScript(() => {
      const internals = (window as any).__TAURI_INTERNALS__;
      if (!internals) return;
      const original = internals.invoke.bind(internals);
      let attempts = 0;
      internals.invoke = async (cmd: string, args?: Record<string, unknown>) => {
        if (cmd === 'generate_pairing_token') {
          attempts++;
          if (attempts <= 2) throw new Error('mock pairing token failure');
        }
        return original(cmd, args);
      };
    });

    await page.goto('/');
    await expect(page.locator('[aria-label="Main navigation"]')).toBeVisible({ timeout: 15_000 });
    await page.keyboard.press('Control+n');

    // Error step
    await expect(page.locator('text=Pairing Failed')).toBeVisible({ timeout: 10_000 });
    await expect(page.locator('text=Failed to generate pairing token')).toBeVisible();
    const tryAgain = page.locator('button:has-text("Try Again")');
    await expect(tryAgain).toBeVisible();

    // Retry succeeds → scan step
    await tryAgain.click();
    await expect(page.locator('text=Scan with your mobile device')).toBeVisible({ timeout: 10_000 });
    await expect(page.locator('text=Pairing Failed')).not.toBeVisible();
  });

  test('settings load failure falls back to default device name Desktop', async ({ mockedPage: page }) => {
    // Always-fail get_settings
    await page.addInitScript(() => {
      const internals = (window as any).__TAURI_INTERNALS__;
      if (!internals) return;
      const original = internals.invoke.bind(internals);
      internals.invoke = async (cmd: string, args?: Record<string, unknown>) => {
        if (cmd === 'get_settings') throw new Error('mock get_settings failure');
        return original(cmd, args);
      };
    });

    await page.goto('/');
    await expect(page.locator('[aria-label="Main navigation"]')).toBeVisible({ timeout: 15_000 });
    await page.keyboard.press('Control+,');

    // Settings still renders with the component's hardcoded default: 'Desktop'
    const deviceName = page.locator('#settings-device-name');
    await expect(deviceName).toBeVisible({ timeout: 10_000 });
    await expect(deviceName).toHaveValue('Desktop');
    // Save button still present (user can proceed despite load failure)
    await expect(page.locator('button:has-text("Save Settings")')).toBeVisible();
  });

  test('clipboard shows empty state when history is empty', async ({ mockedPage: page }) => {
    await page.goto('/');
    await expect(page.locator('[aria-label="Main navigation"]')).toBeVisible({ timeout: 15_000 });

    // Clipboard + files are ONE merged masonry surface (no tabs)
    const dock = page.locator('[aria-label="Main navigation"]');
    await dock.locator('[aria-label="Files"]').click();

    const empty = page.locator('[role="status"][aria-label^="Your clipboard is empty"]');
    await expect(empty).toBeVisible({ timeout: 10_000 });
    await expect(empty).toHaveAttribute(
      'aria-label',
      'Your clipboard is empty. Copy text or files on any connected device to see them here'
    );
  });

  test('pairing shows Maximum devices reached when 5 devices exist', async ({ mockedPage: page }) => {
    // Return 5 devices so App passes deviceCount=5 (>= MAX_DEVICES) to PairingFlow
    await page.addInitScript(() => {
      const internals = (window as any).__TAURI_INTERNALS__;
      if (!internals) return;
      const original = internals.invoke.bind(internals);
      const five = Array.from({ length: 5 }, (_, i) => ({
        id: `dev-${i}`,
        name: `Device ${i}`,
        device_type: 'phone',
        os: 'android',
        status: 'connected',
        last_seen: 0,
      }));
      internals.invoke = async (cmd: string, args?: Record<string, unknown>) => {
        if (cmd === 'get_devices') return five;
        return original(cmd, args);
      };
    });

    await page.goto('/');
    await expect(page.locator('[aria-label="Main navigation"]')).toBeVisible({ timeout: 15_000 });
    await page.keyboard.press('Control+n');

    await expect(page.locator('text=Maximum devices reached')).toBeVisible({ timeout: 10_000 });
    await expect(page.locator('text=Unpair an existing device first (limit: 5)')).toBeVisible();
    // No QR / scan step in limit mode
    await expect(page.locator('text=Scan with your mobile device')).not.toBeVisible();

    // Close button dismisses
    await page.locator('[role="alert"] button:has-text("Close")').click();
    await expect(page.locator('text=Maximum devices reached')).not.toBeVisible({ timeout: 5_000 });
  });
});
