//! Counters and the two exposition formats that render them.
//!
//! `/health` returns a JSON document and `/metrics` returns Prometheus text.
//! They read the same counters, so the two can never disagree about what the
//! relay is doing.

use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::Value;

use super::limits::{MAX_CONNECTIONS, SIZE_BUCKETS};
use super::state::AppState;

pub(crate) struct Metrics {
    // Counters – connections
    pub(crate) connections_connected: AtomicU64,
    pub(crate) connections_disconnected: AtomicU64,
    // Counters – auth
    pub(crate) auth_attempts_success: AtomicU64,
    pub(crate) auth_attempts_failure: AtomicU64,
    // Counters – messages
    pub(crate) messages_routed: AtomicU64,
    pub(crate) messages_dropped_not_found: AtomicU64,
    pub(crate) messages_dropped_timeout: AtomicU64,
    pub(crate) messages_dropped_rate_limit: AtomicU64,
    /// Replay rejection or expired timestamp on a `relay_route`.
    pub(crate) messages_dropped_replay: AtomicU64,
    /// Signature verification failure (bad key, wrong key_id, tampered body).
    pub(crate) messages_dropped_hmac_failed: AtomicU64,
    /// A message the relay has no handler for — silently dropped before, now
    /// counted and answered with an `error` frame.
    pub(crate) messages_dropped_unknown_type: AtomicU64,
    // Histogram – message size (sum + count + per-bucket counters)
    pub(crate) message_size_bytes_sum: AtomicU64,
    pub(crate) message_size_bytes_count: AtomicU64,
    /// `SIZE_BUCKETS.len()` entries; cumulative counts are derived at scrape time.
    pub(crate) message_size_buckets: Vec<AtomicU64>,
}

impl Metrics {
    pub(crate) fn new() -> Self {
        Self {
            connections_connected: AtomicU64::new(0),
            connections_disconnected: AtomicU64::new(0),
            auth_attempts_success: AtomicU64::new(0),
            auth_attempts_failure: AtomicU64::new(0),
            messages_routed: AtomicU64::new(0),
            messages_dropped_not_found: AtomicU64::new(0),
            messages_dropped_timeout: AtomicU64::new(0),
            messages_dropped_rate_limit: AtomicU64::new(0),
            messages_dropped_replay: AtomicU64::new(0),
            messages_dropped_hmac_failed: AtomicU64::new(0),
            messages_dropped_unknown_type: AtomicU64::new(0),
            message_size_bytes_sum: AtomicU64::new(0),
            message_size_bytes_count: AtomicU64::new(0),
            message_size_buckets: (0..SIZE_BUCKETS.len()).map(|_| AtomicU64::new(0)).collect(),
        }
    }

    /// Record an observed message size into the histogram.
    pub(crate) fn observe_message_size(&self, bytes: usize) {
        self.message_size_bytes_sum
            .fetch_add(bytes as u64, Ordering::Relaxed);
        self.message_size_bytes_count
            .fetch_add(1, Ordering::Relaxed);
        for (i, bound) in SIZE_BUCKETS.iter().enumerate() {
            if bytes as u64 <= *bound {
                self.message_size_buckets[i].fetch_add(1, Ordering::Relaxed);
                break;
            }
        }
    }
}

/// The JSON document `GET /health` returns.
///
/// Deliberately a summary rather than a second copy of `/metrics`: the two read
/// the same counters, so they cannot disagree, but only one of them is meant to
/// be read by a human.
pub(crate) fn build_metrics_json(state: &AppState) -> Value {
    let active = state.active_connections.load(Ordering::Relaxed);
    let m = &state.metrics;
    let conn_connected = m.connections_connected.load(Ordering::Relaxed);
    let conn_disconnected = m.connections_disconnected.load(Ordering::Relaxed);
    let auth_ok = m.auth_attempts_success.load(Ordering::Relaxed);
    let auth_fail = m.auth_attempts_failure.load(Ordering::Relaxed);
    let routed = m.messages_routed.load(Ordering::Relaxed);
    let dropped_nf = m.messages_dropped_not_found.load(Ordering::Relaxed);
    let dropped_to = m.messages_dropped_timeout.load(Ordering::Relaxed);
    let dropped_rl = m.messages_dropped_rate_limit.load(Ordering::Relaxed);
    let dropped_replay = m.messages_dropped_replay.load(Ordering::Relaxed);
    let dropped_hmac = m.messages_dropped_hmac_failed.load(Ordering::Relaxed);
    let dropped_unknown = m.messages_dropped_unknown_type.load(Ordering::Relaxed);
    let dropped_total =
        dropped_nf + dropped_to + dropped_rl + dropped_replay + dropped_hmac + dropped_unknown;

    serde_json::json!({
        "status": "ok",
        "active_connections": active,
        "max_connections": MAX_CONNECTIONS,
        "total_connections": conn_connected,
        // Previously read into `_`-prefixed bindings and never emitted, so
        // `/health` and `/metrics` disagreed about the same counters.
        "total_connections_disconnected": conn_disconnected,
        "total_auth_successes": auth_ok,
        "total_auth_failures": auth_fail,
        "total_messages_routed": routed,
        "total_messages_dropped": dropped_total,
        "messages_dropped": {
            "not_found": dropped_nf,
            "timeout": dropped_to,
            "rate_limit": dropped_rl,
            "replay": dropped_replay,
            "hmac_failed": dropped_hmac,
            "unknown_type": dropped_unknown,
        },
        "tls_pin": state.tls_pin,
    })
}

/// Build Prometheus exposition format text.
pub(crate) fn build_prometheus_metrics(state: &AppState) -> String {
    let active = state.active_connections.load(Ordering::Relaxed);
    let registered_devices = state.clients.try_read().map(|c| c.len()).unwrap_or(0);
    let m = &state.metrics;
    let conn_connected = m.connections_connected.load(Ordering::Relaxed);
    let conn_disconnected = m.connections_disconnected.load(Ordering::Relaxed);
    let auth_ok = m.auth_attempts_success.load(Ordering::Relaxed);
    let auth_fail = m.auth_attempts_failure.load(Ordering::Relaxed);
    let routed = m.messages_routed.load(Ordering::Relaxed);
    let dropped_nf = m.messages_dropped_not_found.load(Ordering::Relaxed);
    let dropped_to = m.messages_dropped_timeout.load(Ordering::Relaxed);
    let dropped_rl = m.messages_dropped_rate_limit.load(Ordering::Relaxed);
    let dropped_replay = m.messages_dropped_replay.load(Ordering::Relaxed);
    let dropped_hmac = m.messages_dropped_hmac_failed.load(Ordering::Relaxed);
    let dropped_unknown = m.messages_dropped_unknown_type.load(Ordering::Relaxed);
    let msg_size_sum = m.message_size_bytes_sum.load(Ordering::Relaxed);
    let msg_size_count = m.message_size_bytes_count.load(Ordering::Relaxed);

    let mut buf = String::with_capacity(2048);

    // --- Counters: connections ---
    buf.push_str("# HELP conduit_relay_connections_total Total connection events\n");
    buf.push_str("# TYPE conduit_relay_connections_total counter\n");
    buf.push_str(&format!(
        "conduit_relay_connections_total{{event=\"connected\"}} {}\n",
        conn_connected
    ));
    buf.push_str(&format!(
        "conduit_relay_connections_total{{event=\"disconnected\"}} {}\n",
        conn_disconnected
    ));
    buf.push('\n');

    // --- Counters: auth ---
    buf.push_str("# HELP conduit_relay_auth_attempts_total Total auth attempts\n");
    buf.push_str("# TYPE conduit_relay_auth_attempts_total counter\n");
    buf.push_str(&format!(
        "conduit_relay_auth_attempts_total{{result=\"success\"}} {}\n",
        auth_ok
    ));
    buf.push_str(&format!(
        "conduit_relay_auth_attempts_total{{result=\"failure\"}} {}\n",
        auth_fail
    ));
    buf.push('\n');

    // --- Counters: messages ---
    buf.push_str("# HELP conduit_relay_messages_routed_total Total messages routed\n");
    buf.push_str("# TYPE conduit_relay_messages_routed_total counter\n");
    buf.push_str(&format!(
        "conduit_relay_messages_routed_total{{source=\"device\"}} {}\n",
        routed
    ));
    buf.push('\n');

    buf.push_str("# HELP conduit_relay_messages_dropped_total Total messages dropped\n");
    buf.push_str("# TYPE conduit_relay_messages_dropped_total counter\n");
    for (reason, value) in [
        ("not_found", dropped_nf),
        ("timeout", dropped_to),
        ("rate_limit", dropped_rl),
        ("replay", dropped_replay),
        ("hmac_failed", dropped_hmac),
        ("unknown_type", dropped_unknown),
    ] {
        buf.push_str(&format!(
            "conduit_relay_messages_dropped_total{{reason=\"{reason}\"}} {value}\n"
        ));
    }
    buf.push('\n');

    // --- Gauges ---
    buf.push_str("# HELP conduit_relay_active_connections Current active connections\n");
    buf.push_str("# TYPE conduit_relay_active_connections gauge\n");
    buf.push_str(&format!("conduit_relay_active_connections {}\n", active));
    buf.push('\n');

    buf.push_str("# HELP conduit_relay_max_connections Maximum allowed connections\n");
    buf.push_str("# TYPE conduit_relay_max_connections gauge\n");
    buf.push_str(&format!(
        "conduit_relay_max_connections {}\n",
        MAX_CONNECTIONS
    ));
    buf.push('\n');

    // --- Gauges: internal map sizes ---
    // These are the two structures that used to grow without bound (VULNERABILITY 5);
    // making them visible is how an operator would notice a regression.
    buf.push_str("# HELP conduit_relay_registered_devices Devices in the routing table\n");
    buf.push_str("# TYPE conduit_relay_registered_devices gauge\n");
    buf.push_str(&format!(
        "conduit_relay_registered_devices {}\n",
        registered_devices
    ));
    buf.push('\n');

    buf.push_str(
        "# HELP conduit_relay_rate_limit_tracked_ips Source addresses in the connection rate limiter\n",
    );
    buf.push_str("# TYPE conduit_relay_rate_limit_tracked_ips gauge\n");
    buf.push_str(&format!(
        "conduit_relay_rate_limit_tracked_ips {}\n",
        state.rate_limiter.tracked_addresses()
    ));
    buf.push('\n');

    // --- Histogram: message size ---
    // Real cumulative buckets, not just `+Inf`: an operator needs to see the
    // distribution (control messages vs. bulk payloads) to size the relay.
    buf.push_str("# HELP conduit_relay_message_size_bytes Message size in bytes\n");
    buf.push_str("# TYPE conduit_relay_message_size_bytes histogram\n");
    // Prometheus bucket series are cumulative and must be monotonically
    // increasing; a message lands in the first bucket whose bound it fits.
    let mut cumulative = 0u64;
    for (i, bound) in SIZE_BUCKETS.iter().enumerate() {
        cumulative += m.message_size_buckets[i].load(Ordering::Relaxed);
        buf.push_str(&format!(
            "conduit_relay_message_size_bytes_bucket{{le=\"{bound}\"}} {cumulative}\n"
        ));
    }
    buf.push_str(&format!(
        "conduit_relay_message_size_bytes_bucket{{le=\"+Inf\"}} {}\n",
        msg_size_count
    ));
    buf.push_str(&format!(
        "conduit_relay_message_size_bytes_sum {}\n",
        msg_size_sum
    ));
    buf.push_str(&format!(
        "conduit_relay_message_size_bytes_count {}\n",
        msg_size_count
    ));

    buf
}
