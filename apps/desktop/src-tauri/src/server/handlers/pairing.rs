use log::{error, info, warn};
use serde_json::Value;

use conduit_protocol::types::*;

use super::{WsContext, mark_paired, send_to_client};
use crate::automation::{TriggerContext, TriggerEvent};
use crate::security::normalize_pairing_token;
use crate::sync::ConnectedClient;

/// The single pairing-authorisation check, shared by `pairing/request` and
/// `pairing/accept`.
///
/// Both actions are *pairing*: each one derives a shared secret from an
/// attacker-supplied `public_key`, writes a `devices` row, inserts a
/// `ConnectedClient` **and** a `ws_to_device_id` entry (which the auth gate
/// reads as "paired"), then fires `DeviceConnect` automation triggers — i.e.
/// hands over the whole device. `accept` used to skip the check entirely, so
/// any peer that could open a socket could pair itself by sending one frame.
///
/// Factoring the check here is the point: `request` and `accept` cannot drift
/// apart again, and neither can a future third pairing action.
///
/// Consumes the token on success (one token, one device). Returns the
/// normalised token so callers can log it.
async fn authorize_pairing(
    msg: &Value,
    action: &str,
    client_id: &str,
    ctx: &WsContext,
) -> Option<String> {
    let token_raw = msg.get("token").and_then(|v| v.as_str()).unwrap_or("");
    let token = normalize_pairing_token(token_raw);
    if !ctx.token_store.contains_valid(&token).await {
        warn!(
            "Pairing rejected: invalid or expired token on '{}' from {}",
            action, client_id
        );
        // Answer the peer so it learns *why* rather than seeing silence.
        super::send_to_client(
            ctx,
            client_id,
            &serde_json::to_string(&ErrorMessage {
                msg_type: "error".into(),
                code: "invalid_pairing_token".into(),
                message:
                    "The pairing code is invalid or has expired. Generate a new one on the desktop."
                        .into(),
                server_version: Some(PROTOCOL_VERSION),
            })
            .expect("ErrorMessage serializes"),
        )
        .await;
        return None;
    }
    ctx.token_store.remove(&token).await;
    Some(token)
}

pub async fn handle_pairing_request(msg: Value, client_id: &str, ctx: &WsContext) {
    let peer_public_key = msg.get("public_key").and_then(|v| v.as_str()).unwrap_or("");
    let device_info = msg.get("device_info").cloned().unwrap_or(Value::Null);

    let device_name = device_info
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown Device");
    let device_type = device_info
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("phone");

    if authorize_pairing(&msg, "request", client_id, ctx)
        .await
        .is_none()
    {
        return;
    }
    info!(
        "Pairing accepted for device: {} ({})",
        device_name, client_id
    );

    let shared_secret = match ctx.encryption.derive_shared_secret(peer_public_key) {
        Ok(s) => s,
        Err(e) => {
            warn!(
                "Pairing failed: invalid public key from {}: {}",
                client_id, e
            );
            return;
        }
    };
    let shared_secret_hex = hex::encode(shared_secret);

    let stable_id = ctx
        .storage
        .get_all_devices()
        .await
        .unwrap_or_default()
        .into_iter()
        .find(|d| d.public_key == peer_public_key)
        .map(|d| d.id)
        .unwrap_or_else(|| client_id.to_string());

    mark_paired(ctx, client_id, &stable_id).await;
    let device_os = device_info
        .get("os")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_lowercase();
    let device_os = match device_os.as_str() {
        "ios" | "ipados" => "ios",
        "android" => "android",
        "windows" | "win32" => "windows",
        "macos" | "darwin" => "macos",
        "linux" => "linux",
        other => {
            warn!("Unknown OS '{}' from device info — storing as-is", other);
            other
        }
    };

    let connected_client = ConnectedClient {
        device_id: stable_id.clone(),
        device_name: device_name.to_string(),
        device_type: device_type.to_string(),
        shared_secret: shared_secret_hex.clone(),
        last_heartbeat: chrono::Utc::now().timestamp(),
        battery_level: None,
    };

    ctx.sync_engine.write().await.add_client(connected_client);

    let now = chrono::Utc::now().timestamp();
    let stored_device = crate::storage::StoredDevice {
        id: stable_id.clone(),
        name: device_name.to_string(),
        device_type: device_type.to_string(),
        os: device_os.to_string(),
        public_key: peer_public_key.to_string(),
        shared_secret: shared_secret_hex,
        paired_at: now,
        last_seen: now,
        battery: None,
        signal: None,
        status: "paired".to_string(),
    };
    if let Err(e) = ctx.storage.save_device(&stored_device).await {
        error!("Failed to persist paired device {}: {}", stable_id, e);
    }

    let response = PairingAccept {
        msg_type: "pairing".into(),
        action: "accept".into(),
        protocol_version: Some(PROTOCOL_VERSION),
        public_key: ctx.encryption.public_key_hex(),
        device_info: Some(DeviceInfo {
            name: "Conduit Desktop".into(),
            device_type: "desktop".into(),
            os: Some(std::env::consts::OS.to_string()),
            battery: None,
        }),
        // The peer cannot derive or choose this. It has to be told, because the
        // peer is the only party that can stamp it into `source_device`, and
        // the hub resolves the shared secret by that field when opening the
        // next `encrypted` envelope. Omitting it left the phone sending the
        // literal "mobile", which never resolved, so every encrypted frame the
        // phone sent was dropped at `server/mod.rs`'s decrypt step.
        device_id: Some(stable_id.clone()),
        // The peer needs the hub's own id to verify anything the relay
        // forwards it: a relayed v2 frame is checked under the sender's route
        // key, and the sender's id is bound into that key's derivation.
        hub_device_id: Some(ctx.device_id.as_str().to_string()),
    };
    let response = serde_json::to_string(&response).expect("PairingAccept serializes");

    send_to_client(ctx, client_id, &response).await;

    let connect_ctx = TriggerContext {
        event: TriggerEvent::DeviceConnect,
        source_device_id: stable_id.clone(),
        battery_level: None,
        wifi_ssid: None,
        app_package: None,
    };
    crate::server::WsServer::evaluate_device_triggers(&connect_ctx, ctx).await;

    info!("Device paired: {} ({})", device_name, stable_id);
}

pub async fn handle_pairing_accept(msg: Value, client_id: &str, ctx: &WsContext) {
    // SECURITY: identical to `request`. This action used to run with no token
    // check at all, so one frame was enough to become a paired device with a
    // `ws_to_device_id` entry — i.e. to reach `automation/rule`,
    // `screen_mirror` and `remote_input`.
    if authorize_pairing(&msg, "accept", client_id, ctx)
        .await
        .is_none()
    {
        return;
    }

    let peer_public_key = msg.get("public_key").and_then(|v| v.as_str()).unwrap_or("");
    let device_info = msg.get("device_info").cloned().unwrap_or(Value::Null);

    let device_name = device_info
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown Device");
    let device_type = device_info
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("phone");
    let os = device_info
        .get("os")
        .and_then(|v| v.as_str())
        .unwrap_or("android");
    let battery = device_info
        .get("battery")
        .and_then(|v| v.as_i64())
        .unwrap_or(100) as i32;

    let shared_secret = match ctx.encryption.derive_shared_secret(peer_public_key) {
        Ok(s) => s,
        Err(e) => {
            warn!(
                "Pairing accept failed: invalid public key from {}: {}",
                client_id, e
            );
            return;
        }
    };
    let shared_secret_hex = hex::encode(shared_secret);

    let stable_id = ctx
        .storage
        .get_all_devices()
        .await
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
        battery_level: Some(battery),
    };

    ctx.sync_engine.write().await.add_client(connected_client);

    let now = chrono::Utc::now().timestamp();
    let stored_device = crate::storage::StoredDevice {
        id: stable_id.clone(),
        name: device_name.to_string(),
        device_type: device_type.to_string(),
        os: os.to_string(),
        public_key: peer_public_key.to_string(),
        shared_secret: shared_secret_hex,
        paired_at: now,
        last_seen: now,
        battery: Some(battery),
        signal: None,
        status: "paired".to_string(),
    };
    if let Err(e) = ctx.storage.save_device(&stored_device).await {
        error!("Failed to persist paired device {}: {}", stable_id, e);
    }

    // The `ws_to_device_id` entry is what the auth gate reads as "paired";
    // it must be written on the same code path that persisted the device.
    mark_paired(ctx, client_id, &stable_id).await;

    let connect_ctx = TriggerContext {
        event: TriggerEvent::DeviceConnect,
        source_device_id: stable_id.clone(),
        battery_level: Some(battery),
        wifi_ssid: None,
        app_package: None,
    };
    crate::server::WsServer::evaluate_device_triggers(&connect_ctx, ctx).await;

    info!("Pairing accepted from: {} ({})", device_name, stable_id);
}

/// Exchange the per-launch local capability for the `local_desktop` identity.
///
/// This is the replacement for "any loopback peer is auto-paired". A connection
/// that cannot present [`crate::security::local_capability`]'s token stays
/// unpaired: it may `ping`/`pong`, pair, announce itself, and receive unicast
/// replies to its own requests — and nothing else. In particular it cannot
/// reach `automation/rule`, `screen_mirror`, `remote_input` or any broadcast.
///
/// The token is compared in constant time and is regenerated every launch, so
/// it cannot be replayed from a previous run and cannot be brute-forced.
pub async fn handle_local_auth(msg: Value, client_id: &str, ctx: &WsContext) {
    let presented = msg.get("token").and_then(|v| v.as_str()).unwrap_or("");
    if !crate::security::local_capability().verify(presented) {
        warn!("Local capability rejected for connection {client_id}");
        super::send_to_client(
            ctx,
            client_id,
            &serde_json::to_string(&ErrorMessage {
                msg_type: "error".into(),
                code: "invalid_local_capability".into(),
                message: "The local capability token was not accepted.".into(),
                server_version: Some(PROTOCOL_VERSION),
            })
            .expect("ErrorMessage serializes"),
        )
        .await;
        return;
    }
    super::mark_local_desktop(ctx, client_id).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::handlers::test_helpers::{add_test_unpaired_client, create_test_ctx};
    use crate::server::handlers::{has_identity, paired_device_id};

    // ── handle_pairing_request tests ──────────────────────────────────────────

    /// The `pairing/accept` reply must name the id the hub filed the peer
    /// under.
    ///
    /// This is the fix for a silent total loss of the phone's encrypted
    /// traffic. The hub resolves the shared secret for an `encrypted` envelope
    /// by the `source_device` the sender stamps on it, and the peer has no way
    /// to learn that value except this field. When it was absent, the phone
    /// sent the literal `"mobile"`, the lookup missed, and every frame it sent
    /// was dropped at the decrypt step — with only a server-side `warn!`, and
    /// a phone that still showed "Connected".
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_accept_reply_assigns_a_device_id_the_peer_can_learn() {
        let ctx = create_test_ctx();
        let ws_tx = add_test_unpaired_client(&ctx, "client_001").await;
        let mut ws_rx = ws_tx.subscribe();
        let _keep = ws_tx;

        ctx.token_store.insert("ABCDEF".to_string()).await;

        let peer_enc = crate::encryption::EncryptionManager::new_random();
        let msg = serde_json::json!({
            "type": "pairing",
            "action": "request",
            "public_key": peer_enc.public_key_hex(),
            "token": "ABCDEF",
            "device_info": { "name": "Pixel 7", "type": "phone", "os": "android" }
        });

        handle_pairing_request(msg, "client_001", &ctx).await;

        let reply = ws_rx
            .try_recv()
            .expect("pairing must reply with an accept frame");
        let accept: serde_json::Value = serde_json::from_str(&reply).expect("accept frame is JSON");

        let assigned = accept
            .get("device_id")
            .and_then(|v| v.as_str())
            .expect("accept must carry the assigned device_id");
        assert!(!assigned.is_empty(), "assigned id must not be empty");

        // It has to be the id the hub actually filed, or the peer stamps a value
        // that resolves to nothing and we are back where we started.
        let stored = ctx.storage.get_all_devices().await.unwrap();
        assert_eq!(
            assigned, stored[0].id,
            "accept must carry the same devices.id the hub persisted"
        );
        assert_eq!(
            paired_device_id(&ctx, "client_001").await.as_deref(),
            Some(assigned),
            "the assigned id must match the connection's mapped identity"
        );

        // And the hub must have a shared secret filed under exactly that id,
        // since that is the lookup the peer's next envelope performs.
        assert!(
            ctx.sync_engine.read().await.get_client(assigned).is_some(),
            "a shared secret must be resolvable by the assigned id"
        );
    }

    /// A desktop that predates assigned ids omits the field; the reply must
    /// still be well-formed rather than panicking or serialising `"null"`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_accept_device_id_is_optional_on_the_wire() {
        let absent: PairingAccept = serde_json::from_value(serde_json::json!({
            "type": "pairing",
            "action": "accept",
            "public_key": "aabb",
        }))
        .expect("an accept without device_id must still parse");
        assert_eq!(absent.device_id, None);

        // `skip_serializing_if` means it is genuinely absent, not null — a
        // client that force-unwraps `json["device_id"]` must not see a crash.
        let json = serde_json::to_value(&absent).unwrap();
        assert!(
            json.get("device_id").is_none(),
            "device_id must be omitted, not null"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_request_happy_path() {
        let ctx = create_test_ctx();
        let _ws_tx = add_test_unpaired_client(&ctx, "client_001").await;

        // Insert a valid token
        ctx.token_store.insert("ABCDEF".to_string()).await;

        // Generate a real peer key pair to get a valid public key
        let peer_enc = crate::encryption::EncryptionManager::new_random();
        let peer_pubkey = peer_enc.public_key_hex();

        let msg = serde_json::json!({
            "type": "pairing",
            "action": "request",
            "public_key": peer_pubkey,
            "token": "ABCDEF",
            "device_info": {
                "name": "Pixel 7",
                "type": "phone",
                "os": "android"
            }
        });

        handle_pairing_request(msg, "client_001", &ctx).await;

        // Token should be consumed
        assert!(!ctx.token_store.contains_valid("ABCDEF").await);

        // Device should be persisted
        let devices = ctx.storage.get_all_devices().await.unwrap();
        assert!(!devices.is_empty(), "device should be saved after pairing");
        let device = &devices[0];
        assert_eq!(device.name, "Pixel 7");
        assert_eq!(device.device_type, "phone");
        assert_eq!(device.os, "android");

        // Client should be added to sync engine
        let client = ctx.sync_engine.read().await;
        assert_eq!(client.connected_clients.len(), 1);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_request_invalid_token() {
        let ctx = create_test_ctx();
        let _ws_tx = add_test_unpaired_client(&ctx, "client_001").await;

        // Insert a token, but the request uses a different one
        ctx.token_store.insert("VALID1".to_string()).await;

        let peer_enc = crate::encryption::EncryptionManager::new_random();
        let msg = serde_json::json!({
            "type": "pairing",
            "action": "request",
            "public_key": peer_enc.public_key_hex(),
            "token": "WRONG1",
            "device_info": { "name": "Evil", "type": "phone", "os": "android" }
        });

        handle_pairing_request(msg, "client_001", &ctx).await;

        // Original token should still be valid (not consumed)
        assert!(ctx.token_store.contains_valid("VALID1").await);

        // No devices should be saved
        assert!(ctx.storage.get_all_devices().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_request_empty_token() {
        let ctx = create_test_ctx();
        let _ws_tx = add_test_unpaired_client(&ctx, "c1").await;

        let peer_enc = crate::encryption::EncryptionManager::new_random();
        let msg = serde_json::json!({
            "type": "pairing",
            "action": "request",
            "public_key": peer_enc.public_key_hex(),
            "token": "",
            "device_info": { "name": "No Token", "type": "phone", "os": "android" }
        });

        handle_pairing_request(msg, "c1", &ctx).await;
        assert!(ctx.storage.get_all_devices().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_request_expired_token() {
        let mut ctx = create_test_ctx();
        let _ws_tx = add_test_unpaired_client(&ctx, "c1").await;

        // Create a token store with 1ms TTL and wait for it to expire
        let short_lived = crate::security::TokenStore::new(std::time::Duration::from_millis(1));
        short_lived.insert("EXP01".to_string()).await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;

        // Swap in the short-lived store
        let orig_token_store = ctx.token_store.clone();
        ctx.token_store = std::sync::Arc::new(short_lived);

        let peer_enc = crate::encryption::EncryptionManager::new_random();
        let msg = serde_json::json!({
            "type": "pairing",
            "action": "request",
            "public_key": peer_enc.public_key_hex(),
            "token": "EXP01",
            "device_info": { "name": "Expired", "type": "phone", "os": "android" }
        });

        handle_pairing_request(msg, "c1", &ctx).await;
        assert!(ctx.storage.get_all_devices().await.unwrap().is_empty());

        // Restore original
        ctx.token_store = orig_token_store;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_request_invalid_public_key() {
        let ctx = create_test_ctx();
        let _ws_tx = add_test_unpaired_client(&ctx, "c1").await;
        ctx.token_store.insert("TOKEN1".to_string()).await;

        let msg = serde_json::json!({
            "type": "pairing",
            "action": "request",
            "public_key": "not-a-valid-hex-key",
            "token": "TOKEN1",
            "device_info": { "name": "Bad Key", "type": "phone", "os": "android" }
        });

        handle_pairing_request(msg, "c1", &ctx).await;

        // Token should still be consumed (it was valid, but key derivation failed)
        assert!(!ctx.token_store.contains_valid("TOKEN1").await);
        assert!(ctx.storage.get_all_devices().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_request_missing_fields_uses_defaults() {
        let ctx = create_test_ctx();
        let _ws_tx = add_test_unpaired_client(&ctx, "c1").await;
        ctx.token_store.insert("TOK01".to_string()).await;

        let peer_enc = crate::encryption::EncryptionManager::new_random();
        let msg = serde_json::json!({
            "type": "pairing",
            "action": "request",
            "public_key": peer_enc.public_key_hex(),
            "token": "TOK01"
            // No device_info
        });

        handle_pairing_request(msg, "c1", &ctx).await;

        let devices = ctx.storage.get_all_devices().await.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "Unknown Device");
        assert_eq!(devices[0].device_type, "phone");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_request_token_with_dashes() {
        let ctx = create_test_ctx();
        let _ws_tx = add_test_unpaired_client(&ctx, "c1").await;
        // User enters token with dashes from UI
        ctx.token_store.insert("ABCDEF".to_string()).await;

        let peer_enc = crate::encryption::EncryptionManager::new_random();
        let msg = serde_json::json!({
            "type": "pairing",
            "action": "request",
            "public_key": peer_enc.public_key_hex(),
            "token": "ABC-DEF",
            "device_info": { "name": "Dashed", "type": "phone", "os": "ios" }
        });

        handle_pairing_request(msg, "c1", &ctx).await;

        // Token (with dashes stripped) should match
        assert!(!ctx.token_store.contains_valid("ABCDEF").await);
        assert_eq!(ctx.storage.get_all_devices().await.unwrap().len(), 1);
    }

    // ── handle_pairing_accept tests ───────────────────────────────────────────

    /// REGRESSION (CRITICAL): `pairing/accept` used to run with **no** token
    /// check. One frame was enough to become a paired device — a `devices` row,
    /// a `ConnectedClient`, a `ws_to_device_id` entry (which the auth gate
    /// reads as "paired") and a fired `DeviceConnect` automation trigger.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_accept_invalid_no_token_pairs_nothing() {
        let ctx = create_test_ctx();
        let ws_tx = add_test_unpaired_client(&ctx, "attacker").await;
        let mut rx = ws_tx.subscribe();

        let peer_enc = crate::encryption::EncryptionManager::new_random();
        let msg = serde_json::json!({
            "type": "pairing",
            "action": "accept",
            "public_key": peer_enc.public_key_hex(),
            "device_info": { "name": "Attacker", "type": "phone", "os": "android" }
        });

        handle_pairing_accept(msg, "attacker", &ctx).await;

        assert!(
            ctx.storage.get_all_devices().await.unwrap().is_empty(),
            "an unauthenticated pairing/accept must not persist a device"
        );
        assert!(
            ctx.sync_engine.read().await.connected_clients.is_empty(),
            "an unauthenticated pairing/accept must not add a ConnectedClient"
        );
        assert!(
            !has_identity(&ctx, "attacker").await,
            "an unauthenticated pairing/accept must not grant the paired identity"
        );

        let err = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("the peer must be told why")
            .unwrap();
        assert!(
            err.contains("invalid_pairing_token"),
            "expected invalid_pairing_token, got: {err}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_accept_invalid_wrong_token_pairs_nothing() {
        let ctx = create_test_ctx();
        let _ws_tx = add_test_unpaired_client(&ctx, "attacker").await;
        ctx.token_store.insert("GOOD1".to_string()).await;

        let peer_enc = crate::encryption::EncryptionManager::new_random();
        let msg = serde_json::json!({
            "type": "pairing",
            "action": "accept",
            "public_key": peer_enc.public_key_hex(),
            "token": "WRONG",
            "device_info": { "name": "Attacker", "type": "phone", "os": "android" }
        });

        handle_pairing_accept(msg, "attacker", &ctx).await;

        assert!(ctx.storage.get_all_devices().await.unwrap().is_empty());
        assert!(
            ctx.token_store.contains_valid("GOOD1").await,
            "a failed accept must not consume someone else's token"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_accept_token_is_single_use() {
        let ctx = create_test_ctx();
        let _ws_tx = add_test_unpaired_client(&ctx, "c1").await;
        ctx.token_store.insert("ONCE1".to_string()).await;

        for _ in 0..2 {
            let peer_enc = crate::encryption::EncryptionManager::new_random();
            let msg = serde_json::json!({
                "type": "pairing", "action": "accept",
                "public_key": peer_enc.public_key_hex(), "token": "ONCE1"
            });
            handle_pairing_accept(msg, "c1", &ctx).await;
        }

        assert!(
            !ctx.token_store.contains_valid("ONCE1").await,
            "the token must be consumed by the first successful accept"
        );
        assert_eq!(
            ctx.storage.get_all_devices().await.unwrap().len(),
            1,
            "the same token must not be replayable for a second device"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_accept_happy_path() {
        let ctx = create_test_ctx();
        let _ws_tx = add_test_unpaired_client(&ctx, "client_002").await;
        ctx.token_store.insert("ACCEPT".to_string()).await;

        let peer_enc = crate::encryption::EncryptionManager::new_random();
        let msg = serde_json::json!({
            "type": "pairing",
            "action": "accept",
            "public_key": peer_enc.public_key_hex(),
            "token": "ACCEPT",
            "device_info": {
                "name": "iPhone 15",
                "type": "phone",
                "os": "ios",
                "battery": 85
            }
        });

        handle_pairing_accept(msg, "client_002", &ctx).await;

        let devices = ctx.storage.get_all_devices().await.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "iPhone 15");
        assert_eq!(devices[0].device_type, "phone");
        assert_eq!(devices[0].os, "ios");
        assert_eq!(devices[0].battery, Some(85));
        assert!(
            has_identity(&ctx, "client_002").await,
            "a token-authorised accept must grant the paired identity"
        );
    }

    /// `request` and `accept` share one authorisation helper, so this pins that
    /// they cannot drift: a token that authorises a `request` authorises an
    /// `accept` and vice versa.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_request_and_accept_share_one_authorization_rule() {
        let ctx = create_test_ctx();
        let _a = add_test_unpaired_client(&ctx, "a").await;
        let _b = add_test_unpaired_client(&ctx, "b").await;
        ctx.token_store.insert("SHARED".to_string()).await;

        let enc = crate::encryption::EncryptionManager::new_random();
        handle_pairing_request(
            serde_json::json!({
                "type": "pairing", "action": "request",
                "public_key": enc.public_key_hex(), "token": "shared"
            }),
            "a",
            &ctx,
        )
        .await;
        assert!(has_identity(&ctx, "a").await);

        ctx.token_store.insert("SHARED2".to_string()).await;
        handle_pairing_accept(
            serde_json::json!({
                "type": "pairing", "action": "accept",
                "public_key": enc.public_key_hex(), "token": "SHARED2"
            }),
            "b",
            &ctx,
        )
        .await;
        assert!(has_identity(&ctx, "b").await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_accept_invalid_key() {
        let ctx = create_test_ctx();
        let _ws_tx = add_test_unpaired_client(&ctx, "c1").await;
        ctx.token_store.insert("BADKEY".to_string()).await;

        let msg = serde_json::json!({
            "type": "pairing",
            "action": "accept",
            "public_key": "zzzz",
            "token": "BADKEY",
            "device_info": { "name": "Bad", "type": "phone", "os": "android" }
        });

        handle_pairing_accept(msg, "c1", &ctx).await;
        assert!(ctx.storage.get_all_devices().await.unwrap().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_accept_missing_device_info_uses_defaults() {
        let ctx = create_test_ctx();
        let _ws_tx = add_test_unpaired_client(&ctx, "c1").await;
        ctx.token_store.insert("NOINFO".to_string()).await;

        let peer_enc = crate::encryption::EncryptionManager::new_random();
        let msg = serde_json::json!({
            "type": "pairing",
            "action": "accept",
            "public_key": peer_enc.public_key_hex(),
            "token": "NOINFO"
        });

        handle_pairing_accept(msg, "c1", &ctx).await;

        let devices = ctx.storage.get_all_devices().await.unwrap();
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].name, "Unknown Device");
        assert_eq!(devices[0].device_type, "phone");
    }

    // ── handle_local_auth (V1a) ──────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_auth_valid_capability_grants_local_desktop() {
        let ctx = create_test_ctx();
        let _ws_tx = add_test_unpaired_client(&ctx, "webview").await;
        assert!(!has_identity(&ctx, "webview").await);

        handle_local_auth(
            serde_json::json!({
                "type": "pairing", "action": "local_auth",
                "token": crate::security::local_capability().token()
            }),
            "webview",
            &ctx,
        )
        .await;

        assert!(has_identity(&ctx, "webview").await);
        assert_eq!(
            paired_device_id(&ctx, "webview").await.as_deref(),
            Some(super::super::LOCAL_DESKTOP_ID)
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_auth_invalid_capability_grants_nothing() {
        let ctx = create_test_ctx();
        let ws_tx = add_test_unpaired_client(&ctx, "stranger").await;
        let mut rx = ws_tx.subscribe();

        for bad in [
            serde_json::json!({"type":"pairing","action":"local_auth"}),
            serde_json::json!({"type":"pairing","action":"local_auth","token":""}),
            serde_json::json!({"type":"pairing","action":"local_auth","token":"guess"}),
            serde_json::json!({"type":"pairing","action":"local_auth","token":"A"}),
        ] {
            handle_local_auth(bad, "stranger", &ctx).await;
            assert!(
                !has_identity(&ctx, "stranger").await,
                "a bad capability must never grant the local_desktop identity"
            );
        }

        let err = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("the peer must be told why")
            .unwrap();
        assert!(
            err.contains("invalid_local_capability"),
            "expected invalid_local_capability, got: {err}"
        );
    }

    // ── Rate limiting test (pairing type limit is 5) ──────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn pairing_rate_limit_enforced() {
        let ctx = create_test_ctx();
        let _ws_tx = add_test_unpaired_client(&ctx, "c1").await;

        // The rate limiter allows 5 pairing messages per 60s for the global limiter
        // (RateLimitConfig default: 100 messages per 10s). Pairing requests go through
        // per-type limiter which allows 5 per 60s.
        // Exhaust pairing quota: 5 valid tokens
        for i in 0..5 {
            let token = format!("TOK{:02}", i);
            ctx.token_store.insert(token.clone()).await;

            let peer_enc = crate::encryption::EncryptionManager::new_random();
            let msg = serde_json::json!({
                "type": "pairing",
                "action": "request",
                "public_key": peer_enc.public_key_hex(),
                "token": token,
                "device_info": { "name": format!("Device {}", i), "type": "phone", "os": "android" }
            });

            // These should all go through — token is valid, key is valid
            let allowed = ctx.per_type_limiter.check_type_limit("c1", "pairing").await;
            assert!(allowed, "pairing message {} should be allowed", i);
            handle_pairing_request(msg, "c1", &ctx).await;
        }

        // 6th should be rate-limited by the per-type limiter
        let allowed = ctx.per_type_limiter.check_type_limit("c1", "pairing").await;
        assert!(!allowed, "6th pairing message should be rate-limited");
    }
}
