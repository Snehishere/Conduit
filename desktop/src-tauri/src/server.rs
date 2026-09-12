use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use log::{error, info, warn};
use serde_json::Value;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, RwLock};
use tokio_tungstenite::accept_async;
use tungstenite::Message;

use crate::automation::{self, TriggerContext, TriggerEvent};
use crate::encryption::EncryptionManager;
use crate::file_transfer::FileTransferEngine;
use crate::storage::Storage;
use crate::sync::{ConnectedClient, SyncEngine};
use crate::audio::AudioStream;

type ClientSender = broadcast::Sender<String>;
type Clients = Arc<RwLock<HashMap<String, ClientSender>>>;

struct WsContext {
    clients: Clients,
    sync_engine: Arc<RwLock<SyncEngine>>,
    encryption: Arc<EncryptionManager>,
    file_engine: Arc<FileTransferEngine>,
    pending_tokens: Arc<RwLock<std::collections::HashSet<String>>>,
    storage: Arc<Storage>,
    automation_engine: Arc<RwLock<automation::AutomationEngine>>,
    audio_stream: Arc<AudioStream>,
}

impl Clone for WsContext {
    fn clone(&self) -> Self {
        Self {
            clients: self.clients.clone(),
            sync_engine: self.sync_engine.clone(),
            encryption: self.encryption.clone(),
            file_engine: self.file_engine.clone(),
            pending_tokens: self.pending_tokens.clone(),
            storage: self.storage.clone(),
            automation_engine: self.automation_engine.clone(),
            audio_stream: self.audio_stream.clone(),
        }
    }
}

pub struct WsServer {
    shutdown_tx: Option<tokio::sync::broadcast::Sender<()>>,
    ctx: WsContext,
    app_handle: Option<tauri::AppHandle>,
}

impl WsServer {
    #[allow(clippy::too_many_arguments)]
    pub async fn new(
        addr: String,
        sync_engine: Arc<RwLock<SyncEngine>>,
        encryption: Arc<EncryptionManager>,
        file_engine: Arc<FileTransferEngine>,
        pending_tokens: Arc<RwLock<std::collections::HashSet<String>>>,
        storage: Arc<Storage>,
        automation_engine: Arc<RwLock<automation::AutomationEngine>>,
        audio_stream: Arc<AudioStream>,
    ) -> Self {
        let addr: SocketAddr = match addr.parse() {
            Ok(a) => a,
            Err(e) => {
                error!("Invalid WS address, falling back to 0.0.0.0:9527: {}", e);
                "0.0.0.0:9527".parse().unwrap() // guaranteed valid
            }
        };
        let clients: Clients = Arc::new(RwLock::new(HashMap::new()));
        let (shutdown_tx, _) = broadcast::channel(1);

        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                error!("Failed to bind WS server on {} (another instance running?): {}. Server will start without listening.", addr, e);
                let ctx = WsContext {
                    clients: clients.clone(),
                    sync_engine,
                    encryption,
                    file_engine,
                    pending_tokens,
                    storage,
                    automation_engine,
                    audio_stream,
                };
                return WsServer {
                    shutdown_tx: Some(shutdown_tx),
                    ctx,
                    app_handle: None,
                };
            }
        };
        info!("WebSocket server listening on {}", addr);

        let ctx = WsContext {
            clients: clients.clone(),
            sync_engine,
            encryption,
            file_engine,
            pending_tokens,
            storage,
            automation_engine,
            audio_stream,
        };

        let ctx_clone = ctx.clone();
        let shutdown_rx = shutdown_tx.subscribe();

        // Load paired devices from SQLite into sync engine
        if let Ok(devices) = ctx.storage.get_all_devices() {
            let mut engine = ctx.sync_engine.write().await;
            for device in devices {
                if device.status == "paired" {
                    let connected_client = crate::sync::ConnectedClient {
                        device_id: device.id,
                        device_name: device.name,
                        device_type: device.device_type,
                        shared_secret: device.shared_secret,
                        last_heartbeat: device.last_seen,
                        battery_level: device.battery,
                    };
                    engine.add_client(connected_client);
                }
            }
        }

        tokio::spawn(Self::accept_loop(listener, ctx_clone, shutdown_rx));

        WsServer {
            shutdown_tx: Some(shutdown_tx),
            ctx,
            app_handle: None,
        }
    }

    pub fn set_app_handle(&mut self, app_handle: tauri::AppHandle) {
        self.app_handle = Some(app_handle);
    }

    async fn accept_loop(
        listener: TcpListener,
        ctx: WsContext,
        mut shutdown_rx: broadcast::Receiver<()>,
    ) {
        loop {
            tokio::select! {
                accept = listener.accept() => {
                    match accept {
                        Ok((stream, peer)) => {
                            info!("New connection from {}", peer);
                            let ctx = ctx.clone();
                            let client_id = uuid::Uuid::new_v4().to_string();
                            tokio::spawn(Self::handle_client(stream, ctx, client_id));
                        }
                        Err(e) => error!("Accept error: {}", e),
                    }
                }
                _ = shutdown_rx.recv() => {
                    info!("Server shutting down");
                    break;
                }
            }
        }
    }

    async fn handle_client(stream: TcpStream, ctx: WsContext, client_id: String) {
        let ws_stream = match accept_async(stream).await {
            Ok(ws) => ws,
            Err(e) => {
                error!("WebSocket handshake error: {}", e);
                return;
            }
        };

        let (mut write, mut read) = ws_stream.split();
        let (tx, mut rx) = broadcast::channel(256);

        {
            let mut clients_lock = ctx.clients.write().await;
            clients_lock.insert(client_id.clone(), tx.clone());
        }

        info!("Client registered: {}", client_id);

        let my_id = client_id.clone();
        let broadcast_task = tokio::spawn(async move {
            while let Ok(msg) = rx.recv().await {
                if let Err(e) = write.send(Message::Text(msg)).await {
                    error!("Send error to {}: {}", my_id, e);
                    break;
                }
            }
        });

        while let Some(Ok(msg)) = read.next().await {
            if let Message::Text(text) = msg {
                info!("Received from {}: {}", client_id, &text[..text.len().min(200)]);
                Self::handle_message(&text, &client_id, &ctx).await;
            }
        }

        ctx.clients.write().await.remove(&client_id);
        ctx.sync_engine.write().await.remove_client(&client_id);
        // Fire DeviceDisconnect automation triggers
        let disc_ctx = TriggerContext {
            event: TriggerEvent::DeviceDisconnect,
            source_device_id: client_id.to_string(),
            battery_level: None,
            wifi_ssid: None,
            app_package: None,
        };
        Self::evaluate_device_triggers(&disc_ctx, &ctx).await;
        broadcast_task.abort();
        info!("Client disconnected: {}", client_id);
    }

    async fn evaluate_device_triggers(event_ctx: &TriggerContext, ctx: &WsContext) {
        let engine = ctx.automation_engine.read().await;
        for rule in engine.get_rules() {
            if !rule.enabled {
                continue;
            }
            if engine.evaluate_trigger(&rule.trigger, event_ctx).await {
                if !automation::is_desktop_executable(&rule.action) {
                    continue;
                }
                let log = automation::execute_action(&rule.action);
                let _ = ctx.storage.log_automation_execution(
                    &rule.id,
                    &automation::trigger_tag(&rule.trigger),
                    log.timestamp,
                    log.success,
                    log.message.as_deref(),
                );
                info!("Automation device trigger fired for rule: {}", rule.name);
            }
        }
    }

    async fn handle_message(text: &str, client_id: &str, ctx: &WsContext) {
        let msg: Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(e) => {
                error!("Failed to parse message: {}", e);
                return;
            }
        };

        let msg_type = msg.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let action = msg.get("action").and_then(|v| v.as_str()).unwrap_or("");

        match (msg_type, action) {
            ("pairing", "request") => {
                Self::handle_pairing_request(msg, client_id, ctx).await;
            }
            ("pairing", "accept") => {
                Self::handle_pairing_accept(msg, client_id, ctx).await;
            }
            ("notification", "post") => {
                Self::handle_notification_post(msg, client_id, ctx).await;
            }
            ("notification", "dismiss") => {
                Self::handle_notification_dismiss(msg, client_id, ctx).await;
            }
            ("notification", "reply") => {
                Self::handle_notification_reply(msg, client_id, ctx).await;
            }
            ("clipboard", "sync") => {
                Self::handle_clipboard_sync(msg, client_id, ctx).await;
            }
            ("file", "request") => {
                Self::handle_file_request(msg, client_id, ctx).await;
            }
            ("file", "accept") => {
                Self::handle_file_accept(msg, client_id, ctx).await;
            }
            ("file", "chunk") => {
                Self::handle_file_chunk(msg, client_id, ctx).await;
            }
            ("file", "progress") => {
                Self::handle_file_progress(msg, client_id, ctx).await;
            }
            ("file", "complete") => {
                Self::handle_file_complete(msg, client_id, ctx).await;
            }
            ("file", "cancel") => {
                Self::handle_file_cancel(msg, client_id, ctx).await;
            }
            ("file", "resume") => {
                Self::handle_file_resume(msg, client_id, ctx).await;
            }
            ("sms", _) => {
                Self::handle_sms(msg, client_id, ctx).await;
            }
            ("call", _) => {
                Self::handle_call(msg, client_id, ctx).await;
            }
            ("audio", _) => {
                Self::handle_audio(msg, client_id, ctx).await;
            }
            ("status", _) => {
                Self::handle_status_update(msg, client_id, ctx).await;
            }
            ("automation", "rule") => {
                Self::handle_automation_rule(msg, ctx).await;
            }
            ("automation", "delete") => {
                Self::handle_automation_delete(msg, ctx).await;
            }
            ("automation", "sync") => {
                Self::handle_automation_sync(msg, ctx).await;
            }
            ("automation", "triggered") => {
                Self::handle_automation_triggered(msg, ctx).await;
            }
            ("automation", "") => {
                // Duplicate-key wire form: the outer `action` slot holds the rule
                // action object, leaving no action string. Detect rule packets
                // structurally and treat them as single-rule upserts.
                if msg.get("trigger").is_some() {
                    Self::handle_automation_rule(msg, ctx).await;
                } else {
                    warn!("Automation: unknown action in packet");
                }
            }
            ("screen_mirror", _) => {
                Self::handle_relay(msg, client_id, ctx).await;
            }
            ("remote_input", _) => {
                Self::handle_relay(msg, client_id, ctx).await;
            }
            ("tv", _) => {
                Self::handle_relay(msg, client_id, ctx).await;
            }
            ("watch", _) => {
                Self::handle_relay(msg, client_id, ctx).await;
            }
            _ => {
                warn!("Unknown message type: {} action: {}", msg_type, action);
            }
        }
    }

    async fn handle_pairing_request(msg: Value, client_id: &str, ctx: &WsContext) {
        let peer_public_key = msg.get("public_key").and_then(|v| v.as_str()).unwrap_or("");
        let token = msg.get("token").and_then(|v| v.as_str()).unwrap_or("");
        let device_info = msg.get("device_info").cloned().unwrap_or(Value::Null);

        let device_name = device_info.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown Device");
        let device_type = device_info.get("type").and_then(|v| v.as_str()).unwrap_or("phone");

        let token_valid = ctx.pending_tokens.read().await.contains(token);
        if !token_valid {
            warn!("Pairing rejected: invalid or expired token from {}", client_id);
            return;
        }
        ctx.pending_tokens.write().await.remove(token);
        info!("Pairing accepted for device: {} ({})", device_name, client_id);

        let shared_secret = match ctx.encryption.derive_shared_secret(peer_public_key) {
            Ok(s) => s,
            Err(e) => {
                warn!("Pairing failed: invalid public key from {}: {}", client_id, e);
                return;
            }
        };
        let shared_secret_hex = hex::encode(shared_secret);

        // Deduplicate: reuse existing device row if public_key already paired.
        let stable_id = ctx
            .storage
            .get_all_devices()
            .unwrap_or_default()
            .into_iter()
            .find(|d| d.public_key == peer_public_key)
            .map(|d| d.id)
            .unwrap_or_else(|| client_id.to_string());

        let connected_client = ConnectedClient {
            device_id: stable_id.clone(),
            device_name: device_name.to_string(),
            device_type: device_type.to_string(),
            shared_secret: shared_secret_hex.clone(),
            last_heartbeat: chrono::Utc::now().timestamp(),
            battery_level: None,
        };

        ctx.sync_engine.write().await.add_client(connected_client);

        // Persist paired device to database
        let now = chrono::Utc::now().timestamp();
        let stored_device = crate::storage::StoredDevice {
            id: stable_id.clone(),
            name: device_name.to_string(),
            device_type: device_type.to_string(),
            os: "android".to_string(),
            public_key: peer_public_key.to_string(),
            shared_secret: shared_secret_hex,
            paired_at: now,
            last_seen: now,
            battery: None,
            signal: None,
            status: "paired".to_string(),
        };
        if let Err(e) = ctx.storage.save_device(&stored_device) {
            error!("Failed to persist paired device {}: {}", stable_id, e);
        }

        let response = serde_json::json!({
            "type": "pairing",
            "action": "accept",
            "public_key": ctx.encryption.public_key_hex(),
            "device_info": {
                "name": "Conduit Desktop",
                "type": "desktop",
                "os": std::env::consts::OS,
                "battery": 100,
            }
        });

        let clients_lock = ctx.clients.read().await;
        if let Some(tx) = clients_lock.get(client_id) {
            let _ = tx.send(response.to_string());
        }

        // Fire DeviceConnect automation triggers
        let connect_ctx = TriggerContext {
            event: TriggerEvent::DeviceConnect,
            source_device_id: stable_id.clone(),
            battery_level: None,
            wifi_ssid: None,
            app_package: None,
        };
        Self::evaluate_device_triggers(&connect_ctx, ctx).await;

        info!("Device paired: {} ({})", device_name, stable_id);
    }

    async fn handle_pairing_accept(msg: Value, client_id: &str, ctx: &WsContext) {
        let peer_public_key = msg.get("public_key").and_then(|v| v.as_str()).unwrap_or("");
        let device_info = msg.get("device_info").cloned().unwrap_or(Value::Null);

        let device_name = device_info.get("name").and_then(|v| v.as_str()).unwrap_or("Unknown Device");
        let device_type = device_info.get("type").and_then(|v| v.as_str()).unwrap_or("phone");
        let _os = device_info.get("os").and_then(|v| v.as_str()).unwrap_or("android");
        let _battery = device_info.get("battery").and_then(|v| v.as_i64()).unwrap_or(100) as i32;

        let shared_secret = match ctx.encryption.derive_shared_secret(peer_public_key) {
            Ok(s) => s,
            Err(e) => {
                warn!("Pairing accept failed: invalid public key from {}: {}", client_id, e);
                return;
            }
        };
        let shared_secret_hex = hex::encode(shared_secret);

        let stable_id = ctx
            .storage
            .get_all_devices()
            .unwrap_or_default()
            .into_iter()
            .find(|d| d.public_key == peer_public_key)
            .map(|d| d.id)
            .unwrap_or_else(|| client_id.to_string());

        let connected_client = ConnectedClient {
            device_id: stable_id.clone(),
            device_name: device_name.to_string(),
            device_type: device_type.to_string(),
            shared_secret: shared_secret_hex.clone(),
            last_heartbeat: chrono::Utc::now().timestamp(),
            battery_level: Some(_battery),
        };

        ctx.sync_engine.write().await.add_client(connected_client);

        // Persist paired device to database
        let now = chrono::Utc::now().timestamp();
        let stored_device = crate::storage::StoredDevice {
            id: stable_id.clone(),
            name: device_name.to_string(),
            device_type: device_type.to_string(),
            os: _os.to_string(),
            public_key: peer_public_key.to_string(),
            shared_secret: shared_secret_hex,
            paired_at: now,
            last_seen: now,
            battery: Some(_battery),
            signal: None,
            status: "paired".to_string(),
        };
        if let Err(e) = ctx.storage.save_device(&stored_device) {
            error!("Failed to persist paired device {}: {}", stable_id, e);
        }

        // Fire DeviceConnect automation triggers
        let connect_ctx = TriggerContext {
            event: TriggerEvent::DeviceConnect,
            source_device_id: stable_id.clone(),
            battery_level: Some(_battery),
            wifi_ssid: None,
            app_package: None,
        };
        Self::evaluate_device_triggers(&connect_ctx, ctx).await;

        info!("Pairing accepted from: {} ({})", device_name, stable_id);
    }

    async fn handle_notification_post(msg: Value, client_id: &str, ctx: &WsContext) {
        // Persist the notification
        let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let device_id = msg.get("device_id").and_then(|v| v.as_str()).unwrap_or(client_id);
        let app = msg.get("app").and_then(|v| v.as_str()).unwrap_or("unknown");
        let title = msg.get("title").and_then(|v| v.as_str()).unwrap_or("");
        let body = msg.get("body").and_then(|v| v.as_str()).unwrap_or("");
        let timestamp = msg.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);

        if !id.is_empty() {
            if let Err(e) = ctx.storage.save_notification(&crate::storage::NotificationParams {
                id,
                device_id,
                app,
                title,
                body,
                timestamp,
                actions: None,
            }) {
                warn!("Failed to persist notification from {}: {}", client_id, e);
            }
        }

        // Forward to other clients
        let clients_lock = ctx.clients.read().await;
        for (id, client_tx) in clients_lock.iter() {
            if id != client_id {
                let _ = client_tx.send(msg.to_string());
            }
        }
    }

    async fn handle_notification_dismiss(msg: Value, client_id: &str, ctx: &WsContext) {
        let clients_lock = ctx.clients.read().await;
        for (id, client_tx) in clients_lock.iter() {
            if id != client_id {
                let _ = client_tx.send(msg.to_string());
            }
        }
    }

    async fn handle_notification_reply(msg: Value, client_id: &str, ctx: &WsContext) {
        let clients_lock = ctx.clients.read().await;
        for (id, client_tx) in clients_lock.iter() {
            if id != client_id {
                let _ = client_tx.send(msg.to_string());
            }
        }
    }

    async fn handle_clipboard_sync(msg: Value, client_id: &str, ctx: &WsContext) {
        let clients_lock = ctx.clients.read().await;
        for (id, client_tx) in clients_lock.iter() {
            if id != client_id {
                let _ = client_tx.send(msg.to_string());
            }
        }
    }

    async fn handle_sms(msg: Value, client_id: &str, ctx: &WsContext) {
        let clients_lock = ctx.clients.read().await;
        for (id, client_tx) in clients_lock.iter() {
            if id != client_id {
                let _ = client_tx.send(msg.to_string());
            }
        }
    }

    async fn handle_call(msg: Value, client_id: &str, ctx: &WsContext) {
        let clients_lock = ctx.clients.read().await;
        for (id, client_tx) in clients_lock.iter() {
            if id != client_id {
                let _ = client_tx.send(msg.to_string());
            }
        }
    }

    async fn handle_audio(msg: Value, client_id: &str, ctx: &WsContext) {
        let action = msg.get("action").and_then(|v| v.as_str()).unwrap_or("");

        match action {
            "stream_start" => {
                info!("Audio stream started from {}", client_id);
                // Start capturing and streaming audio to this client
                let audio_stream = ctx.audio_stream.clone();
                let ctx_clone = ctx.clone();
                let client_id_clone = client_id.to_string();

                tokio::spawn(async move {
                    if let Err(e) = audio_stream.start_capture().await {
                        error!("Failed to start audio capture: {}", e);
                        return;
                    }

                    let mut rx = audio_stream.subscribe();
                    while let Ok(pcm_data) = rx.recv().await {
                        // Convert i16 PCM to bytes for base64 encoding
                        let mut bytes = Vec::with_capacity(pcm_data.len() * 2);
                        for sample in &pcm_data {
                            bytes.extend_from_slice(&sample.to_le_bytes());
                        }

                        let msg = serde_json::json!({
                            "type": "audio",
                            "action": "stream_data",
                            "data": base64::Engine::encode(
                                &base64::engine::general_purpose::STANDARD,
                                &bytes
                            ),
                            "format": "pcm16",
                            "sample_rate": 16000,
                            "channels": 1,
                            "from": client_id_clone,
                        });

                        let clients_lock = ctx_clone.clients.read().await;
                        for (id, client_tx) in clients_lock.iter() {
                            if id != &client_id_clone {
                                let _ = client_tx.send(msg.to_string());
                            }
                        }
                    }
                });

                // Notify the sender that streaming started
                let response = serde_json::json!({
                    "type": "audio",
                    "action": "stream_started",
                    "from": client_id,
                });
                let clients_lock = ctx.clients.read().await;
                if let Some(tx) = clients_lock.get(client_id) {
                    let _ = tx.send(response.to_string());
                }
            }
            "stream_stop" => {
                info!("Audio stream stopped from {}", client_id);
                ctx.audio_stream.stop_capture();

                // Notify other clients
                let clients_lock = ctx.clients.read().await;
                for (id, client_tx) in clients_lock.iter() {
                    if id != client_id {
                        let _ = client_tx.send(msg.to_string());
                    }
                }
            }
            "stream_data" => {
                // Received PCM audio data from another device - play it
                if let Some(data) = msg.get("data").and_then(|v| v.as_str()) {
                    if let Ok(bytes) = base64::Engine::decode(
                        &base64::engine::general_purpose::STANDARD,
                        data,
                    ) {
                        // Convert bytes to i16 PCM samples (drop trailing odd byte, log it)
                        if bytes.len() % 2 != 0 {
                            warn!("Audio stream_data: odd byte length {}, dropping last byte", bytes.len());
                        }
                        let pcm_data: Vec<i16> = bytes
                            .as_chunks::<2>()
                            .0
                            .iter()
                            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
                            .collect();

                        let audio_stream = ctx.audio_stream.clone();
                        tokio::spawn(async move {
                            if let Err(e) = audio_stream.play_audio(&pcm_data).await {
                                error!("Failed to play audio: {}", e);
                            }
                        });
                    }
                }
            }
            "route" => {
                // Legacy route signaling - relay to other clients
                let clients_lock = ctx.clients.read().await;
                for (id, client_tx) in clients_lock.iter() {
                    if id != client_id {
                        let _ = client_tx.send(msg.to_string());
                    }
                }
            }
            _ => {
                // Unknown audio action - relay to other clients
                let clients_lock = ctx.clients.read().await;
                for (id, client_tx) in clients_lock.iter() {
                    if id != client_id {
                        let _ = client_tx.send(msg.to_string());
                    }
                }
            }
        }
    }

    async fn handle_status_update(msg: Value, client_id: &str, ctx: &WsContext) {
        let battery = msg.get("battery").and_then(|v| v.as_i64());

        {
            let mut engine = ctx.sync_engine.write().await;
            if let Some(client) = engine.connected_clients.get_mut(client_id) {
                if let Some(b) = battery {
                    client.battery_level = Some(b as i32);
                }
                client.last_heartbeat = chrono::Utc::now().timestamp();
            }
        }

        if let Some(b) = battery {
            let context = TriggerContext {
                event: TriggerEvent::BatteryUpdate,
                source_device_id: client_id.to_string(),
                battery_level: Some(b as i32),
                wifi_ssid: None,
                app_package: None,
            };
            let engine = ctx.automation_engine.read().await;
            for rule in engine.get_rules() {
                if rule.enabled && engine.evaluate_trigger(&rule.trigger, &context).await {
                    if !automation::is_desktop_executable(&rule.action) {
                        continue;
                    }
                    let log = automation::execute_action(&rule.action);
                    let _ = ctx.storage.log_automation_execution(
                        &rule.id,
                        "battery_level",
                        log.timestamp,
                        log.success,
                        log.message.as_deref(),
                    );
                }
            }
        }
    }

    async fn handle_automation_rule(msg: Value, ctx: &WsContext) {
        let rule = match automation::parse_rule_packet(&msg) {
            Ok(r) => r,
            Err(e) => {
                warn!("Automation rule rejected: {}", e);
                return;
            }
        };
        if let Err(e) = automation::validate_rule(&rule) {
            warn!("Automation rule rejected ({}): {}", rule.id, e);
            return;
        }
        if let Err(e) = ctx.storage.save_automation_rule(&rule) {
            error!("Automation rule: failed to save {}: {}", rule.id, e);
            return;
        }
        let reloaded = ctx.storage.get_all_automation_rules().unwrap_or_default();
        let mut engine = ctx.automation_engine.write().await;
        engine.update_rule(rule.clone());
        engine.load_rules(reloaded);
        info!("Automation rule upserted: {} '{}'", rule.id, rule.name);
    }

    async fn handle_automation_delete(msg: Value, ctx: &WsContext) {
        let rule_id = msg
            .get("rule_id")
            .and_then(|v| v.as_str())
            .or_else(|| msg.get("id").and_then(|v| v.as_str()))
            .or_else(|| msg.get("rule").and_then(|r| r.get("id")).and_then(|v| v.as_str()))
            .unwrap_or("");
        if rule_id.is_empty() {
            warn!("Automation delete: missing rule_id");
            return;
        }
        if let Err(e) = ctx.storage.delete_automation_rule(rule_id) {
            warn!("Automation delete: failed to delete {}: {}", rule_id, e);
            return;
        }
        ctx.automation_engine.write().await.remove_rule(rule_id);
        info!("Automation rule deleted: {}", rule_id);
    }

    async fn handle_automation_sync(msg: Value, ctx: &WsContext) {
        let Some(rules_json) = msg.get("rules") else {
            warn!("Automation sync: missing rules array");
            return;
        };
        let rules: Vec<Value> = match rules_json.as_array() {
            Some(arr) => arr.clone(),
            None => {
                error!("Automation sync: rules must be an array");
                return;
            }
        };

        let mut incoming_ids = Vec::new();
        for value in rules {
            let rule = match automation::parse_rule_packet(&value) {
                Ok(r) => r,
                Err(e) => {
                    warn!("Automation sync: skipping invalid rule: {}", e);
                    continue;
                }
            };
            if let Err(e) = automation::validate_rule(&rule) {
                warn!("Automation sync: skipping invalid rule {}: {}", rule.id, e);
                continue;
            }
            incoming_ids.push(rule.id.clone());
            if let Err(e) = ctx.storage.save_automation_rule(&rule) {
                error!("Automation sync: failed to save rule {}: {}", rule.id, e);
            }
        }

        for existing in ctx.storage.get_all_automation_rules().unwrap_or_default() {
            if !incoming_ids.contains(&existing.id) {
                // Only prune on explicit full syncs — partial/mobile-only syncs must not wipe desktop rules.
                let full_sync = msg.get("full_sync").and_then(|v| v.as_bool()).unwrap_or(false);
                if full_sync {
                    let _ = ctx.storage.delete_automation_rule(&existing.id);
                } else {
                    info!("Automation sync: keeping existing rule {} (not in partial sync)", existing.id);
                }
            }
        }

        let reloaded = ctx.storage.get_all_automation_rules().unwrap_or_default();
        let rule_count = reloaded.len();
        let mut engine = ctx.automation_engine.write().await;
        engine.load_rules(reloaded);
        info!("Automation rules synced: {} rules", rule_count);
    }

    async fn handle_automation_triggered(msg: Value, ctx: &WsContext) {
        let rule_id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let trigger_type = msg.get("trigger_type").and_then(|v| v.as_str()).unwrap_or("");
        let device_id = msg.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
        if rule_id.is_empty() || trigger_type.is_empty() {
            warn!("Automation triggered: missing id or trigger_type");
            return;
        }

        let battery = if device_id.is_empty() {
            None
        } else {
            ctx.sync_engine
                .read()
                .await
                .connected_clients
                .get(device_id)
                .and_then(|c| c.battery_level)
        };

        let engine = ctx.automation_engine.read().await;
        let Some(rule) = engine.get_rule(rule_id) else {
            warn!("Automation triggered: unknown rule {}", rule_id);
            return;
        };

        if !automation::re_evaluate_rule(rule, trigger_type, device_id, battery) {
            info!(
                "Automation triggered: rule {} skipped (not enabled / tag or scope mismatch)",
                rule_id
            );
            return;
        }

        let timestamp = chrono::Utc::now().timestamp();
        if automation::is_desktop_executable(&rule.action) {
            let log = automation::execute_action(&rule.action);
            let _ = ctx.storage.log_automation_execution(
                &rule.id,
                trigger_type,
                log.timestamp,
                log.success,
                log.message.as_deref(),
            );
            info!("Automation: rule {} executed ({})", rule_id, trigger_type);
        } else {
            let _ = ctx.storage.log_automation_execution(
                &rule.id,
                trigger_type,
                timestamp,
                true,
                Some("action executes on the target phone/tablet; desktop logged only"),
            );
            info!(
                "Automation: rule {} targets a remote device ({}), no desktop action",
                rule_id,
                automation::action_tag(&rule.action)
            );
        }
    }

    async fn handle_file_request(msg: Value, client_id: &str, ctx: &WsContext) {
        let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let name = msg.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
        let size = msg.get("size").and_then(|v| v.as_i64()).unwrap_or(0) as u64;
        let mime = msg.get("mime").and_then(|v| v.as_str()).unwrap_or("application/octet-stream");
        let from = msg.get("from").and_then(|v| v.as_str()).unwrap_or("");

        ctx.file_engine.start_incoming(id, name, size, mime, from).await;

        // Forward the request to the target (or broadcast if no specific target)
        let to = msg.get("to").and_then(|v| v.as_str()).unwrap_or("");
        let clients_lock = ctx.clients.read().await;
        if to.is_empty() {
            for (cid, client_tx) in clients_lock.iter() {
                if cid != client_id {
                    let _ = client_tx.send(msg.to_string());
                }
            }
        } else if let Some(client_tx) = clients_lock.get(to) {
            let _ = client_tx.send(msg.to_string());
        }
    }

    async fn handle_file_accept(msg: Value, client_id: &str, ctx: &WsContext) {
        let clients_lock = ctx.clients.read().await;
        for (id, client_tx) in clients_lock.iter() {
            if id != client_id {
                let _ = client_tx.send(msg.to_string());
            }
        }
    }

    async fn handle_file_chunk(msg: Value, client_id: &str, ctx: &WsContext) {
        let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let index = msg.get("index").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
        let data = msg.get("data").and_then(|v| v.as_str()).unwrap_or("");

        match ctx.file_engine.receive_chunk(id, index, data).await {
            Ok(received) => {
                if ctx.file_engine.is_complete(id).await {
                    match ctx.file_engine.finalize_incoming(id).await {
                        Ok(path) => {
                            info!("File transfer complete: {}", path);
                            let complete_msg = serde_json::json!({
                                "type": "file",
                                "action": "complete",
                                "id": id,
                                "path": path,
                            });
                            let clients_lock = ctx.clients.read().await;
                            for (cid, client_tx) in clients_lock.iter() {
                                if cid != client_id {
                                    let _ = client_tx.send(complete_msg.to_string());
                                }
                            }
                        }
                        Err(e) => error!("Failed to finalize file: {}", e),
                    }
                } else {
                    let progress_msg = serde_json::json!({
                        "type": "file",
                        "action": "progress",
                        "id": id,
                        "percent": (received as f64 / msg.get("total").and_then(|v| v.as_i64()).unwrap_or(1) as f64 * 100.0) as u32,
                    });
                    let clients_lock = ctx.clients.read().await;
                    for (cid, client_tx) in clients_lock.iter() {
                        if cid != client_id {
                            let _ = client_tx.send(progress_msg.to_string());
                        }
                    }
                }
            }
            Err(e) => error!("Failed to receive chunk: {}", e),
        }
    }

    async fn handle_file_progress(msg: Value, client_id: &str, ctx: &WsContext) {
        let clients_lock = ctx.clients.read().await;
        for (id, client_tx) in clients_lock.iter() {
            if id != client_id {
                let _ = client_tx.send(msg.to_string());
            }
        }
    }

    async fn handle_file_complete(msg: Value, client_id: &str, ctx: &WsContext) {
        let clients_lock = ctx.clients.read().await;
        for (id, client_tx) in clients_lock.iter() {
            if id != client_id {
                let _ = client_tx.send(msg.to_string());
            }
        }
    }

    async fn handle_file_cancel(msg: Value, client_id: &str, ctx: &WsContext) {
        let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");
        ctx.file_engine.cancel_incoming(id).await;

        let clients_lock = ctx.clients.read().await;
        for (cid, client_tx) in clients_lock.iter() {
            if cid != client_id {
                let _ = client_tx.send(msg.to_string());
            }
        }
    }

    async fn handle_file_resume(msg: Value, client_id: &str, ctx: &WsContext) {
        let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let name = msg.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
        let size = msg.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
        let mime = msg.get("mime").and_then(|v| v.as_str()).unwrap_or("application/octet-stream");
        let from = msg.get("from").and_then(|v| v.as_str()).unwrap_or("");

        let chunks_loaded = ctx.file_engine
            .resume_incoming(id, name, size, mime, from)
            .await
            .unwrap_or(0);

        info!(
            "File resume requested: {} ({} chunks loaded from disk)",
            name, chunks_loaded
        );

        // Notify the other side how many chunks to skip
        let response = serde_json::json!({
            "type": "file",
            "action": "resume_ack",
            "id": id,
            "chunks_loaded": chunks_loaded,
        });

        let clients_lock = ctx.clients.read().await;
        for (cid, client_tx) in clients_lock.iter() {
            if cid != client_id {
                let _ = client_tx.send(response.to_string());
            }
        }
    }

    async fn handle_relay(msg: Value, client_id: &str, ctx: &WsContext) {
        let clients_lock = ctx.clients.read().await;
        for (id, client_tx) in clients_lock.iter() {
            if id != client_id {
                let _ = client_tx.send(msg.to_string());
            }
        }
    }

    pub async fn broadcast(&self, message: String) {
        let clients = self.ctx.clients.read().await;
        for (id, client_tx) in clients.iter() {
            if let Err(e) = client_tx.send(message.clone()) {
                warn!("Failed to broadcast to {}: {}", id, e);
            }
        }
    }

    pub async fn send_to(&self, device_id: &str, message: String) -> bool {
        let clients = self.ctx.clients.read().await;
        if let Some(client_tx) = clients.get(device_id) {
            if let Err(e) = client_tx.send(message) {
                warn!("Failed to send to {}: {}", device_id, e);
                return false;
            }
            return true;
        }
        false
    }

    pub fn shutdown(&self) {
        if let Some(tx) = &self.shutdown_tx {
            let _ = tx.send(());
        }
    }
}
