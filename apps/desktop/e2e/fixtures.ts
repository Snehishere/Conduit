import { test as base, expect, type Page } from '@playwright/test';

export type { Page };

/**
 * Mock the Tauri IPC layer so the app boots in a plain Chromium browser.
 * __TAURI_INTERNALS__ is what @tauri-apps/api/core uses under the hood.
 *
 * Every command invoked during app boot must return a correctly-typed value —
 * returning `null` where the app expects an array/object crashes the React
 * error boundary (e.g. `notifications.filter(...)` on null).
 */
function mockTauriInternals(page: Page) {
  return page.addInitScript(() => {
    // Skip the onboarding overlay — it renders `fixed inset-0` and would
    // intercept every click in a fresh browser context.
    try {
      localStorage.setItem('conduit_onboarded', 'true');
    } catch {
      /* ignore — storage may be unavailable before origin is set */
    }

    // Minimal 1x1 PNG for generate_qr_code so <img> renders
    const PNG_1PX =
      'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';

    const defaultSettings = {
      device_name: 'Test Desktop', max_devices: 5, auto_connect: true,
      sync_notifications: true, sync_clipboard: true, sync_files: true,
      theme: 'dark', accent_color: '#34d399', last_version: '0.1.0',
      auto_start: true, minimize_to_tray: true, default_download_folder: '',
      auto_accept_files: true, notifications_enabled: true, notification_sound: true,
      notification_previews: true, crash_reports: false, analytics: false,
    };

    const defaults: Record<string, unknown> = {
      // Device / system info
      get_device_info: {
        device_id: 'test-desktop',
        name: 'Test Desktop',
        type: 'desktop',
        os: 'windows',
        public_key: 'mock-pub-key',
      },
      get_system_info: { ram_used_mb: 512, cpu_usage: 12, disk_free_gb: 100 },
      get_local_ip: '192.168.1.100',
      get_current_version: '0.1.0',

      // Settings — read from localStorage so save/reload round-trips work
      get_settings: (() => {
        try {
          const raw = localStorage.getItem('conduit_test_settings');
          if (raw) return JSON.parse(raw);
        } catch { /* fall through */ }
        return defaultSettings;
      })(),
      save_settings: null,
      save_settings_field: null,

      // Collections — MUST be arrays (hooks call .filter/.map/.length)
      get_devices: [],
      get_notifications: [],
      get_file_transfers: [],
      get_clipboard_history: [],

      // Pairing
      generate_pairing_token: 'abcd1234efgh',
      generate_qr_code: PNG_1PX,
      fix_firewall: null,

      // Updates
      check_for_update: { available: false },
      install_update: { success: true, version: '0.1.0', message: 'Mock update installed' },

      // File transfer actions
      send_file: {
        transfer_id: 'mock-transfer-1', name: 'file.bin', size: 1024,
        mime: 'application/octet-stream', total_chunks: 1,
      },
      resume_file_transfer: { transfer_id: 'mock-transfer-1', chunks_loaded: 0 },
    };

    (window as any).__TAURI_INTERNALS__ = {
      invoke: async (cmd: string, _args?: Record<string, unknown>) => {
        // Log every invoke for test assertions (window.__invokeLog)
        try {
          const log = ((window as any).__invokeLog ||= []);
          log.push({ cmd, args: _args });
        } catch { /* ignore */ }

        // Persist settings across page reloads within a test
        if (cmd === 'save_settings') {
          try {
            const s = (_args as any)?.settings;
            if (s) localStorage.setItem('conduit_test_settings', JSON.stringify(s));
          } catch { /* ignore */ }
          return null;
        }
        if (cmd === 'get_settings') {
          try {
            const raw = localStorage.getItem('conduit_test_settings');
            if (raw) return JSON.parse(raw);
          } catch { /* ignore */ }
          return defaults.get_settings;
        }

        if (cmd in defaults) return defaults[cmd];
        // Defensive fallback: unknown read commands resolve to an empty
        // collection instead of null so hooks can't crash on .filter().
        if (/^(get|list|query|fetch|load)_/.test(cmd)) return [];
        return null;
      },
      listen: async (_event: string, _handler: unknown) => {
        return () => {}; // unsubscribe no-op
      },
      // Used by @tauri-apps/api/core transformCallback (event listeners)
      transformCallback: (callback: (...a: unknown[]) => void, identifier?: number) => {
        const key = `_tauri_callback_${identifier ?? Math.floor(Math.random() * 1e9)}`;
        (window as any)[key] = callback;
        return key;
      },
      // Used by getCurrentWindow() / getCurrentWebview()
      metadata: {
        currentWindow: { label: 'main' },
        currentWebview: { label: 'webview' },
      },
    };
  });
}

/** Shared test fixture that injects Tauri mocks automatically */
export const test = base.extend<{ mockedPage: Page }>({
  mockedPage: async ({ page }, use) => {
    mockTauriInternals(page);
    await use(page);
  },
});

export { expect };
