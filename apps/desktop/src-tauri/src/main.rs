#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod automation;
mod commands;
mod discovery;
mod encryption;
mod error;
mod file_transfer;
mod relay;
mod security;
mod server;
mod storage;
mod sync;
mod tls;
mod tray;

pub use error::{ConduitError, Result};

// ─── Network constants ────────────────────────────────────────────────────────
// The port numbers live in the shared protocol crate so there is exactly one
// definition in the workspace; these aliases exist only for the crate-local
// `WS_PORT` / `WSS_PORT` spelling used throughout the desktop code.
pub use conduit_protocol::types::{LAN_WS_PORT as WS_PORT, LAN_WSS_PORT as WSS_PORT};

/// Bind address for the plaintext WebSocket listener (all interfaces).
///
/// The port is spelled inline because a `&'static str` cannot be built from a
/// numeric constant; `ws_bind_addr_matches_ws_port` pins it to [`WS_PORT`] so
/// the two cannot drift apart silently.
pub const WS_BIND_ADDR: &str = "0.0.0.0:9527";
/// mDNS / DNS-SD service type used for LAN discovery.
pub const MDNS_SERVICE_TYPE: &str = "_conduit._tcp";
/// Human-readable app name used in system tray, window title, etc.
pub const APP_NAME: &str = "Conduit";

use std::sync::Arc;
use tokio::sync::RwLock;

use chrono::Timelike;
use log::{debug, error, info, warn};
use tauri::Manager;

pub struct AppState {
    pub storage: Arc<storage::Storage>,
    pub sync_engine: Arc<RwLock<sync::SyncEngine>>,
    pub ws_server: Arc<RwLock<Option<server::WsServer>>>,
    pub discovery: Arc<RwLock<Option<discovery::DiscoveryService>>>,
    pub device_id: String,
    pub encryption: encryption::EncryptionManager,
    pub file_engine: Arc<file_transfer::FileTransferEngine>,
    pub token_store: Arc<security::TokenStore>,
    pub automation_engine: Arc<RwLock<automation::AutomationEngine>>,
    pub audio_stream: Arc<audio::AudioStream>,
    /// The relay this app hosts, if the user has turned it on.
    ///
    /// A background task in this process, not a separate program: see
    /// [`relay`] for why, and for what it does and does not change about the
    /// threat model.
    pub relay_host: Arc<relay::RelayHost>,
}

fn load_or_create_device_id() -> String {
    let path = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("conduit")
        .join("device_id.txt");
    if let Ok(existing) = std::fs::read_to_string(&path) {
        let trimmed = existing.trim().to_string();
        if !trimmed.is_empty() {
            return trimmed;
        }
    }
    let new_id = uuid::Uuid::new_v4().to_string();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, &new_id);
    new_id
}

fn main() {
    // Set up log file at ~/.conduit/logs/conduit.log
    let log_dir = dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("conduit")
        .join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let log_path = log_dir.join("conduit.log");

    // Build env_logger with file output via Target::Pipe
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .ok();

    let mut builder =
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"));
    builder.filter_module("mdns_sd", log::LevelFilter::Warn);

    if let Some(file) = log_file {
        use std::io::Write;
        // Tee writes to both the file and stderr
        struct TeeWriter {
            file: std::fs::File,
        }
        impl Write for TeeWriter {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                let _ = self.file.write_all(buf);
                let _ = std::io::stderr().write_all(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                self.file.flush()?;
                std::io::stderr().flush()
            }
        }
        builder.target(env_logger::Target::Pipe(Box::new(TeeWriter { file })));
    }

    builder.init();
    info!("Conduit starting — log file: {}", log_path.display());

    let rt = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            log::error!("Failed to create tokio runtime: {}", e);
            eprintln!("Failed to create tokio runtime: {}", e);
            std::process::exit(1);
        }
    };

    // Keep the runtime active for all sync and async setup and Tauri's event loop.
    let _guard = rt.enter();

    // `rt` itself is not `Clone`, and the Tauri builder below takes everything
    // it captures by move. A `Handle` is cloneable and can `block_on`, so this
    // is what survives to the post-run cleanup.
    // `Arc` because the setup closure below must be `'static`, and a bare
    // `Handle` borrow of `rt` cannot outlive it. `rt.handle()` returns a
    // reference, so the owned `Handle` is cloned out of it first.
    let cleanup_rt = Arc::new(rt.handle().clone());
    let cleanup_rt_in_setup = cleanup_rt.clone();

    // Run async initialization synchronously before handing off to Tauri's event loop.
    let (device_id, storage, encryption) = rt.block_on(async {
        let device_id = load_or_create_device_id();
        let storage = match storage::Storage::new() {
            Ok(s) => Arc::new(s),
            Err(e) => {
                log::error!("Failed to initialize storage: {}", e);
                eprintln!("Failed to initialize storage: {}", e);
                std::process::exit(1);
            }
        };
        let encryption = match encryption::EncryptionManager::new() {
            Ok(e) => e,
            Err(e) => {
                log::error!("Failed to initialize encryption manager: {}", e);
                eprintln!("Failed to initialize encryption manager: {}", e);
                std::process::exit(1);
            }
        };
        (device_id, storage, encryption)
    });

    // ─── Shell-command allowlist (constructed once, at startup) ───────────────
    //
    // `CommandAllowlist` had ~40 unit tests and `#[allow(dead_code)]` markers
    // while **every** production call site passed `None` to
    // `execute_action_with_allowlist`, so a remotely-supplied
    // `RunShellCommand` rule ran unchecked — the RCE half of the vulnerability.
    //
    // It is built here, from the `allowed_commands` setting, exactly once, and
    // held process-wide in `security::command_allowlist()`. Every call site
    // takes a snapshot via `security::current_command_allowlist()` and passes
    // `Some(..)`, so there is no longer a way to reach `execute_action` for a
    // rule-triggered action without passing the allowlist.
    //
    // An absent or corrupt setting yields an empty list, which blocks *all*
    // shell execution — deny-by-default, as `CommandAllowlist` documents.
    let allowed_commands = rt.block_on(async {
        let raw = storage.get_setting(commands::ALLOWED_COMMANDS_KEY).await;
        security::parse_allowed_commands_setting(raw.as_deref())
    });
    security::init_command_allowlist(allowed_commands.clone());
    log::info!(
        "Shell command allowlist: {} command(s) permitted",
        allowed_commands.len()
    );

    let state = Arc::new(AppState {
        storage: storage.clone(),
        sync_engine: Arc::new(RwLock::new(sync::SyncEngine::new())),
        ws_server: Arc::new(RwLock::new(None)),
        discovery: Arc::new(RwLock::new(None)),
        device_id: device_id.clone(),
        encryption,
        // The engine is attached to storage so `default_download_folder` is
        // honoured — and re-read for every transfer, so changing it in
        // Settings takes effect without a restart.
        file_engine: Arc::new(
            file_transfer::FileTransferEngine::new().with_storage(storage.clone()),
        ),
        token_store: Arc::new(security::TokenStore::new(std::time::Duration::from_secs(
            60,
        ))), // 1 min expiry
        automation_engine: Arc::new(RwLock::new(automation::AutomationEngine::new())),
        audio_stream: Arc::new(audio::AudioStream::new()),
        relay_host: Arc::new(relay::RelayHost::new(storage.clone(), device_id.clone())),
    });

    // Survives the Tauri builder, which takes `state` by move, so the relay can
    // be stopped after the event loop returns.
    let cleanup_state = state.clone();

    let app = tauri::Builder::default()
        // NOTE: `tauri-plugin-shell` is deliberately NOT registered. Its
        // `open()` used to be reachable from the webview via `shell:allow-open`
        // with whatever path a paired device put in a `file/complete` frame.
        // Opening a received file now goes through the `open_downloaded_file`
        // command, which resolves the path from local state. See
        // `capabilities/default.json`.
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        // Declared explicitly: `AdvancedSection.tsx` (Settings → Import)
        // imports `@tauri-apps/plugin-fs` and calls `readTextFile`. Previously
        // the JS package existed with no Rust plugin and no `fs:*` permission,
        // so the button was permanently broken.
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
        }))
        .manage(state.clone())
        .invoke_handler(tauri::generate_handler![
            commands::get_devices,
            commands::start_discovery,
            commands::stop_discovery,
            commands::generate_pairing_token,
            commands::generate_qr_code,
            commands::get_clipboard_history,
            commands::toggle_clipboard_pin,
            commands::delete_clipboard_entry,
            commands::clear_clipboard_history,
            commands::get_device_info,
            commands::send_encrypted_message,
            commands::get_system_info,
            commands::get_settings,
            commands::get_relay_status,
            commands::save_settings,
            commands::save_settings_field,
            commands::delete_paired_device,
            commands::send_file,
            commands::approve_files_for_send,
            commands::open_downloaded_file,
            commands::accept_file_transfer,
            commands::cancel_file_transfer,
            commands::resume_file_transfer,
            commands::get_file_transfers,
            commands::get_current_version,
            commands::save_window_state,
            commands::get_window_state,
            commands::get_local_ip,
            commands::fix_firewall,
            // Notification actions. These were previously invoked from
            // `useWebSocket.tsx` but never registered, so every call rejected
            // and the notification list was always empty.
            commands::get_notifications,
            commands::dismiss_notification,
            commands::reply_notification,
            commands::sync_clipboard,
            // Settings → Advanced → Allowed Commands. The allowlist is the
            // only thing standing between a peer-supplied automation rule and
            // `Command::new`; see the startup construction above.
            commands::get_allowed_commands,
            commands::set_allowed_commands,
            // Per-launch capability for the desktop's own webview socket. A
            // loopback peer that cannot present it stays unpaired.
            commands::get_local_ws_token,
        ])
        .setup(move |app| {
            let cleanup_rt = cleanup_rt_in_setup.clone();
            // One task owns the whole relay story, in the order it has to
            // happen: read the settings once, start the listener, then join it.
            // Splitting these up would let the client dial a relay that was
            // never started, or read the settings twice and disagree with
            // itself across a save.
            {
                let state_relay = state.clone();
                cleanup_rt.spawn(async move {
                    let settings = match state_relay.storage.get_settings().await {
                        Ok(s) => s,
                        Err(e) => {
                            // A desktop that cannot read its settings should not
                            // open a listener the user may have turned off.
                            warn!("Relay left off: could not read settings: {e}");
                            commands::settings::ConduitSettings::default().with_relay_enabled(false)
                        }
                    };
                    if !settings.relay_enabled {
                        debug!("Relay is off in settings; not starting it or joining it");
                        return;
                    }

                    let token = match relay::resolve_relay_token() {
                        Ok(t) => t,
                        Err(e) => {
                            // Without a token the relay cannot authenticate its
                            // own client either, so there is nothing to retry.
                            warn!("Relay not started: {e}");
                            return;
                        }
                    };
                    let device_id = state_relay.device_id.clone();

                    state_relay.relay_host.start(&settings).await;

                    // Join the relay this app hosts, so the desktop appears in
                    // its routing table. A relay only delivers to connections it
                    // holds, so without this the desktop is invisible to every
                    // phone that is not on the same LAN. The hub may not exist
                    // yet, and the client retries with backoff either way.
                    let ws_server = state_relay.ws_server.clone();
                    for _ in 0..100 {
                        let guard = ws_server.read().await;
                        if let Some(ws) = guard.as_ref() {
                            ws.spawn_relay_client(
                                relay::local_relay_url(),
                                device_id.clone(),
                                token.clone(),
                            );
                            return;
                        }
                        drop(guard);
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                    warn!("Relay is running but the hub never came up, so it was not joined");
                });
            }

            // Load automation rules from database
            let state_auto = state.clone();
            tokio::spawn(async move {
                match state_auto.storage.get_all_automation_rules().await {
                    Ok(rules) => {
                        let engine_arc = state_auto.automation_engine.clone();
                        let mut e = engine_arc.write().await;
                        e.load_rules(rules);
                    }
                    Err(e) => error!("Failed to load automation rules: {}", e),
                }
            });

            // Start WebSocket server
            let state_ws = state.clone();
            let sync_engine = state_ws.sync_engine.clone();
            let encryption = Arc::new(state_ws.encryption.clone());
            let file_engine_clone = state_ws.file_engine.clone();
            let token_store_clone = state_ws.token_store.clone();
            let storage_clone_ws = state_ws.storage.clone();
            let _storage_clone_ws2 = state_ws.storage.clone();
            let automation_clone = state_ws.automation_engine.clone();
            let audio_stream_clone = state_ws.audio_stream.clone();
            let app_handle_ws = app.handle().clone();
            cleanup_rt.spawn(async move {
                let mut ws = server::WsServer::new(
                    crate::WS_BIND_ADDR.to_string(),
                    sync_engine,
                    encryption.clone(),
                    file_engine_clone,
                    token_store_clone,
                    storage_clone_ws,
                    automation_clone,
                    audio_stream_clone,
                    Arc::new(state_ws.device_id.clone()),
                    state_ws.relay_host.route_keys(),
                )
                .await;
                ws.set_app_handle(app_handle_ws);

                // The relay client is spawned by the task above, which waits
                // for this server to appear.
                *state_ws.ws_server.write().await = Some(ws);
            });

            // Start mDNS discovery
            let state_disc = state.clone();
            let storage_clone_disc = state.storage.clone();
            let app_handle = app.handle().clone();
            cleanup_rt.spawn(async move {
                let device_name_clone = storage_clone_disc
                    .get_settings()
                    .await
                    .map(|s| s.device_name)
                    .unwrap_or_else(|_| gethostname::gethostname().to_string_lossy().to_string());
                let mut disc = discovery::DiscoveryService::new(
                    state_disc.device_id.clone(),
                    device_name_clone,
                );
                disc.set_app_handle(app_handle);
                disc.start().await;
                *state_disc.discovery.write().await = Some(disc);
            });

            // Background timer for time-based automation triggers (checks every 60s)
            let state_auto_timer = state.clone();
            cleanup_rt.spawn(async move {
                let mut last_minute = String::new();
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    let now = chrono::Local::now();
                    let current_minute = format!("{:02}:{:02}", now.hour(), now.minute());
                    if current_minute == last_minute {
                        continue;
                    }
                    last_minute = current_minute.clone();

                    // Snapshot triggered rules under a single lock to avoid TOCTOU.
                    let triggered_rules = {
                        let engine = state_auto_timer.automation_engine.read().await;
                        let ids = engine.check_time_triggers();
                        ids.into_iter()
                            .filter_map(|id| engine.get_rule(&id).cloned())
                            .collect::<Vec<_>>()
                    };

                    if !triggered_rules.is_empty() {
                        // SECURITY: was `execute_action` — i.e. no allowlist —
                        // on the one path that fires without any peer involved
                        // (the time trigger), which meant a rule persisted by a
                        // remote peer executed on the next minute boundary.
                        let allowlist = security::current_command_allowlist();
                        for rule in &triggered_rules {
                            let mut log = crate::automation::execute_action_with_allowlist(
                                &rule.action,
                                Some(&allowlist),
                            );
                            log.id = rule.id.clone();
                            log.trigger_type =
                                format!("time:{}", crate::automation::trigger_tag(&rule.trigger));
                            if let Err(e) = state_auto_timer
                                .storage
                                .log_automation_execution(
                                    &log.id,
                                    &log.trigger_type,
                                    log.timestamp,
                                    log.success,
                                    log.message.as_deref(),
                                )
                                .await
                            {
                                error!("Failed to save automation log: {}", e);
                            }
                            info!("Executed time trigger action for rule: {}", rule.name);
                        }
                    }
                }
            });

            // Setup system tray
            tray::setup_tray(app.handle())?;

            // Restore window state
            let app_handle_win = app.handle().clone();
            let state_win = state.clone();
            cleanup_rt.spawn(async move {
                // Small delay to let the window initialize
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;

                if let Some(json) = state_win.storage.get_setting("window_state").await
                    && let Ok(win_state) = serde_json::from_str::<commands::WindowState>(&json)
                    && let Some(window) = app_handle_win.get_webview_window("main")
                {
                    if let (Some(x), Some(y)) = (win_state.x, win_state.y) {
                        let _ = window.set_position(tauri::Position::Physical(
                            tauri::PhysicalPosition {
                                x: x as i32,
                                y: y as i32,
                            },
                        ));
                    }
                    if let (Some(w), Some(h)) = (win_state.width, win_state.height) {
                        let _ = window.set_size(tauri::Size::Physical(tauri::PhysicalSize {
                            width: w as u32,
                            height: h as u32,
                        }));
                    }
                    if win_state.maximized {
                        let _ = window.maximize();
                    }
                }
            });

            // Save window state on close
            let app_handle_close = app.handle().clone();
            let state_close = state.clone();
            if let Some(window) = app_handle_close.get_webview_window("main") {
                let win_clone = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::Destroyed = event {
                        let physical_pos = win_clone.outer_position().ok();
                        let physical_size = win_clone.outer_size().ok();
                        let is_maximized = win_clone.is_maximized().unwrap_or(false);

                        let ws = commands::WindowState {
                            x: physical_pos.map(|p| p.x as f64),
                            y: physical_pos.map(|p| p.y as f64),
                            width: physical_size.map(|s| s.width as f64),
                            height: physical_size.map(|s| s.height as f64),
                            maximized: is_maximized,
                        };

                        if let Ok(json) = serde_json::to_string(&ws) {
                            // Sync context (window event) — use try_lock retry helper.
                            let _ = state_close.storage.save_setting_sync("window_state", &json);
                        }
                    }
                });
            }

            Ok(())
        })
        .run(tauri::generate_context!());

    // The event loop has returned, so the app is exiting. Stop the relay before
    // the runtime is torn down: its shutdown drains connections with a bounded
    // window and performs a final flush of the replay cache, and a nonce
    // accepted in the last seconds before exit must not be replayable after the
    // next launch.
    cleanup_rt.block_on(async {
        cleanup_state.relay_host.stop().await;
    });

    app.expect("error while running tauri application");
}

#[cfg(test)]
mod integration_tests {
    use super::*;

    // ── LAN port constants ────────────────────────────────────────────────────
    //
    // The desktop binds two listeners: plaintext on WS_PORT and TLS on
    // WSS_PORT. Both must resolve to the shared protocol constants, and the
    // mobile app only ever dials WSS_PORT for a LAN peer.

    #[test]
    fn ws_port_re_exports_the_shared_protocol_constant() {
        assert_eq!(WS_PORT, conduit_protocol::types::LAN_WS_PORT);
        assert_eq!(WS_PORT, 9527);
    }

    #[test]
    fn wss_port_re_exports_the_shared_protocol_constant() {
        assert_eq!(WSS_PORT, conduit_protocol::types::LAN_WSS_PORT);
        assert_eq!(WSS_PORT, 9531);
    }

    #[test]
    fn ws_bind_addr_matches_ws_port() {
        // WS_BIND_ADDR inlines the port in a `&'static str`; pin it to the
        // constant so the two cannot drift.
        assert!(
            WS_BIND_ADDR.ends_with(&format!(":{}", WS_PORT)),
            "WS_BIND_ADDR ({}) must bind the plaintext port {WS_PORT}",
            WS_BIND_ADDR
        );
        assert_eq!(WS_BIND_ADDR, format!("0.0.0.0:{}", WS_PORT));
    }

    #[test]
    fn ws_bind_addr_is_not_the_tls_port() {
        assert!(
            !WS_BIND_ADDR.ends_with(&format!(":{}", WSS_PORT)),
            "WS_BIND_ADDR must not bind the TLS port {WSS_PORT}"
        );
    }

    #[test]
    fn tls_listener_binds_a_different_port_than_the_plaintext_one() {
        assert_ne!(
            WS_PORT, WSS_PORT,
            "the plaintext and TLS listeners cannot share a port"
        );
    }

    #[test]
    fn ws_bind_addr_parses_as_a_socket_address() {
        assert!(
            WS_BIND_ADDR.parse::<std::net::SocketAddr>().is_ok(),
            "WS_BIND_ADDR must be a parseable SocketAddr"
        );
    }

    // ── encryption ───────────────────────────────────────────────────────────

    #[test]
    fn encryption_full_round_trip_two_peers() {
        let alice = encryption::EncryptionManager::new_random();
        let bob = encryption::EncryptionManager::new_random();

        let alice_secret = alice.derive_shared_secret(&bob.public_key_hex()).unwrap();
        let bob_secret = bob.derive_shared_secret(&alice.public_key_hex()).unwrap();
        assert_eq!(alice_secret, bob_secret);

        let msg = "End-to-end encrypted message from Alice to Bob";
        let (nonce, ciphertext) = alice.encrypt(&hex::encode(alice_secret), msg).unwrap();
        let decrypted = bob
            .decrypt(&hex::encode(bob_secret), &nonce, &ciphertext)
            .unwrap();
        assert_eq!(decrypted, msg);
    }

    #[test]
    fn encryption_multiple_messages_same_session() {
        let alice = encryption::EncryptionManager::new_random();
        let bob = encryption::EncryptionManager::new_random();
        let secret = alice.derive_shared_secret(&bob.public_key_hex()).unwrap();
        let bob_secret = bob.derive_shared_secret(&alice.public_key_hex()).unwrap();

        let messages = vec![
            "hello",
            "world",
            "",
            "a".repeat(10_000).leak(),
            "special chars: éàüñ",
        ];
        for msg in messages {
            let (nonce, ciphertext) = alice.encrypt(&hex::encode(secret), msg).unwrap();
            let decrypted = bob
                .decrypt(&hex::encode(bob_secret), &nonce, &ciphertext)
                .unwrap();
            assert_eq!(decrypted, msg);
        }
    }

    #[test]
    fn encryption_eve_cannot_decrypt() {
        let alice = encryption::EncryptionManager::new_random();
        let bob = encryption::EncryptionManager::new_random();
        let eve = encryption::EncryptionManager::new_random();

        let secret = alice.derive_shared_secret(&bob.public_key_hex()).unwrap();
        let (nonce, ciphertext) = alice
            .encrypt(&hex::encode(secret), "secret message")
            .unwrap();

        let eve_secret = eve.derive_shared_secret(&bob.public_key_hex()).unwrap();
        let result = eve.decrypt(&hex::encode(eve_secret), &nonce, &ciphertext);
        assert!(result.is_err());
    }

    #[test]
    fn automation_full_rule_lifecycle() {
        let mut engine = automation::AutomationEngine::new();

        let rule = automation::AutomationRule {
            id: "rule_001".into(),
            name: "Morning alert".into(),
            enabled: true,
            trigger: automation::TriggerType::Time {
                time: "08:00".into(),
            },
            action: automation::ActionType::SendNotification {
                title: "Good morning".into(),
                body: "Start your day".into(),
            },
            trusted_source_only: false,
        };

        assert!(automation::validate_rule(&rule).is_ok());

        engine.add_rule(rule.clone());
        assert_eq!(engine.get_rules().len(), 1);
        assert_eq!(engine.get_rule("rule_001").unwrap().name, "Morning alert");

        let updated = automation::AutomationRule {
            enabled: false,
            ..rule
        };
        engine.update_rule(updated);
        assert!(!engine.get_rule("rule_001").unwrap().enabled);

        engine.remove_rule("rule_001");
        assert!(engine.get_rules().is_empty());
    }

    #[test]
    fn automation_execute_all_action_types() {
        let actions = vec![
            automation::ActionType::SendNotification {
                title: "t".into(),
                body: "b".into(),
            },
            automation::ActionType::SetPhoneProfile {
                profile: "silent".into(),
            },
            automation::ActionType::RouteAudio {
                device_id: "d1".into(),
            },
            automation::ActionType::RunShellCommand {
                command: "echo hello".into(),
            },
            automation::ActionType::ToggleWiFi { enabled: true },
            automation::ActionType::ToggleBluetooth { enabled: false },
            automation::ActionType::OpenUrl {
                url: "https://example.com".into(),
            },
            automation::ActionType::OpenApp {
                app_package: "com.test".into(),
            },
            automation::ActionType::SetWindowState {
                state: "minimize".into(),
            },
        ];

        for action in actions {
            let log = automation::execute_action(&action);
            assert!(
                log.message.is_some(),
                "every action should produce a message"
            );
            assert!(!log.trigger_type.is_empty());
        }
    }

    #[test]
    fn file_transfer_chunk_math() {
        const CHUNK_SIZE: usize = 64 * 1024;

        let cases: Vec<(u64, u32)> = vec![
            (0, 0),
            (1, 1),
            (CHUNK_SIZE as u64, 1),
            (CHUNK_SIZE as u64 + 1, 2),
            (CHUNK_SIZE as u64 * 3, 3),
            (CHUNK_SIZE as u64 * 3 + 1, 4),
            (1_000_000, (1_000_000_usize).div_ceil(CHUNK_SIZE) as u32),
        ];

        for (size, expected) in cases {
            let total = (size as usize).div_ceil(CHUNK_SIZE) as u32;
            assert_eq!(total, expected, "size={}", size);
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn automation_trigger_evaluation_chain() {
        let mut engine = automation::AutomationEngine::new();

        engine.add_rule(automation::AutomationRule {
            id: "connect_d1".into(),
            name: "On D1 connect".into(),
            enabled: true,
            trigger: automation::TriggerType::DeviceConnect {
                device_id: "d1".into(),
            },
            action: automation::ActionType::RouteAudio {
                device_id: "speaker".into(),
            },
            trusted_source_only: false,
        });

        engine.add_rule(automation::AutomationRule {
            id: "connect_wild".into(),
            name: "Any connect".into(),
            enabled: true,
            trigger: automation::TriggerType::DeviceConnect {
                device_id: "*".into(),
            },
            action: automation::ActionType::SendNotification {
                title: "Connected".into(),
                body: "".into(),
            },
            trusted_source_only: false,
        });

        engine.add_rule(automation::AutomationRule {
            id: "batt_low".into(),
            name: "Battery warning".into(),
            enabled: true,
            trigger: automation::TriggerType::BatteryLevel {
                below: 20,
                device_id: Some("d1".into()),
            },
            action: automation::ActionType::SendNotification {
                title: "Low battery".into(),
                body: "Charge now".into(),
            },
            trusted_source_only: false,
        });

        let ctx = automation::TriggerContext {
            event: automation::TriggerEvent::DeviceConnect,
            source_device_id: "d1".into(),
            ..Default::default()
        };
        assert!(
            engine
                .evaluate_trigger(&engine.get_rule("connect_d1").unwrap().trigger, &ctx)
                .await
        );
        assert!(
            engine
                .evaluate_trigger(&engine.get_rule("connect_wild").unwrap().trigger, &ctx)
                .await
        );

        let ctx_batt = automation::TriggerContext {
            event: automation::TriggerEvent::BatteryUpdate,
            source_device_id: "d1".into(),
            battery_level: Some(15),
            ..Default::default()
        };
        assert!(
            engine
                .evaluate_trigger(&engine.get_rule("batt_low").unwrap().trigger, &ctx_batt)
                .await
        );
    }
}
