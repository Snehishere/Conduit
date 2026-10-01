//! Fixed capacities, and the two rate limiters built on them.
//!
//! These are deliberately not configurable. They exist to bound memory and to
//! stop one client starving the others, and a value an operator can raise
//! through a config file is a value they will raise until something breaks.
//! The tunables a deployment legitimately needs — ports, enable flags, the auth
//! deadline — live in [`crate::Config`].

use std::collections::{HashMap, VecDeque};
use std::net::IpAddr;
use std::time::Instant;

/// Maximum inbound text-frame size before the connection is dropped.
///
/// Spec-mandated at 1 MB (PROTOCOL.md §8 `message_too_large`; the relay
/// capacity plan in docs/archive/ references this exact constant as
/// `MAX_TEXT_SIZE=1MB`). Bounded deliberately: `MAX_CONNECTIONS` allows 10k
/// concurrent sockets, so the per-frame ceiling is what keeps total inbound
/// buffering from becoming a memory-exhaustion vector.
pub(crate) const MAX_TEXT_SIZE: usize = 1024 * 1024; // 1 MB

/// Maximum inbound binary-frame size before the connection is dropped.
///
/// Deliberately the same number as [`MAX_TEXT_SIZE`]: the two are the same
/// resource, and a relay that caps one and not the other is not capped. The
/// largest legitimate binary payload is a 64 KiB file chunk plus its nonce, MAC
/// and v2 header (`CHUNK_SIZE` on both clients is `64 * 1024`), so there is an
/// order of magnitude of headroom below this, and `SIZE_BUCKETS` already tops
/// out at exactly 1 MiB — the metrics were built assuming this ceiling.
///
/// Until this existed, `handle_binary_frame` was handed whatever the socket
/// produced with no check at all, while the two Text arms enforced
/// [`MAX_TEXT_SIZE`]. `accept_async` was also given no `WebSocketConfig`, so
/// tungstenite's 64 MiB `max_message_size` was the only bound in the path.
pub(crate) const MAX_BINARY_SIZE: usize = MAX_TEXT_SIZE;
pub(crate) const MAX_CONNECTIONS: usize = 10_000;

/// How many messages one connection's outbound queue may hold.
///
/// This is a bound on *messages* and says nothing about bytes: 1024 messages
/// at the [`MAX_BINARY_SIZE`] ceiling would be 1 GiB. The byte ceiling is
/// [`QUEUE_BYTE_BUDGET`]; this exists only so a slow consumer cannot make the
/// relay allocate unbounded message *headers* regardless of that budget.
pub(crate) const QUEUE_DEPTH: usize = 1024;

/// Bytes one connection may hold in its outbound queue.
///
/// See [`crate::state::Queue`].
pub(crate) const QUEUE_BYTE_BUDGET: usize = 16 * 1024 * 1024;

/// Per-message token-bucket refill rate (messages per second).
pub(crate) const MSG_RATE_PER_SEC: f64 = 100.0;
/// Per-message token-bucket burst capacity.
pub(crate) const MSG_BURST: f64 = 50.0;

/// How long a forwarded message may sit in a target's outbound queue before
/// it is abandoned. Without this the relay's read loop blocks on a slow or
/// wedged client and stalls that connection indefinitely.
pub(crate) const FORWARD_TIMEOUT_SECS: u64 = 5;

/// A device id as documented in PROTOCOL.md §`device_id`: 1..=64 lowercase hex
/// characters, optionally grouped by single hyphens.
///
/// Enforced because `device_id` becomes a map key in `state.clients` and in the
/// nonce replay cache. With `MAX_TEXT_SIZE` at 1 MB an unvalidated id let a
/// single authenticated frame insert a megabyte-scale key into both maps.
pub(crate) const MAX_DEVICE_ID_LEN: usize = 64;

/// Validate a device id: 1..=64 lowercase hex characters and hyphens.
///
/// Uppercase is rejected on purpose — ids are compared as map keys, so
/// accepting two spellings of the same id would let one device evict another
/// from the routing table.
///
/// Hyphens are allowed because that is what the app actually produces. The
/// desktop mints its own id as `Uuid::new_v4().to_string()` and a phone's id is
/// either that or the per-connection UUID the pairing handshake falls back to,
/// so a strict "bare hex" rule would reject every real device at the auth gate.
/// PROTOCOL.md's "first 16 hex chars of the X25519 key" remains a valid
/// spelling; it is a floor, not the only one.
pub(crate) fn is_valid_device_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_DEVICE_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b) || b == b'-')
        // A hyphen cannot start or end the id, and cannot repeat: those are the
        // shapes that would make two spellings of one id look different.
        && !id.starts_with('-')
        && !id.ends_with('-')
        && !id.contains("--")
        && id.bytes().any(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Histogram bucket upper bounds (bytes) for `conduit_relay_message_size_bytes`.
///
/// Spans the interesting range for JSON control messages (~100 B) through to
/// the 1 MB frame ceiling. Without real buckets the histogram is a single
/// `+Inf` series that tells an operator nothing about where traffic sits.
pub(crate) const SIZE_BUCKETS: [u64; 8] =
    [128, 512, 2_048, 8_192, 32_768, 131_072, 524_288, 1_048_576];

/// Per-connection token bucket for inbound message rate limiting.
///
/// Drops excess messages (counted in `messages_dropped_rate_limit`) without
/// closing the connection, so a noisy client cannot starve the relay.
pub(crate) struct MessageRateLimiter {
    tokens: f64,
    last_refill: Instant,
    rate_per_sec: f64,
    burst: f64,
}

impl MessageRateLimiter {
    pub(crate) fn new() -> Self {
        Self::with_params(MSG_RATE_PER_SEC, MSG_BURST)
    }

    pub(crate) fn with_params(rate_per_sec: f64, burst: f64) -> Self {
        Self {
            tokens: burst,
            last_refill: Instant::now(),
            rate_per_sec,
            burst,
        }
    }

    /// Consume one token if available; refill from elapsed time first.
    pub(crate) fn allow(&mut self) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.last_refill = now;
        self.tokens = (self.tokens + elapsed * self.rate_per_sec).min(self.burst);
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

pub(crate) struct RateLimiter {
    windows: std::sync::Mutex<HashMap<IpAddr, VecDeque<Instant>>>,
    max_attempts: usize,
    window_secs: u64,
}

impl RateLimiter {
    pub(crate) fn new(max_attempts: usize, window_secs: u64) -> Self {
        Self {
            windows: std::sync::Mutex::new(HashMap::new()),
            max_attempts,
            window_secs,
        }
    }

    pub(crate) fn allow(&self, ip: IpAddr) -> bool {
        let mut windows = self.lock_windows();
        let now = Instant::now();
        let cutoff = now - std::time::Duration::from_secs(self.window_secs);

        let attempts = windows.entry(ip).or_default();
        while let Some(&front) = attempts.front() {
            if front < cutoff {
                attempts.pop_front();
            } else {
                break;
            }
        }

        if attempts.len() >= self.max_attempts {
            return false;
        }

        attempts.push_back(now);
        true
    }

    /// Drop entries whose whole window has expired; return how many were removed.
    ///
    /// Previously entries were pruned *only* when that same IP reconnected, so
    /// an attacker rotating source addresses (a botnet, IPv6 privacy addresses,
    /// a spoofed-range sweep) grew this map without bound. The relay calls
    /// [`RateLimiter::sweep`] on a timer so the map size is bounded by the
    /// number of *recently active* addresses, not by all addresses ever seen.
    pub(crate) fn sweep(&self) -> usize {
        let mut windows = self.lock_windows();
        let cutoff = Instant::now() - std::time::Duration::from_secs(self.window_secs);
        let before = windows.len();
        windows.retain(|_, attempts| {
            while let Some(&front) = attempts.front() {
                if front < cutoff {
                    attempts.pop_front();
                } else {
                    break;
                }
            }
            !attempts.is_empty()
        });
        before - windows.len()
    }

    /// Number of tracked addresses (used by tests and `/health`).
    pub(crate) fn tracked_addresses(&self) -> usize {
        self.lock_windows().len()
    }

    pub(crate) fn lock_windows(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<IpAddr, VecDeque<Instant>>> {
        // A poisoned lock means a previous holder panicked mid-update. The map
        // is a plain cache of timestamps, so recovering the data is strictly
        // better than propagating the panic into the accept loop.
        self.windows.lock().unwrap_or_else(|e| e.into_inner())
    }
}
