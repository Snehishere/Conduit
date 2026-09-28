use crate::AppState;
use crate::error::{ConduitError, Result};
use log::warn;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
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

#[tauri::command]
pub async fn get_devices(state: State<'_, ManagedState>) -> Result<Vec<DeviceResponse>> {
    let storage = state.storage.clone();
    let stored = tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(storage.get_all_devices())
    })
    .await
    .map_err(|e| ConduitError::Other(format!("get_devices task join error: {e}")))??;

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
                status: if is_connected {
                    "connected".to_string()
                } else {
                    d.status
                },
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
pub async fn start_discovery(state: State<'_, ManagedState>) -> Result<String> {
    let mut disc = state.discovery.write().await;
    if disc.is_some() {
        return Ok("Discovery already running".to_string());
    }

    let device_name = state
        .storage
        .get_settings()
        .await
        .map(|s| s.device_name)
        .unwrap_or_else(|_| gethostname::gethostname().to_string_lossy().to_string());
    let mut discovery =
        crate::discovery::DiscoveryService::new(state.device_id.clone(), device_name);
    discovery.start().await;
    *disc = Some(discovery);
    Ok("mDNS discovery started".to_string())
}

#[tauri::command]
pub async fn stop_discovery(state: State<'_, ManagedState>) -> Result<String> {
    let mut disc = state.discovery.write().await;
    if let Some(mut discovery) = disc.take() {
        discovery.stop();
        Ok("mDNS discovery stopped".to_string())
    } else {
        Ok("Discovery not running".to_string())
    }
}

#[tauri::command]
pub async fn generate_pairing_token(state: State<'_, ManagedState>) -> Result<String> {
    // Enforce `max_devices` *before* a pairing token is issued: the token is the
    // credential that lets a new device complete pairing, so refusing here keeps
    // the hub from ever exceeding the user's configured limit.
    let max_devices = match state.storage.get_settings().await {
        Ok(settings) => settings.max_devices,
        Err(e) => {
            // Fall back to the documented default rather than blocking pairing on
            // a transient read failure — the count below still has to succeed.
            warn!("Could not read max_devices, assuming the default: {e}");
            crate::commands::DEFAULT_MAX_DEVICES
        }
    };
    // Fail *closed* if the device count cannot be read: over-pairing is worse
    // than refusing a pairing the user can retry.
    let paired = state
        .storage
        .get_all_devices()
        .await
        .map_err(|e| {
            warn!("Refusing to issue a pairing token: device count unavailable: {e}");
            ConduitError::Storage(format!("Could not verify the device limit: {e}"))
        })?
        .iter()
        .filter(|d| d.status == "paired")
        .count();
    if paired as u32 >= max_devices {
        warn!(
            "Pairing refused: max_devices limit reached ({}/{}). Revoke a device first.",
            paired, max_devices
        );
        return Err(ConduitError::Other(format!(
            "Device limit reached ({paired} of {max_devices}). Unpair a device in Settings → General before pairing another."
        )));
    }

    log::info!("Generating pairing token...");
    use rand::RngExt;
    use rand::distr::Alphanumeric;
    let token: String = rand::rng()
        .sample_iter(&Alphanumeric)
        .take(6)
        .map(char::from)
        .collect::<String>()
        .to_uppercase();

    log::info!("Pairing token generated");
    state.token_store.insert(token.clone()).await;
    log::info!("Token added to store (expires in 60 seconds)");
    Ok(token)
}

#[tauri::command]
pub fn generate_qr_code(data: String) -> Result<String> {
    use base64::Engine;
    use qrcode::QrCode;
    use qrcode::render::svg;

    let code = QrCode::new(data.as_bytes())
        .map_err(|e| ConduitError::Other(format!("QR code error: {}", e)))?;
    let svg_string = code.render::<svg::Color>().module_dimensions(4, 4).build();
    let encoded = base64::engine::general_purpose::STANDARD.encode(svg_string.as_bytes());
    Ok(format!("data:image/svg+xml;base64,{}", encoded))
}

#[tauri::command]
pub async fn get_device_info(state: State<'_, ManagedState>) -> Result<serde_json::Value> {
    Ok(serde_json::json!({
        "device_id": state.device_id,
        "public_key": state.encryption.public_key_hex(),
        "name": gethostname::gethostname().to_string_lossy().to_string(),
        "type": "desktop",
        "os": std::env::consts::OS,
    }))
}

#[tauri::command]
pub async fn send_encrypted_message(
    state: State<'_, ManagedState>,
    target_device_id: String,
    plaintext: String,
) -> Result<()> {
    let engine = state.sync_engine.read().await;
    let client = engine
        .get_client(&target_device_id)
        .ok_or_else(|| ConduitError::DeviceNotFound(target_device_id.clone()))?;

    let (nonce, ciphertext) = state
        .encryption
        .encrypt(&client.shared_secret, &plaintext)?;

    let data_hex = hex::encode(&ciphertext);
    let hmac_hex = state
        .encryption
        .generate_hmac(&client.shared_secret, &data_hex)?;

    let encrypted_msg = serde_json::json!({
        "type": "encrypted",
        "protocol_version": 1,
        "nonce": hex::encode(&nonce),
        "data": data_hex,
        "hmac": hmac_hex,
        "source_device": state.device_id,
    });

    drop(engine);

    if let Some(ws) = state.ws_server.read().await.as_ref()
        && !ws
            .send_to(&target_device_id, encrypted_msg.to_string())
            .await
    {
        warn!(
            "Encrypted message: target device {} not connected, dropping",
            target_device_id
        );
    }

    Ok(())
}

#[tauri::command]
pub async fn delete_paired_device(state: State<'_, ManagedState>, id: String) -> Result<()> {
    // Disconnect the device if it's currently connected
    if let Some(ws) = state.ws_server.read().await.as_ref() {
        ws.disconnect_client(&id).await;
    }
    let storage = state.storage.clone();
    tokio::task::spawn_blocking(move || {
        tokio::runtime::Handle::current().block_on(storage.delete_device(&id))
    })
    .await
    .map_err(|e| ConduitError::Other(format!("delete_paired_device task join error: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::test_helpers::{create_app_with_state, create_test_state};
    use crate::storage::StoredDevice;
    use crate::sync::ConnectedClient;
    use tauri::Manager;

    fn stored_device(id: &str, name: &str) -> StoredDevice {
        StoredDevice {
            id: id.to_string(),
            name: name.to_string(),
            device_type: "phone".to_string(),
            os: "android".to_string(),
            public_key: "pk".to_string(),
            shared_secret: "sk".to_string(),
            paired_at: 1_700_000_000,
            last_seen: 1_700_000_100,
            battery: Some(80),
            signal: None,
            status: "paired".to_string(),
        }
    }

    // ── get_devices ───────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_devices_valid_empty_storage_returns_hub_first() {
        let state = create_test_state();
        let app = create_app_with_state(state);

        let devices = get_devices(app.state()).await.unwrap();
        assert_eq!(devices.len(), 1, "empty storage still returns the hub");
        assert_eq!(devices[0].device_type, "desktop");
        assert_eq!(devices[0].id, "test-hub-device");
        assert_eq!(devices[0].status, "connected");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_devices_valid_paired_device_included_after_hub() {
        let state = create_test_state();
        state
            .storage
            .save_device(&stored_device("dev_1", "Pixel"))
            .await
            .unwrap();
        let app = create_app_with_state(state);

        let devices = get_devices(app.state()).await.unwrap();
        assert_eq!(devices.len(), 2);
        // Hub must be first even though stored device has a newer last_seen.
        assert_eq!(devices[0].device_type, "desktop");
        assert_eq!(devices[1].id, "dev_1");
        assert_eq!(devices[1].name, "Pixel");
        // Not connected → stored status passes through
        assert_eq!(devices[1].status, "paired");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_devices_edge_connected_client_missing_from_storage_is_appended() {
        let state = create_test_state();
        let client = ConnectedClient {
            device_id: "ghost_dev".to_string(),
            device_name: "Ghost".to_string(),
            device_type: "phone".to_string(),
            shared_secret: "00".repeat(32),
            last_heartbeat: 1_700_000_200,
            battery_level: None,
        };
        state.sync_engine.write().await.add_client(client);
        let app = create_app_with_state(state);

        let devices = get_devices(app.state()).await.unwrap();
        let ghost = devices
            .iter()
            .find(|d| d.id == "ghost_dev")
            .expect("connected-but-unstored client must be listed");
        assert_eq!(ghost.status, "connected");
        assert_eq!(ghost.os, "unknown");
    }

    // ── generate_pairing_token ────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn generate_pairing_token_valid_returns_6_uppercase_alnum_and_stores_it() {
        let state = create_test_state();
        let app = create_app_with_state(state.clone());

        let token = generate_pairing_token(app.state()).await.unwrap();
        assert_eq!(token.len(), 6, "token must be 6 chars, got {token}");
        assert!(
            token
                .chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()),
            "token must be uppercase alphanumeric: {token}"
        );
        assert!(
            state.token_store.contains_valid(&token).await,
            "generated token must be immediately valid in the token store"
        );
    }

    /// REGRESSION: `max_devices` was persisted and shown as an editable number
    /// but read nowhere, so a hub could be paired with unlimited devices. Once
    /// the limit is reached, no further pairing token may be issued.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn generate_pairing_token_invalid_max_devices_reached_returns_error() {
        let state = create_test_state();
        state
            .storage
            .save_setting("max_devices", "2")
            .await
            .unwrap();
        for i in 0..2 {
            state
                .storage
                .save_device(&stored_device(&format!("dev_{i}"), "Phone"))
                .await
                .unwrap();
        }
        let app = create_app_with_state(state.clone());

        let err = generate_pairing_token(app.state()).await.unwrap_err();
        assert!(
            err.to_string().contains("Device limit reached (2 of 2)"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn generate_pairing_token_valid_under_max_devices_still_issues_token() {
        let state = create_test_state();
        state
            .storage
            .save_setting("max_devices", "2")
            .await
            .unwrap();
        state
            .storage
            .save_device(&stored_device("dev_0", "Phone"))
            .await
            .unwrap();
        let app = create_app_with_state(state.clone());

        let token = generate_pairing_token(app.state()).await.unwrap();
        assert_eq!(token.len(), 6);
    }

    /// Unpaired/removed devices must not count against the limit, otherwise a
    /// revoked device would permanently consume a slot.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn generate_pairing_token_valid_unpaired_devices_do_not_count() {
        let state = create_test_state();
        state
            .storage
            .save_setting("max_devices", "1")
            .await
            .unwrap();
        let mut revoked = stored_device("dev_revoked", "Old");
        revoked.status = "revoked".to_string();
        state.storage.save_device(&revoked).await.unwrap();
        let app = create_app_with_state(state.clone());

        let token = generate_pairing_token(app.state()).await.unwrap();
        assert_eq!(token.len(), 6, "revoked devices must not consume a slot");
    }

    // ── generate_qr_code ──────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn generate_qr_code_valid_returns_svg_data_uri() {
        let out = generate_qr_code("CONDUIT-ABC123".to_string()).unwrap();
        assert!(
            out.starts_with("data:image/svg+xml;base64,"),
            "expected SVG data URI, got: {}",
            &out[..out.len().min(60)]
        );
    }

    #[test]
    fn generate_qr_code_invalid_oversized_input_returns_error() {
        // QR symbols have a hard capacity limit (~2953 bytes at ECC L).
        let result = generate_qr_code("A".repeat(5000));
        assert!(
            result.is_err(),
            "oversized input must be rejected by QrCode::new"
        );
        assert!(result.unwrap_err().to_string().contains("QR code error"));
    }

    // ── get_device_info ───────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn get_device_info_valid_returns_desktop_identity() {
        let state = create_test_state();
        let app = create_app_with_state(state);

        let info = get_device_info(app.state()).await.unwrap();
        assert_eq!(info["device_id"], "test-hub-device");
        assert_eq!(info["type"], "desktop");
        assert!(
            info["public_key"].as_str().is_some_and(|k| !k.is_empty()),
            "public_key must be a non-empty hex string"
        );
    }

    // ── send_encrypted_message ────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_encrypted_message_invalid_unknown_target_returns_error() {
        let state = create_test_state();
        let app = create_app_with_state(state);

        let result = send_encrypted_message(
            app.state(),
            "no_such_device".to_string(),
            "hello".to_string(),
        )
        .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Device not found"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn send_encrypted_message_valid_target_without_ws_server_returns_ok() {
        let state = create_test_state();

        // Register a paired peer with a real shared secret.
        let peer = crate::encryption::EncryptionManager::new_random();
        let secret = state
            .encryption
            .derive_shared_secret(&peer.public_key_hex())
            .expect("derive shared secret");
        state.sync_engine.write().await.add_client(ConnectedClient {
            device_id: "peer_1".to_string(),
            device_name: "Peer".to_string(),
            device_type: "phone".to_string(),
            shared_secret: hex::encode(secret),
            last_heartbeat: chrono::Utc::now().timestamp(),
            battery_level: None,
        });
        let app = create_app_with_state(state);

        // ws_server is None → message is dropped after encryption, but the
        // command itself must succeed (encrypt + HMAC path exercised).
        let result =
            send_encrypted_message(app.state(), "peer_1".to_string(), "secret".to_string()).await;
        assert!(result.is_ok(), "encryption path must succeed: {:?}", result);
    }

    // ── delete_paired_device / stop_* ─────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn delete_paired_device_valid_removes_device_from_storage() {
        let state = create_test_state();
        state
            .storage
            .save_device(&stored_device("dev_del", "Doomed"))
            .await
            .unwrap();
        let app = create_app_with_state(state.clone());

        delete_paired_device(app.state(), "dev_del".to_string())
            .await
            .unwrap();
        let remaining = state.storage.get_all_devices().await.unwrap();
        assert!(
            remaining.iter().all(|d| d.id != "dev_del"),
            "device must be removed after delete_paired_device"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn stop_discovery_valid_when_not_running_returns_ok_message() {
        let state = create_test_state();
        let app = create_app_with_state(state);

        let msg = stop_discovery(app.state()).await.unwrap();
        assert_eq!(msg, "Discovery not running");
    }

    // ── missing state ─────────────────────────────────────────────────────────

    #[test]
    #[should_panic(expected = "state() called before manage")]
    fn pairing_commands_missing_state_fails_loudly() {
        let app = tauri::test::mock_app();
        let _ = app.state::<Arc<AppState>>();
    }

    // NOTE: `start_ws_server` / `start_discovery` are intentionally NOT tested —
    // they bind real network ports / send mDNS announcements (side effects on
    // the host machine and possible conflicts with a running Conduit instance).
}
