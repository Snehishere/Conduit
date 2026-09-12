mod server;
mod discovery;
mod encryption;
mod storage;
mod sync;
mod commands;
mod tray;
mod file_transfer;
mod automation;
mod audio;
mod error;

pub use error::{ConduitError, Result};

use std::sync::Arc;
use tokio::sync::RwLock;
use chrono::Timelike;
use log::{error, info};

pub struct AppState {
    pub storage: Arc<storage::Storage>,
    pub sync_engine: Arc<RwLock<sync::SyncEngine>>,
    pub ws_server: Arc<RwLock<Option<server::WsServer>>>,
    pub discovery: Arc<RwLock<Option<discovery::DiscoveryService>>>,
    pub device_id: String,
    pub encryption: encryption::EncryptionManager,
    pub file_engine: Arc<file_transfer::FileTransferEngine>,
    pub pending_tokens: Arc<RwLock<std::collections::HashSet<String>>>,
    pub automation_engine: Arc<RwLock<automation::AutomationEngine>>,
    pub audio_stream: Arc<audio::AudioStream>,
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
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .filter_module("mdns_sd", log::LevelFilter::Warn)
        .init();

    let rt = match tokio::runtime::Runtime::new() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("Failed to create tokio runtime: {}", e);
            std::process::exit(1);
        }
    };
    rt.block_on(async {
        let device_id = load_or_create_device_id();
        let storage = match storage::Storage::new() {
            Ok(s) => Arc::new(s),
            Err(e) => {
                eprintln!("Failed to initialize storage: {}", e);
                std::process::exit(1);
            }
        };
        let encryption = encryption::EncryptionManager::new();

        let state = Arc::new(AppState {
            storage: storage.clone(),
            sync_engine: Arc::new(RwLock::new(sync::SyncEngine::new())),
            ws_server: Arc::new(RwLock::new(None)),
            discovery: Arc::new(RwLock::new(None)),
            device_id: device_id.clone(),
            encryption,
            file_engine: Arc::new(file_transfer::FileTransferEngine::new()),
            pending_tokens: Arc::new(RwLock::new(std::collections::HashSet::new())),
            automation_engine: Arc::new(RwLock::new(automation::AutomationEngine::new())),
            audio_stream: Arc::new(audio::AudioStream::new()),
        });

        tauri::Builder::default()
            .plugin(tauri_plugin_shell::init())
            .plugin(tauri_plugin_clipboard_manager::init())
            .plugin(tauri_plugin_dialog::init())
            .manage(state.clone())
            .invoke_handler(tauri::generate_handler![
                commands::get_devices,
                commands::get_device,
                commands::start_ws_server,
                commands::stop_ws_server,
                commands::start_discovery,
                commands::stop_discovery,
                commands::generate_pairing_token,
                commands::generate_qr_code,
                commands::send_notification,
                commands::dismiss_notification,
                commands::reply_notification,
                commands::sync_clipboard,
                commands::get_notifications,
                commands::get_device_info,
                commands::get_connected_devices,
                commands::send_encrypted_message,
                commands::get_system_info,
                commands::get_settings,
                commands::save_settings,
                commands::delete_paired_device,
                commands::send_file,
                commands::accept_file_transfer,
                commands::cancel_file_transfer,
                commands::resume_file_transfer,
                commands::get_file_transfers,
                commands::get_downloads_path,
                commands::get_automation_rules,
                commands::create_automation_rule,
                commands::update_automation_rule,
                commands::delete_automation_rule,
                commands::toggle_automation_rule,
                commands::execute_automation_action,
                commands::get_automation_logs,
                commands::start_audio_stream,
                commands::stop_audio_stream,
                commands::get_audio_devices,
            ])
            .setup(move |app| {
                // Load automation rules from database
                let state_auto = state.clone();
                if let Ok(rules) = state_auto.storage.get_all_automation_rules() {
                    let engine_arc = state_auto.automation_engine.clone();
                    tokio::spawn(async move {
                        let mut e = engine_arc.write().await;
                        e.load_rules(rules);
                    });
                }

                // Start WebSocket server
                let state_ws = state.clone();
                let rt = tokio::runtime::Handle::current();
                let sync_engine = state_ws.sync_engine.clone();
                let encryption = Arc::new(state_ws.encryption.clone());
                let file_engine_clone = state_ws.file_engine.clone();
                let pending_tokens_clone = state_ws.pending_tokens.clone();
                let storage_clone = state_ws.storage.clone();
                let automation_clone = state_ws.automation_engine.clone();
                let audio_stream_clone = state_ws.audio_stream.clone();
                let app_handle_ws = app.handle().clone();
                rt.spawn(async move {
                    let mut ws = server::WsServer::new(
                        "0.0.0.0:9527".to_string(),
                        sync_engine,
                        encryption,
                        file_engine_clone,
                        pending_tokens_clone,
                        storage_clone,
                        automation_clone,
                        audio_stream_clone,
                    ).await;
                    ws.set_app_handle(app_handle_ws);
                    *state_ws.ws_server.write().await = Some(ws);
                });

                // Start mDNS discovery
                let state_disc = state.clone();
                let app_handle = app.handle().clone();
                rt.spawn(async move {
                    let mut disc = discovery::DiscoveryService::new(state_disc.device_id.clone());
                    disc.set_app_handle(app_handle);
                    disc.start().await;
                    *state_disc.discovery.write().await = Some(disc);
                });

                // Background timer for time-based automation triggers (checks every 60s)
                let state_auto_timer = state.clone();
                rt.spawn(async move {
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
                            for rule in &triggered_rules {
                                let mut log = crate::automation::execute_action(&rule.action);
                                log.id = rule.id.clone();
                                log.trigger_type = format!("time:{}", crate::automation::trigger_tag(&rule.trigger));
                                if let Err(e) = state_auto_timer.storage.log_automation_execution(
                                    &log.id, &log.trigger_type, log.timestamp, log.success, log.message.as_deref(),
                                ) {
                                    error!("Failed to save automation log: {}", e);
                                }
                                info!("Executed time trigger action for rule: {}", rule.name);
                            }
                        }
                    }
                });

                // Setup system tray
                tray::setup_tray(app.handle())?;

                Ok(())
            })
            .run(tauri::generate_context!())
            .expect("error while running tauri application");
    });
}
