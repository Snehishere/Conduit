import { test, expect } from './fixtures';

test('bubbles rest — no idle jiggle/drift', async ({ mockedPage: page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__;
    if (!internals) return;
    const original = internals.invoke.bind(internals);
    internals.invoke = async (cmd: string) => {
      if (cmd === 'get_devices') {
        return [{ id: 'dev-1', name: 'Pixel 9', device_type: 'phone', os: 'android', status: 'connected', last_seen: 0 }];
      }
      return original(cmd);
    };
  });
  await page.goto('/');
  await expect(page.locator('[aria-label="Main navigation"]')).toBeVisible({ timeout: 15_000 });
  await page.waitForTimeout(2500); // let the entry animation settle

  const ball = page.locator('.glass-ball').first();
  await expect(ball).toBeVisible({ timeout: 10_000 });
  const box = await ball.boundingBox();
  expect(box).toBeTruthy();
  const clip = {
    x: Math.max(0, Math.floor(box!.x) - 4),
    y: Math.max(0, Math.floor(box!.y) - 4),
    width: Math.ceil(box!.width) + 8,
    height: Math.ceil(box!.height) + 8,
  };

  const first = await page.screenshot({ clip });
  await page.waitForTimeout(1200);
  const second = await page.screenshot({ clip });

  // Idle state must be pixel-identical (no wobble/drift/shimmer)
  expect(Buffer.compare(first, second)).toBe(0);
});

test('bubble is draggable, then settles still again', async ({ mockedPage: page }) => {
  await page.addInitScript(() => {
    const internals = (window as any).__TAURI_INTERNALS__;
    if (!internals) return;
    const original = internals.invoke.bind(internals);
    internals.invoke = async (cmd: string) => {
      if (cmd === 'get_devices') {
        return [{ id: 'dev-1', name: 'Pixel 9', device_type: 'phone', os: 'android', status: 'connected', last_seen: 0 }];
      }
      return original(cmd);
    };
  });
  await page.goto('/');
  await expect(page.locator('[aria-label="Main navigation"]')).toBeVisible({ timeout: 15_000 });
  const ball = page.locator('.glass-ball').first();
  await expect(ball).toBeVisible({ timeout: 10_000 });
  await page.waitForTimeout(2500);

  const before = (await ball.boundingBox())!;
  const cx = before.x + before.width / 2;
  const cy = before.y + before.height / 2;
  await page.mouse.move(cx, cy);
  await page.mouse.down();
  await page.mouse.move(cx - 160, cy - 90, { steps: 8 });
  await page.mouse.up();

  await page.waitForTimeout(1500); // throw + bounce + settle
  const after = (await ball.boundingBox())!;
  const moved = Math.hypot(after.x - before.x, after.y - before.y);
  expect(moved, 'drag/throw moved the bubble').toBeGreaterThan(40);

  // Poll until the physics actually comes to rest (wall bounces + damping)
  let stableFor = 0;
  let prev: { x: number; y: number } | null = null;
  const start = Date.now();
  while (Date.now() - start < 15_000 && stableFor < 4) {
    await page.waitForTimeout(300);
    const b = await ball.boundingBox();
    if (b && prev && Math.abs(b.x - prev.x) < 0.5 && Math.abs(b.y - prev.y) < 0.5) stableFor++;
    else stableFor = 0;
    if (b) prev = { x: b.x, y: b.y };
  }
  expect(stableFor, 'bubble settled').toBe(4);

  const clip = {
    x: Math.max(0, Math.floor(after.x) - 10),
    y: Math.max(0, Math.floor(after.y) - 10),
    width: Math.ceil(after.width) + 20,
    height: Math.ceil(after.height) + 20,
  };
  const settled1 = await page.screenshot({ clip });
  await page.waitForTimeout(900);
  const settled2 = await page.screenshot({ clip });
  expect(Buffer.compare(settled1, settled2), 'bubble came fully to rest').toBe(0);
});
