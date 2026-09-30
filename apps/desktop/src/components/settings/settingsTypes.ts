export type SettingsCategory =
  | 'general'
  | 'appearance'
  | 'notifications'
  | 'files'
  | 'relay'
  | 'advanced'
  | 'about';

/**
 * Frontend mirror of the Rust `ConduitSettings` struct
 * (`apps/desktop/src-tauri/src/commands/settings.rs`).
 *
 * ## Why this is not generated
 *
 * `packages/protocol/schema.json` (and therefore the generated
 * `src/types/websocket.ts`) describes the **wire protocol** — pairing,
 * notification, clipboard, file, audio, screen-mirror and relay *messages*. It
 * carries no settings definition, so there is nothing to derive this from.
 *
 * ## How drift is prevented
 *
 * This interface has drifted from the Rust struct before (`notification_apps`
 * and `relay_url` were missing here, which made `save_settings` fail outright
 * with `missing field notification_apps`). Two guards stop that recurring:
 *
 * 1. **Build-time (the real guard).** The Rust test
 *    `frontend_settings_type_key_set_matches_conduit_settings`
 *    (`commands/settings.rs`) parses *this file* and fails unless its key set is
 *    exactly the key set `ConduitSettings` serialises to. Rust wins, so a
 *    setting cannot be added on one side only without `cargo test` going red.
 * 2. **Compile-time.** `SETTINGS_KEYS` below is typed
 *    `Record<keyof SettingsData, true>`, so `tsc` rejects a missing *or* extra
 *    key without anyone having to remember the list.
 *
 * Naming is snake_case to match serde's field names verbatim — the payload of
 * `invoke('save_settings', { settings })` *is* this object.
 */
export interface SettingsData {
  device_name: string;
  max_devices: number;
  sync_notifications: boolean;
  sync_clipboard: boolean;
  sync_files: boolean;
  notification_apps: string[];
  theme: string;
  accent_color: string;
  last_version: string;
  minimize_to_tray: boolean;
  default_download_folder: string;
  auto_accept_files: boolean;
  notifications_enabled: boolean;
  relay_url: string;
  // relay_enabled / relay_port / relay_health_port / relay_hostname configure
  // the relay this app hosts in-process. On by default, because the relay is a
  // background part of this app rather than something to deploy; the ports
  // match the values in packages/protocol. relay_hostname is what the
  // self-signed certificate must be valid for, and is empty (meaning
  // "localhost") unless the relay is published under a real name.
  relay_enabled: boolean;
  relay_port: number;
  relay_health_port: number;
  relay_hostname: string;
}

export type SettingsKey = keyof SettingsData;

/**
 * Runtime list of every setting key.
 *
 * `satisfies Record<SettingsKey, true>` makes this exhaustive *at compile
 * time*: omitting a key, or listing one that does not exist on
 * {@link SettingsData}, is a `tsc` error.
 */
export const SETTINGS_KEYS = {
  device_name: true,
  max_devices: true,
  sync_notifications: true,
  sync_clipboard: true,
  sync_files: true,
  notification_apps: true,
  theme: true,
  accent_color: true,
  last_version: true,
  minimize_to_tray: true,
  default_download_folder: true,
  auto_accept_files: true,
  notifications_enabled: true,
  relay_url: true,
  relay_enabled: true,
  relay_port: true,
  relay_health_port: true,
  relay_hostname: true,
} as const satisfies Record<SettingsKey, true>;

/** `SETTINGS_KEYS` as an array, for runtime iteration and tests. */
export const SETTINGS_KEY_LIST = Object.keys(SETTINGS_KEYS) as SettingsKey[];

/**
 * Values a fresh install starts with, matching the Rust defaults
 * (`ConduitSettings::default()` / the `#[serde(default = …)]` attributes).
 *
 * Used as the pre-load state so a failed `get_settings` still renders a valid
 * object instead of `undefined` fields.
 */
export const DEFAULT_SETTINGS: SettingsData = {
  device_name: 'Desktop',
  max_devices: 5,
  sync_notifications: true,
  sync_clipboard: true,
  sync_files: true,
  notification_apps: ['WhatsApp', 'Telegram', 'Slack', 'Discord'],
  theme: 'dark',
  accent_color: '#00f0ff',
  last_version: '',
  minimize_to_tray: true,
  default_download_folder: '',
  auto_accept_files: true,
  notifications_enabled: true,
  relay_url: 'ws://127.0.0.1:9531',
  relay_enabled: true,
  relay_port: 9529,
  relay_health_port: 9530,
  relay_hostname: '',
};

export type UpdateSetting = <K extends SettingsKey>(
  key: K,
  value: SettingsData[K]
) => void;

export interface SectionProps {
  settings: SettingsData;
  updateSetting: UpdateSetting;
}
