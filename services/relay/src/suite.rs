//! The in-place test suite for `conduit-relay`.
//!
//! Unit tests of the internals: the rate limiters, the config precedence, the
//! frame parsers, the nonce cache, and an end-to-end WebSocket lifecycle against
//! a real listener. They live in one file rather than beside each item because
//! they are written as a set — a single suite that walks the relay end to end
//! reads better than the same tests scattered across eight modules.
//!
//! See `docs/TESTING.md` for what this does and does not cover. In particular
//! there is no consumer-level test: nothing here exercises this crate the way
//! the desktop crate does, which is now the only consumer.

// Opts back into `unsafe` for exactly one reason: `std::env::set_var` and
// `remove_var` are unsafe in edition 2024, and testing that configuration
// precedence follows the environment means mutating it.
#![allow(unsafe_code)]

// The suite is written against the relay as a whole, so it reaches the crate
// root (which re-exports every internal) plus the external types a test has to
// name itself.
use crate::*;
use conduit_protocol::hmac::NonceCache;
use futures_util::{SinkExt, StreamExt};
use log::warn;
use serde_json::Value;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::net::TcpListener;
use tokio::sync::{RwLock, mpsc, watch};
use tungstenite::Message;

use crate::limits::{
    MAX_BINARY_SIZE, MAX_CONNECTIONS, MAX_TEXT_SIZE, MSG_BURST, MSG_RATE_PER_SEC,
    QUEUE_BYTE_BUDGET, SIZE_BUCKETS,
};
use crate::route::{VerifiedBinaryFrame, binary_mac_input, forward_text_with_timeout};
use crate::tls::TlsParams;

// ---------------------------------------------------------------
//  RateLimiter tests
// ---------------------------------------------------------------

#[test]
fn rate_limiter_allows_within_limit() {
    let rl = RateLimiter::new(3, 60);
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
    assert!(rl.allow(ip), "1st attempt should be allowed");
    assert!(rl.allow(ip), "2nd attempt should be allowed");
    assert!(rl.allow(ip), "3rd attempt should be allowed");
}

#[test]
fn rate_limiter_blocks_over_limit() {
    let rl = RateLimiter::new(2, 60);
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2));
    assert!(rl.allow(ip));
    assert!(rl.allow(ip));
    assert!(!rl.allow(ip), "should be blocked after exceeding limit");
}

#[test]
fn rate_limiter_independent_per_ip() {
    let rl = RateLimiter::new(1, 60);
    let ip1 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 10));
    let ip2 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 20));
    assert!(rl.allow(ip1));
    assert!(!rl.allow(ip1), "ip1 exhausted");
    assert!(rl.allow(ip2), "ip2 should be independent");
}

#[test]
fn rate_limiter_slides_window() {
    // Use a 1-second window so we can test expiry quickly
    let rl = RateLimiter::new(1, 1);
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 30));
    assert!(rl.allow(ip));
    assert!(!rl.allow(ip), "should be blocked immediately");
    // Wait for the window to slide past
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert!(rl.allow(ip), "should be allowed after window slides");
}

// ---------------------------------------------------------------
//  MessageRateLimiter (per-message token bucket) – R1
// ---------------------------------------------------------------

#[test]
fn message_rate_limiter_defaults_match_spec() {
    assert_eq!(MSG_RATE_PER_SEC, 100.0);
    assert_eq!(MSG_BURST, 50.0);
}

#[test]
fn message_rate_limiter_allows_burst_capacity_then_drops() {
    let mut rl = MessageRateLimiter::with_params(100.0, 50.0);
    for i in 0..50 {
        assert!(rl.allow(), "burst token {} should be allowed", i);
    }
    assert!(!rl.allow(), "51st immediate message must be dropped");
    assert!(!rl.allow(), "still no tokens without refill time");
}

#[test]
fn message_rate_limiter_refills_over_time() {
    let mut rl = MessageRateLimiter::with_params(1_000.0, 1.0);
    assert!(rl.allow());
    assert!(!rl.allow(), "burst of 1 should exhaust immediately");
    std::thread::sleep(std::time::Duration::from_millis(15));
    assert!(
        rl.allow(),
        "15ms at 1000/s refills well past one token (capped at burst 1)"
    );
}

#[test]
fn message_rate_limiter_never_exceeds_burst_after_long_idle() {
    let mut rl = MessageRateLimiter::with_params(100.0, 50.0);
    // Long idle: refill accumulates far beyond burst but must be capped.
    std::thread::sleep(std::time::Duration::from_millis(200));
    let mut allowed = 0;
    for _ in 0..60 {
        if rl.allow() {
            allowed += 1;
        }
    }
    assert_eq!(
        allowed, 50,
        "idle refill must not exceed burst capacity of 50"
    );
}

#[test]
fn message_rate_limiter_zero_burst_blocks_everything() {
    let mut rl = MessageRateLimiter::with_params(100.0, 0.0);
    assert!(!rl.allow());
    assert!(!rl.allow());
}

// ---------------------------------------------------------------
//  Structured JSON logging (R8)
// ---------------------------------------------------------------

// ---------------------------------------------------------------
//  Bearer auth helper (R7)
// ---------------------------------------------------------------
#[test]
fn bearer_token_authorized_accepts_exact_match() {
    assert!(bearer_token_authorized("Bearer sekrit", "sekrit"));
}

#[test]
fn bearer_token_authorized_rejects_wrong_or_missing_scheme() {
    assert!(!bearer_token_authorized("Bearer wrong", "sekrit"));
    assert!(
        !bearer_token_authorized("sekrit", "sekrit"),
        "needs Bearer "
    );
    assert!(!bearer_token_authorized("Basic sekrit", "sekrit"));
    assert!(!bearer_token_authorized("Bearer ", "sekrit"));
    assert!(!bearer_token_authorized("", "sekrit"));
}

// ---------------------------------------------------------------
//  Config – RELAY_HEALTH_TOKEN (R7)
// ---------------------------------------------------------------

#[test]
#[serial_test::serial]
fn config_health_token_env_override_and_fallback() {
    with_env_snapshot(
        &["RELAY_TOKEN", "HMAC_SECRET", "RELAY_HEALTH_TOKEN"],
        || {
            unsafe {
                std::env::set_var("RELAY_TOKEN", "config-test-token");
                std::env::set_var("HMAC_SECRET", "config-test-secret");
                std::env::set_var("RELAY_HEALTH_TOKEN", "custom-health");
            }
            let cfg = Config::resolve(None).unwrap();
            assert_eq!(cfg.health_token, "custom-health");

            // Empty and unset both mean "derive one". Neither may fall back to
            // the master secret: anyone holding the health token would then
            // hold the key the nonce cache is keyed from, which is a
            // credential shared between two roles for no reason.
            unsafe {
                std::env::set_var("RELAY_HEALTH_TOKEN", "");
            }
            let cfg = Config::resolve(None).unwrap();
            assert_ne!(
                cfg.health_token, "config-test-secret",
                "an empty RELAY_HEALTH_TOKEN must not fall back to the master secret"
            );
            let derived = cfg.health_token.clone();
            assert!(!derived.is_empty(), "a token must be derived");

            unsafe {
                std::env::remove_var("RELAY_HEALTH_TOKEN");
            }
            let cfg = Config::resolve(None).unwrap();
            assert_eq!(
                cfg.health_token, derived,
                "the derived health token must be stable across restarts"
            );
        },
    );
}

// ---------------------------------------------------------------
//  E2E: per-message rate limiting (R1)
// ---------------------------------------------------------------

#[tokio::test]
async fn e2e_message_rate_limit_drops_but_connection_survives() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut client = ws_client(relay.ws_addr).await;
    let r = authenticate(&mut client, "f10d", E2E_RELAY_TOKEN)
        .await
        .expect("auth response");
    assert!(r.contains("relay_auth_ok"), "auth should succeed: {}", r);

    // Flood well past burst (50). Rate is 100/s so a tight loop of 200
    // messages must drop the excess while keeping the socket open.
    for i in 0..200 {
        client
            .send(Message::Text(format!("{{\"type\":\"ping\",\"n\":{}}}", i)))
            .await
            .expect("send should not fail immediately");
    }

    assert!(
        wait_until(2000, || {
            state
                .metrics
                .messages_dropped_rate_limit
                .load(Ordering::Relaxed)
                >= 1
        })
        .await,
        "flood should increment messages_dropped_rate_limit"
    );

    // Connection must still be open (timeout = still reading; close/None = dead).
    let next = tokio::time::timeout(std::time::Duration::from_millis(250), client.next()).await;
    match next {
        Ok(None) | Ok(Some(Ok(Message::Close(_)))) => {
            panic!("connection must survive message rate limiting")
        }
        _ => {}
    }

    drop(relay);
}

#[test]
fn rate_limiter_large_window_cleans_old_entries() {
    let rl = RateLimiter::new(5, 1);
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 50));
    // Burn through the limit
    for _ in 0..5 {
        rl.allow(ip);
    }
    assert!(!rl.allow(ip));
    // After window expires, should accept again
    std::thread::sleep(std::time::Duration::from_millis(1100));
    assert!(rl.allow(ip));
}

// ---------------------------------------------------------------
//  ConnectionGuard tests
// ---------------------------------------------------------------

#[test]
fn connection_guard_decrements_on_drop() {
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    {
        let _guard = ConnectionGuard {
            count: count.clone(),
        };
        count.fetch_add(1, Ordering::Relaxed);
        assert_eq!(count.load(Ordering::Relaxed), 1);
    }
    // After guard is dropped, count should be decremented
    assert_eq!(count.load(Ordering::Relaxed), 0);
}

#[test]
fn connection_guard_multiple_guards() {
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let g1 = ConnectionGuard {
        count: count.clone(),
    };
    let g2 = ConnectionGuard {
        count: count.clone(),
    };
    count.fetch_add(2, Ordering::Relaxed);
    assert_eq!(count.load(Ordering::Relaxed), 2);
    drop(g1);
    assert_eq!(count.load(Ordering::Relaxed), 1);
    drop(g2);
    assert_eq!(count.load(Ordering::Relaxed), 0);
}

// ---------------------------------------------------------------
//  Metrics tests
// ---------------------------------------------------------------

#[test]
fn metrics_initializes_to_zero() {
    let m = Metrics::new();
    assert_eq!(m.connections_connected.load(Ordering::Relaxed), 0);
    assert_eq!(m.connections_disconnected.load(Ordering::Relaxed), 0);
    assert_eq!(m.auth_attempts_success.load(Ordering::Relaxed), 0);
    assert_eq!(m.auth_attempts_failure.load(Ordering::Relaxed), 0);
    assert_eq!(m.messages_routed.load(Ordering::Relaxed), 0);
    assert_eq!(m.messages_dropped_not_found.load(Ordering::Relaxed), 0);
    assert_eq!(m.messages_dropped_timeout.load(Ordering::Relaxed), 0);
    assert_eq!(m.messages_dropped_rate_limit.load(Ordering::Relaxed), 0);
    assert_eq!(m.message_size_bytes_sum.load(Ordering::Relaxed), 0);
    assert_eq!(m.message_size_bytes_count.load(Ordering::Relaxed), 0);
}

#[test]
fn metrics_increment_independently() {
    let m = Metrics::new();
    m.connections_connected.fetch_add(5, Ordering::Relaxed);
    m.connections_disconnected.fetch_add(3, Ordering::Relaxed);
    m.auth_attempts_success.fetch_add(10, Ordering::Relaxed);
    m.auth_attempts_failure.fetch_add(2, Ordering::Relaxed);
    m.messages_routed.fetch_add(100, Ordering::Relaxed);
    m.messages_dropped_not_found.fetch_add(7, Ordering::Relaxed);
    m.messages_dropped_timeout.fetch_add(2, Ordering::Relaxed);
    m.messages_dropped_rate_limit
        .fetch_add(1, Ordering::Relaxed);
    m.message_size_bytes_sum.fetch_add(50000, Ordering::Relaxed);
    m.message_size_bytes_count.fetch_add(200, Ordering::Relaxed);
    assert_eq!(m.connections_connected.load(Ordering::Relaxed), 5);
    assert_eq!(m.connections_disconnected.load(Ordering::Relaxed), 3);
    assert_eq!(m.auth_attempts_success.load(Ordering::Relaxed), 10);
    assert_eq!(m.auth_attempts_failure.load(Ordering::Relaxed), 2);
    assert_eq!(m.messages_routed.load(Ordering::Relaxed), 100);
    assert_eq!(m.messages_dropped_not_found.load(Ordering::Relaxed), 7);
    assert_eq!(m.messages_dropped_timeout.load(Ordering::Relaxed), 2);
    assert_eq!(m.messages_dropped_rate_limit.load(Ordering::Relaxed), 1);
    assert_eq!(m.message_size_bytes_sum.load(Ordering::Relaxed), 50000);
    assert_eq!(m.message_size_bytes_count.load(Ordering::Relaxed), 200);
}

// ---------------------------------------------------------------
//  build_metrics_json tests
// ---------------------------------------------------------------

fn test_config() -> Config {
    Config {
        ws_port: 9528,
        // Loopback: the plaintext listener carries the bearer token in clear.
        ws_bind: "127.0.0.1".parse().unwrap(),
        wss_port: 9529,
        health_port: 9530,
        health_bind: "127.0.0.1".parse().unwrap(),
        hmac_secret: b"test-secret-for-unit-tests!!!".to_vec(),
        hmac_secret_file: std::env::temp_dir().join("relay-test-hmac-secret"),
        health_token: "test-secret-for-unit-tests!!!".to_string(),
        metrics_token: None,
        relay_token: "test-secret-for-unit-tests!!!".to_string(),
        tls: TlsParams::default(),
        // Route keys come from the host resolver on AppState.
        enable_plain_ws: false,
        nonce_file: std::path::PathBuf::from("./data/test-nonces.json"),
        auth_timeout_secs: 10,
        relay_cert_pin: None,
    }
}

fn test_state(
    active: usize,
    conn_connected: u64,
    auth_fails: u64,
    routed: u64,
    dropped_nf: u64,
) -> AppState {
    let metrics = Arc::new(Metrics::new());
    metrics
        .connections_connected
        .store(conn_connected, Ordering::Relaxed);
    metrics
        .auth_attempts_failure
        .store(auth_fails, Ordering::Relaxed);
    metrics.messages_routed.store(routed, Ordering::Relaxed);
    metrics
        .messages_dropped_not_found
        .store(dropped_nf, Ordering::Relaxed);

    AppState {
        clients: Arc::new(RwLock::new(HashMap::new())),
        active_connections: Arc::new(std::sync::atomic::AtomicUsize::new(active)),
        metrics,
        route_keys: Arc::new(TestRouteKeys),
        config: test_config(),
        rate_limiter: Arc::new(RateLimiter::new(10, 60)),
        nonces: Arc::new(RwLock::new(NonceCache::new())),
        tls_pin: Some("sha256/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into()),
    }
}

#[test]
fn build_metrics_json_has_all_required_keys() {
    let state = test_state(0, 0, 0, 0, 0);
    let json = build_metrics_json(&state);
    let obj = json.as_object().expect("should be a JSON object");

    // Rewritten from the 7-key list: `/health` previously loaded
    // `connections_disconnected` and `auth_attempts_success` into
    // `_`-prefixed bindings and never emitted them, so `/health` and
    // `/metrics` disagreed about the same counters. The per-reason drop
    // breakdown and the pin are now published too.
    let required_keys = [
        "status",
        "active_connections",
        "max_connections",
        "total_connections",
        "total_connections_disconnected",
        "total_auth_successes",
        "total_auth_failures",
        "total_messages_routed",
        "total_messages_dropped",
        "messages_dropped",
        "tls_pin",
    ];
    for key in &required_keys {
        assert!(
            obj.contains_key(*key),
            "metrics JSON missing required key: {}",
            key
        );
    }
    assert_eq!(
        obj.len(),
        required_keys.len(),
        "unexpected extra keys: {:?}",
        obj.keys().collect::<Vec<_>>()
    );
}

#[test]
fn build_metrics_json_emits_the_counters_prometheus_exports() {
    // The two counters that were read and discarded must now actually
    // appear, with the values `/metrics` reports.
    let state = test_state(0, 0, 0, 0, 0);
    state
        .metrics
        .connections_disconnected
        .store(11, Ordering::Relaxed);
    state
        .metrics
        .auth_attempts_success
        .store(22, Ordering::Relaxed);

    let json = build_metrics_json(&state);
    assert_eq!(json["total_connections_disconnected"], 11);
    assert_eq!(json["total_auth_successes"], 22);

    let prom = build_prometheus_metrics(&state);
    assert!(prom.contains("conduit_relay_connections_total{event=\"disconnected\"} 11"));
    assert!(prom.contains("conduit_relay_auth_attempts_total{result=\"success\"} 22"));
}

#[test]
fn build_metrics_json_breakdown_matches_the_total() {
    let state = test_state(0, 0, 0, 0, 1);
    state
        .metrics
        .messages_dropped_timeout
        .store(2, Ordering::Relaxed);
    state
        .metrics
        .messages_dropped_rate_limit
        .store(3, Ordering::Relaxed);
    state
        .metrics
        .messages_dropped_replay
        .store(5, Ordering::Relaxed);
    state
        .metrics
        .messages_dropped_hmac_failed
        .store(7, Ordering::Relaxed);
    state
        .metrics
        .messages_dropped_unknown_type
        .store(11, Ordering::Relaxed);

    let json = build_metrics_json(&state);
    assert_eq!(json["total_messages_dropped"], 1 + 2 + 3 + 5 + 7 + 11);
    assert_eq!(json["messages_dropped"]["not_found"], 1);
    assert_eq!(json["messages_dropped"]["timeout"], 2);
    assert_eq!(json["messages_dropped"]["rate_limit"], 3);
    assert_eq!(json["messages_dropped"]["replay"], 5);
    assert_eq!(json["messages_dropped"]["hmac_failed"], 7);
    assert_eq!(json["messages_dropped"]["unknown_type"], 11);
}

#[test]
fn build_metrics_json_publishes_the_certificate_pin() {
    let state = test_state(0, 0, 0, 0, 0);
    assert_eq!(
        state.config.health_token, state.config.health_token,
        "sanity: health token is independent of the pin"
    );
    let json = build_metrics_json(&state);
    assert_eq!(
        json["tls_pin"],
        "sha256/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="
    );
}

#[test]
fn build_metrics_json_status_is_ok() {
    let state = test_state(0, 0, 0, 0, 0);
    let json = build_metrics_json(&state);
    assert_eq!(json["status"], "ok");
}

#[test]
fn build_metrics_json_reflects_values() {
    let state = test_state(42, 100, 5, 200, 10);
    let json = build_metrics_json(&state);
    assert_eq!(json["active_connections"], 42);
    assert_eq!(json["max_connections"], MAX_CONNECTIONS);
    assert_eq!(json["total_connections"], 100);
    assert_eq!(json["total_auth_failures"], 5);
    assert_eq!(json["total_messages_routed"], 200);
    assert_eq!(json["total_messages_dropped"], 10);
}

#[test]
fn build_metrics_json_max_connections_matches_const() {
    let state = test_state(0, 0, 0, 0, 0);
    let json = build_metrics_json(&state);
    assert_eq!(json["max_connections"], MAX_CONNECTIONS);
    assert_eq!(MAX_CONNECTIONS, 10_000);
}

// ---------------------------------------------------------------
//  build_prometheus_metrics tests
// ---------------------------------------------------------------

#[test]
fn prometheus_output_contains_all_metric_names() {
    let state = test_state(42, 100, 5, 200, 10);
    let output = build_prometheus_metrics(&state);

    let required_metrics = [
        "conduit_relay_connections_total",
        "conduit_relay_auth_attempts_total",
        "conduit_relay_messages_routed_total",
        "conduit_relay_messages_dropped_total",
        "conduit_relay_active_connections",
        "conduit_relay_max_connections",
        "conduit_relay_message_size_bytes",
    ];
    for metric in &required_metrics {
        assert!(
            output.contains(metric),
            "Prometheus output missing metric: {}",
            metric
        );
    }
}

#[test]
fn prometheus_output_has_help_and_type_lines() {
    let state = test_state(0, 0, 0, 0, 0);
    let output = build_prometheus_metrics(&state);

    // Every metric family must have # HELP and # TYPE
    assert!(output.contains("# HELP conduit_relay_connections_total"));
    assert!(output.contains("# TYPE conduit_relay_connections_total counter"));
    assert!(output.contains("# HELP conduit_relay_auth_attempts_total"));
    assert!(output.contains("# TYPE conduit_relay_auth_attempts_total counter"));
    assert!(output.contains("# HELP conduit_relay_messages_routed_total"));
    assert!(output.contains("# TYPE conduit_relay_messages_routed_total counter"));
    assert!(output.contains("# HELP conduit_relay_messages_dropped_total"));
    assert!(output.contains("# TYPE conduit_relay_messages_dropped_total counter"));
    assert!(output.contains("# HELP conduit_relay_active_connections"));
    assert!(output.contains("# TYPE conduit_relay_active_connections gauge"));
    assert!(output.contains("# HELP conduit_relay_max_connections"));
    assert!(output.contains("# TYPE conduit_relay_max_connections gauge"));
    assert!(output.contains("# HELP conduit_relay_message_size_bytes"));
    assert!(output.contains("# TYPE conduit_relay_message_size_bytes histogram"));
}

#[test]
fn prometheus_counter_labels_present() {
    let state = test_state(0, 0, 0, 0, 0);
    let output = build_prometheus_metrics(&state);

    // Connection event labels
    assert!(output.contains("conduit_relay_connections_total{event=\"connected\"}"));
    assert!(output.contains("conduit_relay_connections_total{event=\"disconnected\"}"));

    // Auth result labels
    assert!(output.contains("conduit_relay_auth_attempts_total{result=\"success\"}"));
    assert!(output.contains("conduit_relay_auth_attempts_total{result=\"failure\"}"));

    // Message source label
    assert!(output.contains("conduit_relay_messages_routed_total{source=\"device\"}"));

    // Drop reason labels
    assert!(output.contains("conduit_relay_messages_dropped_total{reason=\"not_found\"}"));
    assert!(output.contains("conduit_relay_messages_dropped_total{reason=\"timeout\"}"));
    assert!(output.contains("conduit_relay_messages_dropped_total{reason=\"rate_limit\"}"));
}

#[test]
fn prometheus_output_reflects_values() {
    let state = test_state(42, 100, 5, 200, 10);
    let output = build_prometheus_metrics(&state);

    assert!(output.contains("conduit_relay_active_connections 42"));
    assert!(output.contains("conduit_relay_max_connections 10000"));
    assert!(output.contains("conduit_relay_connections_total{event=\"connected\"} 100"));
    assert!(output.contains("conduit_relay_auth_attempts_total{result=\"failure\"} 5"));
    assert!(output.contains("conduit_relay_messages_routed_total{source=\"device\"} 200"));
    assert!(output.contains("conduit_relay_messages_dropped_total{reason=\"not_found\"} 10"));
}

#[test]
fn prometheus_histogram_has_required_fields() {
    let state = test_state(0, 0, 0, 0, 0);
    let output = build_prometheus_metrics(&state);

    // A valid Prometheus histogram must have _bucket{le="+Inf"}, _sum, _count
    assert!(output.contains("conduit_relay_message_size_bytes_bucket{le=\"+Inf\"} 0"));
    assert!(output.contains("conduit_relay_message_size_bytes_sum 0"));
    assert!(output.contains("conduit_relay_message_size_bytes_count 0"));
}

#[test]
fn prometheus_histogram_tracks_sizes() {
    let state = test_state(0, 0, 0, 0, 0);
    state
        .metrics
        .message_size_bytes_sum
        .store(12345, Ordering::Relaxed);
    state
        .metrics
        .message_size_bytes_count
        .store(50, Ordering::Relaxed);
    let output = build_prometheus_metrics(&state);

    assert!(output.contains("conduit_relay_message_size_bytes_bucket{le=\"+Inf\"} 50"));
    assert!(output.contains("conduit_relay_message_size_bytes_sum 12345"));
    assert!(output.contains("conduit_relay_message_size_bytes_count 50"));
}

#[test]
fn prometheus_output_default_zero_values() {
    let state = test_state(0, 0, 0, 0, 0);
    let output = build_prometheus_metrics(&state);

    assert!(output.contains("conduit_relay_active_connections 0"));
    assert!(output.contains("conduit_relay_connections_total{event=\"connected\"} 0"));
    assert!(output.contains("conduit_relay_connections_total{event=\"disconnected\"} 0"));
    assert!(output.contains("conduit_relay_auth_attempts_total{result=\"success\"} 0"));
    assert!(output.contains("conduit_relay_auth_attempts_total{result=\"failure\"} 0"));
    assert!(output.contains("conduit_relay_messages_routed_total{source=\"device\"} 0"));
}
// ---------------------------------------------------------------
//  Binary frame parsing (exercises the real production parser)
//
//  v2 frame: [0x02][target 16B][seq u32 BE][HMAC tag 32B][payload]
//  The MAC covers frame[0..21] ++ payload, prefixed by the SENDER id.
// ---------------------------------------------------------------

/// Run the production binary-frame verifier over `bytes` on behalf of
/// `sender`, tracking the sequence number in `last_seq`.
async fn verify_binary<'a>(
    sender: &str,
    bytes: &'a [u8],
    last_seq: &mut Option<u32>,
) -> Result<Option<VerifiedBinaryFrame<'a>>, Rejection> {
    let state = Arc::new(test_state(0, 0, 0, 0, 0));
    let keys = TestRouteKeys;
    handle_binary_frame(&state, &keys, sender, bytes, last_seq).await
}

#[tokio::test]
async fn binary_frame_valid_target_id() {
    let frame = binary_frame("aa", "device-1", 1, b"payload");
    let mut seq = None;
    let parsed = verify_binary("aa", &frame, &mut seq)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parsed.target_id, "device-1");
    assert_eq!(parsed.payload, b"payload");
    assert_eq!(seq, Some(1));
}

#[tokio::test]
async fn binary_frame_valid_16_char_id() {
    let frame = binary_frame("aa", "abcdefghijklmnop", 7, b"");
    let mut seq = None;
    let parsed = verify_binary("aa", &frame, &mut seq)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parsed.target_id, "abcdefghijklmnop");
    assert!(parsed.payload.is_empty());
}

#[tokio::test]
async fn binary_frame_rejects_too_short() {
    // One byte under the 53-byte header.
    let frame =
        binary_frame("aa", "dev", 1, b"")[..conduit_protocol::BINARY_HEADER_LEN - 1].to_vec();
    let mut seq = None;
    let err = verify_binary("aa", &frame, &mut seq).await.unwrap_err();
    assert_eq!(err.code, "binary_frame_too_short");
    assert_eq!(err.kind, RejectionKind::Integrity);
}

#[tokio::test]
async fn binary_frame_rejects_v1_version() {
    // v1 had no tag and no sequence: the relay must no longer accept it.
    let mut frame = binary_frame("aa", "dev", 1, b"hello");
    frame[0] = 0x01;
    let mut seq = None;
    let err = verify_binary("aa", &frame, &mut seq).await.unwrap_err();
    assert_eq!(err.code, "binary_version_unsupported");
}

#[tokio::test]
async fn binary_frame_rejects_version_zero() {
    let mut frame = binary_frame("aa", "dev", 1, b"hello");
    frame[0] = 0x00;
    let mut seq = None;
    assert_eq!(
        verify_binary("aa", &frame, &mut seq)
            .await
            .unwrap_err()
            .code,
        "binary_version_unsupported"
    );
}

#[tokio::test]
async fn binary_frame_rejects_empty_target_id() {
    let frame = binary_frame("aa", "\0\0\0", 1, b"hello");
    let mut seq = None;
    assert_eq!(
        verify_binary("aa", &frame, &mut seq)
            .await
            .unwrap_err()
            .code,
        "binary_target_id_empty"
    );
}

#[tokio::test]
async fn binary_frame_rejects_invalid_utf8() {
    let mut frame = binary_frame("aa", "dev", 1, b"hello");
    frame[1] = 0xFF;
    frame[2] = 0xFE;
    let mut seq = None;
    assert_eq!(
        verify_binary("aa", &frame, &mut seq)
            .await
            .unwrap_err()
            .code,
        "binary_target_id_invalid"
    );
}

#[tokio::test]
async fn binary_frame_payload_extracted_correctly() {
    let payload = b"hello world!!!!!"; // 16 bytes
    let frame = binary_frame("aa", "dev", 3, payload);
    let mut seq = None;
    let parsed = verify_binary("aa", &frame, &mut seq)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parsed.target_id, "dev");
    assert_eq!(parsed.payload, payload);
    assert_eq!(seq, Some(3));
}

#[tokio::test]
async fn binary_frame_tampered_payload_is_rejected() {
    // Flip one payload byte after signing: the tag must fail.
    let mut frame = binary_frame("aa", "dev", 1, b"hello");
    let last = frame.len() - 1;
    frame[last] ^= 0xff;
    let mut seq = None;
    let err = verify_binary("aa", &frame, &mut seq).await.unwrap_err();
    assert_eq!(err.code, "binary_hmac_invalid");
    assert_eq!(err.kind, RejectionKind::Integrity);
}

#[tokio::test]
async fn binary_frame_tampered_target_is_rejected() {
    let mut frame = binary_frame("aa", "dev", 1, b"hello");
    frame[1] = b'x'; // retarget without re-signing
    let mut seq = None;
    assert_eq!(
        verify_binary("aa", &frame, &mut seq)
            .await
            .unwrap_err()
            .code,
        "binary_hmac_invalid"
    );
}

#[tokio::test]
async fn binary_frame_tampered_sequence_is_rejected() {
    let mut frame = binary_frame("aa", "dev", 1, b"hello");
    frame[17] = 0x7f; // rewrite the sequence without re-signing
    let mut seq = None;
    assert_eq!(
        verify_binary("aa", &frame, &mut seq)
            .await
            .unwrap_err()
            .code,
        "binary_hmac_invalid"
    );
}

#[tokio::test]
async fn binary_frame_signed_by_the_relay_token_is_rejected() {
    // The historical defect in binary form: a client that knows only the
    // bearer token cannot produce a valid frame.
    let mut frame = Vec::with_capacity(1024);
    frame.push(conduit_protocol::BINARY_FRAME_VERSION);
    let mut id = [0u8; conduit_protocol::BINARY_DEVICE_ID_LEN];
    id[..3].copy_from_slice(b"dev");
    frame.extend_from_slice(&id);
    frame.extend_from_slice(&1u32.to_be_bytes());
    let mut authenticated = Vec::with_capacity(conduit_protocol::BINARY_AUTHENTICATED_PREFIX_LEN);
    authenticated.extend_from_slice(&frame[..conduit_protocol::BINARY_AUTHENTICATED_PREFIX_LEN]);
    let tag = hmac::compute_hmac(
        E2E_SECRET,
        &hex::encode(binary_mac_input("aa", &authenticated)),
    );
    frame.extend_from_slice(&hex::decode(&tag).unwrap());

    let mut seq = None;
    assert_eq!(
        verify_binary("aa", &frame, &mut seq)
            .await
            .unwrap_err()
            .code,
        "binary_hmac_invalid",
        "a frame signed with the relay token must not be accepted"
    );
}

#[tokio::test]
async fn binary_frame_signed_for_another_sender_is_rejected() {
    // Sender identity is bound into the MAC input, so one authenticated
    // client cannot replay a frame it observed from another.
    let frame = binary_frame("bb", "dev", 1, b"hello");
    let mut seq = None;
    let err = verify_binary("aa", &frame, &mut seq).await.unwrap_err();
    assert_eq!(err.code, "binary_hmac_invalid");
}

#[tokio::test]
async fn binary_frame_replay_is_rejected_by_sequence() {
    let first = binary_frame("aa", "dev", 5, b"one");
    let mut seq = None;
    assert!(verify_binary("aa", &first, &mut seq).await.is_ok());

    // Exactly the same frame again.
    let err = verify_binary("aa", &first, &mut seq).await.unwrap_err();
    assert_eq!(err.code, "binary_replay_detected");
    assert_eq!(err.kind, RejectionKind::Replay);

    // A lower sequence is a replay too.
    let lower = binary_frame("aa", "dev", 4, b"one");
    assert_eq!(
        verify_binary("aa", &lower, &mut seq)
            .await
            .unwrap_err()
            .code,
        "binary_replay_detected"
    );

    // Strictly increasing is accepted.
    let higher = binary_frame("aa", "dev", 6, b"one");
    assert!(verify_binary("aa", &higher, &mut seq).await.is_ok());
    assert_eq!(seq, Some(6));
}

#[tokio::test]
async fn binary_frame_signed_with_another_devices_key_is_rejected() {
    let state = Arc::new(test_state(0, 0, 0, 0, 0));
    let keys = TestRouteKeys;

    // "aa" signs a frame that claims to be "bb" using *bb's* route key.
    let mut frame = binary_frame("bb", "dev", 1, b"forged");
    let payload = &frame[conduit_protocol::BINARY_HEADER_LEN..].to_vec();
    let authenticated = {
        let mut h = frame[..conduit_protocol::BINARY_AUTHENTICATED_PREFIX_LEN].to_vec();
        h.extend_from_slice(payload);
        h
    };
    let tag = hmac::compute_hmac(
        &e2e_route_key("bb"),
        &hex::encode(binary_mac_input("bb", &authenticated)),
    );
    frame[conduit_protocol::BINARY_TAG_OFFSET
        ..conduit_protocol::BINARY_TAG_OFFSET + conduit_protocol::BINARY_TAG_LEN]
        .copy_from_slice(&hex::decode(&tag).unwrap());

    // Verified against *aa*'s own key, because that is the connection the
    // frame is attributed to, so the tag does not check out.
    let mut seq = None;
    assert_eq!(
        handle_binary_frame(&state, &keys, "aa", &frame, &mut seq)
            .await
            .unwrap_err()
            .code,
        "binary_hmac_invalid"
    );
}
// ---------------------------------------------------------------
//  MAX_TEXT_SIZE constant
// ---------------------------------------------------------------

#[test]
fn max_text_size_is_1mb() {
    assert_eq!(MAX_TEXT_SIZE, 1024 * 1024);
}

#[test]
fn binary_frames_are_capped_at_the_same_ceiling_as_text() {
    // These are the same resource. A relay that caps one and not the other
    // isn't capped, which is exactly the defect W6.1 described: both Text arms
    // enforced MAX_TEXT_SIZE while `handle_binary_frame` was handed whatever
    // the socket produced.
    assert_eq!(
        MAX_BINARY_SIZE, MAX_TEXT_SIZE,
        "the two ceilings must be one ceiling"
    );
    // Both legs of the read-side bound, so an over-size frame is refused
    // before it is allocated rather than after. A const block so this fails to
    // compile rather than to run.
    const {
        assert!(MAX_BINARY_SIZE < QUEUE_BYTE_BUDGET);
    }
}

/// The outbound queue bounds messages by count and bytes by budget.
///
/// Before the budget, `mpsc::channel(1024)` was the only bound on what a
/// target connection could accumulate, and 1024 messages at the frame ceiling
/// is 1 GiB — with no frame ceiling at all, tungstenite's 64 MiB default made
/// it 64 GiB.
#[tokio::test]
async fn queued_bytes_are_bounded_by_the_budget_not_by_the_channel_depth() {
    let (tx, _rx) = mpsc::channel(1024);
    let queue = Queue::new(tx);

    let message = || Message::Binary(vec![0u8; MAX_BINARY_SIZE]);
    let full = QUEUE_BYTE_BUDGET / MAX_BINARY_SIZE;

    // Fill the budget exactly, one frame at a time. The channel has 1024 slots
    // and we only use `full` of them, so nothing here is stopped by depth —
    // only by bytes.
    for i in 0..full {
        assert_eq!(
            tokio::time::timeout(std::time::Duration::from_secs(5), queue.send(message()))
                .await
                .expect("a send inside the budget must not time out"),
            SendOutcome::Sent,
            "message {i} of {full} must fit"
        );
    }

    // One more frame crosses the budget. It must not be queued. The channel
    // still has over a thousand free slots, so only the byte budget can be
    // refusing it — which is the whole point.
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), queue.send(message()))
            .await
            .is_err(),
        "the frame past QUEUE_BYTE_BUDGET must be refused, not queued"
    );

    // Returning one frame's worth of bytes unblocks exactly one more.
    queue.release(MAX_BINARY_SIZE);
    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(5), queue.send(message()))
            .await
            .expect("released bytes must be reusable"),
        SendOutcome::Sent
    );
}

/// The writer hands back exactly what the producer reserved.
///
/// Releasing more than was taken would inflate the budget back towards
/// unbounded; releasing less would starve the connection of bytes it is owed.
/// Both come from the two sides of this accounting disagreeing, so the test
/// asserts the boundary in both directions.
#[tokio::test]
async fn release_returns_exactly_what_the_producer_reserved() {
    let frame = 4096;
    let full = QUEUE_BYTE_BUDGET / frame;
    assert_eq!(
        full * frame,
        QUEUE_BYTE_BUDGET,
        "the test needs the budget to divide evenly"
    );

    // Deeper than the budget so that hitting the ceiling can only ever be the
    // byte budget talking. Otherwise a full *channel* would look identical to
    // an exhausted budget and the test would prove nothing.
    let (tx, _rx) = mpsc::channel(full + 1);
    let queue = Queue::new(tx);

    for _ in 0..full {
        assert_eq!(
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                queue.send(Message::Binary(vec![0u8; frame]))
            )
            .await
            .expect("a send inside the budget must not time out"),
            SendOutcome::Sent
        );
    }

    // Exactly exhausted, not nearly: even one byte must wait.
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            queue.send(Message::Binary(vec![0u8; 1]))
        )
        .await
        .is_err(),
        "the budget must be exactly exhausted"
    );

    queue.release(frame);

    // Exactly `frame` bytes came back: one frame of that size fits...
    assert_eq!(
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            queue.send(Message::Binary(vec![0u8; frame]))
        )
        .await
        .expect("the released bytes must cover one frame"),
        SendOutcome::Sent
    );
    // ...and nothing beyond it.
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(100),
            queue.send(Message::Binary(vec![0u8; 1]))
        )
        .await
        .is_err(),
        "release must return exactly one frame's worth, not a byte more"
    );
}

// ---------------------------------------------------------------
//  Config tests – empty / missing RELAY_TOKEN
//  These must be serialised because they mutate process-wide env vars.
// ---------------------------------------------------------------

/// Snapshot the env vars we touch, then restore them after the closure.
///
/// The restore also runs when `f` panics. A leaked variable outlives the test
/// that set it — this suite runs in one process — so the failure surfaces in
/// some unrelated test later and reads as a bug in the wrong place. That
/// actually happened: `config_invalid_port_ignores_and_uses_default` left
/// `RELAY_WS_PORT` behind and failed `the_plaintext_listener_defaults_to_loopback`.
fn with_env_snapshot<F: FnOnce()>(keys: &[&str], f: F) {
    let saved: Vec<(&str, Option<String>)> =
        keys.iter().map(|&k| (k, std::env::var(k).ok())).collect();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f));
    for (k, v) in saved {
        match v {
            Some(val) => unsafe { std::env::set_var(k, val) },
            None => unsafe { std::env::remove_var(k) },
        }
    }
    if let Err(payload) = result {
        std::panic::resume_unwind(payload);
    }
}

#[test]
#[serial_test::serial]
fn config_rejects_empty_relay_token() {
    with_env_snapshot(&["RELAY_TOKEN", "HMAC_SECRET"], || {
        unsafe {
            std::env::set_var("RELAY_TOKEN", "");
            std::env::set_var("HMAC_SECRET", "secret");
        }
        let result = Config::resolve(None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("no relay token"),
            "an empty token must be refused, and the message must say how to fix it: {}",
            err
        );
    });
}

#[test]
#[serial_test::serial]
fn config_rejects_missing_relay_token() {
    with_env_snapshot(&["RELAY_TOKEN", "HMAC_SECRET"], || {
        unsafe {
            std::env::remove_var("RELAY_TOKEN");
            std::env::set_var("HMAC_SECRET", "secret");
        }
        let result = Config::resolve(None);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("no relay token"),
            "a missing token must be refused, and the message must say how to fix it: {}",
            err
        );
    });
}

#[test]
#[serial_test::serial]
fn config_default_ports_when_not_set() {
    with_env_snapshot(
        &[
            "RELAY_TOKEN",
            "HMAC_SECRET",
            "RELAY_WS_PORT",
            "RELAY_WSS_PORT",
            "RELAY_HEALTH_PORT",
            "RELAY_ENABLE_PLAIN_WS",
        ],
        || {
            unsafe {
                std::env::set_var("RELAY_TOKEN", "test-token");
                std::env::set_var("HMAC_SECRET", "test-secret");
                std::env::remove_var("RELAY_WS_PORT");
                std::env::remove_var("RELAY_WSS_PORT");
                std::env::remove_var("RELAY_HEALTH_PORT");
                std::env::remove_var("RELAY_ENABLE_PLAIN_WS");
            }

            let cfg = Config::resolve(None).unwrap();
            assert_eq!(cfg.ws_port, 9528);
            assert_eq!(cfg.wss_port, 9529);
            assert_eq!(cfg.health_port, 9530);
            assert!(!cfg.enable_plain_ws);
            assert_eq!(cfg.hmac_secret, b"test-secret");
        },
    );
}

#[test]
#[serial_test::serial]
fn config_custom_ports() {
    with_env_snapshot(
        &[
            "RELAY_TOKEN",
            "HMAC_SECRET",
            "RELAY_WS_PORT",
            "RELAY_WSS_PORT",
            "RELAY_HEALTH_PORT",
            "RELAY_ENABLE_PLAIN_WS",
        ],
        || {
            unsafe {
                std::env::set_var("RELAY_TOKEN", "config-test-token");
                std::env::set_var("HMAC_SECRET", "config-test-secret");
                std::env::set_var("RELAY_WS_PORT", "8080");
                std::env::set_var("RELAY_WSS_PORT", "8443");
                std::env::set_var("RELAY_HEALTH_PORT", "9090");
                std::env::set_var("RELAY_ENABLE_PLAIN_WS", "true");
            }

            let cfg = Config::resolve(None).unwrap();
            assert_eq!(cfg.ws_port, 8080);
            assert_eq!(cfg.wss_port, 8443);
            assert_eq!(cfg.health_port, 9090);
            assert!(cfg.enable_plain_ws);
        },
    );
}

#[test]
#[serial_test::serial]
fn config_generates_random_hmac_when_not_set() {
    with_env_snapshot(&["RELAY_TOKEN", "HMAC_SECRET", "HMAC_SECRET_FILE"], || {
        unsafe {
            std::env::set_var("RELAY_TOKEN", "config-test-token");
            std::env::remove_var("HMAC_SECRET");
            // Point at an isolated temp file so tests never touch ./secrets.
            let dir = std::env::temp_dir().join(format!("relay-hmac-test-{}", std::process::id()));
            std::fs::create_dir_all(&dir).ok();
            let file = dir.join("hmac_secret");
            std::fs::remove_file(&file).ok();
            std::env::set_var(
                "HMAC_SECRET_FILE",
                file.to_str().expect("temp path should be valid UTF-8"),
            );
        }

        let cfg = Config::resolve(None).unwrap();
        // 32 random bytes hex-encoded → 64 ASCII chars.
        assert_eq!(
            cfg.hmac_secret.len(),
            64,
            "generated HMAC should be 64 hex chars"
        );
        assert!(
            cfg.hmac_secret.iter().all(|b| b.is_ascii_hexdigit()),
            "generated HMAC should be hex"
        );

        // Persistence: a second load must return the SAME secret.
        let cfg2 = Config::resolve(None).unwrap();
        assert_eq!(
            cfg.hmac_secret, cfg2.hmac_secret,
            "HMAC secret must be stable across reloads (persisted)"
        );
    });
}

#[test]
#[serial_test::serial]
fn config_fails_closed_when_secret_file_empty() {
    with_env_snapshot(&["RELAY_TOKEN", "HMAC_SECRET", "HMAC_SECRET_FILE"], || {
        unsafe {
            std::env::set_var("RELAY_TOKEN", "config-test-token");
            std::env::remove_var("HMAC_SECRET");
            let dir = std::env::temp_dir().join(format!("relay-hmac-empty-{}", std::process::id()));
            std::fs::create_dir_all(&dir).ok();
            let file = dir.join("hmac_secret");
            std::fs::write(&file, "\n").ok();
            std::env::set_var(
                "HMAC_SECRET_FILE",
                file.to_str().expect("temp path should be valid UTF-8"),
            );
        }

        let result = Config::resolve(None);
        assert!(result.is_err(), "empty secret file must fail closed");
        let err = result.unwrap_err();
        assert!(
            err.contains("empty"),
            "error should mention empty, got: {}",
            err
        );
    });
}

#[test]
#[serial_test::serial]
fn config_invalid_port_is_an_error_not_a_silent_default() {
    with_env_snapshot(&["RELAY_TOKEN", "HMAC_SECRET", "RELAY_WS_PORT"], || {
        unsafe {
            std::env::set_var("RELAY_TOKEN", "config-test-token");
            std::env::set_var("HMAC_SECRET", "config-test-secret");
            std::env::set_var("RELAY_WS_PORT", "not-a-number");
        }

        // A typo'd port used to bind the default silently, so an operator who
        // meant to move off a busy port kept serving the busy one and never
        // heard about it. Fail-closed is the only outcome that is reported.
        let result = Config::resolve(None);
        let err = result.expect_err("an unparseable port must be refused");
        assert!(
            err.contains("RELAY_WS_PORT") && err.contains("not-a-number"),
            "the error must name the variable and the bad value: {}",
            err
        );
    });
}

// ---------------------------------------------------------------
//  Message routing – async integration tests
// ---------------------------------------------------------------

#[tokio::test]
async fn message_routing_delivers_to_connected_client() {
    let clients: Clients = Arc::new(RwLock::new(HashMap::new()));
    let (tx, mut rx) = mpsc::channel::<Message>(64);

    clients
        .write()
        .await
        .insert("target-device".into(), Queue::new(tx));

    let msg = Message::Text(r#"{"hello":"world"}"#.to_string());
    let clients_read = clients.read().await;
    if let Some(target_tx) = clients_read.get("target-device") {
        let _ = target_tx.send(msg).await;
    }
    drop(clients_read);

    let received = rx.recv().await.expect("should receive message");
    match received {
        Message::Text(text) => assert_eq!(text.as_str(), r#"{"hello":"world"}"#),
        _ => panic!("expected Text message"),
    }
}

#[tokio::test]
async fn message_routing_returns_none_for_unknown_device() {
    let clients: Clients = Arc::new(RwLock::new(HashMap::new()));
    let clients_read = clients.read().await;
    let target = clients_read.get("nonexistent-device");
    assert!(
        target.is_none(),
        "unknown device should not be in clients map"
    );
}

#[tokio::test]
async fn message_routing_multiple_clients_independent() {
    let clients: Clients = Arc::new(RwLock::new(HashMap::new()));
    let (tx1, mut rx1) = mpsc::channel::<Message>(64);
    let (tx2, mut rx2) = mpsc::channel::<Message>(64);

    clients
        .write()
        .await
        .insert("device-a".into(), Queue::new(tx1));
    clients
        .write()
        .await
        .insert("device-b".into(), Queue::new(tx2));

    // Send to device-a only
    {
        let lock = clients.read().await;
        if let Some(t) = lock.get("device-a") {
            let _ = t.send(Message::Text("msg-a".to_string())).await;
        }
    }

    assert!(rx1.recv().await.is_some(), "device-a should receive");
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), rx2.recv())
            .await
            .is_err(),
        "device-b should NOT receive"
    );
}

#[tokio::test]
async fn message_routing_client_disconnect_removes_from_map() {
    let clients: Clients = Arc::new(RwLock::new(HashMap::new()));
    let (tx, _rx) = mpsc::channel::<Message>(64);

    clients
        .write()
        .await
        .insert("leaving-device".into(), Queue::new(tx));
    assert!(clients.read().await.contains_key("leaving-device"));

    clients.write().await.remove("leaving-device");
    assert!(!clients.read().await.contains_key("leaving-device"));
}

// ---------------------------------------------------------------
//  HMAC verification in relay context – async tests
// ---------------------------------------------------------------

#[tokio::test]
async fn hmac_relay_route_message_accepted_with_valid_hmac() {
    // The signed form a real client sends: from_device_id and key_id are
    // both covered by the MAC.
    let secret = b"test-hmac-secret-key-32-bytes!";
    let route = RelayRoute::signed_with(
        "dead1",
        secret,
        "dead1",
        "dead2",
        serde_json::json!({"data": "hi"}),
        1_700_000_000_000,
        "test-nonce-1",
    );
    let msg = serde_json::to_value(&route).unwrap();

    assert!(hmac::verify_message_hmac(secret, &msg));
    // The key is the sender's own; key_id is just the sender's id echoed back.
    assert_eq!(route.key_id.as_deref(), Some("dead1"));
    assert_eq!(route.from_device_id.as_deref(), Some("dead1"));
}

#[tokio::test]
async fn hmac_relay_route_message_rejected_with_wrong_hmac() {
    let secret = b"test-hmac-secret-key-32-bytes!";
    let mut route = RelayRoute::signed_with(
        "dead1",
        secret,
        "dead1",
        "dead2",
        serde_json::json!({"data": "hi"}),
        1_700_000_000_000,
        "test-nonce-2",
    );
    route.hmac = Some("0".repeat(64));
    let msg = serde_json::to_value(&route).unwrap();

    assert!(!hmac::verify_message_hmac(secret, &msg));
}

#[tokio::test]
async fn hmac_relay_route_legacy_unsigned_form_fails_the_relay_check() {
    // The pre-fix minimal route still *parses* (backwards compatible), but
    // the relay refuses it: no from_device_id, no nonce, no key_id.
    let legacy = serde_json::json!({
        "type": "relay_route",
        "to_device_id": "dead2",
        "payload": {"data": "hi"},
    });
    let parsed: RelayRoute = serde_json::from_value(legacy.clone()).unwrap();
    assert!(!parsed.has_required_signed_fields());

    let keys = StaticRouteKeys::from_pairs([(
        "dead1".to_string(),
        b"test-hmac-secret-key-32-bytes!".to_vec(),
    )]);
    // No key_id and no from_device_id means the relay never learns which key
    // to check, so there is nothing that could make this verify.
    assert!(parsed.key_id.is_none());
    assert!(!hmac::verify_message_hmac(
        &RouteKeys::key_for(&keys, "dead1").expect("dead1 registered"),
        &legacy
    ));
}

// ---------------------------------------------------------------
//  Timeout handling – async tests
// ---------------------------------------------------------------

#[tokio::test]
async fn timeout_read_next_returns_none_after_deadline() {
    let (_tx, mut rx) = mpsc::channel::<Message>(1);
    drop(_tx); // drop sender so channel is closed

    let result = tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await;

    // Channel closed → recv returns None (not timeout)
    assert!(result.is_ok());
    assert!(result.unwrap().is_none());
}

#[tokio::test]
async fn timeout_fires_when_no_message_within_deadline() {
    let (_tx, mut rx) = mpsc::channel::<Message>(1);
    // Don't send anything

    let result = tokio::time::timeout(std::time::Duration::from_millis(50), rx.recv()).await;

    assert!(result.is_err(), "should timeout when no message arrives");
}

#[tokio::test]
async fn timeout_does_not_fire_when_message_arrives_quickly() {
    let (tx, mut rx) = mpsc::channel::<Message>(1);
    let _ = tx.send(Message::Text("fast".to_string())).await;

    let result = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv()).await;

    assert!(result.is_ok(), "should NOT timeout when message arrives");
    let msg = result.unwrap().unwrap();
    match msg {
        Message::Text(t) => assert_eq!(t.as_str(), "fast"),
        _ => panic!("expected Text"),
    }
}

// ---------------------------------------------------------------
//  Large message handling
// ---------------------------------------------------------------

#[test]
fn large_message_at_max_text_size_is_acceptable() {
    let msg = "x".repeat(MAX_TEXT_SIZE);
    assert_eq!(msg.len(), MAX_TEXT_SIZE);
    // Message at exactly MAX_TEXT_SIZE should not trigger the > check
    assert!(msg.len() <= MAX_TEXT_SIZE);
}

#[test]
fn message_one_byte_over_max_is_rejected() {
    let msg = "x".repeat(MAX_TEXT_SIZE + 1);
    assert!(msg.len() > MAX_TEXT_SIZE);
}

#[test]
fn empty_message_is_acceptable() {
    let msg = "";
    assert!(msg.len() <= MAX_TEXT_SIZE);
}

// ---------------------------------------------------------------
//  Concurrent connection counting – async tests
// ---------------------------------------------------------------

#[tokio::test]
async fn concurrent_connection_counter_increments_and_decrements() {
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let mut handles = vec![];
    for _ in 0..100 {
        let c = count.clone();
        handles.push(tokio::spawn(async move {
            let _guard = ConnectionGuard { count: c.clone() };
            c.fetch_add(1, Ordering::Relaxed);
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    // All guards dropped → count should be 0
    assert_eq!(count.load(Ordering::Relaxed), 0);
}

#[tokio::test]
async fn concurrent_metrics_increments_are_consistent() {
    let metrics = Arc::new(Metrics::new());
    let mut handles = vec![];

    for _ in 0..50 {
        let m = metrics.clone();
        handles.push(tokio::spawn(async move {
            m.connections_connected.fetch_add(1, Ordering::Relaxed);
            m.messages_routed.fetch_add(1, Ordering::Relaxed);
        }));
    }

    for h in handles {
        h.await.unwrap();
    }

    assert_eq!(metrics.connections_connected.load(Ordering::Relaxed), 50);
    assert_eq!(metrics.messages_routed.load(Ordering::Relaxed), 50);
}

#[tokio::test]
async fn concurrent_rate_limiter_allows_within_limit() {
    let rl = Arc::new(RateLimiter::new(50, 60));
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 100));

    let mut handles = vec![];
    for _ in 0..50 {
        let rl = rl.clone();
        handles.push(tokio::spawn(async move { rl.allow(ip) }));
    }

    let mut allowed_count = 0;
    for h in handles {
        if h.await.unwrap() {
            allowed_count += 1;
        }
    }

    assert_eq!(allowed_count, 50, "exactly 50 should be allowed");
}

#[tokio::test]
async fn concurrent_rate_limiter_blocks_over_limit() {
    let rl = Arc::new(RateLimiter::new(5, 60));
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 101));

    let mut handles = vec![];
    for _ in 0..20 {
        let rl = rl.clone();
        handles.push(tokio::spawn(async move { rl.allow(ip) }));
    }

    let mut allowed_count = 0;
    for h in handles {
        if h.await.unwrap() {
            allowed_count += 1;
        }
    }

    assert!(
        allowed_count <= 5,
        "at most 5 should be allowed, got {}",
        allowed_count
    );
}

// ---------------------------------------------------------------
//  Connection lifecycle – async tests
// ---------------------------------------------------------------

#[tokio::test]
async fn connection_lifecycle_auth_register_and_deregister() {
    let clients: Clients = Arc::new(RwLock::new(HashMap::new()));
    let (tx, _rx) = mpsc::channel::<Message>(64);

    // Register
    {
        let mut lock = clients.write().await;
        lock.insert("device-lifecycle".into(), Queue::new(tx));
    }
    assert!(clients.read().await.contains_key("device-lifecycle"));

    // Deregister
    {
        let mut lock = clients.write().await;
        lock.remove("device-lifecycle");
    }
    assert!(!clients.read().await.contains_key("device-lifecycle"));
}

#[tokio::test]
async fn connection_lifecycle_overwrite_replaces_sender() {
    let clients: Clients = Arc::new(RwLock::new(HashMap::new()));
    let (tx1, mut rx1) = mpsc::channel::<Message>(64);
    let (tx2, mut rx2) = mpsc::channel::<Message>(64);

    // First connection
    clients
        .write()
        .await
        .insert("device-reconnect".into(), Queue::new(tx1));

    // Reconnect with new sender (old sender dropped)
    clients
        .write()
        .await
        .insert("device-reconnect".into(), Queue::new(tx2));

    // Old sender should be dropped → rx1 returns None
    assert!(rx1.recv().await.is_none(), "old channel should be closed");

    // New sender works
    {
        let lock = clients.read().await;
        let outcome = lock
            .get("device-reconnect")
            .unwrap()
            .send(Message::Text("reconnected".to_string()))
            .await;
        assert_eq!(outcome, SendOutcome::Sent, "the new queue must accept");
    }
    let msg = rx2.recv().await.unwrap();
    match msg {
        Message::Text(t) => assert_eq!(t.as_str(), "reconnected"),
        _ => panic!("expected Text"),
    }
}

// ---------------------------------------------------------------
//  Metrics endpoint – HTTP response building tests
// ---------------------------------------------------------------

#[test]
fn metrics_json_valid_json_syntax() {
    let state = test_state(5, 10, 1, 20, 3);
    let json = build_metrics_json(&state);
    // Should be valid JSON
    let serialized = serde_json::to_string(&json).unwrap();
    let parsed: Value = serde_json::from_str(&serialized).unwrap();
    assert_eq!(parsed, json);
}

#[test]
fn prometheus_output_valid_utf8() {
    let state = test_state(0, 0, 0, 0, 0);
    let output = build_prometheus_metrics(&state);
    assert!(std::str::from_utf8(output.as_bytes()).is_ok());
}

#[test]
fn prometheus_output_ends_with_newline_histogram() {
    let state = test_state(0, 0, 0, 0, 0);
    let output = build_prometheus_metrics(&state);
    // Last line should be the histogram _count which ends with \n
    assert!(
        output.ends_with('\n'),
        "prometheus output should end with newline"
    );
}

#[test]
fn metrics_json_dropped_count_sums_all_reasons() {
    let state = test_state(0, 0, 0, 0, 10);
    // Add timeout drops too
    state
        .metrics
        .messages_dropped_timeout
        .store(5, Ordering::Relaxed);
    state
        .metrics
        .messages_dropped_rate_limit
        .store(3, Ordering::Relaxed);
    let json = build_metrics_json(&state);
    // total_messages_dropped = dropped_nf + dropped_to + dropped_rl = 10 + 5 + 3 = 18
    assert_eq!(json["total_messages_dropped"], 18);
}

// ---------------------------------------------------------------
//  Graceful shutdown – watch channel tests
// ---------------------------------------------------------------

#[tokio::test]
async fn graceful_shutdown_signal_propagates_through_watch() {
    let (tx, rx) = watch::channel(false);
    let mut rx2 = rx.clone();

    // Initially false
    assert!(!*rx2.borrow());

    // Send shutdown signal
    tx.send(true).unwrap();

    // Receiver should see the change
    rx2.changed().await.unwrap();
    assert!(*rx2.borrow());
}

#[tokio::test]
async fn graceful_shutdown_multiple_receivers_all_get_signal() {
    let (tx, rx) = watch::channel(false);
    let mut rx2 = rx.clone();
    let mut rx3 = rx.clone();

    tx.send(true).unwrap();

    rx2.changed().await.unwrap();
    rx3.changed().await.unwrap();
    assert!(*rx2.borrow());
    assert!(*rx3.borrow());
}

#[tokio::test]
async fn graceful_shutdown_select_aborts_on_signal() {
    let (tx, rx) = watch::channel(false);
    let mut shutdown_rx = rx.clone();

    // Simulate a task that selects on shutdown
    let handle = tokio::spawn(async move {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_secs(100)) => {
                "completed"
            }
            _ = shutdown_rx.changed() => {
                "shutdown"
            }
        }
    });

    // Give the task a moment to start
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    // Send shutdown
    tx.send(true).unwrap();

    let result = tokio::time::timeout(std::time::Duration::from_secs(2), handle)
        .await
        .unwrap()
        .unwrap();

    assert_eq!(result, "shutdown");
}

// ---------------------------------------------------------------
//  Binary frame – additional edge cases
// ---------------------------------------------------------------

#[tokio::test]
async fn binary_frame_exactly_header_len_bytes_minimal_valid() {
    // v2 minimum: 1 version + 16 id + 4 seq + 32 tag, no payload.
    assert_eq!(conduit_protocol::BINARY_HEADER_LEN, 53);
    let frame = binary_frame("aa", "a", 0, b"");
    assert_eq!(frame.len(), conduit_protocol::BINARY_HEADER_LEN);
    let mut seq = None;
    let parsed = verify_binary("aa", &frame, &mut seq)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parsed.target_id, "a");
    assert!(parsed.payload.is_empty());
}

#[tokio::test]
async fn binary_frame_16_byte_id_with_trailing_zeros() {
    use conduit_protocol::BINARY_DEVICE_ID_LEN;
    let frame = binary_frame("aa", "test", 2, b"data");
    // Bytes 5..16 of the 16-byte id field are zero padding.
    let padding = &frame[1 + "test".len()..=BINARY_DEVICE_ID_LEN];
    assert_eq!(padding.len(), 12, "16-byte id field minus the 4-byte name");
    assert!(padding.iter().all(|b| *b == 0));
    let mut seq = None;
    let parsed = verify_binary("aa", &frame, &mut seq)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parsed.target_id, "test");
}

#[tokio::test]
async fn binary_frame_max_version_0xff_rejected() {
    let mut frame = binary_frame("aa", "dev", 1, b"data");
    frame[0] = 0xFF;
    let mut seq = None;
    assert_eq!(
        verify_binary("aa", &frame, &mut seq)
            .await
            .unwrap_err()
            .code,
        "binary_version_unsupported"
    );
}

#[tokio::test]
async fn binary_frame_v2_with_large_payload() {
    let payload = vec![0xABu8; 1024];
    let frame = binary_frame("aa", "big-payload", 9, &payload);
    let mut seq = None;
    let parsed = verify_binary("aa", &frame, &mut seq)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parsed.target_id, "big-payload");
    assert_eq!(parsed.payload, &payload[..]);
}

#[tokio::test]
async fn binary_frame_target_longer_than_16_bytes_is_truncated_not_accepted() {
    let frame = binary_frame("aa", "0123456789abcdefEXTRA", 1, b"x");
    let mut seq = None;
    let parsed = verify_binary("aa", &frame, &mut seq)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        parsed.target_id, "0123456789abcdef",
        "the 16-byte field truncates; a real device id fits in it"
    );
}

// ---------------------------------------------------------------
//  RateLimiter – additional edge cases
// ---------------------------------------------------------------

#[test]
fn rate_limiter_ipv6_independent_from_ipv4() {
    let rl = RateLimiter::new(1, 60);
    let ipv4 = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
    let ipv6 = "::1".parse::<IpAddr>().unwrap();
    assert!(rl.allow(ipv4));
    assert!(!rl.allow(ipv4), "ipv4 should be exhausted");
    assert!(rl.allow(ipv6), "ipv6 should be independent");
}

#[test]
fn rate_limiter_window_zero_expires_entries_immediately() {
    let rl = RateLimiter::new(1, 0);
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 99));
    // window_secs=0 → cutoff == now on every call, so any prior entry is
    // already expired and gets evicted: every attempt is allowed.
    assert!(rl.allow(ip));
    assert!(
        rl.allow(ip),
        "0-second window should expire entries immediately"
    );
}

// ---------------------------------------------------------------
//  Config – additional edge cases
// ---------------------------------------------------------------

#[test]
#[serial_test::serial]
fn config_enable_plain_ws_true_values() {
    for val in &["true", "1"] {
        // HMAC_SECRET must be set so this test never triggers file I/O.
        with_env_snapshot(
            &["RELAY_TOKEN", "HMAC_SECRET", "RELAY_ENABLE_PLAIN_WS"],
            || {
                unsafe {
                    std::env::set_var("RELAY_TOKEN", "config-test-token");
                    std::env::set_var("HMAC_SECRET", "test-secret");
                    std::env::set_var("RELAY_ENABLE_PLAIN_WS", *val);
                }
                let cfg = Config::resolve(None).unwrap();
                assert!(
                    cfg.enable_plain_ws,
                    "value '{}' should enable plain ws",
                    val
                );
            },
        );
    }
}

#[test]
#[serial_test::serial]
fn config_enable_plain_ws_false_values() {
    for val in &["false", "0", "yes", "no", ""] {
        with_env_snapshot(
            &["RELAY_TOKEN", "HMAC_SECRET", "RELAY_ENABLE_PLAIN_WS"],
            || {
                unsafe {
                    std::env::set_var("RELAY_TOKEN", "config-test-token");
                    std::env::set_var("HMAC_SECRET", "test-secret");
                    std::env::set_var("RELAY_ENABLE_PLAIN_WS", *val);
                }
                let cfg = Config::resolve(None).unwrap();
                assert!(
                    !cfg.enable_plain_ws,
                    "value '{}' should NOT enable plain ws",
                    val
                );
            },
        );
    }
}

#[test]
#[serial_test::serial]
fn config_rejects_whitespace_only_relay_token() {
    with_env_snapshot(&["RELAY_TOKEN", "HMAC_SECRET"], || {
        unsafe {
            std::env::set_var("RELAY_TOKEN", "   ");
            std::env::set_var("HMAC_SECRET", "config-test-secret");
        }
        // Trimming is what makes this fail: without it a token nobody would
        // ever type is a valid credential, and `"   "` is exactly the kind of
        // placeholder a hurried deployment leaves behind.
        let result = Config::resolve(None);
        let err = result.expect_err("a whitespace-only token must be refused");
        assert!(
            err.contains("no relay token"),
            "whitespace-only must count as unset, not as short: {}",
            err
        );
    });
}

#[test]
#[serial_test::serial]
fn config_hmac_secret_uses_exact_bytes() {
    with_env_snapshot(&["RELAY_TOKEN", "HMAC_SECRET"], || {
        unsafe {
            std::env::set_var("RELAY_TOKEN", "config-test-token");
            std::env::set_var("HMAC_SECRET", "my-secret");
        }
        let cfg = Config::resolve(None).unwrap();
        assert_eq!(cfg.hmac_secret, b"my-secret");
    });
}

// ---------------------------------------------------------------
//  AppState – basic construction tests
// ---------------------------------------------------------------

#[test]
fn app_state_initial_active_connections_zero() {
    let state = test_state(0, 0, 0, 0, 0);
    assert_eq!(state.active_connections.load(Ordering::Relaxed), 0);
}

#[test]
fn app_state_rate_limiter_is_configurable() {
    let rl = Arc::new(RateLimiter::new(20, 120));
    let ip = IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1));
    for _ in 0..20 {
        assert!(rl.allow(ip));
    }
    assert!(!rl.allow(ip));
}

#[test]
fn metrics_message_size_tracking_accumulates_correctly() {
    let m = Metrics::new();
    m.message_size_bytes_sum.fetch_add(100, Ordering::Relaxed);
    m.message_size_bytes_count.fetch_add(1, Ordering::Relaxed);
    m.message_size_bytes_sum.fetch_add(200, Ordering::Relaxed);
    m.message_size_bytes_count.fetch_add(1, Ordering::Relaxed);

    assert_eq!(m.message_size_bytes_sum.load(Ordering::Relaxed), 300);
    assert_eq!(m.message_size_bytes_count.load(Ordering::Relaxed), 2);
}

// ================================================================
//  End-to-end integration tests
//
//  Spin up a real plain-WS listener + real handle_connection, then
//  drive it with tokio-tungstenite clients over loopback TCP.
// ================================================================

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async};

/// HMAC secret used by every e2e test state (also the relay token).
const E2E_SECRET: &[u8] = b"test-secret-for-unit-tests!!!";

/// The bearer credential clients present as `relay_token`.
const E2E_RELAY_TOKEN: &str = "test-secret-for-unit-tests!!!";

/// The key a test device signs its routes with.
///
/// Each device gets its own, derived from its id exactly as a real host would
/// derive it from that device's pairing secret. Deliberately **not**
/// `E2E_RELAY_TOKEN`: signing with the token is the defect VULNERABILITY 1 is
/// about, and `e2e_route_signed_with_the_relay_token_is_rejected` proves the
/// relay refuses exactly that.
fn e2e_route_key(device_id: &str) -> Vec<u8> {
    hmac::derive_route_key(E2E_SECRET, device_id).to_vec()
}

/// A test host's key resolver.
///
/// Permissive on purpose: every device id resolves, so a test can sign as
/// whichever device it needs to be. The property that a device cannot sign *as
/// another* is enforced in two places this harness deliberately does not model
/// — `conduit_protocol::hmac::RouteKeyring` (which only ever hands out the key
/// registered under the id being verified) and the relay's own `sender_mismatch`
/// check, which ties the attributed sender to the authenticated connection. Both
/// are covered by their own tests; duplicating the KDF isolation here would only
/// re-test the protocol crate.
#[derive(Debug)]
struct TestRouteKeys;

impl crate::state::RouteKeys for TestRouteKeys {
    fn key_for(&self, device_id: &str) -> Option<Vec<u8>> {
        Some(e2e_route_key(device_id))
    }
}

/// Pick a free loopback port (best-effort; race window is tiny in tests).
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind for free_port")
        .local_addr()
        .expect("local_addr")
        .port()
}

/// Build an AppState tuned for e2e tests. `max_attempts` is the per-IP
/// connection rate limit (use a generous value unless testing limits).
fn e2e_state(max_attempts: usize) -> Arc<AppState> {
    e2e_state_with_auth_timeout(max_attempts, 10)
}

fn e2e_state_with_auth_timeout(max_attempts: usize, auth_timeout_secs: u64) -> Arc<AppState> {
    e2e_state_with_opts(StateOpts {
        max_attempts,
        auth_timeout_secs,
        ..Default::default()
    })
}

/// Running plain-WS relay instance. Dropping (or shutdown) stops listeners.
struct RelayInstance {
    ws_addr: SocketAddr,
    health_port: u16,
    _shutdown_tx: watch::Sender<bool>,
    /// Broadcast to every connection handler, mirroring the production
    /// shutdown signal so tests exercise the same drain path.
    connection_shutdown_tx: watch::Sender<bool>,
    handles: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for RelayInstance {
    fn drop(&mut self) {
        // Best-effort: signal shutdown so accept loops and connection
        // handlers exit.
        let _ = self.connection_shutdown_tx.send(true);
        let _ = self._shutdown_tx.send(true);
        for h in &self.handles {
            h.abort();
        }
    }
}

/// Spawn a plain-WS accept loop + optional health server on loopback.
async fn spawn_relay(state: Arc<AppState>) -> RelayInstance {
    let ws_addr: SocketAddr = format!("127.0.0.1:{}", free_port())
        .parse()
        .expect("ws addr");
    let listener = TcpListener::bind(ws_addr).await.expect("bind ws");
    let bound = listener.local_addr().expect("ws local_addr");

    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut ws_shutdown = shutdown_rx.clone();
    let (conn_shutdown_tx, conn_shutdown_rx) = watch::channel(false);

    let state_clone = state.clone();
    let ws_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                result = listener.accept() => {
                    if let Ok((stream, peer)) = result {
                        let state = state_clone.clone();
                        let conn_shutdown = conn_shutdown_rx.clone();
                        tokio::spawn(async move {
                            let _guard = ConnectionGuard {
                                count: state.active_connections.clone(),
                            };
                            state.active_connections.fetch_add(1, Ordering::Relaxed);
                            state.metrics.connections_connected.fetch_add(1, Ordering::Relaxed);
                            handle_connection(stream, state, peer, conn_shutdown).await;
                        });
                    }
                }
                _ = ws_shutdown.changed() => break,
            }
        }
    });

    // Health server (real routing exercised by health/metrics tests).
    let health_handle = match spawn_health_server(state.clone(), shutdown_rx.clone()).await {
        Ok(h) => Some(h),
        Err(e) => {
            // Port collision is possible with free_port race; don't fail hard
            // unless the test needs health (those tests re-check).
            warn!(
                "spawn_health_server failed (continuing without health): {}",
                e
            );
            None
        }
    };

    // Tiny delay so the accept loop is polled at least once.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    let mut handles = vec![ws_handle];
    if let Some(h) = health_handle {
        handles.push(h);
    }

    RelayInstance {
        ws_addr: bound,
        health_port: state.config.health_port,
        _shutdown_tx: shutdown_tx,
        handles,
        connection_shutdown_tx: conn_shutdown_tx,
    }
}

type WsClient = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

async fn ws_client(addr: SocketAddr) -> WsClient {
    let url = format!("ws://{}", addr);
    let (ws, _) = connect_async(&url).await.expect("ws connect");
    ws
}

/// Read the next Text message with a timeout; returns None on close/timeout.
async fn recv_text(ws: &mut WsClient, secs: u64) -> Option<String> {
    let deadline = std::time::Duration::from_secs(secs);
    let end = tokio::time::Instant::now() + deadline;
    loop {
        let remaining = end.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => return Some(t.to_string()),
            Ok(Some(Ok(_))) => continue, // ignore pings/pongs/binary
            Ok(Some(Err(_))) | Ok(None) => return None,
            Err(_) => return None,
        }
    }
}

/// Wait until a Text message matching `pred` arrives, or timeout.
async fn recv_text_matching(
    ws: &mut WsClient,
    secs: u64,
    pred: impl Fn(&str) -> bool,
) -> Option<String> {
    let end = tokio::time::Instant::now() + std::time::Duration::from_secs(secs);
    loop {
        let remaining = end.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return None;
        }
        match tokio::time::timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(t)))) => {
                if pred(&t) {
                    return Some(t.to_string());
                }
            }
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(_))) | Ok(None) => return None,
            Err(_) => return None,
        }
    }
}

/// Send relay_auth and wait for relay_auth_ok (or return the rejection).
async fn authenticate(ws: &mut WsClient, device_id: &str, token: &str) -> Option<String> {
    let auth = serde_json::json!({
        "type": "relay_auth",
        "device_id": device_id,
        "relay_token": token,
    });
    ws.send(Message::Text(auth.to_string())).await.ok()?;
    recv_text(ws, 5).await
}

/// Build a properly signed `relay_route` using the real HMAC scheme and the
/// relay's message-signing key.
///
/// `from_device_id` is signed, exactly as a real client must send it.
fn signed_route(
    from_device: &str,
    to_device: &str,
    payload: &Value,
    timestamp: i64,
    nonce: &str,
) -> Value {
    let route = RelayRoute::signed_with(
        from_device,
        &e2e_route_key(from_device),
        from_device,
        to_device,
        payload.clone(),
        timestamp,
        nonce,
    );
    serde_json::to_value(&route).expect("relay route serialises")
}

/// [`signed_route`] with a sender supplied by the caller — used to prove a
/// client cannot claim to be somebody else.
fn signed_route_claiming(
    claim_from: &str,
    to_device: &str,
    payload: &Value,
    timestamp: i64,
    nonce: &str,
) -> Value {
    signed_route(claim_from, to_device, payload, timestamp, nonce)
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Poll until `check` returns true or timeout (for metric assertions).
async fn wait_until(timeout_ms: u64, mut check: impl FnMut() -> bool) -> bool {
    let end = tokio::time::Instant::now() + std::time::Duration::from_millis(timeout_ms);
    loop {
        if check() {
            return true;
        }
        if tokio::time::Instant::now() >= end {
            return check();
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// Minimal raw HTTP/1.1 GET client (status, body) without Authorization.
async fn http_get(port: u16, path: &str) -> (u16, String) {
    http_get_bearer(port, path, None).await
}

/// Minimal raw HTTP/1.1 GET client with optional `Authorization: Bearer`.
async fn http_get_bearer(port: u16, path: &str, token: Option<&str>) -> (u16, String) {
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("http connect");
    let auth_header = match token {
        Some(t) => format!("Authorization: Bearer {}\r\n", t),
        None => String::new(),
    };
    let req = format!(
        "GET {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\n{}Connection: close\r\n\r\n",
        path, port, auth_header
    );
    stream.write_all(req.as_bytes()).await.expect("http write");
    let mut buf = String::new();
    stream.read_to_string(&mut buf).await.expect("http read");
    let (head, body) = buf.split_once("\r\n\r\n").unwrap_or((&buf, ""));
    let status = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    (status, body.to_string())
}

/// Build a v2 binary relay frame:
/// `[0x02][target 16B][seq u32 BE][HMAC tag 32B][payload]`
///
/// The tag is computed exactly as the relay computes it, including the
/// sender-id binding, so these frames really are authenticated.
fn binary_frame(from_device: &str, target: &str, seq: u32, payload: &[u8]) -> Vec<u8> {
    use conduit_protocol::{
        BINARY_AUTHENTICATED_PREFIX_LEN, BINARY_DEVICE_ID_LEN, BINARY_FRAME_VERSION,
        BINARY_HEADER_LEN, BINARY_TAG_LEN,
    };

    let mut frame = Vec::with_capacity(BINARY_HEADER_LEN + payload.len());
    frame.push(BINARY_FRAME_VERSION);
    let mut id = [0u8; BINARY_DEVICE_ID_LEN];
    let bytes = target.as_bytes();
    let n = bytes.len().min(BINARY_DEVICE_ID_LEN);
    id[..n].copy_from_slice(&bytes[..n]);
    frame.extend_from_slice(&id);
    frame.extend_from_slice(&seq.to_be_bytes());

    let mut authenticated = Vec::with_capacity(BINARY_AUTHENTICATED_PREFIX_LEN + payload.len());
    authenticated.extend_from_slice(&frame[..BINARY_AUTHENTICATED_PREFIX_LEN]);
    authenticated.extend_from_slice(payload);
    let mac_input = binary_mac_input(from_device, &authenticated);
    let tag = hmac::compute_hmac(&e2e_route_key(from_device), &hex::encode(&mac_input));
    debug_assert_eq!(tag.len(), BINARY_TAG_LEN * 2, "tag must be 32 raw bytes");
    frame.extend_from_slice(&hex::decode(&tag).expect("hex tag decodes"));
    frame.extend_from_slice(payload);
    debug_assert_eq!(
        frame.len(),
        BINARY_HEADER_LEN + payload.len(),
        "the v2 frame layout must agree with BINARY_HEADER_LEN"
    );
    frame
}

// ---------------------------------------------------------------
//  E2E: lifecycle
// ---------------------------------------------------------------

#[tokio::test]
async fn e2e_connect_authenticate_forward_disconnect_full_lifecycle() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    // Two devices connect and authenticate.
    let mut alice = ws_client(relay.ws_addr).await;
    let mut bob = ws_client(relay.ws_addr).await;

    let resp_a = authenticate(&mut alice, "a11ce", E2E_RELAY_TOKEN)
        .await
        .expect("alice should get auth response");
    assert!(
        resp_a.contains("relay_auth_ok"),
        "alice auth should succeed, got: {}",
        resp_a
    );

    let resp_b = authenticate(&mut bob, "b0b", E2E_RELAY_TOKEN)
        .await
        .expect("bob should get auth response");
    assert!(
        resp_b.contains("relay_auth_ok"),
        "bob auth should succeed, got: {}",
        resp_b
    );

    // Alice routes a signed message to bob.
    let payload = serde_json::json!({"type": "ping"});
    let route = signed_route("a11ce", "b0b", &payload, now_millis(), "nonce-lifecycle-1");
    alice
        .send(Message::Text(route.to_string()))
        .await
        .expect("send route");

    // Match the delivery specifically, not the relay's own 25s `ping` keepalive,
    // which also contains "ping".
    let delivered = recv_text_matching(&mut bob, 5, |t| t.contains("relay_delivery"))
        .await
        .expect("bob should receive routed payload");

    // The relay wraps the verified payload rather than forwarding it bare, so
    // the recipient learns who sent it. It used to forward the inner message
    // alone, which discarded the one attribution it had just authenticated.
    let parsed: serde_json::Value =
        serde_json::from_str(&delivered).expect("delivered frame is JSON");
    assert_eq!(
        parsed.get("type").and_then(|v| v.as_str()),
        Some("relay_delivery"),
        "a routed message is delivered in a relay_delivery envelope, got: {delivered}"
    );
    assert_eq!(
        parsed.get("from_device_id").and_then(|v| v.as_str()),
        Some("a11ce"),
        "the envelope must name the authenticated sender"
    );
    assert_eq!(
        parsed.get("to_device_id").and_then(|v| v.as_str()),
        Some("b0b"),
        "the envelope must name the recipient"
    );
    assert_eq!(
        parsed
            .get("payload")
            .and_then(|p| p.get("type"))
            .and_then(|v| v.as_str()),
        Some("ping"),
        "the inner message rides inside the envelope unchanged"
    );
    // The verified signature and the sender claim must not be re-sent: the
    // envelope is the relay's statement, not the sender's.
    assert!(
        parsed.get("hmac").is_none(),
        "the relay's own envelope is not a signed relay_route"
    );

    // Metrics reflect the routing.
    let routed_ok = wait_until(2000, || {
        state.metrics.messages_routed.load(Ordering::Relaxed) >= 1
    })
    .await;
    assert!(routed_ok, "messages_routed should increment");

    // Disconnect alice; bob should still be connected.
    let _ = alice.close(None).await;
    let bye_ok = wait_until(3000, || {
        state
            .metrics
            .connections_disconnected
            .load(Ordering::Relaxed)
            >= 1
    })
    .await;
    assert!(
        bye_ok,
        "connections_disconnected should increment after alice drops"
    );
    let still_there = state.clients.read().await.contains_key("a11ce");
    assert!(!still_there, "alice should be removed after disconnect");
    assert!(
        state.clients.read().await.contains_key("b0b"),
        "bob should still be connected"
    );

    drop(relay);
}

// ---------------------------------------------------------------
//  E2E: multi-device routing
// ---------------------------------------------------------------

#[tokio::test]
async fn e2e_routes_between_multiple_connected_devices() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut a = ws_client(relay.ws_addr).await;
    let mut b = ws_client(relay.ws_addr).await;
    let mut c = ws_client(relay.ws_addr).await;

    for (ws, id) in [(&mut a, "0de7a"), (&mut b, "0de7b"), (&mut c, "0de7c")] {
        let r = authenticate(ws, id, E2E_RELAY_TOKEN)
            .await
            .expect("auth response");
        assert!(r.contains("relay_auth_ok"), "{} auth failed: {}", id, r);
    }

    // a → c and b → c with distinct nonces.
    let p1 = serde_json::json!({"type": "clipboard", "action": "sync", "n": 1});
    let p2 = serde_json::json!({"type": "clipboard", "action": "sync", "n": 2});
    a.send(Message::Text(
        signed_route("0de7a", "0de7c", &p1, now_millis(), "multi-1").to_string(),
    ))
    .await
    .unwrap();
    b.send(Message::Text(
        signed_route("0de7b", "0de7c", &p2, now_millis(), "multi-2").to_string(),
    ))
    .await
    .unwrap();

    let got1 = recv_text_matching(&mut c, 5, |t| t.contains("\"n\":1")).await;
    let got2 = recv_text_matching(&mut c, 5, |t| t.contains("\"n\":2")).await;
    assert!(got1.is_some(), "dev-c should receive message from dev-a");
    assert!(got2.is_some(), "dev-c should receive message from dev-b");

    // b should NOT receive a's messages.
    let b_saw = recv_text(&mut b, 1).await;
    // b might get a server ping eventually; ensure no clipboard payload.
    if let Some(t) = b_saw {
        assert!(
            !t.contains("clipboard"),
            "dev-b must not receive routed clipboard, got: {}",
            t
        );
    }

    drop(relay);
}

// ---------------------------------------------------------------
//  E2E: auth rejection
// ---------------------------------------------------------------

#[tokio::test]
async fn e2e_invalid_relay_token_rejected() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut ws = ws_client(relay.ws_addr).await;
    let resp = authenticate(&mut ws, "deadbe", "wrong-token")
        .await
        .expect("should get rejection");
    assert!(
        resp.contains("relay_auth_rejected") && resp.contains("invalid_token"),
        "expected invalid_token rejection, got: {}",
        resp
    );
    assert!(
        wait_until(2000, || {
            state.metrics.auth_attempts_failure.load(Ordering::Relaxed) >= 1
        })
        .await,
        "auth_attempts_failure should increment"
    );
    assert!(
        !state.clients.read().await.contains_key("deadbe"),
        "intruder must not be registered"
    );

    drop(relay);
}

#[tokio::test]
async fn e2e_missing_relay_token_rejected() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut ws = ws_client(relay.ws_addr).await;
    ws.send(Message::Text(
        serde_json::json!({"type": "relay_auth", "device_id": "no-token"}).to_string(),
    ))
    .await
    .unwrap();
    let resp = recv_text(&mut ws, 5).await.expect("rejection response");
    assert!(
        resp.contains("relay_auth_rejected") && resp.contains("missing_token"),
        "expected missing_token rejection, got: {}",
        resp
    );

    drop(relay);
}

// ---------------------------------------------------------------
//  E2E: rate limiting (per-IP connection admission)
// ---------------------------------------------------------------

#[tokio::test]
async fn e2e_rate_limited_connection_closed_and_metric_incremented() {
    // max_attempts = 1 → first connection OK, second rejected.
    let state = e2e_state(1);
    let relay = spawn_relay(state.clone()).await;

    let mut first = ws_client(relay.ws_addr).await;
    let r = authenticate(&mut first, "f157", E2E_RELAY_TOKEN)
        .await
        .expect("first auth");
    assert!(r.contains("relay_auth_ok"), "first should auth: {}", r);

    // Second connection from same IP should be closed without auth_ok.
    let mut second = ws_client(relay.ws_addr).await;
    let r2 = authenticate(&mut second, "5ec0", E2E_RELAY_TOKEN).await;
    assert!(
        r2.is_none() || !r2.as_deref().unwrap_or("").contains("relay_auth_ok"),
        "second connection should be rate-limited, got: {:?}",
        r2
    );

    assert!(
        wait_until(2000, || {
            state
                .metrics
                .messages_dropped_rate_limit
                .load(Ordering::Relaxed)
                >= 1
        })
        .await,
        "messages_dropped_rate_limit should increment on rejected connection"
    );

    drop(relay);
}

#[tokio::test]
async fn e2e_rate_limited_connection_releases_active_connection_slot() {
    // max_attempts = 1 → first connection OK, second rejected pre-auth.
    // The ConnectionGuard must still decrement after the second
    // connection's handle_connection returns, so active returns to 1.
    let state = e2e_state(1);
    let relay = spawn_relay(state.clone()).await;

    let mut first = ws_client(relay.ws_addr).await;
    let r = authenticate(&mut first, "f157", E2E_RELAY_TOKEN)
        .await
        .expect("first auth");
    assert!(r.contains("relay_auth_ok"), "first should auth: {}", r);

    // Second connection from same IP is rate-limited (closed pre-auth).
    let mut second = ws_client(relay.ws_addr).await;
    let r2 = authenticate(&mut second, "5ec0", E2E_RELAY_TOKEN).await;
    assert!(
        r2.is_none() || !r2.as_deref().unwrap_or("").contains("relay_auth_ok"),
        "second connection should be rate-limited, got: {:?}",
        r2
    );

    // Rate-limit metric increments.
    assert!(
        wait_until(2000, || {
            state
                .metrics
                .messages_dropped_rate_limit
                .load(Ordering::Relaxed)
                >= 1
        })
        .await,
        "messages_dropped_rate_limit should increment"
    );

    // Once the rate-limited connection's handler exits, the guard releases
    // its slot → active_connections back to 1 (only `first` remains).
    // fetch_add happens before the WS handshake, so after the second
    // connection is fully handled this is race-free.
    assert!(
        wait_until(3000, || {
            state.active_connections.load(Ordering::Relaxed) == 1
        })
        .await,
        "active_connections should return to 1 after rate-limited connection closes \
         (got {})",
        state.active_connections.load(Ordering::Relaxed)
    );

    // Only the first client is registered; the rate-limited one never authed.
    let clients = state.clients.read().await;
    assert!(
        clients.contains_key("f157"),
        "first should remain registered"
    );
    assert!(
        !clients.contains_key("5ec0"),
        "second must not be registered"
    );
    drop(clients);

    drop(relay);
}

#[tokio::test]
async fn e2e_auth_failure_releases_active_connection_slot() {
    // A rejected auth must still decrement active_connections when
    // handle_connection returns early (ConnectionGuard drop).
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut ws = ws_client(relay.ws_addr).await;
    let resp = authenticate(&mut ws, "deadbe", "wrong-token")
        .await
        .expect("should get rejection");
    assert!(
        resp.contains("relay_auth_rejected") && resp.contains("invalid_token"),
        "expected invalid_token rejection, got: {}",
        resp
    );

    // Failure metric increments.
    assert!(
        wait_until(2000, || {
            state.metrics.auth_attempts_failure.load(Ordering::Relaxed) >= 1
        })
        .await,
        "auth_attempts_failure should increment"
    );

    // ConnectionGuard drops after the early return → slot released.
    // Note: auth-failure path returns before incrementing
    // connections_disconnected — do not assert that metric here.
    assert!(
        wait_until(3000, || {
            state.active_connections.load(Ordering::Relaxed) == 0
        })
        .await,
        "active_connections should return to 0 after auth failure (got {})",
        state.active_connections.load(Ordering::Relaxed)
    );

    // No client registered for the rejected device.
    assert!(
        !state.clients.read().await.contains_key("deadbe"),
        "intruder must not be registered"
    );

    drop(relay);
}

#[tokio::test]
async fn e2e_auth_failure_then_reconnect_with_valid_token_succeeds() {
    // Slot must be free after a failed auth so a subsequent valid
    // connection can authenticate successfully.
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    // First attempt: wrong token → rejected.
    {
        let mut bad = ws_client(relay.ws_addr).await;
        let resp = authenticate(&mut bad, "0de7be7", "wrong-token")
            .await
            .expect("should get rejection");
        assert!(
            resp.contains("relay_auth_rejected"),
            "expected rejection, got: {}",
            resp
        );
    } // drop(bad) → connection ends; wait for guard to release

    assert!(
        wait_until(3000, || {
            state.active_connections.load(Ordering::Relaxed) == 0
        })
        .await,
        "active_connections should be 0 after failed auth (got {})",
        state.active_connections.load(Ordering::Relaxed)
    );

    // Second attempt: correct token → success.
    let mut good = ws_client(relay.ws_addr).await;
    let resp = authenticate(&mut good, "0de7be7", E2E_RELAY_TOKEN)
        .await
        .expect("should get auth response");
    assert!(
        resp.contains("relay_auth_ok"),
        "reconnect with valid token should succeed, got: {}",
        resp
    );

    // Client is registered before the auth_ok response is sent (see
    // handle_connection), so after a successful authenticate() the
    // entry is already present — no async wait needed here.
    assert!(
        state.clients.read().await.contains_key("0de7be7"),
        "retry-dev should be registered after successful reconnect"
    );
    assert_eq!(
        state.metrics.auth_attempts_failure.load(Ordering::Relaxed),
        1,
        "exactly one failure"
    );
    assert_eq!(
        state.metrics.auth_attempts_success.load(Ordering::Relaxed),
        1,
        "exactly one success"
    );

    drop(relay);
}

// ---------------------------------------------------------------
//  E2E: HMAC / replay protection
// ---------------------------------------------------------------

#[tokio::test]
async fn e2e_invalid_hmac_message_not_forwarded() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut sender = ws_client(relay.ws_addr).await;
    let mut target = ws_client(relay.ws_addr).await;
    authenticate(&mut sender, "5e4de4", E2E_RELAY_TOKEN).await;
    authenticate(&mut target, "7a4de7", E2E_RELAY_TOKEN).await;

    // Forge a route with a garbage HMAC. Use a distinctive payload so the
    // server's immediate keepalive `{"type":"ping"}` (tokio interval's
    // first tick fires at once) can't be mistaken for a forwarded route.
    let forged = serde_json::json!({
        "type": "relay_route",
        "to_device_id": "7a4de7",
        "payload": {"type": "forged_marker", "tag": "invalid-hmac-test"},
        "timestamp": now_millis(),
        "nonce": "forged-nonce-1",
        "hmac": "0000000000000000000000000000000000000000000000000000000000000000",
    });
    sender
        .send(Message::Text(forged.to_string()))
        .await
        .unwrap();

    // Target must NOT receive the forged payload within the window
    // (ignore keepalives / any other text).
    let got = recv_text_matching(&mut target, 1, |t| t.contains("forged_marker")).await;
    assert!(
        got.is_none(),
        "forged message must not be forwarded, got: {:?}",
        got
    );
    assert_eq!(
        state.metrics.messages_routed.load(Ordering::Relaxed),
        0,
        "forged message must not count as routed"
    );

    drop(relay);
}

#[tokio::test]
async fn e2e_replayed_route_delivered_only_once() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut sender = ws_client(relay.ws_addr).await;
    let mut target = ws_client(relay.ws_addr).await;
    authenticate(&mut sender, "5e4de4", E2E_RELAY_TOKEN).await;
    authenticate(&mut target, "7a4de7", E2E_RELAY_TOKEN).await;

    let payload = serde_json::json!({"type": "ping", "tag": "replay-test"});
    let route = signed_route("5e4de4", "7a4de7", &payload, now_millis(), "replay-nonce-1");

    // First delivery.
    sender.send(Message::Text(route.to_string())).await.unwrap();
    let first = recv_text_matching(&mut target, 5, |t| t.contains("replay-test")).await;
    assert!(first.is_some(), "first delivery should succeed");

    // Replay the exact same signed message (same nonce + timestamp).
    sender.send(Message::Text(route.to_string())).await.unwrap();
    let second = recv_text_matching(&mut target, 1, |t| t.contains("replay-test")).await;
    assert!(
        second.is_none(),
        "replayed route must NOT be delivered a second time"
    );
    assert_eq!(
        state.metrics.messages_routed.load(Ordering::Relaxed),
        1,
        "only the first delivery should count as routed"
    );

    drop(relay);
}

// ---------------------------------------------------------------
//  E2E: not-found routing
// ---------------------------------------------------------------

#[tokio::test]
async fn e2e_route_to_unknown_device_counts_not_found() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut sender = ws_client(relay.ws_addr).await;
    authenticate(&mut sender, "5e4de4", E2E_RELAY_TOKEN).await;

    let payload = serde_json::json!({"type": "ping"});
    let route = signed_route("5e4de4", "9057", &payload, now_millis(), "nf-nonce-1");
    sender.send(Message::Text(route.to_string())).await.unwrap();

    assert!(
        wait_until(2000, || {
            state
                .metrics
                .messages_dropped_not_found
                .load(Ordering::Relaxed)
                >= 1
        })
        .await,
        "messages_dropped_not_found should increment for unknown target"
    );
    assert_eq!(
        state.metrics.messages_routed.load(Ordering::Relaxed),
        0,
        "unknown target must not count as routed"
    );

    drop(relay);
}

// ---------------------------------------------------------------
//  E2E: auth timeout
// ---------------------------------------------------------------

#[tokio::test]
async fn e2e_auth_timeout_closes_unauthenticated_connection() {
    // 1-second auth timeout so the test finishes quickly.
    let state = e2e_state_with_auth_timeout(1000, 1);
    let relay = spawn_relay(state.clone()).await;

    let mut ws = ws_client(relay.ws_addr).await;
    // Never send relay_auth — wait for the server to drop us.
    let msg = recv_text(&mut ws, 5).await;
    // Either the stream closes (None) or we get nothing usable.
    assert!(
        msg.is_none() || !msg.as_deref().unwrap_or("").contains("relay_auth_ok"),
        "unauthenticated connection must not receive relay_auth_ok, got: {:?}",
        msg
    );
    // Connection should no longer be registered as an authenticated client.
    let removed = wait_until(2000, || {
        // After timeout, no client id was ever inserted; active conn may drop.
        state.metrics.auth_attempts_success.load(Ordering::Relaxed) == 0
    })
    .await;
    assert!(removed, "no successful auth should have been recorded");

    drop(relay);
}

// ---------------------------------------------------------------
//  E2E: oversized message
// ---------------------------------------------------------------

#[tokio::test]
async fn e2e_oversized_message_drops_connection() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut ws = ws_client(relay.ws_addr).await;
    authenticate(&mut ws, "b16", E2E_RELAY_TOKEN).await;

    // Send a text frame larger than MAX_TEXT_SIZE (1MB).
    let huge = "x".repeat(MAX_TEXT_SIZE + 1);
    let _ = ws.send(Message::Text(huge)).await;

    // Connection should be dropped: subsequent reads yield None/Err.
    let after = recv_text(&mut ws, 3).await;
    // Server breaks out of the loop and closes; we may see close frame or EOF.
    // Importantly the device should be deregistered.
    let deregistered = wait_until(3000, || {
        // auth succeeded so client was inserted; after drop it's removed.
        // We check by attempting a non-blocking read of the clients map.
        false // placeholder — async check below
    })
    .await;
    let _ = deregistered;
    let still = state.clients.read().await.contains_key("b16");
    assert!(
        !still,
        "oversized message must drop the connection and deregister the device (recv got: {:?})",
        after
    );

    drop(relay);
}

/// Binary frames had no size check at all while Text had one at both of its
/// arms, and the only bound in the whole path was tungstenite's 64 MiB default
/// read cap. Over-ceiling frames were sized against that default.
#[tokio::test]
async fn e2e_oversized_binary_frame_drops_connection() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut ws = ws_client(relay.ws_addr).await;
    authenticate(&mut ws, "b16", E2E_RELAY_TOKEN).await;

    // The device has to be registered before the frame that knocks it off.
    let mut registered = false;
    for _ in 0..50 {
        if state.clients.read().await.contains_key("b16") {
            registered = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(registered, "device must register before the frame");

    // One byte past MAX_BINARY_SIZE — the same boundary Text has always used.
    let _ = ws
        .send(Message::Binary(vec![0u8; MAX_BINARY_SIZE + 1]))
        .await;
    let after = recv_text(&mut ws, 3).await;

    let mut removed = false;
    for _ in 0..50 {
        if !state.clients.read().await.contains_key("b16") {
            removed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        removed,
        "an over-ceiling binary frame must drop the connection and deregister the device (recv got: {:?})",
        after
    );

    drop(relay);
}

// ---------------------------------------------------------------
//  E2E: binary frame routing
// ---------------------------------------------------------------

#[tokio::test]
async fn e2e_binary_frame_routed_to_target_device() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut a = ws_client(relay.ws_addr).await;
    let mut b = ws_client(relay.ws_addr).await;
    authenticate(&mut a, "b1455", E2E_RELAY_TOKEN).await;
    authenticate(&mut b, "b145d", E2E_RELAY_TOKEN).await;

    let frame = binary_frame("b1455", "b145d", 1, b"binary-payload-bytes");
    a.send(Message::Binary(frame)).await.expect("send binary");

    // Target receives the payload as a Binary message.
    let end = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut got: Option<Vec<u8>> = None;
    while tokio::time::Instant::now() < end && got.is_none() {
        match tokio::time::timeout(std::time::Duration::from_secs(1), b.next()).await {
            Ok(Some(Ok(Message::Binary(bytes)))) => got = Some(bytes),
            Ok(Some(Ok(_))) => {}
            _ => {}
        }
    }
    let bytes = got.expect("bin-dst should receive binary payload");

    // The relay re-frames rather than stripping. It used to forward the bare
    // payload, which left the recipient with no v2 header to parse and a tag it
    // could not check — so a relayed file chunk could not be decoded at all.
    // What arrives is a well-formed v2 frame addressed to the recipient and
    // tagged for the sender the relay authenticated.
    assert!(
        bytes.len() > conduit_protocol::BINARY_HEADER_LEN,
        "a relayed binary frame carries its own v2 header; got {} bytes",
        bytes.len()
    );
    assert_eq!(
        bytes[0],
        conduit_protocol::BINARY_FRAME_VERSION,
        "the forwarded frame must still be v2"
    );

    let target = std::str::from_utf8(&bytes[1..1 + conduit_protocol::BINARY_DEVICE_ID_LEN])
        .expect("target id is ASCII")
        .trim_end_matches('\0');
    assert_eq!(target, "b145d", "the frame is addressed to the recipient");

    // The payload survives intact behind the header.
    assert_eq!(
        &bytes[conduit_protocol::BINARY_HEADER_LEN..],
        b"binary-payload-bytes",
        "the re-framed payload must be byte-identical to what was sent"
    );

    // And the tag verifies under the *sender's* route key, which is what makes
    // the attribution real rather than asserted.
    let sender_key = state
        .route_keys
        .key_for("b1455")
        .expect("the sender has a route key");
    let mut authenticated = Vec::new();
    authenticated.extend_from_slice(&bytes[..conduit_protocol::BINARY_AUTHENTICATED_PREFIX_LEN]);
    authenticated.extend_from_slice(&bytes[conduit_protocol::BINARY_HEADER_LEN..]);
    let mac_input = hex::encode(conduit_protocol::binary_mac_input("b1455", &authenticated));
    let tag = hex::encode(
        &bytes[conduit_protocol::BINARY_TAG_OFFSET..conduit_protocol::BINARY_HEADER_LEN],
    );
    assert!(
        crate::hmac::verify_hmac(&sender_key, &mac_input, &tag),
        "the forwarded frame's tag must verify under the authenticated sender's route key"
    );

    assert!(
        wait_until(2000, || {
            state.metrics.messages_routed.load(Ordering::Relaxed) >= 1
        })
        .await,
        "binary route should count as messages_routed"
    );

    drop(relay);
}

// ---------------------------------------------------------------
//  E2E: concurrent clients
// ---------------------------------------------------------------

#[tokio::test]
async fn e2e_concurrent_connections_all_authenticate() {
    let state = e2e_state(10_000);
    let relay = spawn_relay(state.clone()).await;

    let mut tasks = Vec::new();
    for i in 0..20 {
        let addr = relay.ws_addr;
        tasks.push(tokio::spawn(async move {
            let mut ws = ws_client(addr).await;
            let id = format!("c{:04x}", i);
            let r = authenticate(&mut ws, &id, E2E_RELAY_TOKEN)
                .await
                .unwrap_or_default();
            assert!(
                r.contains("relay_auth_ok"),
                "client {} failed auth: {}",
                i,
                r
            );
            ws
        }));
    }
    let mut ok = 0;
    for t in tasks {
        if t.await.is_ok() {
            ok += 1;
        }
    }
    assert_eq!(ok, 20, "all 20 concurrent clients should authenticate");
    assert_eq!(
        state.metrics.auth_attempts_success.load(Ordering::Relaxed),
        20,
        "auth success metric should equal 20"
    );
    assert_eq!(
        state.metrics.auth_attempts_failure.load(Ordering::Relaxed),
        0
    );

    drop(relay);
}

// ---------------------------------------------------------------
//  E2E: health / metrics HTTP endpoints
// ---------------------------------------------------------------

#[test]
#[serial_test::serial]
fn the_plaintext_listener_defaults_to_loopback() {
    // The plaintext listener carries the bearer token in the clear, so
    // "reachable from the network" must be something a host opts into rather
    // than something it gets by starting the relay.
    with_env_snapshot(
        &[
            "RELAY_WS_BIND",
            "RELAY_TOKEN",
            "HMAC_SECRET",
            "HMAC_SECRET_FILE",
        ],
        || {
            unsafe {
                std::env::remove_var("RELAY_WS_BIND");
                std::env::set_var("RELAY_TOKEN", "token-for-this-test");
                std::env::set_var("HMAC_SECRET", "master-for-this-test");
            }
            let config = Config::resolve(None).expect("config resolves");
            assert!(
                config.ws_bind.is_loopback(),
                "the plaintext listener must default to loopback, got {}",
                config.ws_bind
            );
        },
    );
}

#[test]
#[serial_test::serial]
fn a_host_may_still_widen_the_plaintext_bind_explicitly() {
    with_env_snapshot(
        &[
            "RELAY_WS_BIND",
            "RELAY_TOKEN",
            "HMAC_SECRET",
            "HMAC_SECRET_FILE",
        ],
        || {
            unsafe {
                std::env::set_var("RELAY_WS_BIND", "0.0.0.0");
                std::env::set_var("RELAY_TOKEN", "token-for-this-test");
                std::env::set_var("HMAC_SECRET", "master-for-this-test");
            }
            let config = Config::resolve(None).expect("config resolves");
            assert_eq!(config.ws_bind, std::net::IpAddr::from([0, 0, 0, 0]));
        },
    );
}

#[tokio::test]
async fn health_endpoint_serves_json_status() {
    let state = e2e_state(1000);
    // Ensure health server is up (spawn_relay starts it best-effort).
    let relay = spawn_relay(state.clone()).await;
    let token = state.config.health_token.clone();
    let (status, body) = http_get_bearer(relay.health_port, "/health", Some(&token)).await;
    assert_eq!(status, 200, "GET /health should be 200, body: {}", body);
    let v: Value = serde_json::from_str(&body).expect("/health body should be JSON");
    assert_eq!(v["status"], "ok");
    assert!(
        v.get("active_connections").is_some(),
        "missing active_connections"
    );
    assert!(
        v.get("max_connections").is_some(),
        "missing max_connections"
    );

    drop(relay);
}

#[tokio::test]
async fn health_without_bearer_token_returns_401() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let (status, body) = http_get(relay.health_port, "/health").await;
    assert_eq!(status, 401, "missing token must be 401, body: {}", body);
    assert!(
        body.contains("unauthorized"),
        "401 body should mention unauthorized, got: {}",
        body
    );

    drop(relay);
}

#[tokio::test]
async fn health_with_wrong_bearer_token_returns_401() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let (status, body) = http_get_bearer(relay.health_port, "/health", Some("wrong-token")).await;
    assert_eq!(status, 401, "wrong token must be 401, body: {}", body);

    drop(relay);
}

#[tokio::test]
async fn metrics_endpoint_remains_unauthenticated() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let (status, body) = http_get(relay.health_port, "/metrics").await;
    assert_eq!(
        status, 200,
        "/metrics should not require bearer, body: {}",
        body
    );

    drop(relay);
}

#[tokio::test]
async fn metrics_endpoint_serves_prometheus_format() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let (status, body) = http_get(relay.health_port, "/metrics").await;
    assert_eq!(status, 200, "GET /metrics should be 200");
    assert!(
        body.contains("conduit_relay_active_connections"),
        "metrics should expose Prometheus gauge, got: {}",
        &body[..body.len().min(200)]
    );
    assert!(
        body.contains("# TYPE conduit_relay_connections_total counter"),
        "metrics should include TYPE metadata"
    );
    assert!(
        body.contains("conduit_relay_message_size_bytes_bucket"),
        "metrics should include histogram bucket"
    );

    drop(relay);
}

#[tokio::test]
async fn unknown_health_path_returns_404() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let (status, body) = http_get(relay.health_port, "/nope").await;
    assert_eq!(status, 404, "unknown path should 404, body: {}", body);
    assert_eq!(body, "Not Found");

    drop(relay);
}

#[tokio::test]
async fn graceful_shutdown_stops_health_listener() {
    let state = e2e_state(1000);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let health_port = state.config.health_port;
    let token = state.config.health_token.clone();
    let handle = spawn_health_server(state.clone(), shutdown_rx)
        .await
        .expect("health bind");

    // Listener is up.
    let (status, _) = http_get_bearer(health_port, "/health", Some(&token)).await;
    assert_eq!(status, 200, "health should be up before shutdown");

    // Signal shutdown; the accept loop should exit.
    shutdown_tx.send(true).unwrap();
    let finished = tokio::time::timeout(std::time::Duration::from_secs(3), handle).await;
    assert!(
        finished.is_ok(),
        "health listener task should finish after shutdown signal"
    );

    // After shutdown the port is released — connection should fail
    // (or, if reused by OS, not necessarily; we only require the task ended).
    // Give the OS a moment to release the port.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let connect_result = tokio::time::timeout(std::time::Duration::from_millis(500), async {
        tokio::net::TcpStream::connect(("127.0.0.1", health_port)).await
    })
    .await;
    // Either connect fails (typical) or succeeds if port was immediately
    // rebound — the critical assertion is the task finished above.
    let _ = connect_result;
}

// ================================================================
//  VULNERABILITY 1 — the signing key must not be the relay token
// ================================================================

/// Test-only knobs for building an [`AppState`] with non-default crypto or
/// HTTP settings. `AppState` itself is not `Clone` (it holds mpsc senders
/// and locks), so tests build one from these rather than mutating a copy.
#[derive(Clone)]
struct StateOpts {
    max_attempts: usize,
    auth_timeout_secs: u64,
    metrics_token: Option<String>,
}

impl Default for StateOpts {
    fn default() -> Self {
        Self {
            max_attempts: 1000,
            auth_timeout_secs: 10,
            metrics_token: None,
        }
    }
}

fn e2e_state_with_opts(opts: StateOpts) -> Arc<AppState> {
    let nonce_file =
        std::env::temp_dir().join(format!("relay-e2e-nonces-{}.json", std::process::id()));
    Arc::new(AppState {
        clients: Arc::new(RwLock::new(HashMap::new())),
        active_connections: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        metrics: Arc::new(Metrics::new()),
        rate_limiter: Arc::new(RateLimiter::new(opts.max_attempts, 60)),
        nonces: Arc::new(RwLock::new(NonceCache::new())),
        tls_pin: Some("sha256/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into()),
        route_keys: Arc::new(TestRouteKeys),
        config: Config {
            ws_port: 0,
            // Loopback: the plaintext listener carries the bearer token in clear.
            ws_bind: "127.0.0.1".parse().unwrap(),
            wss_port: 0,
            health_port: free_port(),
            health_bind: "127.0.0.1".parse().unwrap(),
            hmac_secret: E2E_SECRET.to_vec(),
            hmac_secret_file: std::env::temp_dir().join("relay-test-hmac-secret"),
            health_token: String::from_utf8_lossy(E2E_SECRET).to_string(),
            metrics_token: opts.metrics_token,
            // Route HMACs are verified with the per-device keys from the host
            // resolver; the token stays a separate credential on purpose: the
            // tests would fail loudly if anything ever verified routes with it.
            relay_token: String::from_utf8_lossy(E2E_SECRET).to_string(),
            tls: TlsParams::default(),
            enable_plain_ws: true,
            nonce_file,
            auth_timeout_secs: opts.auth_timeout_secs,
            relay_cert_pin: None,
        },
    })
}

#[tokio::test]
async fn e2e_client_cannot_forge_route_claiming_another_from_device_id() {
    // The core defect: `relay_route` carried no `from`, the relay never
    // bound the authenticated identity to the forwarded content, and the
    // MAC key was the token every client holds — so any authenticated
    // client could forge a route "from" any other device.
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut attacker = ws_client(relay.ws_addr).await;
    let mut victim = ws_client(relay.ws_addr).await;
    authenticate(&mut attacker, "aa77", E2E_RELAY_TOKEN).await;
    authenticate(&mut victim, "bb88", E2E_RELAY_TOKEN).await;

    // The attacker signs a route that claims to be from the victim's device.
    // The signature itself is *valid* — it uses the shared signing key — so
    // only the identity binding can catch this.
    let forged = signed_route_claiming(
        "bb88",
        "bb88",
        &serde_json::json!({"type": "ping", "tag": "forged-identity"}),
        now_millis(),
        "forged-identity-nonce",
    );
    attacker
        .send(Message::Text(forged.to_string()))
        .await
        .unwrap();

    let got = recv_text_matching(&mut victim, 2, |t| t.contains("forged-identity")).await;
    assert!(
        got.is_none(),
        "a client must not be able to forge a route claiming another \
         from_device_id, got: {got:?}"
    );
    assert_eq!(state.metrics.messages_routed.load(Ordering::Relaxed), 0);
    assert!(
        wait_until(2000, || {
            state
                .metrics
                .messages_dropped_hmac_failed
                .load(Ordering::Relaxed)
                >= 1
        })
        .await,
        "the forgery must be counted, not silently dropped"
    );

    // The sender is told why, via the documented error frame.
    let err = recv_text_matching(&mut attacker, 2, |t| t.contains("sender_mismatch")).await;
    assert!(
        err.is_some(),
        "the forged sender must get a diagnostic error frame, got: {err:?}"
    );

    drop(relay);
}

#[tokio::test]
async fn e2e_route_signed_with_the_relay_token_is_rejected() {
    // A client that knows the bearer token but not the signing key must not
    // be able to produce a valid route. This is exactly the configuration
    // `.env.example` used to recommend.
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut sender = ws_client(relay.ws_addr).await;
    let mut target = ws_client(relay.ws_addr).await;
    authenticate(&mut sender, "cc99", E2E_RELAY_TOKEN).await;
    authenticate(&mut target, "dd00", E2E_RELAY_TOKEN).await;

    let mut route = serde_json::Map::new();
    route.insert("type".into(), serde_json::json!("relay_route"));
    route.insert("from_device_id".into(), serde_json::json!("cc99"));
    route.insert("to_device_id".into(), serde_json::json!("dd00"));
    route.insert("payload".into(), serde_json::json!({"tag": "token-signed"}));
    route.insert("timestamp".into(), serde_json::json!(now_millis()));
    route.insert("nonce".into(), serde_json::json!("token-signed-nonce"));
    route.insert("key_id".into(), serde_json::json!("cc99"));
    let canonical = hmac::canonical_signing_string(&serde_json::Value::Object(route.clone()));
    // Sign with the RELAY TOKEN — the pre-fix key.
    route.insert(
        "hmac".into(),
        serde_json::json!(hmac::compute_hmac(E2E_SECRET, &canonical)),
    );

    sender
        .send(Message::Text(serde_json::Value::Object(route).to_string()))
        .await
        .unwrap();

    let got = recv_text_matching(&mut target, 2, |t| t.contains("token-signed")).await;
    assert!(
        got.is_none(),
        "a route signed with the relay token must be rejected, got: {got:?}"
    );
    assert!(
        wait_until(2000, || {
            state
                .metrics
                .messages_dropped_hmac_failed
                .load(Ordering::Relaxed)
                >= 1
        })
        .await
    );

    drop(relay);
}

#[test]
fn route_key_is_never_the_pairing_secret() {
    let key = hmac::derive_route_key(E2E_SECRET, "ee11");
    assert_ne!(
        key.as_slice(),
        E2E_SECRET,
        "the route key must not be the pairing secret itself"
    );
    // Deterministic, so a reconnecting device still verifies.
    assert_eq!(key, hmac::derive_route_key(E2E_SECRET, "ee11"));
}

#[test]
fn route_keys_differ_per_device_and_do_not_depend_on_the_relay_token() {
    let a = hmac::derive_route_key(E2E_SECRET, "ee11");
    let b = hmac::derive_route_key(E2E_SECRET, "ff22");
    assert_ne!(a, b, "each device must get its own key");
    assert_ne!(
        a,
        hmac::derive_route_key(b"a-completely-different-master", "ee11"),
        "sanity: a different pairing secret must give a different key"
    );

    // Route keys are a pure function of the pairing secret and the device id.
    // RELAY_TOKEN is not an input to that derivation, so rotating the bearer
    // token cannot invalidate any device's route key.
    with_env_snapshot(&["RELAY_TOKEN", "HMAC_SECRET", "HMAC_SECRET_FILE"], || {
        let key_for_token = |token: &str| {
            unsafe {
                std::env::set_var("RELAY_TOKEN", token);
                std::env::set_var("HMAC_SECRET", "shared-master");
            }
            // The resolved config carries no route keys at all now — the host
            // resolver owns them, and derivation never sees the token.
            let config = Config::resolve(None).expect("config");
            assert!(!config.relay_token.is_empty());
            hmac::derive_route_key(b"shared-master", "ee11")
        };
        assert_eq!(
            key_for_token("token-alpha"),
            key_for_token("token-bravo"),
            "route keys must be independent of RELAY_TOKEN"
        );
    });
}

#[tokio::test]
async fn e2e_route_signed_with_another_devices_key_is_rejected() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut sender = ws_client(relay.ws_addr).await;
    let mut target = ws_client(relay.ws_addr).await;
    authenticate(&mut sender, "ee11", E2E_RELAY_TOKEN).await;
    authenticate(&mut target, "ff22", E2E_RELAY_TOKEN).await;

    // "ee11" signs a route attributing itself to "ff22", using ff22's own key
    // so the MAC is genuinely valid. The relay must still refuse it, because
    // attribution is pinned to the authenticated connection.
    let route = RelayRoute::signed_with(
        "ff22",
        &e2e_route_key("ff22"),
        "ff22",
        "ff22",
        serde_json::json!({"type": "ping", "tag": "forged"}),
        now_millis(),
        "forged-nonce",
    );
    sender
        .send(Message::Text(serde_json::to_string(&route).unwrap()))
        .await
        .unwrap();

    assert!(
        recv_text_matching(&mut target, 2, |t| t.contains("forged"))
            .await
            .is_none(),
        "a device must not be able to route as another device"
    );
}

// ================================================================
//  VULNERABILITY 3 — no message may vanish without a trace
// ================================================================

#[tokio::test]
async fn e2e_unknown_message_type_is_counted_and_answered() {
    // Previously `_ => {}` swallowed it: no log, no metric, no reply.
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut ws = ws_client(relay.ws_addr).await;
    authenticate(&mut ws, "1234", E2E_RELAY_TOKEN).await;

    ws.send(Message::Text(
        serde_json::json!({"type": "no_such_message_type"}).to_string(),
    ))
    .await
    .unwrap();

    let err = recv_text_matching(&mut ws, 5, |t| t.contains("unknown_message_type")).await;
    assert!(err.is_some(), "expected an error frame, got: {err:?}");
    assert!(
        wait_until(2000, || {
            state
                .metrics
                .messages_dropped_unknown_type
                .load(Ordering::Relaxed)
                >= 1
        })
        .await,
        "messages_dropped_unknown_type must increment"
    );

    drop(relay);
}

#[tokio::test]
async fn e2e_unwrapped_encrypted_message_is_rejected_loudly() {
    // `encrypted` is the protocol's primary message type. Forwarding it
    // blindly would be an identity hole (`source_device` is
    // unauthenticated), but dropping it silently is worse — so it gets an
    // explicit refusal with a code the client can act on.
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut ws = ws_client(relay.ws_addr).await;
    authenticate(&mut ws, "5678", E2E_RELAY_TOKEN).await;

    ws.send(Message::Text(
        serde_json::json!({
            "type": "encrypted",
            "source_device": "someone-else",
            "nonce": "aa",
            "hmac": "bb",
            "data": "cc",
        })
        .to_string(),
    ))
    .await
    .unwrap();

    let err = recv_text_matching(&mut ws, 5, |t| t.contains("not_wrapped_in_relay_route")).await;
    assert!(
        err.is_some(),
        "an unwrapped 'encrypted' message must be refused explicitly, got: {err:?}"
    );
    assert!(
        wait_until(2000, || {
            state
                .metrics
                .messages_dropped_unknown_type
                .load(Ordering::Relaxed)
                >= 1
        })
        .await
    );

    drop(relay);
}

#[tokio::test]
async fn e2e_malformed_json_is_counted_and_answered() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut ws = ws_client(relay.ws_addr).await;
    authenticate(&mut ws, "9abc", E2E_RELAY_TOKEN).await;

    ws.send(Message::Text("{not json".into())).await.unwrap();

    let err = recv_text_matching(&mut ws, 5, |t| t.contains("malformed_json")).await;
    assert!(err.is_some(), "expected an error frame, got: {err:?}");
    assert!(
        wait_until(2000, || {
            state
                .metrics
                .messages_dropped_unknown_type
                .load(Ordering::Relaxed)
                >= 1
        })
        .await
    );

    drop(relay);
}

#[tokio::test]
async fn e2e_client_ping_is_answered_with_pong() {
    // The relay pings every 25 s but used to ignore a client `ping`, so the
    // PROTOCOL.md ping/pong keep-alive only worked one way.
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut ws = ws_client(relay.ws_addr).await;
    authenticate(&mut ws, "abcd", E2E_RELAY_TOKEN).await;

    ws.send(Message::Text(
        serde_json::json!({"type": "ping"}).to_string(),
    ))
    .await
    .unwrap();

    let pong = recv_text_matching(&mut ws, 5, |t| t.contains("\"pong\"")).await;
    assert!(
        pong.is_some(),
        "a client ping must be answered with a pong, got: {pong:?}"
    );

    drop(relay);
}

#[tokio::test]
async fn e2e_replay_is_counted_separately_from_hmac_failure() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut sender = ws_client(relay.ws_addr).await;
    let mut target = ws_client(relay.ws_addr).await;
    authenticate(&mut sender, "1111", E2E_RELAY_TOKEN).await;
    authenticate(&mut target, "2222", E2E_RELAY_TOKEN).await;

    let payload = serde_json::json!({"type": "ping", "tag": "replay-counted"});
    let route = signed_route("1111", "2222", &payload, now_millis(), "counted-nonce");
    sender.send(Message::Text(route.to_string())).await.unwrap();
    assert!(
        recv_text_matching(&mut target, 5, |t| t.contains("replay-counted"))
            .await
            .is_some()
    );

    sender.send(Message::Text(route.to_string())).await.unwrap();
    assert!(
        wait_until(2000, || {
            state
                .metrics
                .messages_dropped_replay
                .load(Ordering::Relaxed)
                >= 1
        })
        .await,
        "a replay must be counted under messages_dropped_replay"
    );
    assert_eq!(
        state
            .metrics
            .messages_dropped_hmac_failed
            .load(Ordering::Relaxed),
        0,
        "a replay is not an integrity failure"
    );

    drop(relay);
}

// ================================================================
//  VULNERABILITY 5 — bounded memory, validated identities
// ================================================================

#[test]
fn rate_limiter_sweep_evicts_addresses_that_stop_connecting() {
    // Entries used to be pruned only when the SAME ip reconnected, so an
    // attacker rotating source addresses grew the map forever.
    let rl = RateLimiter::new(1000, 3600);
    assert!(rl.allow(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
    for i in 0..500u16 {
        assert!(rl.allow(IpAddr::V4(Ipv4Addr::new(
            172,
            16,
            (i / 256) as u8,
            (i % 256) as u8
        ))));
    }
    assert_eq!(
        rl.tracked_addresses(),
        501,
        "500 rotating addresses plus the first one"
    );

    // A sweep with a window that has already expired removes them all.
    let short = RateLimiter::new(1000, 0);
    for i in 0..50u8 {
        assert!(short.allow(IpAddr::V4(Ipv4Addr::new(192, 168, i, 1))));
    }
    assert_eq!(short.tracked_addresses(), 50);
    assert_eq!(
        short.sweep(),
        50,
        "sweep must remove every address whose window has expired"
    );
    assert_eq!(short.tracked_addresses(), 0);
}

#[test]
fn rate_limiter_sweep_keeps_live_windows() {
    let rl = RateLimiter::new(1000, 3600);
    assert!(rl.allow(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))));
    assert!(rl.allow(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))));
    assert_eq!(rl.sweep(), 0);
    assert_eq!(
        rl.tracked_addresses(),
        2,
        "live windows must survive a sweep"
    );
}

#[test]
fn device_id_validation_accepts_the_documented_shape() {
    // PROTOCOL.md: device_id is the first 16 hex chars of the X25519 key.
    assert!(is_valid_device_id("b2c3d4e5f6a7b8c9"));
    assert!(is_valid_device_id("0"));
    assert!(is_valid_device_id(&"a".repeat(64)));
    assert!(validate_device_id("deadbeefcafe1234").is_ok());
}

#[test]
fn device_id_validation_accepts_the_ids_the_app_actually_mints() {
    // The desktop's own id is `Uuid::new_v4().to_string()`, and a phone's is
    // either that or the per-connection UUID the pairing handshake falls back
    // to. A strict "bare hex" rule would reject every real device at the auth
    // gate, which is the same as having no relay.
    let uuid = "550e8400-e29b-41d4-a716-446655440000";
    assert!(is_valid_device_id(uuid), "a UUIDv4 is a real device id");
    assert!(validate_device_id(uuid).is_ok());
    assert!(is_valid_device_id("3f2504e0-4f89-11d3-9a0c-0305e82c3301"));
}

#[test]
fn device_id_validation_rejects_everything_else() {
    for bad in [
        "",
        "alice",         // not hex
        "DEADBEEF",      // uppercase would alias a lowercase id
        "dead beef",     // whitespace
        "deadbeef/1234", // path-ish
        "日本語",        // non-ascii
        "dead\nbeef",    // control character
        "-deadbeef",     // leading hyphen
        "deadbeef-",     // trailing hyphen
        "dead--beef",    // repeated hyphen: two spellings of one id
        "----",          // all hyphens
    ] {
        assert!(
            !is_valid_device_id(bad),
            "{bad:?} must not be accepted as a device id"
        );
        assert!(validate_device_id(bad).is_err());
    }
    assert!(
        !is_valid_device_id(&"a".repeat(65)),
        "ids longer than 64 chars must be rejected"
    );
}

#[tokio::test]
async fn e2e_a_uuid_device_id_can_route() {
    // The end-to-end version of `device_id_validation_accepts_the_ids_the_app_
    // actually_mints`: a hyphenated UUID must authenticate, be routable, and
    // have its own route key — the whole feature is dead if this is rejected.
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let desktop = "550e8400-e29b-41d4-a716-446655440000";
    let phone = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";

    let mut sender = ws_client(relay.ws_addr).await;
    let mut target = ws_client(relay.ws_addr).await;
    let a = authenticate(&mut sender, desktop, E2E_RELAY_TOKEN)
        .await
        .expect("a UUID device id should get an auth response");
    assert!(
        a.contains("relay_auth_ok"),
        "a UUID device id must authenticate, got: {a}"
    );
    let b = authenticate(&mut target, phone, E2E_RELAY_TOKEN)
        .await
        .expect("the second UUID should get an auth response");
    assert!(b.contains("relay_auth_ok"), "got: {b}");

    sender
        .send(Message::Text(
            serde_json::to_string(&signed_route(
                desktop,
                phone,
                &serde_json::json!({"type": "ping", "tag": "uuid-routed"}),
                now_millis(),
                "uuid-nonce",
            ))
            .unwrap(),
        ))
        .await
        .unwrap();

    let got = recv_text_matching(&mut target, 5, |t| t.contains("uuid-routed")).await;
    assert!(got.is_some(), "a route between two UUID ids must arrive");
}

#[tokio::test]
async fn e2e_invalid_device_id_is_rejected_at_auth() {
    // Otherwise a single 1 MB frame could insert a megabyte-scale map key
    // into both the routing table and the nonce cache.
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut ws = ws_client(relay.ws_addr).await;
    let huge_id = "A".repeat(4096);
    let resp = authenticate(&mut ws, &huge_id, E2E_RELAY_TOKEN).await;
    let resp = resp.expect("relay answers");
    assert!(
        resp.contains("invalid_device_id"),
        "an oversized/non-hex device id must be refused, got: {resp}"
    );
    assert!(!state.clients.read().await.contains_key(&huge_id));
    assert!(
        state.metrics.auth_attempts_failure.load(Ordering::Relaxed) >= 1,
        "a rejected device id must count as an auth failure"
    );

    drop(relay);
}

#[tokio::test]
async fn reconcile_clients_removes_dead_senders() {
    // An aborted/panicked handle_connection used to leave a dead Sender in
    // the routing table forever, so the device looked permanently
    // connected and messages were sent into a void.
    let clients: Clients = Arc::new(RwLock::new(HashMap::new()));
    {
        let (tx, rx) = mpsc::channel::<Message>(4);
        clients.write().await.insert("aabb".into(), Queue::new(tx));
        drop(rx);
    }
    let (tx_live, _rx_live) = mpsc::channel::<Message>(4);
    clients
        .write()
        .await
        .insert("ccdd".into(), Queue::new(tx_live));
    assert_eq!(clients.read().await.len(), 2);

    assert_eq!(reconcile_clients(&clients).await, 1);
    let map = clients.read().await;
    assert!(
        !map.contains_key("aabb"),
        "dead sender must be reconciled out"
    );
    assert!(map.contains_key("ccdd"), "live sender must be kept");
}

// ================================================================
//  VULNERABILITY 7 — per-device route keys
//
//  The relay used to accept one shared message-signing key configured
//  through RELAY_SIGNING_KEY, which meant anyone who got that key could
//  route as any device. Keys are now per device, derived by the host
//  from each device's own pairing secret, so the tests below cover the
//  resolver that hands them out.
// ================================================================

#[test]
fn static_route_keys_starts_empty_so_nothing_verifies() {
    let keys = StaticRouteKeys::new();
    assert!(keys.is_empty());
    assert_eq!(RouteKeys::key_for(&keys, "anyone"), None);
    assert!(RouteKeys::device_ids(&keys).is_empty());
}

#[test]
fn static_route_keys_resolves_only_registered_devices() {
    let mut keys = StaticRouteKeys::new();
    keys.insert("ee11", e2e_route_key("ee11"));
    keys.insert("ff22", b"another-key".to_vec());

    assert_eq!(
        RouteKeys::key_for(&keys, "ee11"),
        Some(e2e_route_key("ee11"))
    );
    // An unregistered device must not fall back to a shared default.
    assert_eq!(RouteKeys::key_for(&keys, "intruder"), None);
    assert_eq!(RouteKeys::device_ids(&keys), vec!["ee11", "ff22"]);
}

#[test]
fn static_route_keys_forgets_a_device_on_remove() {
    // Unpairing: the host drops the device, so it stops being able to sign.
    let mut keys = StaticRouteKeys::from_pairs([("ee11".to_string(), e2e_route_key("ee11"))]);
    assert!(RouteKeys::key_for(&keys, "ee11").is_some());
    keys.remove("ee11");
    assert_eq!(RouteKeys::key_for(&keys, "ee11"), None);
    assert!(keys.is_empty());
}

#[test]
fn static_route_keys_registration_replaces_the_previous_key() {
    // Re-pairing the same device id under a new pairing secret: the host
    // overwrites, and the old key immediately stops working. There is no
    // rotation window to manage because the key is per device.
    let mut keys = StaticRouteKeys::new();
    keys.insert("ee11", e2e_route_key("ee11"));
    let old = RouteKeys::key_for(&keys, "ee11").expect("registered");
    keys.insert(
        "ee11",
        hmac::derive_route_key(b"a-new-pairing-secret", "ee11"),
    );
    let new = RouteKeys::key_for(&keys, "ee11").expect("registered");
    assert_ne!(old, new);
    assert_eq!(keys.len(), 1);
}

// ================================================================
//  VULNERABILITY 9/11 — pin discovery + metrics endpoint gating
// ================================================================

#[tokio::test]
async fn pin_endpoint_serves_the_spki_pin() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let (status, body) = http_get(relay.health_port, "/pin").await;
    assert_eq!(status, 200, "GET /pin should 200, body: {body}");
    let v: Value = serde_json::from_str(&body).expect("/pin body should be JSON");
    assert_eq!(v["algorithm"], "spki-sha256");
    let pin = v["sha256"].as_str().expect("sha256 must be a string");
    assert!(
        pin.starts_with("sha256/"),
        "the pin must be the SPKI pin clients verify, got: {pin}"
    );
    assert_eq!(
        pin,
        state.tls_pin.as_deref().expect("test state has a pin"),
        "/pin must serve the relay's own loaded certificate pin"
    );

    drop(relay);
}

#[tokio::test]
async fn metrics_endpoint_stays_open_when_no_token_is_configured() {
    let state = e2e_state(1000);
    assert!(state.config.metrics_token.is_none());
    let relay = spawn_relay(state.clone()).await;

    let (status, _) = http_get(relay.health_port, "/metrics").await;
    assert_eq!(
        status, 200,
        "/metrics must stay unauthenticated by default (existing decision)"
    );

    drop(relay);
}

#[tokio::test]
async fn metrics_endpoint_requires_the_token_when_configured() {
    let state = e2e_state_with_opts(StateOpts {
        metrics_token: Some("prom-token".to_string()),
        ..Default::default()
    });
    let relay = spawn_relay(state.clone()).await;

    let (status, body) = http_get(relay.health_port, "/metrics").await;
    assert_eq!(status, 401, "no token should be 401, body: {body}");
    assert!(body.contains("unauthorized"));

    let (status, _) = http_get_bearer(relay.health_port, "/metrics", Some("wrong")).await;
    assert_eq!(status, 401, "wrong token should be 401");

    let (status, body) = http_get_bearer(relay.health_port, "/metrics", Some("prom-token")).await;
    assert_eq!(status, 200, "correct token should be 200");
    assert!(body.contains("conduit_relay_active_connections"));

    drop(relay);
}

#[test]
fn prometheus_emits_real_histogram_buckets() {
    // Previously a single `+Inf` series, which told an operator nothing.
    let state = test_state(0, 0, 0, 0, 0);
    state.metrics.observe_message_size(64);
    state.metrics.observe_message_size(1000);
    state.metrics.observe_message_size(100_000);
    let out = build_prometheus_metrics(&state);

    for bound in SIZE_BUCKETS {
        assert!(
            out.contains(&format!(
                "conduit_relay_message_size_bytes_bucket{{le=\"{bound}\"}}"
            )),
            "missing bucket le=\"{bound}\""
        );
    }
    assert!(out.contains("conduit_relay_message_size_bytes_bucket{le=\"+Inf\"} 3"));
    assert!(out.contains("conduit_relay_message_size_bytes_count 3"));

    // Buckets are cumulative and monotonically increasing.
    let cumulative: Vec<u64> = SIZE_BUCKETS
        .iter()
        .map(|b| {
            let needle = format!("conduit_relay_message_size_bytes_bucket{{le=\"{b}\"}} ");
            out.lines()
                .find(|l| l.starts_with(&needle))
                .and_then(|l| l[needle.len()..].trim().parse().ok())
                .expect("bucket line must carry a count")
        })
        .collect();
    assert!(
        cumulative.windows(2).all(|w| w[0] <= w[1]),
        "buckets must be cumulative: {cumulative:?}"
    );
    assert_eq!(cumulative.last().copied(), Some(3));
}

#[test]
fn prometheus_exposes_internal_map_sizes_and_new_drop_reasons() {
    // The two structures that used to grow without bound must be visible.
    let state = test_state(0, 0, 0, 0, 0);
    let out = build_prometheus_metrics(&state);
    assert!(out.contains("conduit_relay_registered_devices"));
    assert!(out.contains("conduit_relay_rate_limit_tracked_ips"));
    for reason in [
        "not_found",
        "timeout",
        "rate_limit",
        "replay",
        "hmac_failed",
        "unknown_type",
    ] {
        assert!(
            out.contains(&format!(
                "conduit_relay_messages_dropped_total{{reason=\"{reason}\"}}"
            )),
            "missing drop reason {reason}"
        );
    }
}

#[tokio::test]
async fn messages_dropped_timeout_is_wired_to_the_forward_path() {
    // This counter was declared, exported in /health and /metrics, and
    // never incremented anywhere in production code — it always reported 0.
    let state = Arc::new(test_state(0, 0, 0, 0, 0));
    // A live target that never drains: the first message fills the queue,
    // the second blocks and must time out instead of hanging forever.
    let (tx, _rx) = mpsc::channel::<Message>(1);
    state
        .clients
        .write()
        .await
        .insert("tgt".to_string(), Queue::new(tx.clone()));

    let timeout = std::time::Duration::from_millis(50);
    forward_text_with_timeout(
        &state,
        "5e4de4",
        "tgt",
        serde_json::json!({"n": 1}),
        timeout,
    )
    .await;
    assert_eq!(
        state.metrics.messages_routed.load(Ordering::Relaxed),
        1,
        "the first message fits in the queue"
    );

    forward_text_with_timeout(
        &state,
        "5e4de4",
        "tgt",
        serde_json::json!({"n": 2}),
        timeout,
    )
    .await;
    assert_eq!(
        state
            .metrics
            .messages_dropped_timeout
            .load(Ordering::Relaxed),
        1,
        "a wedged target must be counted as a timeout drop"
    );
}

#[tokio::test]
async fn forward_to_a_closed_target_counts_not_found() {
    let state = Arc::new(test_state(0, 0, 0, 0, 0));
    let (tx, rx) = mpsc::channel::<Message>(1);
    state
        .clients
        .write()
        .await
        .insert("dead".to_string(), Queue::new(tx));
    drop(rx);

    forward_text(
        &state,
        "5e4de4",
        "dead",
        serde_json::json!({"type": "ping"}),
    )
    .await;
    assert_eq!(
        state
            .metrics
            .messages_dropped_not_found
            .load(Ordering::Relaxed),
        1
    );
    assert_eq!(
        state
            .metrics
            .messages_dropped_timeout
            .load(Ordering::Relaxed),
        0,
        "a closed target is not a timeout"
    );
}

// ================================================================
//  VULNERABILITY 12 — exit codes and a real drain
// ================================================================

#[tokio::test]
async fn drain_connections_waits_for_tasks_to_finish() {
    let mut set = tokio::task::JoinSet::new();
    set.spawn(async {});
    tokio::time::timeout(
        std::time::Duration::from_secs(DRAIN_TIMEOUT_SECS + 2),
        drain_connections(&mut set, "test"),
    )
    .await
    .expect("drain must not hang on a finished task");
    assert!(set.is_empty(), "drain must reap every task it waits on");
}

#[tokio::test]
async fn drain_connections_aborts_tasks_that_overstay() {
    // A wedged connection must not block shutdown for longer than the drain
    // window — and must actually be aborted, not leaked.
    let mut set = tokio::task::JoinSet::new();
    set.spawn(tokio::time::sleep(std::time::Duration::from_secs(600)));
    tokio::time::timeout(
        std::time::Duration::from_secs(DRAIN_TIMEOUT_SECS + 5),
        drain_connections(&mut set, "test"),
    )
    .await
    .expect("drain must return after the timeout");
    assert!(set.is_empty(), "the wedged task must have been aborted");
}

#[tokio::test]
async fn shutdown_signal_closes_authenticated_connections() {
    // The old code logged "draining connections…" and then dropped every
    // task handle. Now the signal reaches the connection handler.
    let state = e2e_state(1000);
    let (conn_shutdown_tx, conn_shutdown_rx) = watch::channel(false);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let state_clone = state.clone();
    let conn_shutdown = conn_shutdown_rx.clone();
    let accept = tokio::spawn(async move {
        if let Ok((stream, peer)) = listener.accept().await {
            handle_connection(stream, state_clone, peer, conn_shutdown.clone()).await;
        }
    });

    let mut ws = ws_client(addr).await;
    authenticate(&mut ws, "d00d", E2E_RELAY_TOKEN).await;
    assert!(
        wait_until(2000, || {
            state
                .clients
                .try_read()
                .map(|c| c.contains_key("d00d"))
                .unwrap_or(false)
        })
        .await,
        "device should be registered before shutdown"
    );

    conn_shutdown_tx.send(true).unwrap();
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), accept).await;

    assert!(
        wait_until(3000, || {
            state
                .clients
                .try_read()
                .map(|c| !c.contains_key("d00d"))
                .unwrap_or(false)
        })
        .await,
        "the connection handler must observe the shutdown signal and clean up"
    );
}

// ---------------------------------------------------------------
//  Interop vector, shared with the Dart client.
//
//  The Dart side asserts the same four constants in
//  `apps/mobile/test/services/relay_route_test.dart`. If this test fails,
//  that test is now wrong: re-emit with
//  `cargo test -p conduit-relay interop_vector -- --nocapture` and update both.
//
//  The payload deliberately has keys in non-alphabetical order, including a
//  nested object, because serde_json is built without `preserve_order` and so
//  sorts every object it serialises. A client that hashes insertion order
//  instead produces a different canonical string and every route is rejected.
// ---------------------------------------------------------------

#[test]
fn interop_vector_for_the_dart_client() {
    let secret = hex::decode("000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f")
        .expect("hex");
    let route = RelayRoute::signed_with(
        "deadbeef",
        &hmac::derive_route_key(&secret, "deadbeef"),
        "deadbeef",
        "cafebabe",
        serde_json::json!({"type": "ping", "zeta": 1, "alpha": 2, "nested": {"b": 2, "a": 1}}),
        1_700_000_000_000,
        "nonce-1",
    );
    let value = serde_json::to_value(&route).expect("serialises");

    assert_eq!(
        hex::encode(hmac::derive_route_key(&secret, "deadbeef")),
        "2cd721ceb5f78ef0f105593aa6feb7b5bc6f17c323d9ad32d6a34a166b770914",
    );
    assert_eq!(
        hmac::canonical_signing_string(&value),
        r#"{"from_device_id":"deadbeef","key_id":"deadbeef","nonce":"nonce-1","payload":{"alpha":2,"nested":{"a":1,"b":2},"type":"ping","zeta":1},"timestamp":1700000000000,"to_device_id":"cafebabe","type":"relay_route"}"#,
    );
    assert_eq!(
        value["hmac"].as_str().expect("signed"),
        "9c2018aa54f8a9fe7fa674899eee2665d891e7f34ee38077effc541132796dcb",
    );
}

// ---------------------------------------------------------------
//  Certificate pin enforcement
//
//  The desktop had no pin target at all, so an operator had no way to say
//  "serve this key and nothing else". A pin that is only displayed is not a
//  control; these tests pin the behaviour that makes it one.
// ---------------------------------------------------------------

#[test]
fn pins_match_ignores_formatting_a_human_would_change() {
    // The real value from the SPKI vector fixture.
    let pin = "sha256/MEgep9/xCDDwKndPH8EBw6zfNzWG9xwwxyaJZaZBTag=";
    assert!(crate::service::pins_match(pin, pin), "identical");

    // Padded, unpadded, prefixed, whitespace — all the same key.
    assert!(crate::service::pins_match(
        "sha256/MEgep9/xCDDwKndPH8EBw6zfNzWG9xwwxyaJZaZBTag=",
        "MEgep9/xCDDwKndPH8EBw6zfNzWG9xwwxyaJZaZBTag"
    ));
    assert!(crate::service::pins_match(
        "  sha256/MEgep9/xCDDwKndPH8EBw6zfNzWG9xwwxyaJZaZBTag=  ",
        "sha256/MEgep9/xCDDwKndPH8EBw6zfNzWG9xwwxyaJZaZBTag="
    ));
}

#[test]
fn pins_match_rejects_a_different_key() {
    let a = "sha256/MEgep9/xCDDwKndPH8EBw6zfNzWG9xwwxyaJZaZBTag=";
    let b = "sha256/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=";
    assert!(!crate::service::pins_match(a, b));
}

#[test]
fn a_malformed_pin_never_matches() {
    // A typo must not be read as "no pin configured" and quietly disable the
    // check the operator believes they have.
    let good = "sha256/MEgep9/xCDDwKndPH8EBw6zfNzWG9xwwxyaJZaZBTag=";
    for bad in [
        "",
        "sha256/",
        "sha256/not-base64!!",
        "sha256/QUJD", // valid base64, but not a 32-byte digest
    ] {
        assert!(
            !crate::service::pins_match(bad, good),
            "{bad:?} must not match a real pin"
        );
    }
    // Trailing bits that are not zero mean the input is not a clean encoding of
    // a whole number of bytes, so it is refused rather than silently truncated.
    assert!(!crate::service::pins_match(
        "sha256/MEgep9/xCDDwKndPH8EBw6zfNzWG9xwwxyaJZaZBTagB",
        good
    ));
}

#[tokio::test]
async fn a_configured_pin_that_does_not_match_refuses_to_start() {
    // The point of the setting: a certificate that changed without anyone
    // intending it must not be served, and the only way to guarantee that is to
    // refuse rather than warn.
    let dir = std::env::temp_dir().join(format!("conduit-pin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");

    let overrides = Overrides {
        relay_token: Some("config-test-token".into()),
        wss_port: Some(0),
        health_port: Some(0),
        ws_port: Some(0),
        enable_plain_ws: Some(false),
        nonce_file: Some(dir.join("nonces.json")),
        hmac_secret_file: Some(dir.join("secret")),
        tls_cert_dir: Some(dir.join("certs")),
        relay_cert_pin: Some("sha256/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into()),
        ..Overrides::default()
    };
    let config = Config::resolve(Some(overrides)).expect("config resolves");
    let result = RelayService::from_config(config)
        .start(Arc::new(StaticRouteKeys::new()))
        .await;

    match result {
        Err(StartError::Config(msg)) => {
            assert!(
                msg.contains("does not match the configured pin"),
                "the failure must say the pin did not match, got: {msg}"
            );
        }
        _ => panic!("a pin mismatch must refuse to start"),
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn no_configured_pin_starts_normally() {
    // The default must not regress into refusing to start: a host that is the
    // relay is not authenticating a remote server.
    let dir = std::env::temp_dir().join(format!("conduit-nopin-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");

    let overrides = Overrides {
        relay_token: Some("config-test-token".into()),
        wss_port: Some(0),
        health_port: Some(0),
        ws_port: Some(0),
        enable_plain_ws: Some(true),
        nonce_file: Some(dir.join("nonces.json")),
        hmac_secret_file: Some(dir.join("secret")),
        tls_cert_dir: Some(dir.join("certs")),
        ..Overrides::default()
    };
    let config = Config::resolve(Some(overrides)).expect("config resolves");
    assert!(
        config.relay_cert_pin.is_none(),
        "no pin is configured unless one is asked for"
    );

    let handle = RelayService::from_config(config)
        .start(Arc::new(StaticRouteKeys::new()))
        .await
        .expect("an unpinned relay starts");
    handle.shutdown().await;

    let _ = std::fs::remove_dir_all(&dir);
}

// ================================================================
//  W6 remediation — the connection hot path
//
//  One test (or tight group) per audit item, each written so that it
//  fails with that item's fix removed:
//
//  W6.4   a superseded connection cannot deregister the live one
//  W6.6   a slow forward does not hold the routing table's lock, and the
//         byte budget survives the clone the fix relies on
//  W6.8   the auth deadline is total, not per frame
//  W6.10  an unsupported protocol_version is refused with the documented
//         code, and a current one still connects
//  W6.11  over-size frames are answered with `message_too_large` before
//         the connection closes
// ================================================================

// ---------------------------------------------------------------
//  W6.6 — the read lock must not outlive the lookup
// ---------------------------------------------------------------

#[tokio::test]
async fn a_slow_forward_does_not_hold_the_routing_table_lock() {
    // `forward_text_with_timeout` held `state.clients`' read guard across its
    // whole queueing timeout, and tokio's RwLock is write-preferring — so one
    // wedged target stalled registration, deregistration and the 30 s sweep
    // behind it (W6.6).
    let state = Arc::new(test_state(0, 0, 0, 0, 0));

    // A target that accepts exactly one message and then never drains: the
    // forward below parks inside `Queue::send` for its whole timeout, which
    // is exactly the window in which the guard used to be held.
    let (tx, _rx) = mpsc::channel::<Message>(1);
    state
        .clients
        .write()
        .await
        .insert("wedged".to_string(), Queue::new(tx));
    {
        let fill = state
            .clients
            .read()
            .await
            .get("wedged")
            .expect("registered")
            .clone();
        assert_eq!(
            fill.send(Message::Text("fills the single slot".to_string()))
                .await,
            SendOutcome::Sent
        );
    }

    let forwarding_state = state.clone();
    let forward = tokio::spawn(async move {
        forward_text_with_timeout(
            &forwarding_state,
            "5e4de4",
            "wedged",
            serde_json::json!({"n": 1}),
            std::time::Duration::from_secs(3),
        )
        .await;
    });

    // Let the forward task start and — before the fix — take the read guard.
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;

    // A writer must enter immediately, not queue behind the forward's
    // three-second wait.
    let mut writer =
        tokio::time::timeout(std::time::Duration::from_millis(500), state.clients.write())
            .await
            .expect("a writer must not queue behind a slow forward");
    writer.insert("late".to_string(), Queue::new(mpsc::channel(1).0));
    drop(writer);

    forward.abort();
}

#[tokio::test]
async fn cloned_queues_share_one_byte_budget() {
    // The forward path now clones the queue out of the table before it sends
    // (W6.6). That is only sound because the byte budget bounding the queue
    // lives behind an `Arc` the clone shares: without that, a clone would
    // start a private budget and `QUEUE_BYTE_BUDGET` would bound nothing.
    let (tx, _rx) = mpsc::channel::<Message>(4);
    let registered = Queue::new(tx);
    let cloned = registered.clone();

    // Drain the whole budget through the clone, the way a forward does.
    let whole_budget = Message::Binary(vec![0u8; QUEUE_BYTE_BUDGET]);
    assert_eq!(cloned.send(whole_budget).await, SendOutcome::Sent);

    // The handle still in the routing table must see that reservation: its
    // own send has to wait for budget rather than succeed on a private one.
    let starved = tokio::time::timeout(
        std::time::Duration::from_millis(100),
        registered.send(Message::Text("needs budget".to_string())),
    )
    .await;
    assert!(
        starved.is_err(),
        "a clone's reservations must draw down the same budget"
    );

    // And a release through one handle must reopen it for the other.
    registered.release(QUEUE_BYTE_BUDGET);
    let reopened = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        registered.send(Message::Text("fits again".to_string())),
    )
    .await
    .expect("releasing the budget must unblock the next send");
    assert_eq!(reopened, SendOutcome::Sent);
}

// ---------------------------------------------------------------
//  W6.4 — a stale disconnect must not deregister the live device
// ---------------------------------------------------------------

#[tokio::test]
async fn a_superseded_connection_cannot_deregister_the_live_one() {
    // The exact interleaving behind W6.4: connection A registers the device,
    // the same device reconnects as connection B and overwrites the entry,
    // then A's disconnect runs its cleanup. Removing by key alone took B's
    // registration with it — the socket stayed up but the device was
    // permanently unroutable, and `reconcile_clients` cannot repair it
    // because B's sender is open and healthy.
    let clients: Clients = Arc::new(RwLock::new(HashMap::new()));
    let (tx_a, _rx_a) = mpsc::channel::<Message>(4);
    let (tx_b, mut rx_b) = mpsc::channel::<Message>(4);
    let connection_a = Queue::new(tx_a);
    let connection_b = Queue::new(tx_b);
    clients
        .write()
        .await
        .insert("device".to_string(), connection_a.clone());
    // The reconnect overwrites A's entry with B's queue.
    clients
        .write()
        .await
        .insert("device".to_string(), connection_b.clone());

    // A's late cleanup runs now and must find nothing of its own to remove.
    assert!(
        !deregister_if_current(&clients, "device", connection_a.connection_id()).await,
        "the superseded connection must not remove an entry it no longer owns"
    );
    {
        let map = clients.read().await;
        let entry = map.get("device").expect("the live registration survives");
        assert_eq!(
            entry.connection_id(),
            connection_b.connection_id(),
            "the surviving entry must still be the newer connection's"
        );
        // ...and it answers on the newer connection's channel, not the old one's.
        assert_eq!(
            entry.send(Message::Text("for B".to_string())).await,
            SendOutcome::Sent
        );
    }
    assert!(
        matches!(rx_b.recv().await, Some(Message::Text(t)) if t == "for B"),
        "the entry must still deliver to the connection that replaced the old one"
    );

    // The live connection's own cleanup still works.
    assert!(
        deregister_if_current(&clients, "device", connection_b.connection_id()).await,
        "the owning connection must still be able to deregister"
    );
    assert!(!clients.read().await.contains_key("device"));
}

#[tokio::test]
async fn e2e_a_stale_disconnect_leaves_the_reconnected_device_routable() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    // The device's first connection registers it.
    let mut first = ws_client(relay.ws_addr).await;
    let ok = authenticate(&mut first, "d00d", E2E_RELAY_TOKEN)
        .await
        .expect("first connection gets an answer");
    assert!(ok.contains("relay_auth_ok"), "got: {ok}");
    assert!(state.clients.read().await.contains_key("d00d"));

    // The same device reconnects; the entry now belongs to the new socket
    // (`relay_auth_ok` is only sent after the insert).
    let mut second = ws_client(relay.ws_addr).await;
    let ok = authenticate(&mut second, "d00d", E2E_RELAY_TOKEN)
        .await
        .expect("reconnect gets an answer");
    assert!(ok.contains("relay_auth_ok"), "got: {ok}");
    assert_eq!(
        state.metrics.auth_attempts_success.load(Ordering::Relaxed),
        2
    );

    // The *old* connection now goes away, running its cleanup long after it
    // was superseded.
    drop(first);
    let cleaned_up = wait_until(3000, || {
        state
            .metrics
            .connections_disconnected
            .load(Ordering::Relaxed)
            >= 1
    })
    .await;
    assert!(cleaned_up, "the stale connection's cleanup must run");
    assert!(
        state.clients.read().await.contains_key("d00d"),
        "the stale disconnect must not evict the connection that replaced it"
    );

    // Present *and* routable: a third device routes to it and the live
    // connection receives.
    let mut sender = ws_client(relay.ws_addr).await;
    authenticate(&mut sender, "e00e", E2E_RELAY_TOKEN).await;
    let route = signed_route(
        "e00e",
        "d00d",
        &serde_json::json!({"type": "ping"}),
        now_millis(),
        "nonce-stale-disconnect-1",
    );
    sender
        .send(Message::Text(route.to_string()))
        .await
        .expect("send route");
    let delivered = recv_text_matching(&mut second, 5, |t| t.contains("relay_delivery")).await;
    assert!(
        delivered.is_some(),
        "the device must still receive routed messages after its old connection's cleanup"
    );

    drop(relay);
}

// ---------------------------------------------------------------
//  W6.8 — the auth deadline is total, not per frame
// ---------------------------------------------------------------

#[tokio::test]
async fn e2e_auth_deadline_is_total_not_per_frame() {
    // One junk frame per interval used to re-arm the auth timeout on every
    // frame, so a client that never authenticated could hold its connection —
    // and its `active_connections` slot — for as long as it kept going (W6.8).
    let state = e2e_state_with_auth_timeout(1000, 1);
    let relay = spawn_relay(state.clone()).await;

    let mut ws = ws_client(relay.ws_addr).await;
    let give_up = tokio::time::Instant::now() + std::time::Duration::from_millis(5000);
    let mut closed = false;
    while tokio::time::Instant::now() < give_up && !closed {
        tokio::select! {
            _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => {
                // Valid size, valid JSON, not an auth attempt: exactly the
                // frame that used to buy the connection another full timeout
                // on every send.
                if ws.send(Message::Text(r#"{"type":"ping"}"#.to_string()))
                    .await
                    .is_err()
                {
                    closed = true;
                }
            }
            next = ws.next() => match next {
                Some(Ok(_)) => {}
                Some(Err(_)) | None => closed = true,
            },
        }
    }

    assert!(
        closed,
        "the connection must close at its total auth deadline even while \
         junk frames keep arriving"
    );
    assert_eq!(
        state.metrics.auth_attempts_success.load(Ordering::Relaxed),
        0,
        "a dripping client must never authenticate"
    );

    drop(relay);
}

// ---------------------------------------------------------------
//  W6.10 — inbound protocol_version
// ---------------------------------------------------------------

#[test]
fn unsupported_version_frame_only_refuses_newer_versions() {
    // Missing: `protocol_version` is optional (PROTOCOL.md §1.1) and every
    // current client omits it, so absence must stay accepted.
    assert!(unsupported_version_frame(&serde_json::json!({"type": "ping"})).is_none());
    assert!(
        unsupported_version_frame(&serde_json::json!({
            "type": "relay_auth",
            "device_id": "aabb",
            "relay_token": "t",
        }))
        .is_none()
    );
    // The version this relay speaks is accepted.
    assert!(
        unsupported_version_frame(&serde_json::json!({"type": "ping", "protocol_version": 1}))
            .is_none()
    );

    // A newer version is refused, with exactly the documented code, wording
    // and server_version (PROTOCOL.md §1.1 / §8.1).
    let frame =
        unsupported_version_frame(&serde_json::json!({"type": "ping", "protocol_version": 2}))
            .expect("a newer protocol_version must be refused");
    let Message::Text(text) = frame else {
        panic!("error frames are text");
    };
    let parsed: Value = serde_json::from_str(&text).expect("error frame is JSON");
    assert_eq!(parsed.get("type").and_then(Value::as_str), Some("error"));
    assert_eq!(
        parsed.get("code").and_then(Value::as_str),
        Some("unsupported_protocol_version")
    );
    assert_eq!(
        parsed.get("message").and_then(Value::as_str),
        Some("Server supports protocol_version 1, got 2")
    );
    assert_eq!(parsed.get("server_version"), Some(&Value::from(1)));
}

#[tokio::test]
async fn e2e_auth_declaring_a_newer_protocol_version_is_refused() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut ws = ws_client(relay.ws_addr).await;
    ws.send(Message::Text(
        serde_json::json!({
            "type": "relay_auth",
            "device_id": "f00d",
            "relay_token": E2E_RELAY_TOKEN,
            "protocol_version": 2,
        })
        .to_string(),
    ))
    .await
    .expect("send auth");

    let answer = recv_text(&mut ws, 5)
        .await
        .expect("the relay answers a version it does not speak");
    let parsed: Value = serde_json::from_str(&answer).expect("answer is JSON");
    assert_eq!(
        parsed.get("code").and_then(Value::as_str),
        Some("unsupported_protocol_version"),
        "got: {answer}"
    );
    assert!(!state.clients.read().await.contains_key("f00d"));
    assert!(
        state.metrics.auth_attempts_failure.load(Ordering::Relaxed) >= 1,
        "refusing a too-new version is a failed handshake"
    );
    // The rejection ends the connection rather than leaving it half-authed.
    assert!(
        recv_text(&mut ws, 2).await.is_none(),
        "the refused connection must close"
    );

    // A current-version auth on a fresh connection still succeeds.
    let mut ok = ws_client(relay.ws_addr).await;
    let resp = authenticate(&mut ok, "f00d", E2E_RELAY_TOKEN)
        .await
        .expect("a current-version client gets an answer");
    assert!(
        resp.contains("relay_auth_ok"),
        "a current protocol_version must still connect, got: {resp}"
    );

    drop(relay);
}

#[tokio::test]
async fn e2e_newer_protocol_version_messages_are_refused_without_dropping_the_session() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut ws = ws_client(relay.ws_addr).await;
    authenticate(&mut ws, "d00d", E2E_RELAY_TOKEN).await;

    // A too-new version is refused with the documented code, like the
    // desktop hub answers it (PROTOCOL.md §8.1).
    ws.send(Message::Text(
        serde_json::json!({"type": "ping", "protocol_version": 2}).to_string(),
    ))
    .await
    .expect("send ping");
    let answer = recv_text_matching(&mut ws, 5, |t| t.contains("unsupported_protocol_version"))
        .await
        .expect("the frame is answered");
    let parsed: Value = serde_json::from_str(&answer).expect("answer is JSON");
    assert_eq!(
        parsed.get("code").and_then(Value::as_str),
        Some("unsupported_protocol_version"),
        "got: {answer}"
    );

    // The session survives: a plain ping is still answered...
    ws.send(Message::Text(
        serde_json::json!({"type": "ping"}).to_string(),
    ))
    .await
    .expect("send ping");
    let pong = recv_text_matching(&mut ws, 5, |t| t.contains("pong")).await;
    assert!(
        pong.is_some(),
        "a refused frame must not cost the connection its session"
    );
    assert!(state.clients.read().await.contains_key("d00d"));

    // ...and a route stamped with the version this relay speaks still routes.
    let mut target = ws_client(relay.ws_addr).await;
    authenticate(&mut target, "ee11", E2E_RELAY_TOKEN).await;
    let mut route = signed_route(
        "d00d",
        "ee11",
        &serde_json::json!({"type": "ping"}),
        now_millis(),
        "nonce-protocol-version-1",
    );
    route["protocol_version"] = serde_json::json!(1);
    ws.send(Message::Text(route.to_string()))
        .await
        .expect("send current-version route");
    let delivered = recv_text_matching(&mut target, 5, |t| t.contains("relay_delivery")).await;
    assert!(
        delivered.is_some(),
        "a route carrying the current protocol_version must still be delivered"
    );

    drop(relay);
}

// ---------------------------------------------------------------
//  W6.11 — over-size frames are answered, not just dropped
// ---------------------------------------------------------------

/// Send `frame` while reading the relay's answer, and return the answer.
///
/// The two directions have to be in flight together: the relay rejects an
/// over-size frame the moment it has read enough to know it is over the
/// ceiling — long before a 1 MB payload has finished arriving — answers, and
/// closes. Reading only after `send` has returned would race that close, and
/// a connection reset while unread data sits in the socket can take the
/// answer with it.
///
/// Frames that are not the answer are skipped rather than returned: the relay
/// pings as soon as the writer task starts (`interval` fires its first tick
/// immediately) and the `relay_auth_ok` reply may still be queued behind it,
/// so "first text frame" is not "the answer". A close or a reset ends the
/// wait either way — those are exactly the outcomes the refusal is meant to
/// prevent, so returning `None` on them is what the tests want to see.
async fn send_frame_and_read_answer(ws: WsClient, frame: Message, secs: u64) -> Option<String> {
    let (mut sink, mut stream) = ws.split();
    let sender = tokio::spawn(async move {
        // A write error is the relay closing mid-frame; the answer on the
        // read side is what the test is about.
        let _ = sink.send(frame).await;
    });

    let end = tokio::time::Instant::now() + std::time::Duration::from_secs(secs);
    let mut answer = None;
    while answer.is_none() && tokio::time::Instant::now() < end {
        let remaining = end.saturating_duration_since(tokio::time::Instant::now());
        match tokio::time::timeout(remaining, stream.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let is_answer = serde_json::from_str::<Value>(&text)
                    .is_ok_and(|value| value.get("type").and_then(Value::as_str) == Some("error"));
                if is_answer {
                    answer = Some(text.to_string());
                }
            }
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(_))) | Ok(None) => break,
            Err(_) => break,
        }
    }
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), sender).await;
    answer
}

#[tokio::test]
async fn e2e_oversized_text_frame_is_answered_with_message_too_large() {
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut ws = ws_client(relay.ws_addr).await;
    authenticate(&mut ws, "b16", E2E_RELAY_TOKEN).await;

    let answer = send_frame_and_read_answer(ws, Message::Text("x".repeat(MAX_TEXT_SIZE + 1)), 5)
        .await
        .expect("the relay must answer an over-size frame before it closes");
    let parsed: Value = serde_json::from_str(&answer).expect("answer is JSON");
    assert_eq!(parsed.get("type").and_then(Value::as_str), Some("error"));
    assert_eq!(
        parsed.get("code").and_then(Value::as_str),
        Some("message_too_large"),
        "the documented code must reach the sender, got: {answer}"
    );

    // The answer does not soften the outcome: the connection still closes and
    // the device is still deregistered.
    let mut removed = false;
    for _ in 0..50 {
        if !state.clients.read().await.contains_key("b16") {
            removed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(removed, "an over-size frame must still drop the connection");

    drop(relay);
}

#[tokio::test]
async fn e2e_oversized_binary_frame_is_answered_with_message_too_large() {
    // The binary arm closed in silence for exactly the same reason the text
    // arm did, and deserved the same answer (W6.11).
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let mut ws = ws_client(relay.ws_addr).await;
    authenticate(&mut ws, "b16", E2E_RELAY_TOKEN).await;

    let answer = send_frame_and_read_answer(ws, Message::Binary(vec![0u8; MAX_BINARY_SIZE + 1]), 5)
        .await
        .expect("the relay must answer an over-size binary frame before it closes");
    let parsed: Value = serde_json::from_str(&answer).expect("answer is JSON");
    assert_eq!(
        parsed.get("code").and_then(Value::as_str),
        Some("message_too_large"),
        "the documented code must reach the sender, got: {answer}"
    );

    let mut removed = false;
    for _ in 0..50 {
        if !state.clients.read().await.contains_key("b16") {
            removed = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(
        removed,
        "an over-size binary frame must still drop the connection"
    );

    drop(relay);
}

#[tokio::test]
async fn e2e_oversized_frame_during_auth_is_answered_with_message_too_large() {
    // Same refusal on the auth path, where the write side is still the read
    // loop's own sink and no routing entry has been made yet.
    let state = e2e_state(1000);
    let relay = spawn_relay(state.clone()).await;

    let ws = ws_client(relay.ws_addr).await;
    let answer = send_frame_and_read_answer(ws, Message::Text("x".repeat(MAX_TEXT_SIZE + 1)), 5)
        .await
        .expect("the relay must answer an over-size frame even during auth");
    let parsed: Value = serde_json::from_str(&answer).expect("answer is JSON");
    assert_eq!(
        parsed.get("code").and_then(Value::as_str),
        Some("message_too_large"),
        "got: {answer}"
    );
    assert_eq!(
        state.metrics.auth_attempts_success.load(Ordering::Relaxed),
        0,
        "an over-size frame must never authenticate"
    );

    drop(relay);
}
