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
// The v2 binary-frame layout. These are the field widths and offsets the header
// is parsed with, and `BINARY_DEVICE_ID_LEN` is also the size of the target field
// that `resolve_binary_target` resolves — the whole reason that function exists
// is that this is smaller than a device id.
use conduit_protocol::{
    BINARY_AUTHENTICATED_PREFIX_LEN, BINARY_DEVICE_ID_LEN, BINARY_FRAME_VERSION, BINARY_HEADER_LEN,
    BINARY_TAG_LEN,
};
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
    /// Verified, but the frame names no single deliverable device: its 16-byte
    /// target field resolved to zero or to more than one connected device.
    ///
    /// Not `Integrity` — the frame is authentic, and the tag already said so.
    /// Not `Replay` either. It is counted as a drop with nowhere to go, because
    /// that is what happened: the sender asked for a device that is not there
    /// (or is ambiguous), and nothing was forwarded.
    Unroutable,
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

    pub(crate) fn unroutable(code: &'static str, reason: impl Into<String>) -> Self {
        Self {
            kind: RejectionKind::Unroutable,
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
///
/// `target_field` is the 16 wire bytes and is **not** a device id. A device id is
/// up to 64 characters and these 16 are only its prefix, so this field has to be
/// resolved against the routing table before it can name anybody — see
/// [`resolve_binary_target`], which is what the forward path uses.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct VerifiedBinaryFrame<'a> {
    /// The target field exactly as it arrived, padding included.
    pub(crate) target_field: [u8; BINARY_DEVICE_ID_LEN],
    /// The same field as text with the NUL padding stripped. For logs and error
    /// messages only — never route on this string, it is a prefix.
    pub(crate) target_prefix: &'a str,
    pub(crate) payload: &'a [u8],
}

/// Verify a v2 binary relay frame: version, target id, HMAC tag and sequence.
///
/// Deliberately independent of the routing table. Verification answers "is this
/// an authentic frame from the device on this connection"; routing answers "which
/// single connected device does its target field name", and it is a separate
/// step ([`resolve_binary_target`]) so that a frame which fails it is dropped
/// rather than re-framed at a guessed target.
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
    let field_bytes = &bytes[1..target_end];
    let target_field: [u8; BINARY_DEVICE_ID_LEN] = field_bytes
        .try_into()
        .expect("slice is exactly BINARY_DEVICE_ID_LEN bytes");
    // Parsed from the frame rather than from the copy above, so the prefix
    // borrows `bytes` and can be handed back with the frame's lifetime.
    let target_prefix = match conduit_protocol::parse_binary_target_field(field_bytes) {
        Ok(prefix) => prefix,
        Err(conduit_protocol::BinaryTargetFieldError::NotUtf8) => {
            return Err(Rejection::integrity(
                "binary_target_id_invalid",
                "target device id is not valid UTF-8",
            ));
        }
        Err(conduit_protocol::BinaryTargetFieldError::Empty) => {
            return Err(Rejection::integrity(
                "binary_target_id_empty",
                "target device id is empty",
            ));
        }
    };

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

    Ok(Some(VerifiedBinaryFrame {
        target_field,
        target_prefix,
        payload,
    }))
}

/// What a v2 frame's 16-byte target field resolved to against the routing table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum BinaryTarget {
    /// Exactly one connected device has this canonical field. The String is that
    /// device's **full** id, so callers can re-frame and route with it directly.
    Unique(String),
    /// No connected device does.
    NotConnected,
    /// Two or more connected devices do, and the field cannot say which.
    Ambiguous {
        /// A **lower bound**: the scan stops at the second match, because two is
        /// already enough to refuse and the exact size changes nothing. Counted,
        /// never named — the routing table is not something an authenticated peer
        /// gets to enumerate.
        candidates: usize,
    },
}

/// Resolve a v2 frame's 16-byte target field to exactly one connected device.
///
/// This is the crux of the binary path. The field is 16 bytes
/// ([`conduit_protocol::BINARY_DEVICE_ID_LEN`]) and a real device id is a
/// 36-character UUID, so what arrives is a *prefix*, while
/// [`crate::state::Clients`] is keyed by full ids. Comparing a prefix against a
/// full id never matched, which is why the whole binary file path was dead in
/// both directions.
///
/// The resolution is deliberately three-valued and fails closed on both
/// non-unique answers:
///
/// * `NotConnected` — nobody is there. Dropped and counted; nothing is guessed.
/// * `Ambiguous` — two or more connected devices have the same canonical field,
///   so the bytes cannot say which one is meant. Dropped and counted, and
///   **delivered to neither**. Picking either would hand a file to the wrong
///   device, which is the one outcome worse than losing it.
/// * `Unique` — the only answer that forwards.
///
/// A linear scan, not a reverse index: the cost is 16 bytes of comparison per
/// connected device, against an HMAC over the payload and a copy of it that the
/// same frame has already paid for, and `MAX_CONNECTIONS` bounds it at 10 000. An
/// index would have to be kept in step with registration and deregistration,
/// which is where this table's harder bugs already live (see
/// [`crate::state::deregister_if_current`]). If that trade ever inverts, this is
/// the function to replace — it is the only reader of the field.
///
/// The read guard is held for the scan only; the caller sends to the returned
/// device through [`forward_binary`], which re-reads the table itself.
pub(crate) async fn resolve_binary_target(
    clients: &crate::state::Clients,
    field: &[u8; BINARY_DEVICE_ID_LEN],
) -> BinaryTarget {
    let clients = clients.read().await;
    let mut found: Option<&String> = None;
    let mut candidates = 0usize;
    for device_id in clients.keys() {
        if !conduit_protocol::binary_target_matches(device_id, field) {
            continue;
        }
        candidates += 1;
        if candidates == 1 {
            found = Some(device_id);
        } else {
            // Two is already ambiguous, and the third match changes no decision:
            // `candidates` becomes a lower bound rather than a census.
            break;
        }
    }
    match (found, candidates) {
        (Some(device_id), 1) => BinaryTarget::Unique(device_id.clone()),
        (_, 0) => BinaryTarget::NotConnected,
        _ => BinaryTarget::Ambiguous { candidates },
    }
}

impl BinaryTarget {
    /// The refusal for a target that is not [`BinaryTarget::Unique`], carrying
    /// the code the sender is answered with.
    ///
    /// `Unique` has no refusal — it is the only value that forwards — and
    /// returning one from here would be a bug, so it panics rather than
    /// inventing a code. The reason text names the prefix and the count, never
    /// the candidates: the routing table is not something an authenticated peer
    /// gets to enumerate.
    pub(crate) fn rejection(&self, from_device_id: &str, prefix: &str) -> Rejection {
        match *self {
            BinaryTarget::Unique(_) => {
                panic!("a resolved binary target is not a rejection: {from_device_id} -> {prefix}")
            }
            BinaryTarget::NotConnected => Rejection::unroutable(
                "binary_target_not_found",
                format!(
                    "no connected device is named by the target field {prefix:?} \
                     (from {from_device_id})"
                ),
            ),
            BinaryTarget::Ambiguous { candidates } => Rejection::unroutable(
                "binary_target_ambiguous",
                format!(
                    "the target field {prefix:?} is shared by more than one connected \
                     device (at least {candidates}) and cannot be resolved to one \
                     recipient (from {from_device_id}); nothing was delivered"
                ),
            ),
        }
    }
}

/// Byte string the binary-frame tag is computed over:
/// `from_device_id || 0x1F || frame[0..21] || payload`.
///
/// Binding the sender id is what stops one authenticated client from replaying
/// a frame it captured from another (the 16-byte header names only the
/// *recipient*, and only by prefix).
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
///
/// `to_device_id` is a **full** device id, not a target field: callers resolve
/// the field with [`resolve_binary_target`] first, so the `get` below is an exact
/// hit and its `not_found` arm only covers the window between that resolution
/// and this send (the target disconnected in between).
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
