use crate::AppState;
use crate::error::{ConduitError, Result};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;

type ManagedState = Arc<AppState>;

/// Default value for `max_devices` (see [`default_max_devices`]).
pub const DEFAULT_MAX_DEVICES: u32 = 5;
/// Default value for the legacy `relay_url` setting.
///
/// The desktop does **not** dial this: it hosts the relay in this process and
/// joins it over loopback, which is [`crate::relay::local_relay_url`]. The field
/// is kept because it is persisted in the settings table and the phone's relay
/// address is a real, user-entered thing — but its default now names the port
/// this app actually binds, rather than the 9528 the old standalone service used
/// and nothing listens on any more.
pub const DEFAULT_RELAY_URL: &str = "ws://127.0.0.1:9531";

/// Every field carries a `#[serde(default …)]`.
///
/// This is deliberate. `save_settings` takes the whole struct as one argument, so
/// a *required* field turns any drift between this struct and the frontend
/// `SettingsData` type into a hard `missing field …` deserialisation error that
/// fails the **entire** save — the user clicks "Save" and nothing happens. That
/// is the exact failure this type already suffered from (`notification_apps`
/// was required while the frontend never sent it). With defaults on every
/// field, an unknown-to-the-caller field is simply left at its documented
/// default instead of breaking the whole save.
///
/// The two places that define "the default" — the `#[serde(default = …)]`
/// attributes and the hand-written [`Default`] impl below — call the very same
/// functions, and `settings_default_matches_serde_defaults` pins them together
/// for every single field.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct ConduitSettings {
    #[serde(default)]
    pub device_name: String,
    #[serde(default = "default_max_devices")]
    pub max_devices: u32,
    #[serde(default = "default_true")]
    pub sync_notifications: bool,
    #[serde(default = "default_true")]
    pub sync_clipboard: bool,
    #[serde(default = "default_true")]
    pub sync_files: bool,
    #[serde(default = "default_notification_apps")]
    pub notification_apps: Vec<String>,
    #[serde(default = "default_theme")]
    pub theme: String,
    #[serde(default = "default_accent_color")]
    pub accent_color: String,
    #[serde(default)]
    pub last_version: String,
    #[serde(default = "default_true")]
    pub minimize_to_tray: bool,
    #[serde(default)]
    pub default_download_folder: String,
    #[serde(default = "default_true")]
    pub auto_accept_files: bool,
    #[serde(default = "default_true")]
    pub notifications_enabled: bool,
    #[serde(default = "default_relay_url")]
    pub relay_url: String,
    /// Whether the desktop hosts a relay so a phone on a different network can
    /// reach it.
    ///
    /// On by default. The relay is a background part of this app, not something
    /// to deploy, so the intent is that it is already working by the time a
    /// phone needs it. The costs of that choice are bounded: the listener is
    /// useless without the bearer token *and* a registered device's route key,
    /// and it routes only opaque envelopes (ADR-0007). Turn it off to keep the
    /// desktop strictly LAN-only.
    #[serde(default = "default_true")]
    pub relay_enabled: bool,
    /// Port the in-process relay serves TLS on.
    #[serde(default = "default_relay_port")]
    pub relay_port: u16,
    /// Port the in-process relay serves `/healthz` and `/metrics` on.
    ///
    /// Loopback only, always. See the audit's W6.20.
    #[serde(default = "default_relay_health_port")]
    pub relay_health_port: u16,
    /// The hostname clients use to reach this machine.
    ///
    /// A self-signed certificate is only valid for the names in it, so a relay
    /// reachable at `relay.example.com` and one generated for `localhost` are
    /// not the same relay. Leaving this empty means "localhost", which is right
    /// for a tunnel that terminates elsewhere and wrong for a published port.
    #[serde(default)]
    pub relay_hostname: String,
    /// The SPKI pin the relay's served certificate must present, as
    /// `sha256/<base64>`.
    ///
    /// Empty — the default — means "no pin configured", and the relay starts
    /// normally: it is hosting the connection, so it is not authenticating a
    /// remote server and has nothing to verify. This setting exists for the case
    /// that *does* need it, which is a desktop connecting to a relay it does not
    /// host (an operator's own, behind a tunnel).
    ///
    /// When set, the pin is compared against the certificate the relay actually
    /// loaded and a mismatch stops the relay from starting, rather than
    /// warning. That is the point: a certificate change is either an intended
    /// renewal — in which case the operator updates this — or it is not, and the
    /// only way to tell the two apart is to fail.
    #[serde(default)]
    pub relay_cert_pin: String,
}

fn default_theme() -> String {
    "dark".to_string()
}
fn default_accent_color() -> String {
    "#00f0ff".to_string()
}
fn default_true() -> bool {
    true
}

/// Port the in-process relay serves TLS on by default.
///
/// Matches the relay crate's own default so a published port-forward rule
/// survives the move from a self-hosted relay to the in-process one.
fn default_relay_port() -> u16 {
    crate::relay::DEFAULT_RELAY_PORT
}

/// Port the in-process relay serves `/healthz` and `/metrics` on by default.
fn default_relay_health_port() -> u16 {
    crate::relay::DEFAULT_RELAY_HEALTH_PORT
}
fn default_max_devices() -> u32 {
    DEFAULT_MAX_DEVICES
}
fn default_relay_url() -> String {
    DEFAULT_RELAY_URL.to_string()
}

impl ConduitSettings {
    /// A copy with the relay turned on or off.
    ///
    /// Used at startup, where a settings read that failed has to produce a
    /// "definitely off" configuration: a desktop that cannot read its settings
    /// should not open a listener the user may have turned off.
    pub fn with_relay_enabled(mut self, enabled: bool) -> Self {
        self.relay_enabled = enabled;
        self
    }
}

/// Apps whose notifications are mirrored to other devices by default.
///
/// The desktop has no way to enumerate the apps installed on a *phone*, so this
/// is a curated seed list the user edits in Settings → Notifications. It is the
/// single source of truth for the `notification_apps` default: the
/// `#[serde(default = …)]` attribute, the [`Default`] impl and
/// `Storage::get_settings`' empty-database default all call it.
pub fn default_notification_apps() -> Vec<String> {
    vec![
        "WhatsApp".to_string(),
        "Telegram".to_string(),
        "Slack".to_string(),
        "Discord".to_string(),
    ]
}

/// `ConduitSettings` deliberately does **not** derive `Default`: `#[derive(Default)]`
/// would produce `theme: ""`, `minimize_to_tray: false`, … which contradicts every
/// `#[serde(default = "...")]` attribute above. This hand-written impl reuses those
/// very same functions so the two default definitions cannot drift, and
/// `settings_default_matches_serde_defaults` pins the result.
///
/// Test literals that only need *some* fields should end with
/// `..Default::default()`; the ones that are meant to pin every value spell
/// every field out so a new setting is a compile error rather than a silent
/// omission.
impl Default for ConduitSettings {
    fn default() -> Self {
        Self {
            device_name: String::new(),
            max_devices: default_max_devices(),
            sync_notifications: default_true(),
            sync_clipboard: default_true(),
            sync_files: default_true(),
            notification_apps: default_notification_apps(),
            theme: default_theme(),
            accent_color: default_accent_color(),
            last_version: String::new(),
            minimize_to_tray: default_true(),
            default_download_folder: String::new(),
            auto_accept_files: default_true(),
            notifications_enabled: default_true(),
            relay_url: default_relay_url(),
            relay_enabled: default_true(),
            relay_port: default_relay_port(),
            relay_health_port: default_relay_health_port(),
            relay_hostname: String::new(),
            relay_cert_pin: String::new(),
        }
    }
}

#[tauri::command]
pub async fn get_settings(state: State<'_, ManagedState>) -> Result<ConduitSettings> {
    let storage = state.storage.clone();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(storage.get_settings())
    })
    .await
    .map_err(|e| ConduitError::Other(format!("get_settings task join error: {e}")))?
}

/// What the relay status UI renders.
///
/// A flattened view of [`crate::relay::RelayStatus`], so the frontend does not
/// have to mirror the nested `Option<Status>` and get the nullability wrong.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RelayStatusView {
    /// Whether the relay is accepting connections.
    pub running: bool,
    /// Why it is not, in words. A misconfiguration is a normal state here.
    pub error: Option<String>,
    /// The port the TLS listener bound, if it is up.
    pub port: Option<u16>,
    /// The port the loopback-only listener bound, if it is up.
    pub local_port: Option<u16>,
    /// `sha256/<base64>` SPKI pin, if TLS is up. The phone needs this.
    pub tls_pin: Option<String>,
    /// Live counters, when it is running.
    pub active_connections: usize,
    pub registered_devices: usize,
}

/// The relay's current state, for the settings UI.
///
/// Separate from `get_settings` on purpose: it changes on its own as devices
/// connect and disconnect, so the UI polls it rather than expecting it to be
/// part of a settings save.
///
/// Returns `Result` because that is what an `async` command holding a `State`
/// reference must; there is nothing to fail here, so the error is never used.
#[tauri::command]
pub async fn get_relay_status(state: State<'_, ManagedState>) -> Result<RelayStatusView> {
    let snapshot = state.relay_host.snapshot().await;
    let status = snapshot.status;
    Ok(RelayStatusView {
        running: snapshot.running,
        error: snapshot.error,
        port: status.as_ref().and_then(|s| s.wss_port),
        local_port: status.as_ref().and_then(|s| s.ws_port),
        tls_pin: status.as_ref().and_then(|s| s.tls_pin.clone()),
        active_connections: status.as_ref().map(|s| s.active_connections).unwrap_or(0),
        registered_devices: status.as_ref().map(|s| s.registered_devices).unwrap_or(0),
    })
}

#[tauri::command]
pub async fn save_settings(
    state: State<'_, ManagedState>,
    settings: ConduitSettings,
) -> Result<()> {
    let storage = state.storage.clone();
    let minimize_to_tray = settings.minimize_to_tray;
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(storage.save_settings(&settings))
    })
    .await
    .map_err(|e| ConduitError::Other(format!("save_settings task join error: {e}")))??;

    // The tray reads this synchronously from the window-close event, so push the
    // new value into its cache instead of waiting for a restart to pick it up.
    crate::tray::set_minimize_to_tray(minimize_to_tray);
    Ok(())
}

#[tauri::command]
pub async fn save_settings_field(
    state: State<'_, ManagedState>,
    key: String,
    value: String,
) -> Result<()> {
    let storage = state.storage.clone();
    let key_for_task = key.clone();
    let value_for_task = value.clone();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current()
            .block_on(storage.save_setting(&key_for_task, &value_for_task))
    })
    .await
    .map_err(|e| ConduitError::Other(format!("save_settings_field task join error: {e}")))??;

    // Same live-update contract as `save_settings` for the one setting the tray
    // consults from a synchronous event handler.
    if key == "minimize_to_tray" {
        crate::tray::set_minimize_to_tray(value == "true");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// allowed_commands — the shell-command allowlist
// ---------------------------------------------------------------------------

/// Settings-table key holding the allowlist as a JSON array of strings.
///
/// It is deliberately **not** a `ConduitSettings` field. `ConduitSettings` is
/// validated against `settingsTypes.ts`'s `SettingsData` by
/// `frontend_settings_type_key_set_matches_conduit_settings`, and that file is
/// owned by another agent; adding a field here without the matching TS field
/// would turn that test red. Once `settingsTypes.ts` can be edited, the natural
/// follow-up is to promote this into `ConduitSettings` in one pass.
pub const ALLOWED_COMMANDS_KEY: &str = "allowed_commands";

/// Read the shell-command allowlist.
///
/// Returns the persisted list, or an empty list — which, per
/// `CommandAllowlist`'s documented contract, blocks **all** shell execution.
/// A missing row, an unparsable row, and an explicit `[]` all mean the same
/// thing, and all of them fail closed.
#[tauri::command]
pub async fn get_allowed_commands(state: State<'_, ManagedState>) -> Result<Vec<String>> {
    let storage = state.storage.clone();
    let raw = tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(storage.get_setting(ALLOWED_COMMANDS_KEY))
    })
    .await
    .map_err(|e| ConduitError::Other(format!("get_allowed_commands task join error: {e}")))?;
    Ok(crate::security::parse_allowed_commands_setting(
        raw.as_deref(),
    ))
}

/// Replace the shell-command allowlist and push it into the live one.
///
/// Writing the list and replacing the process-wide `CommandAllowlist` happen
/// together, so a rule that becomes allowed is allowed *now* and a rule that
/// stops being allowed stops *now* — no restart, and no window in which the
/// persisted setting and the enforced list disagree.
#[tauri::command]
pub async fn set_allowed_commands(
    state: State<'_, ManagedState>,
    commands: Vec<String>,
) -> Result<Vec<String>> {
    if commands.len() > crate::security::MAX_ALLOWED_COMMANDS {
        return Err(ConduitError::Other(format!(
            "Too many allowed commands: {} (max {})",
            commands.len(),
            crate::security::MAX_ALLOWED_COMMANDS
        )));
    }
    let sanitized = crate::security::sanitize_allowed_commands(commands);
    let json = serde_json::to_string(&sanitized)
        .map_err(|e| ConduitError::Other(format!("could not encode allowed_commands: {e}")))?;
    let storage = state.storage.clone();
    let key = ALLOWED_COMMANDS_KEY.to_string();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(storage.save_setting(&key, &json))
    })
    .await
    .map_err(|e| ConduitError::Other(format!("set_allowed_commands task join error: {e}")))??;
    crate::security::set_command_allowlist(sanitized.clone());
    Ok(sanitized)
}

/// The per-launch local WebSocket capability token.
///
/// Handed only to the in-process Tauri webview, over Tauri IPC — which a remote
/// peer cannot reach. The webview appends it to the socket URL (or offers it as
/// a `Sec-WebSocket-Protocol`) so the server can register it as `local_desktop`
/// instead of trusting every loopback peer. See the report for the two-line
/// `useWebSocket.tsx` change that consumes it.
#[tauri::command]
pub fn get_local_ws_token() -> Result<String> {
    Ok(crate::security::local_capability().token().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::{create_app_with_state, create_test_state};
    use tauri::Manager;

    fn sample_settings() -> ConduitSettings {
        ConduitSettings {
            device_name: "QA-Box".into(),
            max_devices: 7,
            sync_notifications: false,
            sync_clipboard: true,
            sync_files: false,
            notification_apps: vec!["Signal".into()],
            theme: "light".into(),
            accent_color: "#ff0000".into(),
            last_version: "1.2.3".into(),
            minimize_to_tray: false,
            default_download_folder: "D:\\downloads".into(),
            auto_accept_files: false,
            notifications_enabled: false,
            relay_url: "wss://relay.example:9528".into(),
            relay_enabled: false,
            relay_port: crate::relay::DEFAULT_RELAY_PORT,
            relay_health_port: crate::relay::DEFAULT_RELAY_HEALTH_PORT,
            relay_hostname: String::new(),
            relay_cert_pin: String::new(),
        }
    }

    // ── Default / serde-default parity ────────────────────────────────────────

    /// Pins `ConduitSettings::default()` to the values serde produces for fields
    /// carrying `#[serde(default = "...")]`.
    ///
    /// The JSON below lists *every* field, so this assertion holds regardless of
    /// which fields currently carry a `#[serde(default = "...")]` attribute. Fields
    /// that do carry one are given their documented default value; fields that are
    /// still required on deserialisation are given the same value `Default`
    /// documents. Adding a field to `ConduitSettings` fails
    /// `conduit_settings_key_set_is_pinned` (below) and changing any field's
    /// default fails here — which forces a decision instead of letting the two
    /// definitions drift apart.
    #[test]
    fn settings_default_matches_serde_defaults() {
        let from_serde: ConduitSettings = serde_json::from_value(serde_json::json!({
            "device_name": "",
            "max_devices": 5,
            "sync_notifications": true,
            "sync_clipboard": true,
            "sync_files": true,
            "notification_apps": ["WhatsApp", "Telegram", "Slack", "Discord"],
            "theme": "dark",
            "accent_color": "#00f0ff",
            "last_version": "",
            "minimize_to_tray": true,
            "default_download_folder": "",
            "auto_accept_files": true,
            "notifications_enabled": true,
            "relay_url": "ws://127.0.0.1:9531"
        }))
        .expect("all fields present; deserialisation must succeed");

        let from_default = ConduitSettings::default();

        assert_eq!(
            from_default, from_serde,
            "Default and serde defaults diverged"
        );
    }

    /// The exact JSON key set of `ConduitSettings`, pinned.
    ///
    /// This is the single list the rest of the suite reasons about:
    /// `Storage::save_settings` must write every key here (see
    /// `test_save_settings_writes_every_settings_key`), and the frontend
    /// `SettingsData` type must declare exactly the same names (see
    /// `frontend_settings_type_key_set_matches_conduit_settings`). Because every
    /// field is `#[serde(default)]`, adding or removing a field can no longer be
    /// caught by a failing deserialisation — this test is what catches it.
    const SETTINGS_KEYS: &[&str] = &[
        "device_name",
        "max_devices",
        "sync_notifications",
        "sync_clipboard",
        "sync_files",
        "notification_apps",
        "theme",
        "accent_color",
        "last_version",
        "minimize_to_tray",
        "default_download_folder",
        "auto_accept_files",
        "notifications_enabled",
        "relay_url",
        "relay_enabled",
        "relay_port",
        "relay_health_port",
        "relay_hostname",
        "relay_cert_pin",
    ];

    /// Serialised key set of `ConduitSettings`, sorted, as owned strings.
    fn serialized_setting_keys() -> Vec<String> {
        let value =
            serde_json::to_value(ConduitSettings::default()).expect("ConduitSettings serialises");
        let obj = value.as_object().expect("ConduitSettings is a struct");
        let mut keys: Vec<String> = obj.keys().cloned().collect();
        keys.sort();
        keys
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn the_relay_is_on_by_default() {
        // The relay is a background part of this app, not something the user
        // deploys. A phone on another network must find it already running, so
        // the default cannot be off. This is also the one setting where
        // "sensible-looking" and "what was asked for" disagree, so it is pinned.
        assert!(
            ConduitSettings::default().relay_enabled,
            "the relay must default to running"
        );

        // The path that actually matters. `main.rs` does not consult
        // `ConduitSettings::default()` — it reads the settings *table*, and
        // returns before `relay_host.start()` and `spawn_relay_client` when the
        // result says false. `Storage::get_settings` had its own literal default
        // for this key ("false"), so the two definitions of "the default"
        // disagreed and a fresh install never started a relay. Asserting only the
        // struct default above let that through; only this assertion fails.
        let state = create_test_state();
        let from_empty_db = state.storage.get_settings().await.unwrap();
        assert!(
            from_empty_db.relay_enabled,
            "an empty database must read as relay_enabled = true — this is the \
             value main.rs consults at startup"
        );

        // And an explicit "off" must still be honoured, so the default above is
        // not just the absence of a check.
        state
            .storage
            .save_setting("relay_enabled", "false")
            .await
            .unwrap();
        assert!(
            !state.storage.get_settings().await.unwrap().relay_enabled,
            "a stored \"false\" must still turn the relay off"
        );

        // And the frontend default must agree, or a fresh install would save
        // the opposite value the first time the user touches any other setting:
        // Settings.tsx replaces DEFAULT_SETTINGS wholesale with the loaded
        // result and then saves that object.
        let json = serde_json::json!({ "relay_port": 9529 });
        let parsed: ConduitSettings =
            serde_json::from_value(json).expect("a partial settings payload still deserialises");
        assert!(parsed.relay_enabled);
        assert!(
            frontend_default_settings_relay_enabled(),
            "DEFAULT_SETTINGS in settingsTypes.ts must be relay_enabled: true"
        );
    }

    /// Reads `relay_enabled` out of the `DEFAULT_SETTINGS` literal in
    /// `settingsTypes.ts`.
    ///
    /// The hazard it guards is not hypothetical: the Rust side said "on" in two
    /// places and "off" in `Storage::get_settings`, and the frontend default is a
    /// fourth definition that nothing pinned. A one-line scan is enough — the
    /// alternative is hand-transcribing a TS object into a Rust literal, which is
    /// the same drift it would be checking for.
    fn frontend_default_settings_relay_enabled() -> bool {
        let ts_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../src/components/settings/settingsTypes.ts"
        );
        let source = std::fs::read_to_string(ts_path)
            .unwrap_or_else(|e| panic!("cannot read {ts_path}: {e}"));
        let start = source
            .find("export const DEFAULT_SETTINGS")
            .expect("settingsTypes.ts must declare `export const DEFAULT_SETTINGS`");
        source[start..]
            .lines()
            .map(str::trim)
            .find_map(|line| {
                let value = line.strip_prefix("relay_enabled:")?.trim();
                Some(value.trim_end_matches(',') == "true")
            })
            .expect("DEFAULT_SETTINGS must declare `relay_enabled`")
    }

    #[test]
    fn conduit_settings_key_set_is_pinned() {
        let mut expected: Vec<String> = SETTINGS_KEYS.iter().map(|s| s.to_string()).collect();
        expected.sort();
        assert_eq!(
            serialized_setting_keys(),
            expected,
            "ConduitSettings key set changed — update SETTINGS_KEYS, \
             Storage::save_settings, settingsTypes.ts and Settings.tsx in the same pass"
        );
    }

    /// REGRESSION — `notification_apps` used to be the only field without a
    /// `#[serde(default)]`, so the frontend's `SettingsData` (which did not
    /// declare it) made *every* `save_settings` call fail with
    /// `missing field notification_apps`. The user's "Save" button silently did
    /// nothing. This pins that a payload with none of the optional fields
    /// deserialises.
    #[test]
    fn settings_deserializes_payload_without_the_newer_fields() {
        // Exactly what an older/partial frontend sends: only the original six
        // required fields.
        let partial: ConduitSettings = serde_json::from_value(serde_json::json!({
            "device_name": "Laptop",
            "max_devices": 3,
            "sync_notifications": false,
            "sync_clipboard": false,
            "sync_files": false
        }))
        .expect("a partial payload must not fail deserialisation");

        assert_eq!(partial.device_name, "Laptop");
        assert_eq!(partial.max_devices, 3);
        assert!(!partial.sync_notifications);
        // Absent fields fall back to their documented defaults.
        assert_eq!(partial.notification_apps, default_notification_apps());
        assert_eq!(partial.theme, "dark");
        assert!(partial.minimize_to_tray);
        assert!(partial.notifications_enabled);
        assert_eq!(partial.relay_url, DEFAULT_RELAY_URL);
    }

    #[test]
    fn settings_deserializes_empty_object() {
        let empty: ConduitSettings =
            serde_json::from_value(serde_json::json!({})).expect("{} must deserialise");
        assert_eq!(empty, ConduitSettings::default());
    }

    /// Cross-language conformance: the hand-maintained
    /// `apps/desktop/src/components/settings/settingsTypes.ts` `SettingsData`
    /// interface must declare exactly the keys `ConduitSettings` serialises to.
    ///
    /// `packages/protocol/schema.json` describes the *wire* protocol only and
    /// carries no settings definition, so the frontend type cannot be generated.
    /// Instead this test parses the TypeScript source and compares key sets, which
    /// turns the silent two-way drift (already seen twice: `notification_apps`
    /// and `relay_url` were missing on the frontend) into a build failure.
    #[test]
    fn frontend_settings_type_key_set_matches_conduit_settings() {
        let ts_path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../src/components/settings/settingsTypes.ts"
        );
        let source = std::fs::read_to_string(ts_path)
            .unwrap_or_else(|e| panic!("cannot read {ts_path}: {e}"));

        let ts_keys = parse_settings_data_keys(&source);
        let rust_keys = serialized_setting_keys();

        assert_eq!(
            ts_keys, rust_keys,
            "SettingsData in settingsTypes.ts has drifted from ConduitSettings; \
             every Rust field needs a TS field (snake_case) and vice versa"
        );
    }

    /// Extract the member names of `export interface SettingsData { … }` from
    /// `settingsTypes.ts`.
    fn parse_settings_data_keys(source: &str) -> Vec<String> {
        let start = source
            .find("export interface SettingsData")
            .expect("settingsTypes.ts must declare `export interface SettingsData`");
        let body = &source[start..];
        let open = body.find('{').expect("SettingsData must have a body");
        let body = &body[open + 1..];
        let close = body
            .find("\n}")
            .expect("SettingsData body must be closed by a `}` at column 0");
        let body = &body[..close];

        let mut keys: Vec<String> = Vec::new();
        for line in body.lines() {
            // Drop line comments; the declarations themselves are one per line.
            let line = line.split("//").next().unwrap_or("").trim();
            if line.is_empty() {
                continue;
            }
            let name = line
                .split(':')
                .next()
                .unwrap_or("")
                .trim()
                .trim_end_matches('?')
                .trim()
                .to_string();
            assert!(
                !name.is_empty() && !line.starts_with('{') && !line.starts_with('}'),
                "unparsable line in SettingsData: {line:?} — keep one `field: type;` per line"
            );
            keys.push(name);
        }
        keys.sort();
        keys
    }

    // ── get_settings ──────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_settings_valid_empty_db_returns_documented_defaults() {
        let state = create_test_state();
        let app = create_app_with_state(state);

        let s = get_settings(app.state()).await.unwrap();
        assert_eq!(s.theme, "dark");
        assert_eq!(s.accent_color, "#00f0ff");
        assert_eq!(s.max_devices, 5);
        assert!(s.sync_notifications);
        assert!(s.minimize_to_tray, "default_true fields must default true");
        assert!(s.notifications_enabled);
        assert_eq!(s.relay_url, DEFAULT_RELAY_URL);
        assert!(
            s.relay_enabled,
            "empty database must default the relay on — this is the value \
             main.rs reads before starting or joining the relay"
        );
        assert_eq!(s.relay_port, crate::relay::DEFAULT_RELAY_PORT);
        assert_eq!(s.relay_health_port, crate::relay::DEFAULT_RELAY_HEALTH_PORT);
        assert_eq!(
            s.notification_apps,
            default_notification_apps(),
            "empty database must seed the same default app list serde uses"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_settings_valid_after_save_roundtrips_all_fields() {
        let state = create_test_state();
        let app = create_app_with_state(state.clone());

        save_settings(app.state(), sample_settings()).await.unwrap();
        let loaded = get_settings(app.state()).await.unwrap();

        assert_eq!(loaded, sample_settings(), "every field must round-trip");
    }

    /// REGRESSION — `relay_url` was read back by `get_settings` but never written
    /// by `save_settings`, so a configured relay URL silently reverted to the
    /// default on the next read. See the storage-level test of the same name for
    /// the raw-row assertion.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_settings_valid_relay_url_survives_save() {
        let state = create_test_state();
        let app = create_app_with_state(state);

        save_settings(
            app.state(),
            ConduitSettings {
                relay_url: "wss://relay.example:9528".into(),
                relay_enabled: false,
                relay_port: crate::relay::DEFAULT_RELAY_PORT,
                relay_health_port: crate::relay::DEFAULT_RELAY_HEALTH_PORT,
                relay_hostname: String::new(),
                ..Default::default()
            },
        )
        .await
        .unwrap();

        assert_eq!(
            get_settings(app.state()).await.unwrap().relay_url,
            "wss://relay.example:9528"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn save_settings_valid_second_save_overwrites_previous() {
        let state = create_test_state();
        let app = create_app_with_state(state.clone());

        save_settings(app.state(), sample_settings()).await.unwrap();
        let mut second = sample_settings();
        second.device_name = "Second".into();
        second.max_devices = 1;
        save_settings(app.state(), second).await.unwrap();

        let loaded = get_settings(app.state()).await.unwrap();
        assert_eq!(loaded.device_name, "Second");
        assert_eq!(loaded.max_devices, 1, "no duplicate/stale rows allowed");
    }

    /// `save_settings` must hand the new tray preference to the tray module's
    /// synchronous cache, otherwise "Minimize to tray" would need a restart to
    /// take effect (the window-close event cannot await the database).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn save_settings_valid_updates_tray_minimize_cache() {
        let _guard = crate::tray::lock_minimize_to_tray().await;
        let state = create_test_state();
        let app = create_app_with_state(state);

        save_settings(
            app.state(),
            ConduitSettings {
                minimize_to_tray: false,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(
            !crate::tray::minimize_to_tray(),
            "tray cache must follow save"
        );

        save_settings(
            app.state(),
            ConduitSettings {
                minimize_to_tray: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(crate::tray::minimize_to_tray());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn save_settings_field_valid_updates_tray_minimize_cache() {
        let _guard = crate::tray::lock_minimize_to_tray().await;
        let state = create_test_state();
        let app = create_app_with_state(state);

        save_settings_field(app.state(), "minimize_to_tray".into(), "false".into())
            .await
            .unwrap();
        assert!(!crate::tray::minimize_to_tray());

        save_settings_field(app.state(), "minimize_to_tray".into(), "true".into())
            .await
            .unwrap();
        assert!(crate::tray::minimize_to_tray());
    }

    // ── save_settings_field ───────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn save_settings_field_valid_overwrites_single_key_via_get_settings() {
        let state = create_test_state();
        let app = create_app_with_state(state.clone());

        save_settings_field(app.state(), "theme".into(), "light".into())
            .await
            .unwrap();

        let s = get_settings(app.state()).await.unwrap();
        assert_eq!(s.theme, "light");
        // Other keys untouched.
        assert_eq!(s.accent_color, "#00f0ff");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn save_settings_field_edge_arbitrary_key_is_accepted_without_validation() {
        // CHARACTERIZATION: save_settings_field performs no key/value validation.
        // If a future change whitelists keys, rename → save_settings_field_unknown_key_returns_error.
        let state = create_test_state();
        let app = create_app_with_state(state.clone());

        save_settings_field(
            app.state(),
            "totally_unknown_key".into(),
            "<script>alert(1)</script>".into(),
        )
        .await
        .unwrap();

        let raw = state.storage.get_setting("totally_unknown_key").await;
        assert_eq!(raw.as_deref(), Some("<script>alert(1)</script>"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn save_settings_field_edge_non_numeric_max_devices_silently_falls_back_to_default() {
        // CHARACTERIZATION: get_settings does `.parse().unwrap_or(5)` — a
        // corrupted max_devices value silently reverts to the default instead
        // of erroring. If validation is ever added, rename this test to
        // get_settings_invalid_max_devices_returns_error.
        let state = create_test_state();
        let app = create_app_with_state(state.clone());

        save_settings_field(app.state(), "max_devices".into(), "not-a-number".into())
            .await
            .unwrap();

        let s = get_settings(app.state()).await.unwrap();
        assert_eq!(s.max_devices, 5, "unparsable value falls back to default 5");
    }

    // ── allowed_commands ──────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_allowed_commands_valid_empty_db_returns_empty_deny_by_default() {
        let state = create_test_state();
        let app = create_app_with_state(state);

        let list = get_allowed_commands(app.state()).await.unwrap();
        assert!(
            list.is_empty(),
            "a fresh install must have an empty allowlist, which blocks all shell execution"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_allowed_commands_invalid_corrupt_row_fails_closed() {
        let state = create_test_state();
        state
            .storage
            .save_setting(ALLOWED_COMMANDS_KEY, "not json at all")
            .await
            .unwrap();
        let app = create_app_with_state(state);

        assert!(
            get_allowed_commands(app.state()).await.unwrap().is_empty(),
            "a corrupt allowlist row must read as empty, never as allow-everything"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn set_allowed_commands_valid_roundtrips_and_sanitises() {
        let state = create_test_state();
        let app = create_app_with_state(state);

        let stored = set_allowed_commands(
            app.state(),
            vec![
                "  ls  ".to_string(),
                "ls".to_string(),
                "  ".to_string(),
                "echo".to_string(),
            ],
        )
        .await
        .unwrap();
        assert_eq!(
            stored,
            vec!["ls".to_string(), "echo".to_string()],
            "entries must be trimmed and de-duplicated"
        );
        assert_eq!(get_allowed_commands(app.state()).await.unwrap(), stored);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn set_allowed_commands_valid_takes_effect_immediately() {
        let state = create_test_state();
        let app = create_app_with_state(state);
        let _ = set_allowed_commands(app.state(), vec!["echo".to_string()])
            .await
            .unwrap();

        let list = crate::security::current_command_allowlist();
        assert!(list.is_allowed("echo hello"));
        assert!(!list.is_allowed("ls"), "only the saved command is allowed");

        // And removing it blocks it again without a restart.
        let _ = set_allowed_commands(app.state(), vec![]).await.unwrap();
        let list = crate::security::current_command_allowlist();
        assert!(!list.is_allowed("echo hello"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn set_allowed_commands_invalid_too_many_entries_is_rejected() {
        let state = create_test_state();
        let app = create_app_with_state(state.clone());

        let err = set_allowed_commands(
            app.state(),
            (0..crate::security::MAX_ALLOWED_COMMANDS + 1)
                .map(|i| format!("cmd{i}"))
                .collect(),
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("Too many allowed commands"));
        assert!(
            state
                .storage
                .get_setting(ALLOWED_COMMANDS_KEY)
                .await
                .is_none(),
            "a rejected save must not be partially written"
        );
    }

    // ── get_local_ws_token ────────────────────────────────────────────────────

    #[test]
    fn get_local_ws_token_returns_the_process_capability() {
        let token = get_local_ws_token().unwrap();
        assert_eq!(token, crate::security::local_capability().token());
        assert!(
            token.len() >= 8,
            "the token must be long enough to be unguessable"
        );
        assert!(token.chars().all(|c| c.is_ascii_alphanumeric()));
    }

    // ── missing state ─────────────────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "state() called before manage")]
    fn settings_commands_missing_state_fails_loudly() {
        let app = tauri::test::mock_app();
        let _ = app.state::<Arc<AppState>>();
    }
}
