use crate::automation::{self, ActionType, AutomationRule, TriggerType};
use crate::AppState;
use log::warn;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::Manager;
use tauri::State;

type ManagedState = Arc<AppState>;

#[derive(Serialize, Deserialize, Clone)]
pub struct DeviceResponse {
    pub id: String,
    pub name: String,
    pub device_type: String,
    pub os: String,
    pub battery: Option<i32>,
    pub signal: Option<String>,
    pub status: String,
    pub last_seen: i64,
}

#[derive(Serialize, Deserialize)]
pub struct SystemInfo {
    pub ram_used_mb: f64,
    pub ram_total_mb: f64,
    pub ram_percent: f64,
}

#[derive(Serialize, Deserialize)]
pub struct ConduitSettings {
    pub device_name: String,
    pub max_devices: u32,
    pub auto_connect: bool,
    pub sync_notifications: bool,
    pub sync_clipboard: bool,
    pub sync_files: bool,
    pub notification_apps: Vec<String>,
}

#[tauri::command]
pub async fn get_devices(state: State<'_, ManagedState>) -> Result<Vec<DeviceResponse>, String> {
    let stored = state
        .storage
        .get_all_devices()
        .map_err(|e| e.to_string())?;

    let engine = state.sync_engine.read().await;

    let mut devices: Vec<DeviceResponse> = stored
        .into_iter()
        .map(|d| {
            let is_connected = engine.get_client(&d.id).is_some();
            DeviceResponse {
                id: d.id,
                name: d.name,
                device_type: d.device_type,
                os: d.os,
                battery: d.battery,
                signal: d.signal,
                status: if is_connected { "connected".to_string() } else { d.status },
                last_seen: d.last_seen,
            }
        })
        .collect();

    // Add connected clients that aren't in storage yet
    for (id, client) in &engine.connected_clients {
        if !devices.iter().any(|d| d.id == *id) {
            devices.push(DeviceResponse {
                id: id.clone(),
                name: client.device_name.clone(),
                device_type: client.device_type.clone(),
                os: "unknown".to_string(),
                battery: None,
                signal: None,
                status: "connected".to_string(),
                last_seen: client.last_heartbeat,
            });
        }
    }

    // Always include the hub (desktop) as the first device
    let hub_exists = devices.iter().any(|d| d.device_type == "desktop");
    if !hub_exists {
        devices.insert(
            0,
            DeviceResponse {
                id: state.device_id.clone(),
                name: gethostname::gethostname().to_string_lossy().to_string(),
                device_type: "desktop".to_string(),
                os: std::env::consts::OS.to_string(),
                battery: None,
                signal: None,
                status: "connected".to_string(),
                last_seen: chrono::Utc::now().timestamp(),
            },
        );
    } else {
        // Ensure hub is first
        if let Some(pos) = devices.iter().position(|d| d.device_type == "desktop") {
            let hub = devices.remove(pos);
            devices.insert(0, hub);
        }
    }

    Ok(devices)
}

#[tauri::command]
pub async fn get_device(
    state: State<'_, ManagedState>,
    id: String,
) -> Result<Option<DeviceResponse>, String> {
    let device = state
        .storage
        .get_device(&id)
        .map_err(|e| e.to_string())?;

    Ok(device.map(|d| DeviceResponse {
        id: d.id,
        name: d.name,
        device_type: d.device_type,
        os: d.os,
        battery: d.battery,
        signal: d.signal,
        status: d.status,
        last_seen: d.last_seen,
    }))
}

#[tauri::command]
pub async fn start_ws_server(state: State<'_, ManagedState>) -> Result<String, String> {
    let mut ws = state.ws_server.write().await;
    if ws.is_some() {
        return Ok("WebSocket server already running".to_string());
    }

    let sync_engine = state.sync_engine.clone();
    let encryption = Arc::new(state.encryption.clone());
    let file_engine = state.file_engine.clone();
    let pending_tokens = state.pending_tokens.clone();
    let storage = state.storage.clone();
    let automation_engine = state.automation_engine.clone();
    let server = crate::server::WsServer::new("0.0.0.0:9527".to_string(), sync_engine, encryption, file_engine, pending_tokens, storage, automation_engine, state.audio_stream.clone()).await;
    *ws = Some(server);
    Ok("WebSocket server started on port 9527".to_string())
}

#[tauri::command]
pub async fn stop_ws_server(state: State<'_, ManagedState>) -> Result<String, String> {
    let mut ws = state.ws_server.write().await;
    if let Some(server) = ws.take() {
        server.shutdown();
        Ok("WebSocket server stopped".to_string())
    } else {
        Ok("WebSocket server not running".to_string())
    }
}

#[tauri::command]
pub async fn start_discovery(state: State<'_, ManagedState>) -> Result<String, String> {
    let mut disc = state.discovery.write().await;
    if disc.is_some() {
        return Ok("Discovery already running".to_string());
    }

    let mut discovery = crate::discovery::DiscoveryService::new(state.device_id.clone());
    discovery.start().await;
    *disc = Some(discovery);
    Ok("mDNS discovery started".to_string())
}

#[tauri::command]
pub async fn stop_discovery(state: State<'_, ManagedState>) -> Result<String, String> {
    let mut disc = state.discovery.write().await;
    if let Some(mut discovery) = disc.take() {
        discovery.stop();
        Ok("mDNS discovery stopped".to_string())
    } else {
        Ok("Discovery not running".to_string())
    }
}

#[tauri::command]
pub async fn generate_pairing_token(state: State<'_, ManagedState>) -> Result<String, String> {
    log::info!("Generating pairing token...");
    let token = crate::encryption::generate_token_hex();
    log::info!("Generated token: {}...", &token[..8.min(token.len())]);
    state.pending_tokens.write().await.insert(token.clone());
    log::info!("Token added to pending set, total: {}", state.pending_tokens.read().await.len());
    Ok(token)
}

#[tauri::command]
pub fn generate_qr_code(data: String) -> Result<String, String> {
    use qrcode::QrCode;
    use qrcode::render::svg;
    use base64::Engine;

    let code = QrCode::new(data.as_bytes()).map_err(|e| format!("QR code error: {}", e))?;
    let svg_string = code.render::<svg::Color>().module_dimensions(4, 4).build();
    let encoded = base64::engine::general_purpose::STANDARD.encode(svg_string.as_bytes());
    Ok(format!("data:image/svg+xml;base64,{}", encoded))
}

#[tauri::command]
pub async fn send_notification(
    state: State<'_, ManagedState>,
    device_id: String,
    app: String,
    title: String,
    body: String,
) -> Result<(), String> {
    let notification_id = uuid::Uuid::new_v4().to_string();
    let timestamp = chrono::Utc::now().timestamp();

    state
        .storage
        .save_notification(&crate::storage::NotificationParams {
            id: &notification_id,
            device_id: &device_id,
            app: &app,
            title: &title,
            body: &body,
            timestamp,
            actions: None,
        })
        .map_err(|e| e.to_string())?;

    // Also broadcast to connected WebSocket clients (e.g. mobile devices)
    let msg = serde_json::json!({
        "type": "notification",
        "action": "post",
        "id": notification_id,
        "device_id": device_id,
        "app": app,
        "title": title,
        "body": body,
        "timestamp": timestamp,
    });
    if let Some(ws) = state.ws_server.read().await.as_ref() {
        ws.broadcast(msg.to_string()).await;
    }

    Ok(())
}

#[tauri::command]
pub async fn dismiss_notification(state: State<'_, ManagedState>, id: String) -> Result<(), String> {
    state
        .storage
        .dismiss_notification(&id)
        .map_err(|e| e.to_string())?;
    // Broadcast dismissal to WS peers
    let msg = serde_json::json!({
        "type": "notification",
        "action": "dismiss",
        "id": id,
    });
    if let Some(ws) = state.ws_server.read().await.as_ref() {
        ws.broadcast(msg.to_string()).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn reply_notification(
    state: State<'_, ManagedState>,
    id: String,
    text: String,
) -> Result<(), String> {
    let timestamp = chrono::Utc::now().timestamp();
    state
        .storage
        .save_notification(&crate::storage::NotificationParams {
            id: &id,
            device_id: &state.device_id,
            app: "reply",
            title: "Reply",
            body: &text,
            timestamp,
            actions: None,
        })
        .map_err(|e| e.to_string())?;
    // Broadcast reply to WS peers
    let msg = serde_json::json!({
        "type": "notification",
        "action": "reply",
        "id": id,
        "text": text,
        "timestamp": timestamp,
    });
    if let Some(ws) = state.ws_server.read().await.as_ref() {
        ws.broadcast(msg.to_string()).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn sync_clipboard(
    state: State<'_, ManagedState>,
    content: String,
    mime: String,
    source_device: String,
) -> Result<(), String> {
    let timestamp = chrono::Utc::now().timestamp();
    state
        .storage
        .save_clipboard(&content, &mime, &source_device, timestamp)
        .map_err(|e| e.to_string())?;
    // Relay to WS peers so other callers of this command propagate
    let msg = serde_json::json!({
        "type": "clipboard",
        "action": "sync",
        "content": content,
        "mime": mime,
        "source_device": source_device,
        "timestamp": timestamp,
    });
    if let Some(ws) = state.ws_server.read().await.as_ref() {
        ws.broadcast(msg.to_string()).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_notifications(
    state: State<'_, ManagedState>,
    limit: Option<i64>,
) -> Result<Vec<serde_json::Value>, String> {
    let limit = limit.unwrap_or(50);
    let notifications = state
        .storage
        .get_notifications(limit)
        .map_err(|e| e.to_string())?;

    Ok(notifications
        .into_iter()
        .map(|n| {
            serde_json::json!({
                "id": n.id,
                "device_id": n.device_id,
                "app": n.app,
                "title": n.title,
                "body": n.body,
                "timestamp": n.timestamp,
                "actions": n.actions,
                "dismissed": n.dismissed,
            })
        })
        .collect())
}

#[tauri::command]
pub async fn get_device_info(state: State<'_, ManagedState>) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({
        "device_id": state.device_id,
        "public_key": state.encryption.public_key_hex(),
        "name": gethostname::gethostname().to_string_lossy().to_string(),
        "type": "desktop",
        "os": std::env::consts::OS,
    }))
}

#[tauri::command]
pub async fn get_connected_devices(
    state: State<'_, ManagedState>,
) -> Result<Vec<serde_json::Value>, String> {
    let engine = state.sync_engine.read().await;
    Ok(engine
        .connected_clients
        .values()
        .map(|c| {
            serde_json::json!({
                "device_id": c.device_id,
                "device_name": c.device_name,
                "device_type": c.device_type,
                "last_heartbeat": c.last_heartbeat,
            })
        })
        .collect())
}

#[tauri::command]
pub async fn send_encrypted_message(
    state: State<'_, ManagedState>,
    target_device_id: String,
    plaintext: String,
) -> Result<(), String> {
    let engine = state.sync_engine.read().await;
    let client = engine
        .get_client(&target_device_id)
        .ok_or_else(|| "Device not found".to_string())?;

    let shared_secret_bytes = hex::decode(&client.shared_secret)
        .map_err(|e| format!("Invalid shared secret: {}", e))?;
    if shared_secret_bytes.len() != 32 {
        return Err(format!(
            "Invalid shared secret length: expected 32 bytes, got {}",
            shared_secret_bytes.len()
        ));
    }
    let mut secret_array = [0u8; 32];
    secret_array.copy_from_slice(&shared_secret_bytes);

    let (nonce, ciphertext) = state.encryption.encrypt(&secret_array, &plaintext)
        .map_err(|e| format!("Encryption failed: {}", e))?;

    let encrypted_msg = serde_json::json!({
        "type": "encrypted",
        "nonce": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &nonce),
        "ciphertext": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &ciphertext),
        "from_device": state.device_id,
        "to_device": target_device_id,
    });

    drop(engine);

    if let Some(ws) = state.ws_server.read().await.as_ref() {
        if !ws.send_to(&target_device_id, encrypted_msg.to_string()).await {
            warn!("Encrypted message: target device {} not connected, dropping", target_device_id);
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn get_system_info() -> Result<SystemInfo, String> {
    let sys = sysinfo::System::new_all();
    let total = sys.total_memory() as f64 / 1024.0 / 1024.0;
    let used = sys.used_memory() as f64 / 1024.0 / 1024.0;
    let percent = if total > 0.0 { (used / total) * 100.0 } else { 0.0 };

    Ok(SystemInfo {
        ram_used_mb: (used * 10.0).round() / 10.0,
        ram_total_mb: (total * 10.0).round() / 10.0,
        ram_percent: (percent * 10.0).round() / 10.0,
    })
}

#[tauri::command]
pub async fn get_settings(state: State<'_, ManagedState>) -> Result<ConduitSettings, String> {
    state.storage.get_settings().map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn save_settings(
    state: State<'_, ManagedState>,
    settings: ConduitSettings,
) -> Result<(), String> {
    state.storage.save_settings(&settings).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn delete_paired_device(state: State<'_, ManagedState>, id: String) -> Result<(), String> {
    state.storage.delete_device(&id).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn send_file(
    state: State<'_, ManagedState>,
    target_device_id: String,
    file_path: String,
) -> Result<serde_json::Value, String> {
    let transfer_id = uuid::Uuid::new_v4().to_string();
    let file_engine = state.file_engine.clone();
    let (size, name, mime, total_chunks) = file_engine
        .start_outgoing(&transfer_id, &file_path, &target_device_id)
        .await?;

    let timestamp = chrono::Utc::now().timestamp();
    let transfer_id_for_ws = transfer_id.clone();

    // Save to database
    state
        .storage
        .save_file_transfer(&crate::storage::FileTransferParams {
            id: &transfer_id,
            name: &name,
            size: size as i64,
            mime: &mime,
            from_device: &state.device_id,
            to_device: &target_device_id,
            status: "pending",
            chunks_received: 0,
            total_chunks: total_chunks as i32,
            saved_path: None,
            timestamp,
        })
        .map_err(|e| e.to_string())?;

    // Send file request to target device
    let request = serde_json::json!({
        "type": "file",
        "action": "request",
        "id": transfer_id_for_ws,
        "name": name,
        "size": size,
        "mime": mime,
        "from": state.device_id,
        "to": target_device_id,
    });

    if let Some(ws) = state.ws_server.read().await.as_ref() {
        ws.broadcast(request.to_string()).await;
    }

    // Spawn a task to send chunks after a short delay (waiting for accept)
    let ws_server = state.ws_server.clone();
    let engine = file_engine;
    tokio::spawn(async move {
        // Wait for accept message (poll every 500ms for up to 30 seconds)
        for _ in 0..60 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            if let Some((idx, total, chunk_data)) = engine.get_next_chunk(&transfer_id_for_ws).await {
                let chunk_msg = serde_json::json!({
                    "type": "file",
                    "action": "chunk",
                    "id": transfer_id_for_ws,
                    "index": idx,
                    "total": total,
                    "data": chunk_data,
                });
                if let Some(ws) = ws_server.read().await.as_ref() {
                    ws.broadcast(chunk_msg.to_string()).await;
                }
            } else {
                break;
            }
        }
        engine.remove_outgoing(&transfer_id_for_ws).await;
    });

    Ok(serde_json::json!({
        "transfer_id": transfer_id.clone(),
        "name": name,
        "size": size,
        "mime": mime,
        "total_chunks": total_chunks,
    }))
}

#[tauri::command]
pub async fn accept_file_transfer(
    state: State<'_, ManagedState>,
    id: String,
) -> Result<(), String> {
    let accept = serde_json::json!({
        "type": "file",
        "action": "accept",
        "id": id,
    });

    state
        .storage
        .update_file_transfer_progress(&id, "transferring", 0, None)
        .map_err(|e| e.to_string())?;

    if let Some(ws) = state.ws_server.read().await.as_ref() {
        ws.broadcast(accept.to_string()).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn cancel_file_transfer(
    state: State<'_, ManagedState>,
    id: String,
) -> Result<(), String> {
    let cancel = serde_json::json!({
        "type": "file",
        "action": "cancel",
        "id": id,
        "reason": "User cancelled",
    });

    state
        .storage
        .update_file_transfer_progress(&id, "cancelled", 0, None)
        .map_err(|e| e.to_string())?;

    if let Some(ws) = state.ws_server.read().await.as_ref() {
        ws.broadcast(cancel.to_string()).await;
    }
    Ok(())
}

#[tauri::command]
pub async fn get_file_transfers(
    state: State<'_, ManagedState>,
    limit: Option<i64>,
) -> Result<Vec<crate::file_transfer::FileTransferInfo>, String> {
    let limit = limit.unwrap_or(50);
    state
        .storage
        .get_file_transfers(limit)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn resume_file_transfer(
    state: State<'_, ManagedState>,
    id: String,
    name: String,
    size: u64,
    mime: String,
    from_device: String,
) -> Result<serde_json::Value, String> {
    let loaded = state
        .file_engine
        .resume_incoming(&id, &name, size, &mime, &from_device)
        .await
        .map_err(|e| e.to_string())?;

    state
        .storage
        .update_file_transfer_progress(&id, "transferring", loaded as i32, None)
        .map_err(|e| e.to_string())?;

    Ok(serde_json::json!({
        "transfer_id": id,
        "chunks_loaded": loaded,
    }))
}

#[tauri::command]
pub async fn get_downloads_path() -> Result<String, String> {
    let engine = crate::file_transfer::FileTransferEngine::new();
    Ok(engine.get_downloads_path())
}

#[tauri::command]
pub async fn get_automation_rules(
    state: State<'_, ManagedState>,
) -> Result<Vec<AutomationRule>, String> {
    state
        .storage
        .get_all_automation_rules()
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn create_automation_rule(
    state: State<'_, ManagedState>,
    name: String,
    trigger: TriggerType,
    action: ActionType,
) -> Result<AutomationRule, String> {
    let rule = AutomationRule {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        enabled: true,
        trigger,
        action,
    };

    automation::validate_rule(&rule)?;

    state
        .storage
        .save_automation_rule(&rule)
        .map_err(|e| e.to_string())?;

    state.automation_engine.write().await.add_rule(rule.clone());
    Ok(rule)
}

#[tauri::command]
pub async fn update_automation_rule(
    state: State<'_, ManagedState>,
    rule: AutomationRule,
) -> Result<(), String> {
    automation::validate_rule(&rule)?;

    state
        .storage
        .save_automation_rule(&rule)
        .map_err(|e| e.to_string())?;

    state.automation_engine.write().await.update_rule(rule);
    Ok(())
}

#[tauri::command]
pub async fn delete_automation_rule(
    state: State<'_, ManagedState>,
    id: String,
) -> Result<(), String> {
    state
        .storage
        .delete_automation_rule(&id)
        .map_err(|e| e.to_string())?;

    state.automation_engine.write().await.remove_rule(&id);
    Ok(())
}

#[tauri::command]
pub async fn toggle_automation_rule(
    state: State<'_, ManagedState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    let rules = state
        .storage
        .get_all_automation_rules()
        .map_err(|e| e.to_string())?;

    if let Some(mut rule) = rules.into_iter().find(|r| r.id == id) {
        rule.enabled = enabled;
        state
            .storage
            .save_automation_rule(&rule)
            .map_err(|e| e.to_string())?;
        state.automation_engine.write().await.update_rule(rule);
    }
    Ok(())
}

#[tauri::command]
pub async fn execute_automation_action(
    state: State<'_, ManagedState>,
    app: tauri::AppHandle,
    action: ActionType,
) -> Result<automation::RuleExecutionLog, String> {
    if !automation::is_desktop_executable(&action) {
        return Err(format!(
            "cannot execute '{}' on desktop",
            automation::action_tag(&action)
        ));
    }

    let mut log = automation::execute_action(&action);

    if let Some(window) = app.get_webview_window("main") {
        if let ActionType::SetWindowState { state: window_state } = &action {
            let result = match window_state.as_str() {
                "minimize" => window.minimize().map_err(|e| e.to_string()),
                "maximize" => {
                    if window.is_maximized().unwrap_or(false) {
                        window.unmaximize().map_err(|e| e.to_string())
                    } else {
                        window.maximize().map_err(|e| e.to_string())
                    }
                }
                "restore" => window.unminimize().map_err(|e| e.to_string()),
                "close" => window.close().map_err(|e| e.to_string()),
                _ => return Err(format!("Unknown window state: {}", window_state)),
            };
            log.success = result.is_ok();
            if let Err(e) = result {
                log.message = Some(e);
            }
        }
    }

    let _ = state
        .storage
        .log_automation_execution(
            &log.id,
            &log.trigger_type,
            log.timestamp,
            log.success,
            log.message.as_deref(),
        );

    Ok(log)
}

#[tauri::command]
pub async fn get_automation_logs(
    state: State<'_, ManagedState>,
    limit: Option<i64>,
) -> Result<Vec<automation::RuleExecutionLog>, String> {
    let limit = limit.unwrap_or(50);
    state
        .storage
        .get_automation_logs(limit)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn start_audio_stream(state: State<'_, ManagedState>) -> Result<String, String> {
    state.audio_stream.start_capture().await?;
    Ok("Audio stream started".to_string())
}

#[tauri::command]
pub async fn stop_audio_stream(state: State<'_, ManagedState>) -> Result<String, String> {
    state.audio_stream.stop_capture();
    Ok("Audio stream stopped".to_string())
}

#[tauri::command]
pub async fn get_audio_devices() -> Result<serde_json::Value, String> {
    let input = crate::audio::list_input_devices();
    let output = crate::audio::list_output_devices();
    Ok(serde_json::json!({
        "input": input,
        "output": output,
    }))
}
