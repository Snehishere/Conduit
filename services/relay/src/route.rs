//! Verification and forwarding: the relay’s entire reason to exist.
//!
//! A `relay_route` is verified against the signing keyring, its nonce is
//! consumed by the replay cache, and only then is the inner payload forwarded to
//! the named device. The order matters: an unauthenticated peer must not be
//! able to spend replay-cache entries, and a bad signature must not be
//! forwarded on the strength of a well-formed id.

use std::sync::Arc;

use conduit_protocol::RelayRoute;
use conduit_protocol::hmac::NonceCache;
use log::warn;
use serde_json::Value;
use tokio::sync::RwLock;
use tungstenite::Message;

use std::sync::atomic::Ordering;

use super::limits::{FORWARD_TIMEOUT_SECS, MAX_DEVICE_ID_LEN, is_valid_device_id};
use super::state::{AppState, SendOutcome};

pub(crate) const NOT_WRAPPED_MSG: &str = "'encrypted' messages must be wrapped in a signed relay_route so the sender can be \
     authenticated";

/// Why a message was refused, which selects the metric it is counted under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RejectionKind {
    /// Signature/tag did not verify, or the claimed sender did not match the
    /// authenticated connection identity.
    Integrity,
    /// Replayed nonce or stale timestamp / non-increasing binary sequence.
    Replay,
}

/// A refusal, carrying the machine-readable code sent in the `error` frame.
#[derive(Debug, Clone)]
pub(crate) struct Rejection {
    pub(crate) kind: RejectionKind,
    pub(crate) code: &'static str,
    pub(crate) reason: String,
}

impl Rejection {
    pub(crate) fn integrity(code: &'static str, reason: impl Into<String>) -> Self {
        Self {
            kind: RejectionKind::Integrity,
            code,
            reason: reason.into(),
        }
    }

    pub(crate) fn replay(code: &'static str, reason: impl Into<String>) -> Self {
        Self {
            kind: RejectionKind::Replay,
            code,
            reason: reason.into(),
        }
    }
}

/// Validate a claimed `device_id` from `relay_auth`.
pub(crate) fn validate_device_id(id: &str) -> Result<(), String> {
    if is_valid_device_id(id) {
        return Ok(());
    }
    Err(format!(
        "device_id must be 1-{} lowercase hex characters, optionally separated by single \
         hyphens (PROTOCOL.md's 16-char X25519 prefix is one valid spelling); got {} \
         character(s)",
        MAX_DEVICE_ID_LEN,
        id.len()
    ))
}

/// Verify and de-duplicate an inbound `relay_route`, returning the typed route.
///
/// Steps, in order — each fails closed:
///
/// 1. Parse into [`RelayRoute`] (the canonical type, not raw `Value`).
/// 2. Require `from_device_id`, `timestamp` and `nonce` to be present.
/// 3. Require `from_device_id == authenticated_device_id`. This is the check
///    that makes message-level authenticity real: a client cannot emit a route
///    claiming to be anybody else, because the claim is signed *and* compared
///    against the identity the relay established during `relay_auth`.
/// 4. Verify the HMAC against the keyring (accepting `{current, previous}`
///    during a rotation window, keyed off the signed `key_id`).
/// 5. Replay-check the nonce, scoped to the authenticated device id.
pub(crate) async fn handle_relay_route(
    route_keys: &dyn crate::state::RouteKeys,
    nonces: &Arc<RwLock<NonceCache>>,
    json: &Value,
    authenticated_device_id: &str,
) -> Result<RelayRoute, Rejection> {
    let route: RelayRoute = serde_json::from_value(json.clone())
        .map_err(|e| Rejection::integrity("malformed_relay_route", e.to_string()))?;

    if !route.has_required_signed_fields() {
        return Err(Rejection::integrity(
            "incomplete_relay_route",
            "relay_route requires from_device_id, timestamp and nonce",
        ));
    }

    // The signed claim is compared against the identity established by
    // relay_auth on THIS connection. A valid signature alone is not enough:
    // every client holds the signing key, so the signature only proves the
    // message was produced by a legitimate client, not that it was produced by
    // *this* client.
    if route.from_device() != Some(authenticated_device_id) {
        return Err(Rejection::integrity(
            "sender_mismatch",
            format!(
                "relay_route claims from_device_id {:?} but the authenticated \
                 connection is {authenticated_device_id:?}",
                route.from_device()
            ),
        ));
    }

    // Per-device keys: the key is looked up under the device the message is
    // attributed to — which the `sender_mismatch` check above already pinned to
    // *this* connection — so there is no `key_id` to trust and no shared key to
    // try. A device can sign for itself and for nothing else.
    let Some(secret) = route_keys.key_for(authenticated_device_id) else {
        return Err(Rejection::integrity(
            "unknown_device",
            format!("no route key registered for {authenticated_device_id}"),
        ));
    };
    if !crate::hmac::verify_message_hmac(&secret, json) {
        return Err(Rejection::integrity(
            "hmac_invalid",
            "signature did not verify under the sender's own route key",
        ));
    }

    if route.to_device_id.is_empty() {
        return Err(Rejection::integrity(
            "missing_target",
            "relay_route is missing to_device_id",
        ));
    }

    let timestamp = route.timestamp.unwrap_or_default();
    let nonce = route.nonce.as_deref().unwrap_or_default();
    let accepted = nonces
        .write()
        .await
        .check_replay(authenticated_device_id, timestamp, nonce);
    if !accepted {
        return Err(Rejection::replay(
            "replay_detected",
            "nonce already used, or timestamp outside the accepted window",
        ));
    }

    Ok(route)
}

/// A verified, in-window binary relay frame.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VerifiedBinaryFrame<'a> {
    pub(crate) target_id: String,
    pub(crate) payload: &'a [u8],
}

/// Verify a v2 binary relay frame: version, target id, HMAC tag and sequence.
///
/// `Ok(None)` is reserved for frames that are structurally valid but carry
/// nothing to route (currently unreachable; kept so future "accept and ignore"
/// cases stay explicit rather than becoming silent drops).
pub(crate) async fn handle_binary_frame<'a>(
    _state: &Arc<AppState>,
    route_keys: &dyn crate::state::RouteKeys,
    authenticated_device_id: &str,
    bytes: &'a [u8],
    last_seq: &mut Option<u32>,
) -> Result<Option<VerifiedBinaryFrame<'a>>, Rejection> {
    use conduit_protocol::{
        BINARY_AUTHENTICATED_PREFIX_LEN, BINARY_DEVICE_ID_LEN, BINARY_FRAME_VERSION,
        BINARY_HEADER_LEN, BINARY_TAG_LEN,
    };

    if bytes.len() < BINARY_HEADER_LEN {
        return Err(Rejection::integrity(
            "binary_frame_too_short",
            format!(
                "binary frame is {} bytes; v2 requires at least {BINARY_HEADER_LEN}",
                bytes.len()
            ),
        ));
    }

    if bytes[0] != BINARY_FRAME_VERSION {
        return Err(Rejection::integrity(
            "binary_version_unsupported",
            format!(
                "binary frame version 0x{:02x} is not supported; this relay speaks \
                 0x{BINARY_FRAME_VERSION:02x}",
                bytes[0]
            ),
        ));
    }

    let target_end = 1 + BINARY_DEVICE_ID_LEN;
    let target_id = std::str::from_utf8(&bytes[1..target_end])
        .map_err(|_| {
            Rejection::integrity(
                "binary_target_id_invalid",
                "target device id is not valid UTF-8",
            )
        })?
        .trim_end_matches('\0')
        .to_string();
    if target_id.is_empty() {
        return Err(Rejection::integrity(
            "binary_target_id_empty",
            "target device id is empty",
        ));
    }

    let seq_start = target_end;
    let seq = u32::from_be_bytes([
        bytes[seq_start],
        bytes[seq_start + 1],
        bytes[seq_start + 2],
        bytes[seq_start + 3],
    ]);
    if last_seq.is_some_and(|prev| seq <= prev) {
        return Err(Rejection::replay(
            "binary_replay_detected",
            format!(
                "sequence {seq} is not greater than the last accepted {}",
                last_seq.unwrap_or_default()
            ),
        ));
    }

    let tag_start = BINARY_AUTHENTICATED_PREFIX_LEN;
    let tag_end = tag_start + BINARY_TAG_LEN;
    let tag = &bytes[tag_start..tag_end];
    let payload = &bytes[BINARY_HEADER_LEN..];

    if !verify_binary_tag(route_keys, authenticated_device_id, bytes, payload, tag) {
        return Err(Rejection::integrity(
            "binary_hmac_invalid",
            "binary frame tag did not verify",
        ));
    }

    *last_seq = Some(seq);

    Ok(Some(VerifiedBinaryFrame { target_id, payload }))
}

/// Byte string the binary-frame tag is computed over:
/// `from_device_id || 0x1F || frame[0..21] || payload`.
///
/// Binding the sender id is what stops one authenticated client from replaying
/// a frame it captured from another (the 16-byte header only names the
/// *recipient*).
pub(crate) fn binary_mac_input(
    authenticated_device_id: &str,
    header_and_payload: &[u8],
) -> Vec<u8> {
    // Delegates to the protocol crate so the bytes verified here and the bytes
    // stamped by `conduit_protocol::build_binary_frame` cannot drift apart.
    conduit_protocol::binary_mac_input(authenticated_device_id, header_and_payload)
}

/// Verify a v2 binary frame tag.
///
/// Unlike `relay_route`, the frame carries no `key_id` (there is no room in the
/// fixed header). That is not a problem here: the signer is the device
/// authenticated on *this* connection, whose id is already bound into the MAC
/// input, so there is exactly one key to try. The previous design tried every
/// key in a rotation set, which was both slower and a wider oracle.
pub(crate) fn verify_binary_tag(
    route_keys: &dyn crate::state::RouteKeys,
    authenticated_device_id: &str,
    frame: &[u8],
    payload: &[u8],
    tag: &[u8],
) -> bool {
    use conduit_protocol::BINARY_AUTHENTICATED_PREFIX_LEN;

    let Some(secret) = route_keys.key_for(authenticated_device_id) else {
        warn!("Binary frame from unregistered device {authenticated_device_id}");
        return false;
    };

    let mut header = Vec::with_capacity(BINARY_AUTHENTICATED_PREFIX_LEN + payload.len());
    header.extend_from_slice(&frame[..BINARY_AUTHENTICATED_PREFIX_LEN]);
    header.extend_from_slice(payload);
    let input = binary_mac_input(authenticated_device_id, &header);
    let input_hex = hex::encode(&input);
    let tag_hex = hex::encode(tag);

    crate::hmac::verify_hmac(&secret, &input_hex, &tag_hex)
}

/// Forward a text payload, updating the routing metrics.
pub(crate) async fn forward_text(
    state: &Arc<AppState>,
    from_device_id: &str,
    to_device_id: &str,
    payload: Value,
) {
    forward_text_with_timeout(
        state,
        from_device_id,
        to_device_id,
        payload,
        std::time::Duration::from_secs(FORWARD_TIMEOUT_SECS),
    )
    .await;
}

/// [`forward_text`] with an explicit queueing budget.
///
/// The production path uses [`FORWARD_TIMEOUT_SECS`]; the parameter exists so
/// tests can prove the `messages_dropped_timeout` path without waiting 5 s.
pub(crate) async fn forward_text_with_timeout(
    state: &Arc<AppState>,
    from_device_id: &str,
    to_device_id: &str,
    payload: Value,
    timeout: std::time::Duration,
) {
    // Look the target up, clone its queue, and drop the guard *before* the
    // send below. Holding the read guard across the timeout await meant a
    // single wedged target held the routing table's read lock for the whole
    // `FORWARD_TIMEOUT_SECS`, and tokio's `RwLock` is write-preferring, so
    // registration, deregistration and the 30 s sweep all queued behind it.
    //
    // The clone is what makes this safe to do: `Queue` pairs its sender with
    // an `Arc<Semaphore>` byte budget, and a clone shares that same `Arc`, so
    // a message reserved through this clone draws down the budget the table's
    // entry still points at — the accounting in `state::Queue` is unaffected
    // by how many clones exist. What changes is only the lock: the entry may
    // now be replaced or removed while this send waits, which is exactly the
    // race the guard used to hide, and it resolves the same way it does for
    // any forward that outlives its target — `SendOutcome::Closed` counted as
    // `not_found`.
    let target_queue = {
        let clients = state.clients.read().await;
        let Some(target_queue) = clients.get(to_device_id) else {
            warn!(
                "Relay drop: target {} not connected (from {})",
                to_device_id, from_device_id
            );
            state
                .metrics
                .messages_dropped_not_found
                .fetch_add(1, Ordering::Relaxed);
            return;
        };
        target_queue.clone()
    };

    // Bounded wait: without this a target that stops reading (its outbound
    // queue fills, or its byte budget is exhausted) blocks this sender's read
    // loop indefinitely. The drop is counted as `timeout`, which is what that
    // metric was always meant for. Waiting for budget and waiting for a slot
    // are the same wait from here, and both live inside this timeout.
    let send = target_queue.send(Message::Text(payload.to_string()));
    match tokio::time::timeout(timeout, send).await {
        Ok(SendOutcome::Sent) => {
            state
                .metrics
                .messages_routed
                .fetch_add(1, Ordering::Relaxed);
        }
        Ok(SendOutcome::Closed) => {
            warn!(
                "Relay drop: target {} channel closed (from {})",
                to_device_id, from_device_id
            );
            state
                .metrics
                .messages_dropped_not_found
                .fetch_add(1, Ordering::Relaxed);
        }
        Err(_) => {
            warn!(
                "Relay drop: target {} did not accept within {}s (from {})",
                to_device_id,
                timeout.as_secs(),
                from_device_id
            );
            state
                .metrics
                .messages_dropped_timeout
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Forward a binary payload, updating the routing metrics.
pub(crate) async fn forward_binary(
    state: &Arc<AppState>,
    from_device_id: &str,
    to_device_id: &str,
    payload: &[u8],
) {
    // Clone the queue out and drop the read guard before awaiting, for the
    // same reason as `forward_text_with_timeout`: a slow target must not
    // hold the routing table's write-preferring lock hostage. The clone
    // shares the target's byte budget, so the reservation still counts
    // against `QUEUE_BYTE_BUDGET`.
    let target_queue = {
        let clients = state.clients.read().await;
        let Some(target_queue) = clients.get(to_device_id) else {
            warn!(
                "Binary relay drop: target {} not connected (from {})",
                to_device_id, from_device_id
            );
            state
                .metrics
                .messages_dropped_not_found
                .fetch_add(1, Ordering::Relaxed);
            return;
        };
        target_queue.clone()
    };

    // Budget reserved and released inside `Queue::send`; see `forward_text`.
    let send = target_queue.send(Message::Binary(payload.to_vec()));
    match tokio::time::timeout(std::time::Duration::from_secs(FORWARD_TIMEOUT_SECS), send).await {
        Ok(SendOutcome::Sent) => {
            state
                .metrics
                .messages_routed
                .fetch_add(1, Ordering::Relaxed);
        }
        Ok(SendOutcome::Closed) => {
            warn!(
                "Binary relay drop: target {} channel closed (from {})",
                to_device_id, from_device_id
            );
            state
                .metrics
                .messages_dropped_not_found
                .fetch_add(1, Ordering::Relaxed);
        }
        Err(_) => {
            warn!(
                "Binary relay drop: target {} did not accept within {}s (from {})",
                to_device_id, FORWARD_TIMEOUT_SECS, from_device_id
            );
            state
                .metrics
                .messages_dropped_timeout
                .fetch_add(1, Ordering::Relaxed);
        }
    }
}
