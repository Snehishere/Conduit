//! One WebSocket connection, from accept to close.
//!
//! This is the state machine that makes a socket a *peer*: it gates on
//! authentication before any message is dispatched, then runs a read loop that
//! verifies and forwards. Nothing here trusts the socket — the auth gate is the
//! only thing that decides whether a frame is even looked at.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use conduit_protocol::{
    ErrorMessage, Ping, Pong, RelayAuth, RelayAuthOk, RelayAuthRejected, RelayDelivery,
};
use futures_util::{SinkExt, StreamExt};
use log::{error, info, warn};
use serde_json::Value;
use tokio::sync::{mpsc, watch};
use tokio_tungstenite::accept_async_with_config;
use tungstenite::Message;
use tungstenite::protocol::WebSocketConfig;

use std::net::SocketAddr;

/// How long in-flight connection tasks get to notice the shutdown signal and
/// close on their own before they are aborted.
pub(crate) const DRAIN_TIMEOUT_SECS: u64 = 5;

use super::limits::{MAX_BINARY_SIZE, MAX_TEXT_SIZE, MessageRateLimiter, QUEUE_DEPTH};
use super::route::{
    NOT_WRAPPED_MSG, RejectionKind, forward_binary, forward_text, handle_binary_frame,
    handle_relay_route, validate_device_id,
};
use super::state::{AppState, Queue};

pub(crate) async fn drain_connections(connections: &mut tokio::task::JoinSet<()>, label: &str) {
    if connections.is_empty() {
        return;
    }
    let remaining = connections.len();
    info!(
        "{} listener draining: waiting up to {}s for {} in-flight connection(s)",
        label, DRAIN_TIMEOUT_SECS, remaining
    );
    let drained = tokio::time::timeout(std::time::Duration::from_secs(DRAIN_TIMEOUT_SECS), async {
        while let Some(joined) = connections.join_next().await {
            if let Err(e) = joined {
                warn!(
                    "{} connection task ended abnormally during drain: {}",
                    label, e
                );
            }
        }
    })
    .await;
    if drained.is_err() {
        let aborted = connections.len();
        connections.shutdown().await;
        warn!(
            "{} drain timed out after {}s; aborted {} connection(s)",
            label, DRAIN_TIMEOUT_SECS, aborted
        );
    }
}

pub(crate) async fn handle_connection<S>(
    stream: S,
    state: Arc<AppState>,
    peer: SocketAddr,
    mut shutdown_rx: watch::Receiver<bool>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    // No size limit was ever configured here: `accept_async` uses tungstenite's
    // default `WebSocketConfig`, which caps a message at 64 MiB and a frame at
    // 16 MiB. That was the *only* bound on binary frames, and it is a bound an
    // attacker chooses to approach, so it is set to the same ceiling the Text
    // arms enforce. It is applied at read time, which matters: by the time the
    // application can reject a frame the bytes are already allocated.
    let ws_config = WebSocketConfig {
        max_message_size: Some(MAX_TEXT_SIZE),
        max_frame_size: Some(MAX_BINARY_SIZE),
        ..Default::default()
    };
    let mut ws_stream = match accept_async_with_config(stream, Some(ws_config)).await {
        Ok(ws) => ws,
        Err(e) => {
            error!("WebSocket handshake error from {}", peer);
            warn!("WebSocket handshake error details: {}", e);
            return;
        }
    };

    // Rate limit by IP
    if !state.rate_limiter.allow(peer.ip()) {
        warn!("Rate limit exceeded for {}", peer);
        state
            .metrics
            .messages_dropped_rate_limit
            .fetch_add(1, Ordering::Relaxed);
        let _ = ws_stream.close(None).await;
        return;
    }

    let (mut write, mut read) = ws_stream.split();
    let (tx, mut rx) = mpsc::channel(QUEUE_DEPTH);
    // Every producer below goes through this, not through `tx`: the budget is
    // only correct if nothing can enqueue without reserving.
    let queue = Queue::new(tx);

    let mut device_id: Option<String> = None;
    let mut msg_limiter = MessageRateLimiter::new();

    let auth_timeout = std::time::Duration::from_secs(state.config.auth_timeout_secs);
    while let Ok(Some(Ok(msg))) = tokio::time::timeout(auth_timeout, read.next()).await {
        if matches!(msg, Message::Text(_) | Message::Binary(_)) && !msg_limiter.allow() {
            state
                .metrics
                .messages_dropped_rate_limit
                .fetch_add(1, Ordering::Relaxed);
            warn!("Message rate limit exceeded during auth from {}", peer);
            continue;
        }
        // Checked for both variants, not just Text. The binary arm used to be
        // unbounded: `handle_binary_frame` was handed whatever the socket
        // produced, and the only cap in the whole path was tungstenite's 64 MiB
        // default. Rejecting here closes the connection the same way an
        // over-size Text frame always has.
        if matches!(&msg, Message::Text(t) if t.len() > MAX_TEXT_SIZE)
            || matches!(&msg, Message::Binary(b) if b.len() > MAX_BINARY_SIZE)
        {
            warn!("Auth message too large from {}", peer);
            return;
        }
        if let Message::Text(text) = msg
            && let Ok(json) = serde_json::from_str::<Value>(&text)
            && json.get("type").and_then(|v| v.as_str()) == Some("relay_auth")
        {
            // Primary path: fully-typed relay_auth via conduit-protocol.
            match serde_json::from_value::<RelayAuth>(json.clone()) {
                Ok(auth) => {
                    let expected_bytes = state.config.relay_token.as_bytes();
                    let provided_bytes = auth.relay_token.as_bytes();
                    if expected_bytes.len() != provided_bytes.len()
                        || !bool::from(subtle::ConstantTimeEq::ct_eq(
                            expected_bytes,
                            provided_bytes,
                        ))
                    {
                        warn!("Invalid relay token from {}", peer);
                        state
                            .metrics
                            .auth_attempts_failure
                            .fetch_add(1, Ordering::Relaxed);
                        let _ = write
                            .send(Message::Text(
                                serde_json::to_string(&RelayAuthRejected::invalid_token())
                                    .unwrap_or_default(),
                            ))
                            .await;
                        return;
                    }
                    device_id = Some(auth.device_id.clone());
                    if let Err(reason) = validate_device_id(&auth.device_id) {
                        warn!("Rejected device_id from {}: {}", peer, reason);
                        state
                            .metrics
                            .auth_attempts_failure
                            .fetch_add(1, Ordering::Relaxed);
                        let _ = write.send(error_frame("invalid_device_id", reason)).await;
                        return;
                    }
                    state
                        .clients
                        .write()
                        .await
                        .insert(auth.device_id.clone(), queue.clone());
                    state
                        .metrics
                        .auth_attempts_success
                        .fetch_add(1, Ordering::Relaxed);
                    info!("Device authenticated: {} from {}", auth.device_id, peer);
                    let _ = write
                        .send(Message::Text(
                            serde_json::to_string(&RelayAuthOk::new()).unwrap_or_default(),
                        ))
                        .await;
                    break;
                }
                // Value fallback: preserve original partial-message behavior
                // (missing device_id keeps waiting; missing token rejects).
                Err(_) => match json.get("relay_token").and_then(|v| v.as_str()) {
                    None => {
                        warn!("Missing relay token from {}", peer);
                        state
                            .metrics
                            .auth_attempts_failure
                            .fetch_add(1, Ordering::Relaxed);
                        let _ = write
                            .send(Message::Text(
                                serde_json::to_string(&RelayAuthRejected::missing_token())
                                    .unwrap_or_default(),
                            ))
                            .await;
                        return;
                    }
                    Some(provided) => {
                        let expected_bytes = state.config.relay_token.as_bytes();
                        let provided_bytes = provided.as_bytes();
                        if expected_bytes.len() != provided_bytes.len()
                            || !bool::from(subtle::ConstantTimeEq::ct_eq(
                                expected_bytes,
                                provided_bytes,
                            ))
                        {
                            warn!("Invalid relay token from {}", peer);
                            state
                                .metrics
                                .auth_attempts_failure
                                .fetch_add(1, Ordering::Relaxed);
                            let _ = write
                                .send(Message::Text(
                                    serde_json::to_string(&RelayAuthRejected::invalid_token())
                                        .unwrap_or_default(),
                                ))
                                .await;
                            return;
                        }
                        if let Some(id) = json.get("device_id").and_then(|v| v.as_str()) {
                            if let Err(reason) = validate_device_id(id) {
                                warn!("Rejected device_id from {}: {}", peer, reason);
                                state
                                    .metrics
                                    .auth_attempts_failure
                                    .fetch_add(1, Ordering::Relaxed);
                                let _ = write.send(error_frame("invalid_device_id", reason)).await;
                                return;
                            }
                            device_id = Some(id.to_string());
                            state
                                .clients
                                .write()
                                .await
                                .insert(id.to_string(), queue.clone());
                            state
                                .metrics
                                .auth_attempts_success
                                .fetch_add(1, Ordering::Relaxed);
                            info!("Device authenticated: {} from {}", id, peer);
                            let _ = write
                                .send(Message::Text(
                                    serde_json::to_string(&RelayAuthOk::new()).unwrap_or_default(),
                                ))
                                .await;
                            break;
                        }
                        // Token OK but no device_id — keep waiting.
                    }
                },
            }
        }
    }

    let my_id = match device_id {
        Some(id) => id,
        None => {
            warn!(
                "Connection closed: failed to authenticate within timeout from {}",
                peer
            );
            return;
        }
    };

    // The message-signing keyring — never `relay_token`. Every authenticated
    // client holds the token; if it doubled as the MAC key, any client could
    // forge a `relay_route` claiming to be any other device.
    // Share the process-wide nonce cache: a device that disconnects and
    // reconnects cannot replay a nonce that was already accepted. Entries are
    // scoped to `my_id` (the authenticated identity) so one client cannot
    // evict another's replay protection.
    let nonces = state.nonces.clone();
    // Highest binary sequence number accepted on this connection (replay guard).
    let mut last_binary_seq: Option<u32> = None;
    // Outbound frame counter for deliveries this relay re-frames. Separate from
    // `last_binary_seq`, which guards what arrives: a sender's own numbering and
    // the numbering the relay stamps on what it forwards are different series.
    let mut delivery_seq: u32 = 0;

    let mut ping_interval = tokio::time::interval(tokio::time::Duration::from_secs(25));

    let broadcast_queue = queue.clone();
    let broadcast_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = ping_interval.tick() => {
                    if write
                        .send(Message::Text(
                            serde_json::to_string(&Ping::new()).unwrap_or_default(),
                        ))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
                msg_opt = rx.recv() => {
                    match msg_opt {
                        Some(msg) => {
                            // The message has left the queue, so its bytes go
                            // back now — not after the socket write, which may
                            // fail and drop the whole connection. Producers
                            // reserved exactly this much in `Queue::send`, so
                            // this is where the accounting closes.
                            broadcast_queue.release(msg.len());
                            if write.send(msg).await.is_err() {
                                break;
                            }
                        }
                        None => break,
                    }
                }
            }
        }
    });

    loop {
        let read_next = tokio::time::timeout(tokio::time::Duration::from_secs(60), read.next());
        tokio::pin!(read_next);
        let read_result = tokio::select! {
            biased;
            // A shutdown signal closes the connection, so "draining" is real.
            _ = shutdown_rx.changed() => {
                info!("Connection for {} closing: shutdown signal", my_id);
                break;
            }
            result = &mut read_next => result,
        };
        match read_result {
            Ok(Some(Ok(msg))) => {
                if matches!(msg, Message::Text(_) | Message::Binary(_)) && !msg_limiter.allow() {
                    state
                        .metrics
                        .messages_dropped_rate_limit
                        .fetch_add(1, Ordering::Relaxed);
                    warn!("Message rate limit exceeded from {}", my_id);
                    continue;
                }
                match &msg {
                    Message::Text(text) => {
                        if text.len() > MAX_TEXT_SIZE {
                            warn!("Message too large ({} bytes) from {}", text.len(), my_id);
                            break;
                        }
                        state.metrics.observe_message_size(text.len());

                        let json = match serde_json::from_str::<Value>(text) {
                            Ok(json) => json,
                            Err(e) => {
                                // Previously swallowed in silence: a client whose
                                // messages vanished was indistinguishable from a
                                // healthy relay.
                                warn!("Malformed JSON from {}: {}", my_id, e);
                                state
                                    .metrics
                                    .messages_dropped_unknown_type
                                    .fetch_add(1, Ordering::Relaxed);
                                let _ = queue
                                    .send(error_frame(
                                        "malformed_json",
                                        "message was not valid JSON",
                                    ))
                                    .await;
                                continue;
                            }
                        };

                        match json.get("type").and_then(|v| v.as_str()).unwrap_or("") {
                            "relay_route" => {
                                match handle_relay_route(
                                    state.route_keys.as_ref(),
                                    &nonces,
                                    &json,
                                    &my_id,
                                )
                                .await
                                {
                                    Ok(route) => {
                                        // Wrap rather than forward bare. `handle_relay_route`
                                        // has just established that this connection *is*
                                        // `my_id` and that the signature verifies under that
                                        // device's own route key, so the attribution is
                                        // known here and nowhere else. Forwarding
                                        // `route.payload` alone discarded it, which left the
                                        // receiver unable to tell who sent the message and
                                        // unable to apply its own pairing check — see
                                        // `RelayDelivery` for the full argument.
                                        let delivery = RelayDelivery::new(
                                            my_id.clone(),
                                            route.to_device_id.clone(),
                                            route.payload,
                                        );
                                        forward_text(
                                            &state,
                                            &my_id,
                                            &route.to_device_id,
                                            serde_json::to_value(&delivery)
                                                .unwrap_or(serde_json::Value::Null),
                                        )
                                        .await;
                                    }
                                    Err(rejection) => {
                                        warn!(
                                            "Rejecting relay_route from {}: {}",
                                            my_id, rejection.reason
                                        );
                                        match rejection.kind {
                                            RejectionKind::Replay => {
                                                state
                                                    .metrics
                                                    .messages_dropped_replay
                                                    .fetch_add(1, Ordering::Relaxed);
                                            }
                                            RejectionKind::Integrity => {
                                                state
                                                    .metrics
                                                    .messages_dropped_hmac_failed
                                                    .fetch_add(1, Ordering::Relaxed);
                                            }
                                        }
                                        let _ = queue
                                            .send(error_frame(rejection.code, rejection.reason))
                                            .await;
                                    }
                                }
                            }
                            // A client `ping` used to be swallowed by `_ => {}`
                            // with no answer, even though the relay pings every
                            // 25 s and PROTOCOL.md specifies a 25 s ping/pong
                            // keep-alive in both directions.
                            "ping" => {
                                let _ = queue
                                    .send(Message::Text(
                                        serde_json::to_string(&Pong::new()).unwrap_or_default(),
                                    ))
                                    .await;
                            }
                            "pong" => {}
                            // `encrypted` is the protocol's primary message type,
                            // but the relay cannot attribute it: `source_device` is
                            // unauthenticated, so blind forwarding would let a
                            // client impersonate any sender. Loud, explicit failure
                            // rather than a silent drop.
                            "encrypted" => {
                                warn!(
                                    "Rejecting unwrapped 'encrypted' message from {}: it must be \
                                     sent inside a signed relay_route",
                                    my_id
                                );
                                state
                                    .metrics
                                    .messages_dropped_unknown_type
                                    .fetch_add(1, Ordering::Relaxed);
                                let _ = queue
                                    .send(error_frame(
                                        "not_wrapped_in_relay_route",
                                        NOT_WRAPPED_MSG,
                                    ))
                                    .await;
                            }
                            other => {
                                // The previous `_ => {}` swallowed everything
                                // with zero diagnostics: no log, no metric, no
                                // error frame.
                                warn!("Unknown message type {:?} from {}", other, my_id);
                                state
                                    .metrics
                                    .messages_dropped_unknown_type
                                    .fetch_add(1, Ordering::Relaxed);
                                let _ = queue
                                    .send(error_frame(
                                        "unknown_message_type",
                                        format!(
                                            "message type {other:?} is not routable by the \
                                             relay; wrap it in a signed relay_route"
                                        ),
                                    ))
                                    .await;
                            }
                        }
                    }
                    Message::Binary(bytes) => {
                        // Mirrors the Text arm above, which is the check this
                        // path never had.
                        if bytes.len() > MAX_BINARY_SIZE {
                            warn!(
                                "Binary message too large ({} bytes) from {}",
                                bytes.len(),
                                my_id
                            );
                            break;
                        }
                        state.metrics.observe_message_size(bytes.len());
                        match handle_binary_frame(
                            &state,
                            state.route_keys.as_ref(),
                            &my_id,
                            bytes,
                            &mut last_binary_seq,
                        )
                        .await
                        {
                            Ok(Some(frame)) => {
                                // Re-frame rather than forward the stripped payload.
                                //
                                // The incoming frame was addressed to the recipient and
                                // tagged for this connection's sender, which is exactly
                                // right for verification but useless to the recipient:
                                // the header the far end parses is gone, and the tag was
                                // computed over a MAC input naming a sender it cannot
                                // check. Building a fresh v2 frame here means the
                                // recipient gets a well-formed frame whose tag names the
                                // sender the relay actually authenticated.
                                //
                                // The sequence is re-based per recipient, which is what
                                // the receiver's replay guard expects: it is monotonic per
                                // sending connection, and this is a new connection from
                                // the recipient's point of view.
                                match state.route_keys.key_for(&my_id) {
                                    Some(key) => {
                                        delivery_seq = delivery_seq.wrapping_add(1);
                                        let reframed = conduit_protocol::build_binary_frame(
                                            &key,
                                            &my_id,
                                            &frame.target_id,
                                            delivery_seq,
                                            frame.payload,
                                        );
                                        forward_binary(&state, &my_id, &frame.target_id, &reframed)
                                            .await;
                                    }
                                    None => {
                                        // Unreachable: the tag verified under this
                                        // device's key a moment ago.
                                        warn!(
                                            "Dropping binary frame from {my_id}: route key \
                                             vanished mid-connection"
                                        );
                                        state
                                            .metrics
                                            .messages_dropped_hmac_failed
                                            .fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                            }
                            Ok(None) => {}
                            Err(rejection) => {
                                warn!(
                                    "Rejecting binary frame from {}: {}",
                                    my_id, rejection.reason
                                );
                                match rejection.kind {
                                    RejectionKind::Replay => {
                                        state
                                            .metrics
                                            .messages_dropped_replay
                                            .fetch_add(1, Ordering::Relaxed);
                                    }
                                    RejectionKind::Integrity => {
                                        state
                                            .metrics
                                            .messages_dropped_hmac_failed
                                            .fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                let _ = queue
                                    .send(error_frame(rejection.code, rejection.reason))
                                    .await;
                            }
                        }
                    }
                    _ => {}
                }
            }
            Ok(Some(Err(e))) => {
                warn!("WebSocket read error for {}: {}", my_id, e);
                break;
            }
            Ok(None) => break,
            Err(_) => {
                warn!("Client {} timed out (no pong/message in 60s)", my_id);
                break;
            }
        }
    }

    broadcast_task.abort();
    state.clients.write().await.remove(&my_id);
    state
        .metrics
        .connections_disconnected
        .fetch_add(1, Ordering::Relaxed);
    info!("Device disconnected: {}", my_id);
}

// ============================================================
//  Message handling
// ============================================================

/// Build the documented `error` frame sent back to a client whose message the
/// relay refused.
///
/// Refusals used to be silent `continue`s; the protocol defines an `error`
/// message type precisely so the sender learns why its traffic vanished.
pub(crate) fn error_frame(code: &str, message: impl Into<String>) -> Message {
    let frame = ErrorMessage {
        msg_type: "error".into(),
        code: code.to_string(),
        message: message.into(),
        server_version: Some(conduit_protocol::PROTOCOL_VERSION),
    };
    Message::Text(
        serde_json::to_string(&frame).unwrap_or_else(|_| r#"{"type":"error"}"#.to_string()),
    )
}
