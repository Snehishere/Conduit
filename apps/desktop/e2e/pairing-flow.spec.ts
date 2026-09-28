import { test, expect } from './fixtures';

test.describe('Pairing flow', () => {
  test('pairing flow opens and shows QR code step', async ({ mockedPage: page }) => {
    await page.goto('/');

    // Wait for app to load
    const dock = page.locator('[aria-label="Main navigation"]');
    await expect(dock).toBeVisible({ timeout: 15_000 });

    // Use keyboard shortcut Ctrl+N to open pairing (defined in App.tsx)
    await page.keyboard.press('Control+n');

    // The pairing modal should appear with "Pair Device" heading
    const heading = page.locator('text=Pair Device');
    await expect(heading).toBeVisible({ timeout: 10_000 });

    // It should progress past "generating" to the QR scan step
    const scanInstruction = page.locator('text=Scan with your mobile device');
    await expect(scanInstruction).toBeVisible({ timeout: 10_000 });
  });

  test('pairing flow can be closed with Escape', async ({ mockedPage: page }) => {
    await page.goto('/');

    const dock = page.locator('[aria-label="Main navigation"]');
    await expect(dock).toBeVisible({ timeout: 15_000 });

    // Open pairing
    await page.keyboard.press('Control+n');
    await expect(page.locator('text=Pair Device')).toBeVisible({ timeout: 10_000 });

    // Close with Escape
    await page.keyboard.press('Escape');

    // Pairing modal should be gone
    await expect(page.locator('text=Pair Device')).not.toBeVisible({ timeout: 5_000 });
  });

  test('pairing shows formatted code, QR image, and Troubleshoot button', async ({ mockedPage: page }) => {
    await page.goto('/');
    await expect(page.locator('[aria-label="Main navigation"]')).toBeVisible({ timeout: 15_000 });

    await page.keyboard.press('Control+n');
    await expect(page.locator('text=Pair Device')).toBeVisible({ timeout: 10_000 });
    await expect(page.locator('text=Scan with your mobile device')).toBeVisible({ timeout: 10_000 });

    // Mock token is 'abcd1234efgh' → displayed as 'abcd-1234'
    const code = page.locator('code[aria-label^="Pairing code:"]');
    await expect(code).toBeVisible();
    await expect(code).toHaveText('abcd-1234');
    await expect(code).toHaveAttribute('aria-label', 'Pairing code: abcd-1234');

    // QR image container
    const qr = page.locator('[role="img"][aria-label="QR code for device pairing"]');
    await expect(qr).toBeVisible();

    // Troubleshoot button
    const troubleshoot = page.locator(
      'button[aria-label="Troubleshoot connection — fix Windows Firewall permissions"]'
    );
    await expect(troubleshoot).toBeVisible();
    await expect(troubleshoot).toHaveText('Troubleshoot Connection');
  });
});
