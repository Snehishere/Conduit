use log::{error, info, warn};
use serde_json::Value;

use conduit_protocol::types::*;

use super::{WsContext, broadcast_to_others};

/// Unwrap a relayed v2 binary frame into `(sender, payload)`.
///
/// The relay has already verified the frame and re-stamped the tag for the
/// sender it authenticated, so the recipient re-verifies against that sender's
/// own route key. That is the check that makes the attribution real rather than
/// asserted: a frame naming any other sender cannot verify, because the tag was
/// computed with the sender's id inside the MAC input.
///
/// The 16-byte target field is checked against this desktop's own *canonical
/// field* — its id's first 16 bytes — because that is all a field of that width
/// can carry for a 36-character id (PROTOCOL.md §5.1.4).
///
/// Returns `Err` with a human-readable reason so the caller can log why.
async fn unwrap_relay_binary_frame(
    bytes: &[u8],
    ctx: &WsContext,
) -> Result<(String, Vec<u8>), String> {
    if bytes.len() < BINARY_HEADER_LEN {
        return Err(format!(
            "frame is {} bytes; v2 requires at least {BINARY_HEADER_LEN}",
            bytes.len()
        ));
    }

    let target_end = 1 + BINARY_DEVICE_ID_LEN;
    let target_field: [u8; BINARY_DEVICE_ID_LEN] = bytes[1..target_end]
        .try_into()
        .expect("slice is exactly BINARY_DEVICE_ID_LEN bytes");
    let target_id = match parse_binary_target_field(&target_field) {
        Ok(prefix) => prefix,
        Err(BinaryTargetFieldError::NotUtf8) => {
            return Err("target device id is not valid UTF-8".to_string());
        }
        Err(BinaryTargetFieldError::Empty) => {
            return Err("target device id is empty".to_string());
        }
    };

    // The relay routes only to the named recipient, so a frame naming anyone
    // else means the routing table and the wire disagree.
    //
    // The comparison is against the *canonical field*, not the full id: this
    // desktop's own id is a 36-character UUID and the field is 16 bytes, so it
    // can only ever carry the id's prefix. Comparing the field to the whole id
    // refused every correctly-addressed frame, which is the same defect the relay
    // had on its side. `binary_target_matches` is the single definition both use
    // (PROTOCOL.md §5.1.4), so the receiver accepts exactly the frames the relay
    // resolved to this device.
    let local = ctx.device_id.as_str();
    if !binary_target_matches(local, &target_field) {
        return Err(format!(
            "target field names {target_id:?}, which is not this desktop ({local})"
        ));
    }

    // The relay re-stamps the tag with the authenticated sender, but the frame
    // itself does not carry that id — it is the *sender's* id that went into the
    // MAC input, and the recipient cannot read it out of the header. So the
    // candidate senders are the devices this desktop shares a secret with, and
    // the one whose route key verifies the tag is the sender.
    //
    // Trying candidates is safe precisely because the sender id is inside the MAC
    // input: a frame from `dev_b` cannot verify under `dev_a`'s key.
    let candidates: Vec<String> = {
        let engine = ctx.sync_engine.read().await;
        engine
            .get_all_client_ids()
            .into_iter()
            .filter(|id| {
                engine
                    .get_client(id)
                    .is_some_and(|c| !c.shared_secret.is_empty())
            })
            .collect()
    };
    if candidates.is_empty() {
        return Err("no paired device to attribute this frame to".to_string());
    }

    let payload = &bytes[BINARY_HEADER_LEN..];
    let mut authenticated = Vec::with_capacity(BINARY_AUTHENTICATED_PREFIX_LEN + payload.len());
    authenticated.extend_from_slice(&bytes[..BINARY_AUTHENTICATED_PREFIX_LEN]);
    authenticated.extend_from_slice(payload);
    let actual = hex::encode(&bytes[BINARY_TAG_OFFSET..BINARY_HEADER_LEN]);

    for sender in candidates {
        let Some(route_key) = ctx.route_keys.signing_key(&sender) else {
            continue;
        };
        let key_hex = hex::encode(&route_key);
        let mac_input = hex::encode(conduit_protocol::binary_mac_input(&sender, &authenticated));
        if ctx.encryption.verify_hmac(&key_hex, &mac_input, &actual) {
            return Ok((sender, payload.to_vec()));
        }
    }

    Err("tag did not verify under any paired device's route key".to_string())
}

/// Enforce `auto_accept_files` on an inbound file request.
///
/// The desktop hub is a potential *receiver* as well as a relay, so a paired
/// device can push files at it. With `auto_accept_files` on, the transfer is
/// started immediately and the request is relayed as usual. With it off, the
/// hub declines the transfer, does not buffer a single chunk, and tells the
/// sender why (`file_accept_disabled`) instead of leaving it waiting for an ack
/// that will never come.
///
/// `sync_files` is enforced one layer up, in `WsServer::handle_message`.
pub async fn handle_file_request(msg: Value, client_id: &str, ctx: &WsContext) {
    if !auto_accept_files_enabled(ctx).await {
        let name = msg
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        warn!(
            "File request from {} for '{}' declined: the auto_accept_files setting is off",
            client_id, name
        );
        let err = ErrorMessage {
            msg_type: "error".into(),
            code: "file_accept_disabled".into(),
            message: format!(
                "This device does not auto-accept files; '{}' was declined (auto_accept_files is off in Settings)",
                name
            ),
            server_version: Some(PROTOCOL_VERSION),
        };
        let err = serde_json::to_string(&err).expect("ErrorMessage serializes");
        let clients_lock = ctx.clients.read().await;
        if let Some(tx) = clients_lock.get(client_id) {
            let _ = tx.send(err);
        }
        return;
    }

    let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let name = msg
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let size = msg.get("size").and_then(|v| v.as_i64()).unwrap_or(0) as u64;
    let mime = msg
        .get("mime")
        .and_then(|v| v.as_str())
        .unwrap_or("application/octet-stream");
    let from = msg.get("from").and_then(|v| v.as_str()).unwrap_or("");
    let checksum = msg
        .get("checksum")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    ctx.file_engine
        .start_incoming(id, name, size, mime, from, checksum)
        .await;

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

/// Handle an inbound binary frame, from either a LAN socket or the relay.
///
/// The relay connection carries v2 relay frames, which are a *different* format
/// from the LAN chunk envelope below. They are unwrapped here, at the one place
/// binary bytes enter, and the resulting payload is re-entered as the LAN chunk
/// it always was. Without this the relay's own frame format was parsed as a
/// chunk header and every relayed file transfer failed to decode.
pub async fn handle_binary_message(bytes: Vec<u8>, client_id: &str, ctx: &WsContext) {
    // A relayed frame is v2, not a chunk envelope. Unwrap and re-enter as the
    // authenticated sender so the rest of this function sees what a LAN socket
    // would have delivered. Not recursive: one unwrap, then the LAN path.
    if client_id == crate::server::WsServer::RELAY_CLIENT_ID
        && bytes.first() == Some(&BINARY_FRAME_VERSION)
    {
        match unwrap_relay_binary_frame(&bytes, ctx).await {
            Ok((sender, payload)) => {
                return handle_lan_chunk(payload, &sender, ctx).await;
            }
            Err(reason) => {
                warn!("Refusing a relayed binary frame: {reason}");
                return;
            }
        }
    }

    handle_lan_chunk(bytes, client_id, ctx).await
}

/// A LAN chunk envelope: nonce, metadata, ciphertext.
async fn handle_lan_chunk(bytes: Vec<u8>, client_id: &str, ctx: &WsContext) {
    let stable_id = ctx
        .ws_to_device_id
        .read()
        .await
        .get(client_id)
        .cloned()
        .unwrap_or_default();
    let shared_secret = if let Some(client) = ctx.sync_engine.read().await.get_client(&stable_id) {
        client.shared_secret.clone()
    } else {
        warn!(
            "Received binary message but no shared secret found for {}",
            client_id
        );
        return;
    };

    if bytes.len() < CHUNK_HEADER_LEN {
        warn!("Binary message too short from {}", client_id);
        return;
    }

    let nonce = &bytes[0..CHUNK_NONCE_LEN];
    let json_len =
        u32::from_le_bytes(bytes[CHUNK_NONCE_LEN..CHUNK_HEADER_LEN].try_into().unwrap()) as usize;

    if bytes.len() < CHUNK_HEADER_LEN + json_len {
        warn!("Binary message metadata length mismatch from {}", client_id);
        return;
    }

    let metadata_bytes = &bytes[CHUNK_HEADER_LEN..CHUNK_HEADER_LEN + json_len];
    let ciphertext = &bytes[CHUNK_HEADER_LEN + json_len..];

    let metadata: BinaryFileMetadata = match serde_json::from_slice(metadata_bytes) {
        Ok(v) => v,
        Err(e) => {
            warn!(
                "Failed to parse metadata in binary message from {}: {}",
                client_id, e
            );
            return;
        }
    };
    let total = u64::from(metadata.total.unwrap_or(1));

    match ctx
        .encryption
        .decrypt_binary(&shared_secret, nonce, ciphertext)
    {
        Ok(chunk_data) => {
            match ctx
                .file_engine
                .receive_chunk_binary(&metadata.id, metadata.index, &chunk_data)
                .await
            {
                Ok(received) => {
                    if ctx.file_engine.is_complete(&metadata.id).await {
                        match ctx.file_engine.finalize_incoming(&metadata.id).await {
                            Ok(path) => {
                                info!("File transfer complete: {}", path);
                                let complete_msg = FileComplete {
                                    msg_type: "file".into(),
                                    action: "complete".into(),
                                    id: metadata.id.clone(),
                                    path: Some(path),
                                };
                                let complete_msg = serde_json::to_string(&complete_msg)
                                    .expect("FileComplete serializes");
                                broadcast_to_others(ctx, client_id, &complete_msg).await;
                            }
                            Err(e) => error!("Failed to finalize file: {}", e),
                        }
                    } else {
                        let progress_msg = FileProgress {
                            msg_type: "file".into(),
                            action: "progress".into(),
                            id: metadata.id.clone(),
                            percent: (received as f64 / total as f64 * 100.0) as u32,
                        };
                        let progress_msg =
                            serde_json::to_string(&progress_msg).expect("FileProgress serializes");
                        broadcast_to_others(ctx, client_id, &progress_msg).await;
                    }
                }
                Err(e) => error!("Failed to receive binary chunk: {}", e),
            }
        }
        Err(e) => {
            warn!("Failed to decrypt binary chunk from {}: {}", client_id, e);
        }
    }
}

pub async fn handle_file_accept(msg: Value, client_id: &str, ctx: &WsContext) {
    if let Some(id) = msg.get("id").and_then(|v| v.as_str()) {
        ctx.file_engine.accept_outgoing(id).await;
    }
    broadcast_to_others(ctx, client_id, &msg.to_string()).await;
}

pub async fn handle_file_chunk(msg: Value, client_id: &str, ctx: &WsContext) {
    let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let index = msg.get("index").and_then(|v| v.as_i64()).unwrap_or(0) as u32;
    let data = msg.get("data").and_then(|v| v.as_str()).unwrap_or("");

    match ctx.file_engine.receive_chunk(id, index, data).await {
        Ok(received) => {
            if ctx.file_engine.is_complete(id).await {
                match ctx.file_engine.finalize_incoming(id).await {
                    Ok(path) => {
                        info!("File transfer complete: {}", path);
                        let complete_msg = FileComplete {
                            msg_type: "file".into(),
                            action: "complete".into(),
                            id: id.to_string(),
                            path: Some(path),
                        };
                        let complete_msg =
                            serde_json::to_string(&complete_msg).expect("FileComplete serializes");
                        broadcast_to_others(ctx, client_id, &complete_msg).await;
                    }
                    Err(e) => error!("Failed to finalize file: {}", e),
                }
            } else {
                let total = msg.get("total").and_then(|v| v.as_i64()).unwrap_or(1);
                let progress_msg = FileProgress {
                    msg_type: "file".into(),
                    action: "progress".into(),
                    id: id.to_string(),
                    percent: (received as f64 / total as f64 * 100.0) as u32,
                };
                let progress_msg =
                    serde_json::to_string(&progress_msg).expect("FileProgress serializes");
                broadcast_to_others(ctx, client_id, &progress_msg).await;
            }
        }
        Err(e) => error!("Failed to receive chunk: {}", e),
    }
}

pub async fn handle_file_progress(msg: Value, client_id: &str, ctx: &WsContext) {
    broadcast_to_others(ctx, client_id, &msg.to_string()).await;
}

pub async fn handle_file_complete(msg: Value, client_id: &str, ctx: &WsContext) {
    let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");
    ctx.file_engine.remove_outgoing(id).await;
    broadcast_to_others(ctx, client_id, &msg.to_string()).await;
}

pub async fn handle_file_cancel(msg: Value, client_id: &str, ctx: &WsContext) {
    let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");
    ctx.file_engine.cancel_incoming(id).await;
    ctx.file_engine.remove_outgoing(id).await;
    broadcast_to_others(ctx, client_id, &msg.to_string()).await;
}

pub async fn handle_file_resume(msg: Value, client_id: &str, ctx: &WsContext) {
    let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or("");
    let name = msg
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    let size = msg.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
    let mime = msg
        .get("mime")
        .and_then(|v| v.as_str())
        .unwrap_or("application/octet-stream");
    let from = msg.get("from").and_then(|v| v.as_str()).unwrap_or("");
    let checksum = msg
        .get("checksum")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let chunks_loaded = ctx
        .file_engine
        .resume_incoming(id, name, size, mime, from, checksum)
        .await
        .unwrap_or(0);

    info!(
        "File resume requested: {} ({} chunks loaded from disk)",
        name, chunks_loaded
    );

    let response = FileResumeAck {
        msg_type: "file".into(),
        action: "resume_ack".into(),
        id: id.to_string(),
        chunks_loaded,
    };
    let response = serde_json::to_string(&response).expect("FileResumeAck serializes");

    broadcast_to_others(ctx, client_id, &response).await;
}

/// The `auto_accept_files` setting, defaulting to enabled for a row that has
/// never been written (fresh install) — matching `Storage::get_settings`.
async fn auto_accept_files_enabled(ctx: &WsContext) -> bool {
    match ctx.storage.get_setting("auto_accept_files").await {
        Some(raw) => raw == "true",
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::handlers::test_helpers::{
        add_test_client, add_test_client_mapped, add_test_paired_client, create_test_ctx,
    };
    use std::sync::Arc;

    // ── handle_file_request tests ─────────────────────────────────────────────

    /// The default (row never written) must accept, so a fresh install keeps
    /// working exactly as before the setting was enforced.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_request_valid_accepts_when_auto_accept_files_unset() {
        let ctx = create_test_ctx();
        let _tx = add_test_client(&ctx, "c1").await;

        let msg = serde_json::json!({
            "type": "file", "action": "request", "id": "t_default_accept", "name": "a.bin"
        });

        handle_file_request(msg, "c1", &ctx).await;

        // Same proof the `file_request_missing_fields_uses_defaults` test uses:
        // with `size` defaulting to 0 a registered transfer is vacuously
        // complete, while an unstarted one is not.
        assert!(
            ctx.file_engine.is_complete("t_default_accept").await,
            "unset auto_accept_files defaults to true, so the transfer must be registered"
        );
    }

    /// REGRESSION: `auto_accept_files` used to be persisted and rendered as a
    /// working toggle but read nowhere. With it off, the hub must decline the
    /// transfer, buffer nothing, and tell the sender why.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_request_invalid_declined_when_auto_accept_files_disabled() {
        let ctx = create_test_ctx();
        ctx.storage
            .save_setting("auto_accept_files", "false")
            .await
            .unwrap();
        let _sender = add_test_client(&ctx, "sender").await;
        let peer = add_test_client(&ctx, "peer").await;
        let mut peer_rx = peer.subscribe();

        let msg = serde_json::json!({
            "type": "file", "action": "request", "id": "t_declined", "name": "secret.bin"
        });

        handle_file_request(msg, "sender", &ctx).await;

        // Nothing was buffered: an unstarted transfer is not "complete".
        assert!(
            !ctx.file_engine.is_complete("t_declined").await,
            "a declined transfer must never be registered with the file engine"
        );
        let relayed =
            tokio::time::timeout(std::time::Duration::from_millis(200), peer_rx.recv()).await;
        assert!(relayed.is_err(), "a declined request must not be relayed");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_request_invalid_declined_sends_error_frame_to_sender() {
        let ctx = create_test_ctx();
        ctx.storage
            .save_setting("auto_accept_files", "false")
            .await
            .unwrap();
        let sender = add_test_client(&ctx, "sender").await;
        let mut sender_rx = sender.subscribe();
        let _peer = add_test_client(&ctx, "peer").await;

        let msg = serde_json::json!({
            "type": "file", "action": "request", "id": "t_err", "name": "secret.bin"
        });

        handle_file_request(msg, "sender", &ctx).await;

        let resp = tokio::time::timeout(std::time::Duration::from_millis(500), sender_rx.recv())
            .await
            .expect("sender must be told why")
            .unwrap();
        assert!(
            resp.contains("file_accept_disabled"),
            "expected file_accept_disabled, got: {resp}"
        );
        assert!(
            resp.contains("secret.bin"),
            "the error must name the declined file, got: {resp}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_request_valid_relays_when_auto_accept_files_enabled() {
        let ctx = create_test_ctx();
        ctx.storage
            .save_setting("auto_accept_files", "true")
            .await
            .unwrap();
        let _sender = add_test_client(&ctx, "sender").await;
        let peer = add_test_client(&ctx, "peer").await;
        let mut peer_rx = peer.subscribe();

        let msg = serde_json::json!({
            "type": "file", "action": "request", "id": "t_ok", "name": "ok.bin", "size": 32
        });

        handle_file_request(msg, "sender", &ctx).await;

        let relayed = tokio::time::timeout(std::time::Duration::from_millis(500), peer_rx.recv())
            .await
            .expect("request must be relayed when auto_accept_files is on")
            .unwrap();
        assert!(relayed.contains("t_ok"), "unexpected relay: {relayed}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_request_happy_path_directed() {
        let ctx = create_test_ctx();
        let _sender_tx = add_test_client_mapped(&ctx, "sender_ws", "sender_dev").await;
        let _receiver_tx = add_test_client_mapped(&ctx, "receiver_ws", "receiver_dev").await;

        let msg = serde_json::json!({
            "type": "file",
            "action": "request",
            "id": "transfer_001",
            "name": "photo.jpg",
            "size": 1024,
            "mime": "image/jpeg",
            "from": "sender_dev",
            "to": "receiver_dev"
        });

        handle_file_request(msg, "sender_ws", &ctx).await;

        // Incoming transfer should be registered
        assert!(!ctx.file_engine.is_complete("transfer_001").await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_request_happy_path_broadcast() {
        let ctx = create_test_ctx();
        let _sender = add_test_client(&ctx, "sender").await;
        let receiver = add_test_client(&ctx, "receiver").await;

        let mut receiver_rx = receiver.subscribe();

        let msg = serde_json::json!({
            "type": "file",
            "action": "request",
            "id": "t_broadcast",
            "name": "doc.pdf",
            "size": 2048,
            "mime": "application/pdf",
            "from": "dev_a"
            // No "to" field — should broadcast to all others
        });

        handle_file_request(msg.clone(), "sender", &ctx).await;

        // Receiver should get the broadcast
        let received =
            tokio::time::timeout(std::time::Duration::from_millis(500), receiver_rx.recv()).await;
        assert!(received.is_ok(), "receiver should get the broadcast");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_request_missing_fields_uses_defaults() {
        let ctx = create_test_ctx();
        let _tx = add_test_client(&ctx, "c1").await;

        let msg = serde_json::json!({
            "type": "file",
            "action": "request",
            "id": "t_defaults"
        });

        handle_file_request(msg, "c1", &ctx).await;
        // Should not panic — defaults are applied.
        // Missing size defaults to 0 → total_chunks = 0 → vacuously complete
        // (0 received >= 0 total). is_complete == true proves the transfer was
        // registered with defaults; an unstarted transfer would return false.
        assert!(
            ctx.file_engine.is_complete("t_defaults").await,
            "size-0 transfer registered with defaults should be vacuously complete"
        );
    }

    // ── handle_binary_message tests ───────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn binary_message_too_short() {
        let ctx = create_test_ctx();
        let peer_enc = crate::encryption::EncryptionManager::new_random();
        let _tx = add_test_client_mapped(&ctx, "c1", "dev1").await;

        // Register the client with a shared secret
        let secret = ctx
            .encryption
            .derive_shared_secret(&peer_enc.public_key_hex())
            .unwrap();
        let client = crate::sync::ConnectedClient {
            device_id: "dev1".to_string(),
            device_name: "Test".to_string(),
            device_type: "phone".to_string(),
            shared_secret: hex::encode(secret),
            last_heartbeat: chrono::Utc::now().timestamp(),
            battery_level: None,
        };
        ctx.sync_engine.write().await.add_client(client);

        // Send a binary message that is too short (< 28 bytes)
        let bytes = vec![0u8; 10];
        handle_binary_message(bytes, "c1", &ctx).await;
        // Should return early without panic
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn binary_message_no_shared_secret() {
        let ctx = create_test_ctx();
        let _tx = add_test_client_mapped(&ctx, "c1", "unknown_dev").await;

        // No client registered in sync_engine — should log warning and return
        let bytes = vec![0u8; 50];
        handle_binary_message(bytes, "c1", &ctx).await;
        // Should not panic
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn binary_message_metadata_length_mismatch() {
        let ctx = create_test_ctx();
        let peer_enc = crate::encryption::EncryptionManager::new_random();
        let _tx = add_test_client_mapped(&ctx, "c1", "dev1").await;

        let secret = ctx
            .encryption
            .derive_shared_secret(&peer_enc.public_key_hex())
            .unwrap();
        let client = crate::sync::ConnectedClient {
            device_id: "dev1".to_string(),
            device_name: "Test".to_string(),
            device_type: "phone".to_string(),
            shared_secret: hex::encode(secret),
            last_heartbeat: chrono::Utc::now().timestamp(),
            battery_level: None,
        };
        ctx.sync_engine.write().await.add_client(client);

        // Build a message: 24-byte nonce + 4-byte LE json_len = 100 + only 10 bytes body
        let mut bytes = vec![0u8; 28];
        let json_len: u32 = 100;
        bytes[24..28].copy_from_slice(&json_len.to_le_bytes());
        // Total bytes = 28, but json_len says 100 — should return early
        handle_binary_message(bytes, "c1", &ctx).await;
    }

    // ── unwrap_relay_binary_frame: the 16-byte target field ───────────────────
    //
    //  The frame's target field is 16 bytes and this desktop's own id is a
    //  36-character UUID, so the identity check has to compare the *field* with
    //  the first 16 bytes of this device's id. It used to compare the field with
    //  the whole id, which refused every correctly-addressed relayed frame — the
    //  same defect the relay had on its side.

    /// A receiver-side context: this desktop is `device_id` and one phone is
    /// paired, which is the state a relayed frame arrives in.
    async fn relay_receiving_ctx(device_id: &str) -> (WsContext, String) {
        let mut ctx = create_test_ctx();
        ctx.device_id = Arc::new(device_id.to_string());
        let phone = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
        add_test_paired_client(&ctx, "relay-peer", phone).await;
        (ctx, phone.to_string())
    }

    /// A relayed frame from `sender` addressed to `target`, built by the one
    /// producer the relay re-frames with.
    fn relayed_frame(sender: &str, target: &str) -> Vec<u8> {
        conduit_protocol::build_binary_frame(b"a-test-route-key", sender, target, 1, b"chunk")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relay_frame_naming_this_desktops_uuid_passes_the_identity_check() {
        let desktop = "550e8400-e29b-41d4-a716-446655440000";
        let (ctx, phone) = relay_receiving_ctx(desktop).await;

        // The tag cannot verify here — no route key is registered in this test's
        // keyring — so the assertion is about *how far* the frame got: past the
        // address check, and no further.
        let err = unwrap_relay_binary_frame(&relayed_frame(&phone, desktop), &ctx)
            .await
            .expect_err("the frame cannot be attributed without a registered route key");
        assert!(
            !err.contains("target field names"),
            "a frame addressed to this desktop by its own 36-char id must pass the \
             identity check, got: {err}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relay_frame_naming_another_device_is_refused() {
        let desktop = "550e8400-e29b-41d4-a716-446655440000";
        let other = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
        let (ctx, phone) = relay_receiving_ctx(desktop).await;

        let err = unwrap_relay_binary_frame(&relayed_frame(&phone, other), &ctx)
            .await
            .expect_err("a frame for another device must be refused");
        assert!(
            err.contains("target field names") && err.contains("3f2504e0-4f89-11"),
            "the refusal must name the field that was looked up, got: {err}"
        );
        assert!(
            !err.contains("tag did not verify"),
            "it must be refused as misaddressed, before any tag work: {err}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relay_frame_naming_a_twin_uuid_still_passes_here() {
        // The limit of a receiver-side check, pinned deliberately. Two ids sharing
        // their first 16 bytes are the same field, so a receiver holding one of
        // them cannot tell the frame from one addressed to the other — the
        // identical bytes arrive either way. Refusing here would mean refusing
        // every frame for this device whenever a twin id existed, which is why the
        // relay's `resolve_binary_target` is the place that fails closed on
        // ambiguity: only it can see both devices at once.
        let desktop = "550e8400-e29b-41d4-a716-446655440000";
        let twin = "550e8400-e29b-41d4-b716-446655440000";
        assert_ne!(desktop, twin);
        let (ctx, phone) = relay_receiving_ctx(desktop).await;

        let err = unwrap_relay_binary_frame(&relayed_frame(&phone, twin), &ctx)
            .await
            .expect_err("the frame is not attributable in this test's keyring");
        assert!(
            !err.contains("target field names"),
            "a shared 16-byte prefix is indistinguishable to the receiver, so the \
             relay must be what refuses it; got: {err}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relay_frame_with_an_empty_target_field_is_refused() {
        let desktop = "550e8400-e29b-41d4-a716-446655440000";
        let (ctx, phone) = relay_receiving_ctx(desktop).await;

        let mut frame = relayed_frame(&phone, desktop);
        frame[1..1 + BINARY_DEVICE_ID_LEN].fill(0);
        let err = unwrap_relay_binary_frame(&frame, &ctx)
            .await
            .expect_err("an all-padding target field names nobody");
        assert_eq!(err, "target device id is empty");

        // And a field that is not UTF-8 at all, which a split code point produces.
        frame[1..1 + BINARY_DEVICE_ID_LEN].fill(0xFF);
        let err = unwrap_relay_binary_frame(&frame, &ctx)
            .await
            .expect_err("a non-UTF-8 target field is not a device id");
        assert_eq!(err, "target device id is not valid UTF-8");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_relay_frame_for_a_short_device_id_still_passes_the_identity_check() {
        // Backward compatibility for the ids that fit the field whole.
        let (ctx, phone) = relay_receiving_ctx("device-1").await;
        let err = unwrap_relay_binary_frame(&relayed_frame(&phone, "device-1"), &ctx)
            .await
            .expect_err("the frame cannot be attributed without a registered route key");
        assert!(
            !err.contains("target field names"),
            "a device id that fits the field must still be accepted, got: {err}"
        );
    }

    // ── handle_file_accept tests ──────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_accept_broadcasts() {
        let ctx = create_test_ctx();
        let _sender = add_test_client(&ctx, "sender").await;
        let receiver = add_test_client(&ctx, "receiver").await;
        let mut rx = receiver.subscribe();

        let msg = serde_json::json!({
            "type": "file",
            "action": "accept",
            "id": "transfer_001"
        });

        handle_file_accept(msg, "sender", &ctx).await;

        let received = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await;
        assert!(received.is_ok());
    }

    // ── handle_file_chunk tests ───────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_chunk_receives_and_broadcasts() {
        let ctx = create_test_ctx();
        let _sender = add_test_client(&ctx, "sender").await;
        let receiver = add_test_client(&ctx, "receiver").await;
        let mut rx = receiver.subscribe();

        // Start an incoming transfer
        ctx.file_engine
            .start_incoming(
                "t_chunk",
                "test.bin",
                64,
                "application/octet-stream",
                "dev_a",
                None,
            )
            .await;

        let chunk_data =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [0u8; 64]);

        let msg = serde_json::json!({
            "type": "file",
            "action": "chunk",
            "id": "t_chunk",
            "index": 0,
            "total": 1,
            "data": chunk_data
        });

        handle_file_chunk(msg, "sender", &ctx).await;

        // handle_file_chunk finalizes complete transfers, which removes them
        // from the incoming map — so is_complete returns false afterwards.
        // A second finalize attempt must fail, proving the first one ran.
        assert!(
            ctx.file_engine.finalize_incoming("t_chunk").await.is_err(),
            "transfer should already be finalized and removed"
        );

        // Complete message should be broadcast to others
        let received = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await;
        assert!(received.is_ok());
        let text = received.unwrap().unwrap();
        assert!(
            text.contains("complete"),
            "expected complete broadcast: {text}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_chunk_unknown_transfer() {
        let ctx = create_test_ctx();
        let _tx = add_test_client(&ctx, "c1").await;

        let msg = serde_json::json!({
            "type": "file",
            "action": "chunk",
            "id": "nonexistent",
            "index": 0,
            "data": "AAAA"
        });

        // Should not panic even if transfer doesn't exist
        handle_file_chunk(msg, "c1", &ctx).await;
    }

    // ── handle_file_cancel tests ──────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_cancel_removes_incoming_and_broadcasts() {
        let ctx = create_test_ctx();
        let _sender = add_test_client(&ctx, "sender").await;
        let receiver = add_test_client(&ctx, "receiver").await;
        let mut rx = receiver.subscribe();

        ctx.file_engine
            .start_incoming(
                "t_cancel",
                "f.bin",
                128,
                "application/octet-stream",
                "dev",
                None,
            )
            .await;

        let msg = serde_json::json!({
            "type": "file",
            "action": "cancel",
            "id": "t_cancel"
        });

        handle_file_cancel(msg, "sender", &ctx).await;

        assert!(!ctx.file_engine.is_complete("t_cancel").await);

        let received = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await;
        assert!(received.is_ok());
    }

    // ── handle_file_complete tests ────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_complete_removes_outgoing() {
        let ctx = create_test_ctx();
        let _sender = add_test_client(&ctx, "sender").await;
        let receiver = add_test_client(&ctx, "receiver").await;
        let mut rx = receiver.subscribe();

        // Start an outgoing transfer
        let dir = std::env::temp_dir().join("conduit_test_file_complete");
        std::fs::create_dir_all(&dir).ok();
        let file_path = dir.join("test.bin");
        std::fs::write(&file_path, vec![0u8; 10]).unwrap();

        ctx.file_engine
            .start_outgoing("t_out", &file_path.to_string_lossy(), "dev_b")
            .await
            .ok();

        let msg = serde_json::json!({
            "type": "file",
            "action": "complete",
            "id": "t_out"
        });

        handle_file_complete(msg, "sender", &ctx).await;

        // Outgoing should be removed
        assert!(ctx.file_engine.get_next_chunk("t_out").await.is_none());

        let received = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await;
        assert!(received.is_ok());

        std::fs::remove_dir_all(&dir).ok();
    }

    // ── handle_file_progress tests ────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_progress_broadcasts() {
        let ctx = create_test_ctx();
        let _sender = add_test_client(&ctx, "sender").await;
        let receiver = add_test_client(&ctx, "receiver").await;
        let mut rx = receiver.subscribe();

        let msg = serde_json::json!({
            "type": "file",
            "action": "progress",
            "id": "t1",
            "percent": 50
        });

        handle_file_progress(msg, "sender", &ctx).await;

        let received = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await;
        assert!(received.is_ok());
    }

    // ── handle_file_resume tests ──────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_resume_acks_chunks_loaded() {
        let ctx = create_test_ctx();
        let _sender = add_test_client(&ctx, "sender").await;
        let receiver = add_test_client(&ctx, "receiver").await;
        let mut rx = receiver.subscribe();

        // No existing transfer — resume should ack 0 chunks loaded
        let msg = serde_json::json!({
            "type": "file",
            "action": "resume",
            "id": "t_resume",
            "name": "file.bin",
            "size": 128,
            "mime": "application/octet-stream",
            "from": "dev_a"
        });

        handle_file_resume(msg, "sender", &ctx).await;

        let received = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await;
        assert!(received.is_ok());
        let text = received.unwrap().unwrap();
        assert!(text.contains("chunks_loaded"));
    }

    // ── Rate limiting test ────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn file_rate_limit_enforced() {
        let ctx = create_test_ctx();
        let _tx = add_test_client(&ctx, "c1").await;

        // Deliberately does not repeat the budget: it is a value in
        // `security.rs` (`type_limit_for`), pinned there by
        // `security::tests::test_type_limit_config_values` and
        // `test_per_type_high_throughput_file_limit`, and this test used to
        // hard-code 500 — so raising the limit broke this assertion instead of
        // any assertion about the limiter. What is worth pinning here is the
        // property: the `file` bucket is finite, and it is finite *for one
        // client* rather than globally.
        const PROBE_CEILING: u32 = 10_000;
        let mut allowed = 0;
        while allowed < PROBE_CEILING && ctx.per_type_limiter.check_type_limit("c1", "file").await {
            allowed += 1;
        }
        assert!(
            allowed < PROBE_CEILING,
            "the file bucket accepted {PROBE_CEILING} messages in one window; it is not bounded"
        );

        // And a second client is unaffected by the first one's spending.
        assert!(
            ctx.per_type_limiter.check_type_limit("c2", "file").await,
            "one client exhausting its file budget must not limit another's"
        );
    }
}
