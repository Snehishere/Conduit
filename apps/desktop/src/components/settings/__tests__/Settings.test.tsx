import { render, screen, waitFor, fireEvent } from '@testing-library/react';
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { invoke } from '@tauri-apps/api/core';
import { ThemeProvider } from '../../../lib/theme';
import Settings from '../Settings';
import {
  DEFAULT_SETTINGS,
  SETTINGS_KEY_LIST,
  SETTINGS_KEYS,
  type SettingsData,
} from '../settingsTypes';

// Mock Tauri API. The payload mirrors what the Rust `ConduitSettings` struct
// serialises to — every field, in snake_case. Declared through `vi.hoisted` so
// individual tests can swap in a failing implementation and restore it.
const { baseInvoke } = vi.hoisted(() => {
  const baseInvoke = (cmd: string): Promise<unknown> => {
    if (cmd === 'get_settings') {
      return Promise.resolve({
        device_name: 'Desktop',
        max_devices: 5,
        sync_notifications: true,
        sync_clipboard: true,
        sync_files: true,
        notification_apps: ['WhatsApp', 'Telegram', 'Slack', 'Discord'],
        theme: 'dark',
        accent_color: '#34d399',
        last_version: '0.1.0',
        minimize_to_tray: true,
        default_download_folder: '',
        auto_accept_files: true,
        notifications_enabled: true,
        relay_url: 'ws://127.0.0.1:9531',
        relay_enabled: true,
        relay_port: 9529,
        relay_health_port: 9530,
        relay_hostname: '',
      });
    }
    if (cmd === 'get_relay_status') {
      return Promise.resolve({
        running: true,
        error: null,
        port: 9529,
        local_port: 9531,
        tls_pin: 'sha256/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=',
        active_connections: 1,
        registered_devices: 2,
      });
    }
    if (cmd === 'get_current_version') {
      return Promise.resolve('0.1.0');
    }
    return Promise.resolve({});
  };
  return { baseInvoke };
});

vi.mock('@tauri-apps/api/core', () => ({
  invoke: vi.fn(baseInvoke),
}));

const invokeMock = vi.mocked(invoke);

const renderSettings = () =>
  render(
    <ThemeProvider>
      <Settings />
    </ThemeProvider>
  );

const openCategory = async (label: string) => {
  await waitFor(() => {
    expect(screen.getByText('Settings')).toBeInTheDocument();
  });
  fireEvent.click(screen.getAllByText(label)[0]);
};

describe('Settings', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    invokeMock.mockImplementation(baseInvoke);
  });

  afterEach(() => {
    invokeMock.mockImplementation(baseInvoke);
  });

  it('renders loading state initially', () => {
    renderSettings();
    expect(screen.getByText('Loading settings...')).toBeInTheDocument();
  });

  it('renders settings after loading', async () => {
    renderSettings();
    await waitFor(() => {
      expect(screen.getByText('Settings')).toBeInTheDocument();
    });
    // Sidebar buttons + content headings share names, use getAllByText
    expect(screen.getAllByText('General').length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('Appearance').length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('Notifications').length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('File Transfers').length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('Other Networks').length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('Advanced').length).toBeGreaterThanOrEqual(1);
    expect(screen.getAllByText('About').length).toBeGreaterThanOrEqual(1);
  });

  it('shows General category by default', async () => {
    renderSettings();
    await waitFor(() => {
      expect(screen.getByText('Device name')).toBeInTheDocument();
    });
    expect(screen.getByText('Sync clipboard')).toBeInTheDocument();
  });

  /**
   * `auto_connect` and `auto_start` had no implementation anywhere in the
   * backend (the desktop hub never dials out, and there is no
   * `tauri-plugin-autostart` dependency), so shipping working toggles for them
   * was a lie. They are removed from `ConduitSettings`, from `SettingsData` and
   * from the UI; this pins their absence so they cannot creep back.
   */
  it('no longer offers settings that have no implementation', async () => {
    renderSettings();
    await waitFor(() => {
      expect(screen.getByText('Device name')).toBeInTheDocument();
    });
    expect(screen.queryByText('Auto-connect on startup')).not.toBeInTheDocument();
    expect(screen.queryByText('Auto-start on system boot')).not.toBeInTheDocument();

    // The Privacy category only ever held crash_reports/analytics, which had no
    // telemetry subsystem behind them at all.
    expect(screen.queryByText('Privacy')).not.toBeInTheDocument();
    for (const removed of [
      'Crash reports',
      'Usage analytics',
      'Notification sounds',
      'Show previews',
    ]) {
      expect(screen.queryByText(removed)).not.toBeInTheDocument();
    }
  });

  // The handler clears `conduit_onboarded` and reloads, so it replays the
  // onboarding dialog. It was labelled "Show tutorial again", which pointed at
  // InteractiveTutorial — a component nothing can currently reach.
  it('shows the replay-onboarding button', async () => {
    renderSettings();
    await waitFor(() => {
      expect(screen.getByText('Show onboarding again')).toBeInTheDocument();
    });
  });

  it('can switch to Appearance category', async () => {
    renderSettings();
    await openCategory('Appearance');
    await waitFor(() => {
      expect(screen.getByText('Theme')).toBeInTheDocument();
    });
    expect(screen.getByText('Accent color')).toBeInTheDocument();
  });

  /**
   * REGRESSION: `SettingsData` was missing `notification_apps`, so
   * `save_settings` always failed server-side with `missing field
   * notification_apps` and the Save button silently did nothing. The payload
   * must now carry every key of the type.
   */
  it('saves the full settings payload, including the fields it used to drop', async () => {
    renderSettings();
    await waitFor(() => {
      expect(screen.getByText('Save settings')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByText('Save settings'));

    await waitFor(() => {
      const saveCall = invokeMock.mock.calls.find(([cmd]) => cmd === 'save_settings');
      expect(saveCall).toBeDefined();
    });

    const saveCall = invokeMock.mock.calls.find(([cmd]) => cmd === 'save_settings')!;
    const payload = (saveCall[1] as { settings: SettingsData }).settings;

    expect(Object.keys(payload).sort()).toEqual([...SETTINGS_KEY_LIST].sort());
    expect(payload.notification_apps).toEqual([
      'WhatsApp',
      'Telegram',
      'Slack',
      'Discord',
    ]);
    expect(payload.relay_url).toBe('ws://127.0.0.1:9531');
    await waitFor(() => {
      expect(screen.getByText('Saved')).toBeInTheDocument();
    });
  });

  /**
   * REGRESSION: a rejected `save_settings` used to be `console.error`-ed only —
   * the user got no feedback at all. It must now be visible and announced.
   */
  it('surfaces a failed save instead of failing silently', async () => {
    invokeMock.mockImplementation((cmd: string): Promise<unknown> => {
      if (cmd === 'get_settings') {
        return Promise.resolve({ ...DEFAULT_SETTINGS });
      }
      if (cmd === 'get_current_version') {
        return Promise.resolve('0.1.0');
      }
      return Promise.reject('missing field notification_apps');
    });

    renderSettings();
    await waitFor(() => {
      expect(screen.getByText('Save settings')).toBeInTheDocument();
    });

    fireEvent.click(screen.getByText('Save settings'));

    await waitFor(() => {
      expect(screen.getByRole('alert')).toHaveTextContent(
        'missing field notification_apps'
      );
    });
    expect(screen.queryByText('Saved')).not.toBeInTheDocument();
  });

  // ── notification_apps editor ──────────────────────────────────────────────

  it('edits the notification_apps allowlist', async () => {
    renderSettings();
    await openCategory('Notifications');

    expect(screen.getByText('Slack')).toBeInTheDocument();
    expect(screen.getByText('WhatsApp')).toBeInTheDocument();

    const input = screen.getByPlaceholderText('Add an app name');

    // Add
    fireEvent.change(input, { target: { value: 'Matrix' } });
    fireEvent.click(screen.getByRole('button', { name: 'Add app' }));
    expect(screen.getByText('Matrix')).toBeInTheDocument();

    // Remove
    fireEvent.click(screen.getByRole('button', { name: 'Remove Matrix' }));
    expect(screen.queryByText('Matrix')).not.toBeInTheDocument();
  });

  it('warns that an empty allowlist mirrors nothing', async () => {
    renderSettings();
    await openCategory('Notifications');

    for (const app of ['WhatsApp', 'Telegram', 'Slack', 'Discord']) {
      fireEvent.click(screen.getByRole('button', { name: `Remove ${app}` }));
    }

    expect(
      screen.getByText('No apps listed — notifications will not be mirrored.')
    ).toBeInTheDocument();
  });

  it('shows the persisted relay endpoint in Advanced', async () => {
    renderSettings();
    await openCategory('Advanced');
    expect(screen.getByText('Relay endpoint')).toBeInTheDocument();
    expect(screen.getByText('ws://127.0.0.1:9531')).toBeInTheDocument();
  });

  /**
   * The "Other Networks" panel. The point of these is that a user can tell
   * whether the feature is working *without* reading a log file — a relay that
   * silently failed to bind is the failure mode this whole feature is most
   * likely to hit on a real machine.
   */
  describe('relay panel', () => {
    const openRelay = async () => {
      renderSettings();
      await waitFor(() => {
        expect(screen.getByText('Settings')).toBeInTheDocument();
      });
      fireEvent.click(screen.getAllByText('Other Networks')[0]);
    };

    it('shows the live status rather than claiming success on faith', async () => {
      await openRelay();
      await waitFor(() => {
        expect(screen.getByText('Running')).toBeInTheDocument();
      });
      expect(screen.getByText(/1 device connected/)).toBeInTheDocument();
      expect(screen.getByText(/Listening on port 9529/)).toBeInTheDocument();
    });

    it('says why it is not running instead of showing a green dot', async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'get_relay_status') {
          return Promise.resolve({
            running: false,
            error: 'port 9529 could not be bound: address already in use',
            port: null,
            local_port: null,
            tls_pin: null,
            active_connections: 0,
            registered_devices: 0,
          });
        }
        return baseInvoke(cmd);
      });

      await openRelay();
      await waitFor(() => {
        expect(screen.getByText(/Not running/)).toBeInTheDocument();
      });
      expect(screen.getByText(/address already in use/)).toBeInTheDocument();
      expect(screen.queryByText('Running')).not.toBeInTheDocument();
    });

    it('tells the user the port still has to be opened on their router', async () => {
      // The app hosts the relay but cannot reach the router. Without this the
      // panel would imply a phone on mobile data just works.
      await openRelay();
      await waitFor(() => {
        expect(screen.getByText(/open in your router/)).toBeInTheDocument();
      });
    });

    it('explains that there is nothing else to install', async () => {
      await openRelay();
      await waitFor(() => {
        expect(screen.getByText(/nothing else to install/)).toBeInTheDocument();
      });
    });

    it('survives a failing status read instead of rendering a lie', async () => {
      invokeMock.mockImplementation((cmd: string) => {
        if (cmd === 'get_relay_status') return Promise.reject(new Error('no such command'));
        return baseInvoke(cmd);
      });

      await openRelay();
      await waitFor(() => {
        expect(screen.getByText('Checking…')).toBeInTheDocument();
      });
      expect(screen.queryByText('Running')).not.toBeInTheDocument();
    });

    it('lets the relay be turned off, which is the point of the default', async () => {
      await openRelay();
      await waitFor(() => {
        expect(screen.getByText('Running')).toBeInTheDocument();
      });
      fireEvent.click(screen.getByRole('switch'));
      // With the relay off there is no status, port or router note to show —
      // showing them would imply it is still doing something.
      await waitFor(() => {
        expect(screen.queryByText('Running')).not.toBeInTheDocument();
      });
      expect(screen.queryByText(/Listening on port/)).not.toBeInTheDocument();
    });
  });
});

/**
 * Frontend half of the cross-language conformance check. The authoritative one
 * is the Rust test `frontend_settings_type_key_set_matches_conduit_settings`,
 * which parses this file's `SettingsData` and compares it with the keys
 * `ConduitSettings` serialises to. These assertions cover what Rust cannot see:
 * that the runtime key list and the default object stay in step with the type.
 */
describe('settingsTypes conformance', () => {
  it('SETTINGS_KEYS covers exactly the SettingsData keys', () => {
    expect(Object.keys(SETTINGS_KEYS).sort()).toEqual([...SETTINGS_KEY_LIST].sort());
  });

  it('DEFAULT_SETTINGS has a value for every setting and nothing else', () => {
    expect(Object.keys(DEFAULT_SETTINGS).sort()).toEqual([...SETTINGS_KEY_LIST].sort());
  });

  it('DEFAULT_SETTINGS mirrors the Rust defaults', () => {
    // `ConduitSettings::default()` in commands/settings.rs — the values the
    // `#[serde(default = ...)]` attributes produce, pinned by
    // `settings_default_matches_serde_defaults`.
    expect(DEFAULT_SETTINGS.theme).toBe('dark');
    expect(DEFAULT_SETTINGS.accent_color).toBe('#00f0ff');
    expect(DEFAULT_SETTINGS.max_devices).toBe(5);
    expect(DEFAULT_SETTINGS.relay_url).toBe('ws://127.0.0.1:9531');
    expect(DEFAULT_SETTINGS.notification_apps).toEqual([
      'WhatsApp',
      'Telegram',
      'Slack',
      'Discord',
    ]);
    expect(DEFAULT_SETTINGS.minimize_to_tray).toBe(true);
    expect(DEFAULT_SETTINGS.auto_accept_files).toBe(true);
    expect(DEFAULT_SETTINGS.notifications_enabled).toBe(true);
    // The relay is a background part of the app, not something to deploy, so a
    // fresh install has it running. `the_relay_is_on_by_default` pins the Rust
    // side; this keeps the two defaults from drifting apart.
    expect(DEFAULT_SETTINGS.relay_enabled).toBe(true);
  });
});
