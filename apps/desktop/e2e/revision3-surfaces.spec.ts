import { test, expect, type Page } from './fixtures';

// Screenshot output directory. Overridable with AUDIT_SHOTS; the default is
// repo-relative so this spec works on any machine and in CI. An absolute path
// here would only ever be valid on the machine that wrote it.
const SHOTS = process.env.AUDIT_SHOTS ?? 'test-results/revision3-shots';

async function setTheme(page: Page, theme: 'dark' | 'light') {
  await page.addInitScript((t) => { try { localStorage.setItem('conduit-theme', t); } catch { /* ignore */ } }, theme);
}

async function mockEnv(page: Page, theme: 'dark' | 'light') {
  await page.addInitScript((t) => {
    const internals = (window as any).__TAURI_INTERNALS__;
    if (!internals) return;
    const original = internals.invoke.bind(internals);
    internals.invoke = async (cmd: string, args?: Record<string, unknown>) => {
      const res = await original(cmd, args);
      if (cmd === 'get_devices') {
        return [{ id: 'dev-1', name: 'Pixel 9', device_type: 'phone', os: 'android', status: 'connected', last_seen: 0 }];
      }
      if (cmd === 'get_settings' && res && typeof res === 'object') return { ...(res as object), theme: t };
      return res;
    };
  }, theme);
}

function trackErrors(page: Page, sink: string[]) {
  page.on('console', (msg) => {
    if (msg.type() !== 'error') return;
    const text = msg.text();
    // Browser-only environment noise: no Conduit backend in e2e.
    if (text.includes('WebSocket')) return;
    sink.push('console: ' + text);
  });
  page.on('pageerror', (err) => sink.push('pageerror: ' + String(err)));
}

const VIEWS: Array<{ label: string; shot?: string; heading: RegExp }> = [
  { label: 'Home', shot: 'home', heading: /./ },
  { label: 'Inbox', shot: 'inbox', heading: /Inbox/ },
  { label: 'Calls', heading: /Calls|call/i },
  { label: 'Files', shot: 'files', heading: /Files/ },
  { label: 'Automation', heading: /Automation/ },
  { label: 'Settings', heading: /Settings/ },
];

for (const theme of ['dark', 'light'] as const) {
  test(`revision-3 audit (${theme})`, async ({ mockedPage: page }) => {
    const errors: string[] = [];
    trackErrors(page, errors);
    await setTheme(page, theme);
    await mockEnv(page, theme);
    await page.goto('/');
    const dock = page.locator('[aria-label="Main navigation"]');
    await expect(dock).toBeVisible({ timeout: 15_000 });

    // Single non-expanding dock: 8 nav + pair (+ optional surround)
    await expect(dock.locator('button[aria-label]')).toHaveCount(9);
    await expect(page.locator('[aria-label="Show additional actions"]')).toHaveCount(0);
    await expect(dock.locator('[aria-label="Pair device"]')).toBeVisible();

    // no baked-in glass shine layers anywhere
    await expect(page.locator('.glass-ball__sheen, .glass-ball__spec, .glass-ball__spec2, .glass-ball__caustic, .glass-ball__fresnel')).toHaveCount(0);

    for (const view of VIEWS) {
      await dock.locator(`[aria-label="${view.label}"]`).click();
      await page.waitForTimeout(700);

      // Content must never run under the fixed dock
      const main = await page.locator('main').boundingBox();
      const dockBox = await dock.boundingBox();
      expect(main, 'main box').toBeTruthy();
      expect(dockBox, 'dock box').toBeTruthy();
      expect(main!.y + main!.height, `main bottom vs dock top on ${view.label}`).toBeLessThanOrEqual(dockBox!.y + 0.5);

      if (view.shot) await page.screenshot({ path: `${SHOTS}/${theme}-${view.shot}.png` });
    }

    // Unified inbox: one screen, no tabs/tabpanels (row merging is covered by
    // the UnifiedSurfaces unit test — fixtures ship zero notifications/threads)
    await dock.locator('[aria-label="Inbox"]').click();
    await page.waitForTimeout(600);
    await expect(page.locator('main h1', { hasText: 'Inbox' })).toBeVisible();
    await expect(page.locator('main [role="tab"], main [role="tablist"], main [role="tabpanel"]')).toHaveCount(0);

    // Unified files: masonry container exists (one flow, columns layout)
    await dock.locator('[aria-label="Files"]').click();
    await page.waitForTimeout(600);
    await expect(page.locator('main .columns-2, main [class*="columns-"]')).toHaveCount(1);
    await expect(page.locator('main [role="tab"]')).toHaveCount(0);
    await expect(page.locator('main [role="tabpanel"]')).toHaveCount(0);

    // theme attribute applied
    expect(await page.evaluate(() => document.documentElement.getAttribute('data-theme'))).toBe(theme);

    expect(errors, 'no console/page errors').toEqual([]);
  });
}
