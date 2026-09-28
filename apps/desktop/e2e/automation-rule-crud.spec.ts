import { test, expect, type Page } from './fixtures';

/** Navigate from Home to the Automation panel via the floating dock. */
async function openAutomation(page: Page) {
  await page.goto('/');
  await expect(page.locator('[aria-label="Main navigation"]')).toBeVisible({ timeout: 15_000 });
  await page.locator('[aria-label="Automation"]').click();
  await expect(page.locator('h1:has-text("Automation")')).toBeVisible({ timeout: 10_000 });
}

test.describe('Automation rules CRUD', () => {
  test('empty state → create rule → card appears with Triggered 0 times', async ({ mockedPage: page }) => {
    await openAutomation(page);

    // Empty state
    const empty = page.locator('[role="status"][aria-label^="No automation rules"]');
    await expect(empty).toBeVisible();
    await expect(empty).toHaveAttribute(
      'aria-label',
      'No automation rules. Create rules to automate tasks between your devices'
    );

    // Open create dialog
    await page.locator('button:has-text("New Rule")').click();
    await expect(page.locator('[role="dialog"]')).toBeVisible();
    await expect(page.locator('#rule-editor-title')).toHaveText('Create Rule');

    // Save disabled while name empty
    const saveBtn = page.locator('button:has-text("Save Rule")');
    await expect(saveBtn).toBeDisabled();

    // Type name → Save enables → create
    await page.locator('#rule-name').fill('Morning Sync');
    await expect(saveBtn).toBeEnabled();
    await saveBtn.click();

    // Dialog closes; rule card appears
    await expect(page.locator('[role="dialog"]')).not.toBeVisible();
    await expect(page.locator('text=Morning Sync')).toBeVisible();
    await expect(page.locator('text=Triggered 0 times')).toBeVisible();
    // Default trigger/action labels
    await expect(page.locator('text=Device Connects')).toBeVisible();
    await expect(page.locator('text=Send Notification')).toBeVisible();

    // Empty state gone
    await expect(page.locator('[role="status"][aria-label^="No automation rules"]')).not.toBeVisible();
  });

  test('edit rule renames it and Save updates the card', async ({ mockedPage: page }) => {
    await openAutomation(page);

    // Create first
    await page.locator('button:has-text("New Rule")').click();
    await page.locator('#rule-name').fill('Original Name');
    await page.locator('button:has-text("Save Rule")').click();
    await expect(page.locator('text=Original Name')).toBeVisible();

    // Edit
    await page.locator('[aria-label="Edit Original Name"]').click();
    await expect(page.locator('#rule-editor-title')).toHaveText('Edit Rule');
    await expect(page.locator('#rule-name')).toHaveValue('Original Name');

    await page.locator('#rule-name').fill('Renamed Rule');
    await page.locator('button:has-text("Save Rule")').click();

    await expect(page.locator('[role="dialog"]')).not.toBeVisible();
    await expect(page.locator('text=Renamed Rule')).toBeVisible();
    await expect(page.locator('text=Original Name')).not.toBeVisible();
    // Edit aria-label follows the new name
    await expect(page.locator('[aria-label="Edit Renamed Rule"]')).toBeVisible();
  });

  test('toggle switch flips aria-checked', async ({ mockedPage: page }) => {
    await openAutomation(page);

    await page.locator('button:has-text("New Rule")').click();
    await page.locator('#rule-name').fill('Toggle Me');
    await page.locator('button:has-text("Save Rule")').click();
    await expect(page.locator('text=Toggle Me')).toBeVisible();

    // New rules are created enabled: true
    const toggle = page.locator('[role="switch"][aria-label="Toggle Toggle Me"]');
    await expect(toggle).toHaveAttribute('aria-checked', 'true');

    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-checked', 'false');

    await toggle.click();
    await expect(toggle).toHaveAttribute('aria-checked', 'true');
  });

  test('delete rule removes card and returns empty state', async ({ mockedPage: page }) => {
    await openAutomation(page);

    await page.locator('button:has-text("New Rule")').click();
    await page.locator('#rule-name').fill('Doomed Rule');
    await page.locator('button:has-text("Save Rule")').click();
    await expect(page.locator('text=Doomed Rule')).toBeVisible();

    await page.locator('[aria-label="Delete Doomed Rule"]').click();

    await expect(page.locator('text=Doomed Rule')).not.toBeVisible();
    const empty = page.locator('[role="status"][aria-label^="No automation rules"]');
    await expect(empty).toBeVisible();
  });
});
