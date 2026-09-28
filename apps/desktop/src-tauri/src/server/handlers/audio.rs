use log::{error, info, warn};
use serde_json::Value;

use conduit_protocol::types::*;

use super::WsContext;

pub async fn handle_audio(msg: Value, client_id: &str, ctx: &WsContext) {
    let action = msg.get("action").and_then(|v| v.as_str()).unwrap_or("");

    match action {
        "stream_start" => {
            info!("Audio stream started from {}", client_id);
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
                    let mut bytes = Vec::with_capacity(pcm_data.len() * 2);
                    for sample in &pcm_data {
                        bytes.extend_from_slice(&sample.to_le_bytes());
                    }

                    let msg = AudioStreamData {
                        msg_type: "audio".into(),
                        action: "stream_data".into(),
                        data: base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            &bytes,
                        ),
                        format: "pcm16".into(),
                        sample_rate: 16000,
                        channels: 1,
                        from: client_id_clone.clone(),
                    };
                    let msg_str = serde_json::to_string(&msg).expect("AudioStreamData serializes");

                    let clients_lock = ctx_clone.clients.read().await;
                    for (id, client_tx) in clients_lock.iter() {
                        if id != &client_id_clone {
                            let _ = client_tx.send(msg_str.clone());
                        }
                    }
                }
            });

            let response = AudioStreamStarted {
                msg_type: "audio".into(),
                action: "stream_started".into(),
                from: client_id.to_string(),
            };
            let response = serde_json::to_string(&response).expect("AudioStreamStarted serializes");
            let clients_lock = ctx.clients.read().await;
            if let Some(tx) = clients_lock.get(client_id) {
                let _ = tx.send(response);
            }
        }
        "stream_stop" => {
            info!("Audio stream stopped from {}", client_id);
            ctx.audio_stream.stop_capture();

            let clients_lock = ctx.clients.read().await;
            for (id, client_tx) in clients_lock.iter() {
                if id != client_id {
                    let _ = client_tx.send(msg.to_string());
                }
            }
        }
        "stream_data" => {
            if let Some(data) = msg.get("data").and_then(|v| v.as_str())
                && let Ok(bytes) =
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data)
            {
                if bytes.len() % 2 != 0 {
                    warn!(
                        "Audio stream_data: odd byte length {}, dropping last byte",
                        bytes.len()
                    );
                }
                let pcm_data: Vec<i16> = bytes
                    .chunks(2)
                    .filter(|c| c.len() == 2)
                    .map(|c| i16::from_le_bytes([c[0], c[1]]))
                    .collect();

                let audio_stream = ctx.audio_stream.clone();
                tokio::spawn(async move {
                    if let Err(e) = audio_stream.play_audio(&pcm_data).await {
                        error!("Failed to play audio: {}", e);
                    }
                });
            }
        }
        "playback_start" => {
            // Mobile requests desktop to start receiving audio for duplex playback.
            // Desktop enables its persistent output stream so that subsequent
            // "stream_data" frames are played through speakers in real-time
            // while capture continues independently.
            info!("Duplex playback requested by {}", client_id);
            ctx.audio_stream.start_playback();

            let response = AudioPlaybackStarted {
                msg_type: "audio".into(),
                action: "playback_started".into(),
                from: client_id.to_string(),
            };
            let response =
                serde_json::to_string(&response).expect("AudioPlaybackStarted serializes");
            let clients_lock = ctx.clients.read().await;
            if let Some(tx) = clients_lock.get(client_id) {
                let _ = tx.send(response);
            }
        }
        "playback_stop" => {
            info!("Duplex playback stop requested by {}", client_id);
            ctx.audio_stream.stop_playback();

            let response = AudioPlaybackStopped {
                msg_type: "audio".into(),
                action: "playback_stopped".into(),
                from: client_id.to_string(),
            };
            let response =
                serde_json::to_string(&response).expect("AudioPlaybackStopped serializes");
            let clients_lock = ctx.clients.read().await;
            if let Some(tx) = clients_lock.get(client_id) {
                let _ = tx.send(response);
            }
        }
        "playback_data" => {
            // Incoming PCM from mobile intended for duplex playback through speakers.
            // Unlike "stream_data", this feeds the persistent playback channel
            // without creating a new output stream each time.
            if let Some(data) = msg.get("data").and_then(|v| v.as_str())
                && let Ok(bytes) =
                    base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data)
            {
                let pcm_data: Vec<i16> = bytes
                    .chunks(2)
                    .filter(|c| c.len() == 2)
                    .map(|c| i16::from_le_bytes([c[0], c[1]]))
                    .collect();
                ctx.audio_stream.send_playback_data(pcm_data);
            }
        }
        _ => {
            let clients_lock = ctx.clients.read().await;
            for (id, client_tx) in clients_lock.iter() {
                if id != client_id {
                    let _ = client_tx.send(msg.to_string());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::handlers::test_helpers::{add_test_client, create_test_ctx};

    // ── happy paths ───────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn audio_playback_start_happy_acks_requester() {
        let ctx = create_test_ctx();
        let tx = add_test_client(&ctx, "c1").await;
        let mut rx = tx.subscribe();

        let msg = serde_json::json!({ "type": "audio", "action": "playback_start" });
        handle_audio(msg, "c1", &ctx).await;

        let received = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await;
        assert!(received.is_ok(), "requester must be acked");
        let text = received.unwrap().unwrap();
        assert!(
            text.contains("playback_started"),
            "expected playback_started ack, got: {text}"
        );
        assert!(text.contains("\"c1\""), "ack must echo from=c1: {text}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn audio_playback_stop_happy_acks_requester() {
        let ctx = create_test_ctx();
        let tx = add_test_client(&ctx, "c1").await;
        let mut rx = tx.subscribe();

        let msg = serde_json::json!({ "type": "audio", "action": "playback_stop" });
        handle_audio(msg, "c1", &ctx).await;

        let received = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv()).await;
        assert!(received.is_ok());
        let text = received.unwrap().unwrap();
        assert!(
            text.contains("playback_stopped"),
            "expected playback_stopped ack, got: {text}"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn audio_stream_stop_happy_broadcasts_to_others_only() {
        let ctx = create_test_ctx();
        let sender_tx = add_test_client(&ctx, "sender").await;
        let peer_tx = add_test_client(&ctx, "peer").await;
        let mut sender_rx = sender_tx.subscribe();
        let mut peer_rx = peer_tx.subscribe();

        let msg = serde_json::json!({ "type": "audio", "action": "stream_stop" });
        handle_audio(msg, "sender", &ctx).await;

        // Peer receives the stop event…
        let peer_got =
            tokio::time::timeout(std::time::Duration::from_millis(500), peer_rx.recv()).await;
        assert!(peer_got.is_ok(), "peer must see stream_stop");
        // …sender does not (broadcast-to-others semantics).
        let sender_got =
            tokio::time::timeout(std::time::Duration::from_millis(200), sender_rx.recv()).await;
        assert!(
            sender_got.is_err(),
            "sender must not receive its own stream_stop"
        );
    }

    // ── invalid input ─────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn audio_stream_data_invalid_base64_is_dropped_silently() {
        let ctx = create_test_ctx();
        let _tx = add_test_client(&ctx, "c1").await;
        let peer_tx = add_test_client(&ctx, "peer").await;
        let mut peer_rx = peer_tx.subscribe();

        let msg = serde_json::json!({
            "type": "audio",
            "action": "stream_data",
            "data": "!!!this-is-not-base64!!!"
        });

        // Must not panic; invalid payload must not reach peers as a broadcast.
        handle_audio(msg, "c1", &ctx).await;

        let peer_got =
            tokio::time::timeout(std::time::Duration::from_millis(200), peer_rx.recv()).await;
        assert!(
            peer_got.is_err(),
            "invalid stream_data must not be broadcast to peers"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn audio_missing_action_falls_back_to_broadcast_not_panic() {
        let ctx = create_test_ctx();
        let _tx = add_test_client(&ctx, "c1").await;
        let peer_tx = add_test_client(&ctx, "peer").await;
        let mut peer_rx = peer_tx.subscribe();

        // No action at all → falls through to the `_` branch → broadcast others.
        handle_audio(serde_json::json!({ "type": "audio" }), "c1", &ctx).await;

        let peer_got =
            tokio::time::timeout(std::time::Duration::from_millis(500), peer_rx.recv()).await;
        assert!(
            peer_got.is_ok(),
            "unknown/missing action still relays to peers"
        );
    }

    // ── edge case ─────────────────────────────────────────────────────────────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn audio_stream_data_odd_byte_length_does_not_panic() {
        let ctx = create_test_ctx();
        let _tx = add_test_client(&ctx, "c1").await;

        // 3 bytes = valid base64, but odd length for i16 chunks — last byte dropped.
        let msg = serde_json::json!({
            "type": "audio",
            "action": "stream_data",
            "data": base64::Engine::encode(&base64::engine::general_purpose::STANDARD, [1u8, 2, 3])
        });

        handle_audio(msg, "c1", &ctx).await;
        // No panic == pass; playback spawn failure (headless) is logged, not fatal.
    }

    // ── unauthorized access (auth gate lives in WsServer::handle_message) ────

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn audio_unauthenticated_stream_start_rejected_by_dispatcher() {
        let ctx = create_test_ctx();
        let tx = add_test_client(&ctx, "unpaired_ws").await;
        let mut rx = tx.subscribe();

        let text = serde_json::to_string(&serde_json::json!({
            "type": "audio",
            "action": "stream_start"
        }))
        .unwrap();

        crate::server::WsServer::handle_message(&text, "unpaired_ws", &ctx).await;

        let resp = tokio::time::timeout(std::time::Duration::from_millis(500), rx.recv())
            .await
            .expect("dispatcher must respond to unpaired client")
            .expect("broadcast recv");
        assert!(
            resp.contains("not_authenticated"),
            "expected not_authenticated rejection, got: {resp}"
        );
        assert!(
            !resp.contains("stream_started"),
            "unpaired client must never receive stream_started"
        );
    }
}
