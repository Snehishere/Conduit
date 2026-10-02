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

/// How long a refusal frame gets to reach the wire before the writer task is
/// torn down anyway.
///
/// Covers both halves of getting a refusal out: `Queue::send` needs a slot and
/// byte budget from a queue a wedged peer may have filled, and the writer then
/// needs one poll to flush a ~100-byte frame to a socket that may not be
/// reading. Neither is allowed to hold a connection — or a refusal — hostage,
/// so both are bounded by this one budget.
const REFUSAL_FLUSH: std::time::Duration = std::time::Duration::from_millis(500);

/// How long a socket whose refused frame is still unread keeps the connection
/// alive after that refusal has been written.
///
/// Only over-size refusals pay it, and they pay it because of what refused
/// them: see [`linger_after_refusal`]. Half a second is far longer than a
/// reading peer needs to take a ~100-byte frame off the wire, and it is spent
/// on a connection that is already closing — what was refused is an over-size
/// frame, so the wait cannot be stretched into a way to hold a session open.
const REFUSAL_LINGER: std::time::Duration = std::time::Duration::from_millis(500);

use super::limits::{MAX_BINARY_SIZE, MAX_TEXT_SIZE, MessageRateLimiter, QUEUE_DEPTH};
use super::route::{
    BinaryTarget, NOT_WRAPPED_MSG, RejectionKind, forward_binary, forward_text,
    handle_binary_frame, handle_relay_route, resolve_binary_target, validate_device_id,
};
use super::state::{AppState, Queue, deregister_if_current};

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
    // This connection's identity in the routing table. Both registration sites
    // store a clone of `queue`, so the entry carries this id, and the
    // deregistration at the end matches on it — a connection that has been
    // superseded by a reconnect must not evict the connection that replaced
    // it (W6.4).
    let connection_id = queue.connection_id();

    let mut device_id: Option<String> = None;
    let mut msg_limiter = MessageRateLimiter::new();

    // One deadline for the whole auth phase, not one per frame. Re-arming a
    // fresh `auth_timeout` on every read meant a client that sent one junk
    // frame per interval held an unauthenticated connection — and its slot in
    // `active_connections` — for as long as it cared to keep going (W6.8).
    //
    // The deadline is checked explicitly at the top of each iteration as well
    // as through `timeout_at`: `tokio::time::Timeout` polls the read first, so
    // a client that keeps the socket full would keep `read.next()` permanently
    // ready and the elapsed deadline would never be observed. One frame per
    // iteration bounds the loop even against a flood.
    let auth_deadline = tokio::time::Instant::now()
        + std::time::Duration::from_secs(state.config.auth_timeout_secs);
    loop {
        if tokio::time::Instant::now() >= auth_deadline {
            break;
        }
        let msg = match tokio::time::timeout_at(auth_deadline, read.next()).await {
            Ok(Some(Ok(msg))) => msg,
            // Refused while being read, so it never reached the application's
            // own size checks below: `WebSocketConfig` caps `max_message_size`
            // and `max_frame_size` at the same ceilings, and tungstenite
            // enforces them before handing the frame over. Tell the sender why
            // before the socket goes away — a bare close makes "my messages
            // vanish" and "the peer is gone" indistinguishable (W6.11).
            Ok(Some(Err(e))) => {
                if let Some((offered, ceiling)) = oversize_read_error(&e) {
                    warn!(
                        "Auth message too large from {} ({} > {})",
                        peer, offered, ceiling
                    );
                    let _ = write.send(message_too_large_frame(offered, ceiling)).await;
                    linger_after_refusal().await;
                } else {
                    warn!("WebSocket read error during auth from {}: {}", peer, e);
                }
                return;
            }
            // Peer closed before authenticating, or the total budget ran out.
            Ok(None) | Err(_) => break,
        };
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
        // over-size Text frame always has — but answers first, so the sender
        // learns which rule it broke (W6.11). Unreachable while the read-time
        // ceilings sit at these same values; that is the point of the layering,
        // and this is the layer that still holds if `WebSocketConfig` changes.
        let over_ceiling = match &msg {
            Message::Text(text) if text.len() > MAX_TEXT_SIZE => Some((text.len(), MAX_TEXT_SIZE)),
            Message::Binary(bytes) if bytes.len() > MAX_BINARY_SIZE => {
                Some((bytes.len(), MAX_BINARY_SIZE))
            }
            _ => None,
        };
        if let Some((offered, ceiling)) = over_ceiling {
            warn!(
                "Auth message too large from {} ({} > {})",
                peer, offered, ceiling
            );
            let _ = write.send(message_too_large_frame(offered, ceiling)).await;
            linger_after_refusal().await;
            return;
        }
        if let Message::Text(text) = msg
            && let Ok(json) = serde_json::from_str::<Value>(&text)
            && json.get("type").and_then(|v| v.as_str()) == Some("relay_auth")
        {
            // Refused before a single credential is checked. The typed
            // `RelayAuth` has no `protocol_version` field — serde would drop
            // the value on the floor — so this is the only place it can be
            // seen, and it has to be seen before authentication succeeds (W6.10).
            if let Some(frame) = unsupported_version_frame(&json) {
                warn!("Unsupported protocol_version from {}", peer);
                state
                    .metrics
                    .auth_attempts_failure
                    .fetch_add(1, Ordering::Relaxed);
                let _ = write.send(frame).await;
                return;
            }

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

    // Set when the read loop closes the connection with a refusal frame still
    // sitting in `rx`. The writer drains what is queued and exits, so the
    // refusal reaches the wire before the `broadcast_task.abort()` on the way
    // out — aborting a task that has been woken by our `queue.send` but not
    // polled yet cancels it between the enqueue and the socket write, which is
    // how a "we refused your message" became a silent disconnect (W6.11).
    //
    // The flag doubles as the reader's own record: it is only ever *read*
    // after the loop, so there is one source of truth for "was a refusal
    // signalled".
    let (refusal_tx, mut refusal_rx) = watch::channel(false);

    let broadcast_queue = queue.clone();
    let mut broadcast_task = tokio::spawn(async move {
        loop {
            tokio::select! {
                biased;
                // A refusal is queued: flush it — and anything ahead of it,
                // in FIFO order — then end the writer, so the connection
                // closes after the client has been told why.
                _ = refusal_rx.changed() => {
                    while let Ok(msg) = rx.try_recv() {
                        broadcast_queue.release(msg.len());
                        if write.send(msg).await.is_err() {
                            break;
                        }
                    }
                    break;
                }
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
                            queue_refusal(
                                &queue,
                                &refusal_tx,
                                message_too_large_frame(text.len(), MAX_TEXT_SIZE),
                            )
                            .await;
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

                        // Refused before it is dispatched, but unlike every
                        // other per-message refusal the connection stays up:
                        // the desktop hub answers this the same way
                        // (PROTOCOL.md §8.1), and dropping a live session over
                        // one too-new frame tells the client nothing it can
                        // act on — it just reconnects and does it again (W6.10).
                        if let Some(frame) = unsupported_version_frame(&json) {
                            warn!(
                                "Rejecting message with unsupported protocol_version from {}",
                                my_id
                            );
                            state
                                .metrics
                                .messages_dropped_unknown_type
                                .fetch_add(1, Ordering::Relaxed);
                            let _ = queue.send(frame).await;
                            continue;
                        }

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
                                            // Unreachable here: a `relay_route`
                                            // names its target by full device id,
                                            // and it is `forward_text` that finds
                                            // the table entry missing. Kept
                                            // exhaustive so a new rejection kind
                                            // cannot be added here by accident.
                                            RejectionKind::Unroutable => {
                                                state
                                                    .metrics
                                                    .messages_dropped_not_found
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
                        // path never had — including the answer: the binary
                        // arm used to close in silence for exactly the same
                        // reason (W6.11).
                        if bytes.len() > MAX_BINARY_SIZE {
                            warn!(
                                "Binary message too large ({} bytes) from {}",
                                bytes.len(),
                                my_id
                            );
                            queue_refusal(
                                &queue,
                                &refusal_tx,
                                message_too_large_frame(bytes.len(), MAX_BINARY_SIZE),
                            )
                            .await;
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
                                // The 16-byte target field is a *prefix* of a
                                // device id, not a device id. Resolve it before
                                // anything is re-framed or forwarded, so the
                                // frame can only ever be built for a device that
                                // was proved to be the one and only match.
                                match resolve_binary_target(&state.clients, &frame.target_field)
                                    .await
                                {
                                    BinaryTarget::Unique(target_id) => {
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
                                        // Naming the *resolved full* id is safe and does not
                                        // change a byte of the header: the field is that id's
                                        // first 16 bytes by construction, so re-encoding it
                                        // here reproduces the field that just verified. That
                                        // is the property that keeps the re-framed tag valid.
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
                                                    &target_id,
                                                    delivery_seq,
                                                    frame.payload,
                                                );
                                                forward_binary(
                                                    &state, &my_id, &target_id, &reframed,
                                                )
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
                                    // Zero matches, or more than one. Counted and
                                    // answered, delivered to nobody: an ambiguous
                                    // field is the case where guessing would put
                                    // a file in front of the wrong device.
                                    //
                                    // The sequence number has already advanced
                                    // (the frame was authentic, so it is not a
                                    // replay), which is what stops a sender that
                                    // keeps aiming at a missing device from
                                    // re-sending one sequence number forever.
                                    unroutable => {
                                        let rejection =
                                            unroutable.rejection(&my_id, frame.target_prefix);
                                        warn!("Dropping binary frame: {}", rejection.reason);
                                        state
                                            .metrics
                                            .messages_dropped_not_found
                                            .fetch_add(1, Ordering::Relaxed);
                                        let _ = queue
                                            .send(error_frame(rejection.code, rejection.reason))
                                            .await;
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
                                    // The frame verified and there is simply no
                                    // single device to hand it to. Counted with
                                    // the other "nowhere to go" drops, which is
                                    // what it is; a dedicated
                                    // `messages_dropped_ambiguous` would read
                                    // better but needs a new counter in
                                    // `metrics.rs`, which does not exist. Note
                                    // that a resolution failure never reaches
                                    // here — it is answered in the `Ok` arm
                                    // above — so this arm is defensive.
                                    RejectionKind::Unroutable => {
                                        state
                                            .metrics
                                            .messages_dropped_not_found
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
                // An over-ceiling frame never reaches the `MAX_*_SIZE` checks
                // above: `WebSocketConfig` refuses it while it is being read.
                // That is where most over-size frames are actually caught, so
                // this is where most of them must be answered (W6.11).
                if let Some((offered, ceiling)) = oversize_read_error(&e) {
                    warn!(
                        "Message too large ({} > {} bytes) from {}",
                        offered, ceiling, my_id
                    );
                    queue_refusal(
                        &queue,
                        &refusal_tx,
                        message_too_large_frame(offered, ceiling),
                    )
                    .await;
                } else {
                    warn!("WebSocket read error for {}: {}", my_id, e);
                }
                break;
            }
            Ok(None) => break,
            Err(_) => {
                warn!("Client {} timed out (no pong/message in 60s)", my_id);
                break;
            }
        }
    }

    // A refusal was queued above: give the writer its bounded moment to put it
    // on the wire before it is cancelled. Without this the abort below usually
    // wins the race and the client sees a bare disconnect with no reason
    // (W6.11).
    if *refusal_tx.borrow() {
        let _ = tokio::time::timeout(REFUSAL_FLUSH, &mut broadcast_task).await;
    }
    broadcast_task.abort();
    // Only if this connection still owns its entry: a device that reconnected
    // while we were winding down overwrote it, and evicting that newer
    // registration left a live socket permanently unroutable (W6.4).
    let removed = deregister_if_current(&state.clients, &my_id, connection_id).await;
    if !removed {
        info!(
            "Device {} already re-registered under a newer connection; leaving it alone",
            my_id
        );
    }
    state
        .metrics
        .connections_disconnected
        .fetch_add(1, Ordering::Relaxed);
    info!("Device disconnected: {}", my_id);
    // The refusal has been flushed and this connection is finished — but the
    // socket still holds the frame that caused it, so its close is a RST that
    // would take the refusal with it. Give the peer the REFUSAL_LINGER window
    // to read it before the halves drop (W6.11).
    if *refusal_tx.borrow() {
        linger_after_refusal().await;
    }
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

/// The documented `error` frame for a frame that exceeded a size ceiling.
///
/// Over-size traffic is stopped in five places — the auth loop's Text and
/// Binary arms, the main loop's, and the two read-time ceilings in
/// [`WebSocketConfig`] (which refuse the frame during the read and surface it
/// as the error those loops then answer) — and every path answers with this,
/// so the code and the wording cannot drift apart between them (W6.11:
/// `message_too_large` was documented in PROTOCOL.md §8 and never emitted
/// anywhere in the crate).
pub(crate) fn message_too_large_frame(offered: usize, ceiling: usize) -> Message {
    error_frame(
        "message_too_large",
        format!("message was {offered} bytes; the limit is {ceiling}"),
    )
}

/// `(offered_size, ceiling)` when a read failed because the peer sent a frame
/// larger than the configured `max_message_size`/`max_frame_size`.
///
/// This is how an over-size frame actually arrives: both ceilings in
/// [`WebSocketConfig`] sit at [`super::limits::MAX_TEXT_SIZE`], so tungstenite
/// refuses the frame during the read and the application's own checks never
/// see it. Matching the specific `MessageTooLong` variant matters — the other
/// `Capacity` errors (`TooManyHeaders` during the handshake) have nothing to
/// do with message size and must not be reported as `message_too_large`.
pub(crate) fn oversize_read_error(error: &tungstenite::Error) -> Option<(usize, usize)> {
    match error {
        tungstenite::Error::Capacity(tungstenite::error::CapacityError::MessageTooLong {
            size,
            max_size,
        }) => Some((*size, *max_size)),
        _ => None,
    }
}

/// The `error` frame for a message stamped with a `protocol_version` this
/// relay does not speak, or `None` when the message is acceptable.
///
/// `protocol_version` is optional — PROTOCOL.md §1.1 says a message *MAY*
/// carry one — so a missing field is accepted: the sender declared nothing
/// that could disagree with the relay, and that is what every current client
/// sends. A declared value at or below `PROTOCOL_VERSION` is accepted too, so
/// a current or older client keeps working. Only a *higher* value is refused,
/// because that is the one case where the relay knows it is being handed
/// semantics it has never heard of; the code and the wording are exactly what
/// the desktop hub already answers (PROTOCOL.md §8.1).
///
/// The check belongs on the raw `Value`, not in the typed path: neither
/// `RelayAuth` nor `RelayRoute` has a `protocol_version` field, so serde drops
/// the value on the floor and the typed parse can never see it. Both call
/// sites therefore run this immediately after parsing the frame and before
/// acting on it — before authentication succeeds, and before dispatch.
pub(crate) fn unsupported_version_frame(json: &Value) -> Option<Message> {
    let version = json.get("protocol_version")?.as_u64()?;
    if version > u64::from(conduit_protocol::PROTOCOL_VERSION) {
        return Some(error_frame(
            "unsupported_protocol_version",
            format!(
                "Server supports protocol_version {}, got {}",
                conduit_protocol::PROTOCOL_VERSION,
                version
            ),
        ));
    }
    None
}

/// Hand a refusal frame to the outbound queue and tell the writer to flush it
/// and exit — the read loop is about to `break` and let the connection close.
///
/// Bounded because this runs on the way *out*: `Queue::send` waits for a slot
/// and byte budget from this connection's own queue, which a wedged peer could
/// have filled, and a teardown path that can hang holds an `active_connections`
/// slot open forever. If the refusal cannot be queued in time it is dropped —
/// the close still happens, which is the same guarantee the relay had before.
async fn queue_refusal(queue: &Queue, refusal_tx: &watch::Sender<bool>, frame: Message) {
    if tokio::time::timeout(REFUSAL_FLUSH, queue.send(frame))
        .await
        .is_ok()
    {
        let _ = refusal_tx.send(true);
    }
}

/// Keep a socket open for [`REFUSAL_LINGER`] after an over-size refusal has
/// been written to it, so the peer can actually read the refusal.
///
/// This wait exists because of a TCP detail the protocol never mentions: a
/// socket closed while unread bytes sit in its receive queue is closed with a
/// RST instead of a FIN, and a peer that receives a RST has everything it has
/// not yet handed to the application discarded — including frames that arrived
/// before the RST. An over-size refusal is precisely that situation, because
/// that is what refused it: tungstenite rejects an over-ceiling frame from its
/// header alone, so the payload it refused is still queued on our side when we
/// answer. The refusal frame is written correctly and then destroyed microseconds
/// later, and the client sees the bare disconnect the frame was meant to
/// replace.
///
/// The halves that own the socket are still alive here — the writer has
/// flushed, the reader has not been dropped — so sleeping is enough: a peer
/// that is reading (every real client is in a read loop) takes the frame during
/// the wait, and only afterwards does the drop send its RST, by which point
/// there is nothing left to lose. The wait is unconditional rather than
/// "until the peer acknowledges" because the read loop has just been refused
/// the frame it was reading: polling it again errors on the same header
/// without consuming anything, so it would burn the budget in a spin.
///
/// No other refusal needs this. A bad token, an unsupported
/// `protocol_version`, a rejected signature — all of those are answers to
/// frames the relay consumed in full, so their receive queue is already empty
/// and their close is an ordinary FIN that the peer sees after the answer.
async fn linger_after_refusal() {
    tokio::time::sleep(REFUSAL_LINGER).await;
}
