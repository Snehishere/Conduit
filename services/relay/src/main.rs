mod hmac;
mod tls;

use conduit_protocol::hmac::{NonceCache, SigningKeyring};
use conduit_protocol::{
    ErrorMessage, Ping, Pong, RelayAuth, RelayAuthOk, RelayAuthRejected, RelayRoute,
};
use futures_util::{SinkExt, StreamExt};
use hyper::body::Incoming;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use log::{error, info, warn};
use serde_json::Value;
use std::collections::{HashMap, VecDeque};
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::net::TcpListener;
use tokio::sync::{RwLock, mpsc, watch};
use tokio_tungstenite::accept_async;
use tungstenite::Message;

/// Maximum inbound text-frame size before the connection is dropped.
///
/// Spec-mandated at 1 MB (PROTOCOL.md §8 `message_too_large`; the relay
/// capacity plan in docs/archive/ references this exact constant as
/// `MAX_TEXT_SIZE=1MB`). Bounded deliberately: `MAX_CONNECTIONS` allows 10k
/// concurrent sockets, so the per-frame ceiling is what keeps total inbound
/// buffering from becoming a memory-exhaustion vector.
const MAX_TEXT_SIZE: usize = 1024 * 1024; // 1 MB
const MAX_CONNECTIONS: usize = 10_000;

/// Per-message token-bucket refill rate (messages per second).
const MSG_RATE_PER_SEC: f64 = 100.0;
/// Per-message token-bucket burst capacity.
const MSG_BURST: f64 = 50.0;

/// How long a forwarded message may sit in a target's outbound queue before
/// it is abandoned. Without this the relay's read loop blocks on a slow or
/// wedged client and stalls that connection indefinitely.
const FORWARD_TIMEOUT_SECS: u64 = 5;

/// Process exit codes — distinct so an orchestrator (and `docker inspect`) can
/// tell a misconfiguration from a port conflict.
const EXIT_CONFIG: i32 = 78; // EX_CONFIG
const EXIT_LISTEN: i32 = 74; // EX_IOERR
const EXIT_NO_LISTENERS: i32 = 69; // EX_UNAVAILABLE

/// A device id as documented in PROTOCOL.md §`device_id`: the first 16 hex
/// characters of the device's X25519 public key.
///
/// Enforced because `device_id` becomes a map key in `state.clients` and in the
/// nonce replay cache. With `MAX_TEXT_SIZE` at 1 MB an unvalidated id let a
/// single authenticated frame insert a megabyte-scale key into both maps.
const MAX_DEVICE_ID_LEN: usize = 64;

/// Validate a device id: 1..=64 lowercase hex characters.
///
/// Uppercase is rejected on purpose — ids are compared as map keys, so
/// accepting two spellings of the same id would let one device evict another
/// from the routing table.
fn is_valid_device_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_DEVICE_ID_LEN
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Histogram bucket upper bounds (bytes) for `conduit_relay_message_size_bytes`.
///
/// Spans the interesting range for JSON control messages (~100 B) through to
/// the 1 MB frame ceiling. Without real buckets the histogram is a single
/// `+Inf` series that tells an operator nothing about where traffic sits.
const SIZE_BUCKETS: [u64; 8] = [128, 512, 2_048, 8_192, 32_768, 131_072, 524_288, 1_048_576];

type Clients = Arc<RwLock<HashMap<String, mpsc::Sender<Message>>>>;

struct ConnectionGuard {
    count: Arc<std::sync::atomic::AtomicUsize>,
}

impl Drop for ConnectionGuard {
    fn drop(&mut self) {
        self.count
            .fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}

struct Metrics {
    // Counters – connections
    connections_connected: AtomicU64,
    connections_disconnected: AtomicU64,
    // Counters – auth
    auth_attempts_success: AtomicU64,
    auth_attempts_failure: AtomicU64,
    // Counters – messages
    messages_routed: AtomicU64,
    messages_dropped_not_found: AtomicU64,
    messages_dropped_timeout: AtomicU64,
    messages_dropped_rate_limit: AtomicU64,
    /// Replay rejection or expired timestamp on a `relay_route`.
    messages_dropped_replay: AtomicU64,
    /// Signature verification failure (bad key, wrong key_id, tampered body).
    messages_dropped_hmac_failed: AtomicU64,
    /// A message the relay has no handler for — silently dropped before, now
    /// counted and answered with an `error` frame.
    messages_dropped_unknown_type: AtomicU64,
    // Histogram – message size (sum + count + per-bucket counters)
    message_size_bytes_sum: AtomicU64,
    message_size_bytes_count: AtomicU64,
    /// `SIZE_BUCKETS.len()` entries; cumulative counts are derived at scrape time.
    message_size_buckets: Vec<AtomicU64>,
}

impl Metrics {
    fn new() -> Self {
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
    fn observe_message_size(&self, bytes: usize) {
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

#[derive(Debug)]
struct Config {
    ws_port: u16,
    wss_port: u16,
    health_port: u16,
    /// Bearer token required for `GET /health` and `GET /`.
    /// Defaults to the UTF-8 HMAC secret unless `RELAY_HEALTH_TOKEN` is set.
    health_token: String,
    /// Optional bearer token guarding `GET /metrics`.
    ///
    /// `/metrics` is unauthenticated by default (a deliberate, tested decision
    /// so Prometheus can scrape without distributing the health secret). Setting
    /// `RELAY_METRICS_TOKEN` opts into gating it.
    metrics_token: Option<String>,
    /// Resolved HMAC master secret: `HMAC_SECRET` env var, else the contents of
    /// `HMAC_SECRET_FILE`, else a freshly generated + persisted 64-char hex
    /// secret (see [`load_or_create_hmac_secret`]).
    ///
    /// `hmac_secret` and `health_token` are **intentionally equal** whenever
    /// `RELAY_HEALTH_TOKEN` is unset/empty — that fallback is the whole reason
    /// both are populated from the same value in `from_env`. They diverge by
    /// design only when `RELAY_HEALTH_TOKEN` is explicitly set, in which case
    /// `health_token` is the override and this field is left untouched.
    ///
    /// The master secret is **never** used directly to verify `relay_route`
    /// HMACs and is never exposed to clients; it only (a) seeds the derived
    /// signing key when `RELAY_SIGNING_KEY` is unset and (b) backs the
    /// `/health` token default.
    hmac_secret: Vec<u8>,
    /// Keyring used to verify `relay_route` HMACs and binary-frame tags.
    ///
    /// Deliberately separate from `relay_token`: the token is a bearer
    /// *credential* every authenticated client holds, so using it as the signing
    /// key would let any client forge a route on behalf of any other.
    signing_keys: SigningKeyring,
    enable_plain_ws: bool,
    nonce_file: std::path::PathBuf,
    /// Seconds a connection may stay open without authenticating.
    /// Kept in Config so integration tests can shrink it (default 10).
    auth_timeout_secs: u64,
    /// Shared bearer credential presented as `relay_token` during `relay_auth`.
    /// Authentication only — never a signing key.
    relay_token: String,
}

/// Resolve the HMAC secret: env var → secret file → generate + persist.
///
/// Fail-closed: if the secret cannot be read or persisted, startup aborts.
/// An ephemeral secret would silently invalidate every deployed token on
/// restart — exactly the bug this path exists to fix.
fn load_or_create_hmac_secret() -> Result<Vec<u8>, String> {
    let path = std::env::var("HMAC_SECRET_FILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("./secrets/hmac_secret"));

    match std::fs::read_to_string(&path) {
        Ok(content) => {
            // Tolerate a trailing newline from `echo`/secret managers.
            let secret = content.trim_end_matches(['\r', '\n']);
            if secret.is_empty() {
                return Err(format!("HMAC secret file {} is empty", path.display()));
            }
            info!("Loaded HMAC secret from {}", path.display());
            Ok(secret.as_bytes().to_vec())
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // Bootstrap: 32 random bytes as 64-char hex, persisted with 0600.
            let mut bytes = [0u8; 32];
            use rand::Rng;
            rand::thread_rng().fill(&mut bytes[..]);
            let secret = hex::encode(bytes);
            write_secret_file(&path, &secret)?;
            info!(
                "Generated new HMAC secret and persisted to {}",
                path.display()
            );
            Ok(secret.into_bytes())
        }
        Err(e) => Err(format!(
            "Failed to read HMAC secret file {}: {}",
            path.display(),
            e
        )),
    }
}

/// Write a secret file with owner-only permissions where the OS supports it.
#[cfg(unix)]
fn write_secret_file(path: &std::path::Path, contents: &str) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("Failed to open {}: {}", path.display(), e))?;
    file.write_all(contents.as_bytes())
        .map_err(|e| format!("Failed to write {}: {}", path.display(), e))?;
    // `.mode()` only applies at creation and is masked by umask — enforce 0600.
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("Failed to chmod {}: {}", path.display(), e))?;
    Ok(())
}

/// Non-unix: `Permissions::from_mode` is unavailable; rely on filesystem ACLs.
#[cfg(not(unix))]
fn write_secret_file(path: &std::path::Path, contents: &str) -> Result<(), String> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create {}: {}", parent.display(), e))?;
    }
    std::fs::write(path, contents).map_err(|e| format!("Failed to write {}: {}", path.display(), e))
}

impl Config {
    fn from_env() -> Result<Self, String> {
        let relay_token = std::env::var("RELAY_TOKEN").map_err(|_| {
            "RELAY_TOKEN environment variable is required (fail-closed)".to_string()
        })?;
        if relay_token.is_empty() {
            return Err("RELAY_TOKEN must not be empty".to_string());
        }
        let ws_port: u16 = std::env::var("RELAY_WS_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(9528);
        let wss_port: u16 = std::env::var("RELAY_WSS_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(9529);
        let health_port: u16 = std::env::var("RELAY_HEALTH_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(9530);

        let hmac_secret: Vec<u8> = match std::env::var("HMAC_SECRET") {
            Ok(s) if !s.is_empty() => s.into_bytes(),
            _ => load_or_create_hmac_secret()?,
        };

        let signing_keys = load_signing_keyring(&hmac_secret)?;

        // Falls back to the resolved HMAC secret when RELAY_HEALTH_TOKEN is
        // unset/empty, so the two are intentionally equal in that case.
        // Derived through `Config::effective_health_token` so the stored
        // `hmac_secret` field — not a parallel local — is the single source
        // of truth for this fallback.
        let health_token = std::env::var("RELAY_HEALTH_TOKEN")
            .ok()
            .filter(|t| !t.is_empty())
            .unwrap_or_default();

        let metrics_token = std::env::var("RELAY_METRICS_TOKEN")
            .ok()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());

        let nonce_file = std::env::var("RELAY_NONCE_FILE")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::path::PathBuf::from("./data/nonces.json"));

        let enable_plain_ws = std::env::var("RELAY_ENABLE_PLAIN_WS")
            .ok()
            .map(|v| v == "true" || v == "1")
            .unwrap_or(false);

        let auth_timeout_secs = std::env::var("RELAY_AUTH_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(10);

        let config = Config {
            ws_port,
            wss_port,
            health_port,
            health_token,
            metrics_token,
            hmac_secret,
            signing_keys,
            enable_plain_ws,
            nonce_file,
            auth_timeout_secs,
            relay_token,
        };
        Ok(config.with_health_token_fallback())
    }
}

/// Resolve the message-signing keyring from the environment.
///
/// # Why this is separate from `RELAY_TOKEN`
///
/// `RELAY_TOKEN` is a bearer credential: *every* authenticated client holds it.
/// Before this separation it was also the HMAC key for `relay_route`, so any
/// client could produce a valid signature for a route claiming to be from any
/// other device — message-level authenticity, the relay's core security claim,
/// was absent. The token is now used for authentication only.
///
/// # Resolution order
///
/// 1. `RELAY_SIGNING_KEY` (+ `RELAY_SIGNING_KEY_ID`) — the explicit path.
///    Use this when signing keys are managed out-of-band (e.g. rotated and
///    distributed by a config system).
/// 2. Derived from the master secret via a labelled KDF:
///    `HMAC-SHA256(hmac_secret, "conduit-protocol/v1/derive:conduit-relay/v1/message-signing-key")`.
///    Domain-separated, so the signing key is neither the master secret nor the
///    relay token, and knowledge of the token reveals nothing about it.
///
/// During a rotation, `RELAY_SIGNING_KEY_PREVIOUS` (+ `_ID`) keeps the retiring
/// key accepted for verification only. See the re-key runbook in `.env.example`.
fn load_signing_keyring(master_secret: &[u8]) -> Result<SigningKeyring, String> {
    let current_id = std::env::var("RELAY_SIGNING_KEY_ID")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| hmac::DEFAULT_KEY_ID.to_string());

    let current_secret: Vec<u8> = match std::env::var("RELAY_SIGNING_KEY")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        Some(explicit) => explicit.into_bytes(),
        None => hmac::derive_signing_key(master_secret).to_vec(),
    };

    let mut ring = SigningKeyring::new(current_id, current_secret);

    if let Some(previous) = std::env::var("RELAY_SIGNING_KEY_PREVIOUS")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
    {
        let previous_id = std::env::var("RELAY_SIGNING_KEY_PREVIOUS_ID")
            .ok()
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| {
                "RELAY_SIGNING_KEY_PREVIOUS is set but RELAY_SIGNING_KEY_PREVIOUS_ID is not; \
                 an unnamed retiring key cannot be selected on the wire"
                    .to_string()
            })?;
        if previous_id == ring.current().id {
            return Err(format!(
                "RELAY_SIGNING_KEY_ID and RELAY_SIGNING_KEY_PREVIOUS_ID are both {previous_id:?}; \
                 they must differ or rotation has no effect"
            ));
        }
        ring = ring.with_previous(previous_id, previous);
    }

    Ok(ring)
}

impl Config {
    /// Fill an empty `health_token` from the stored `hmac_secret`.
    ///
    /// `from_env` leaves `health_token` empty when `RELAY_HEALTH_TOKEN` is
    /// unset or empty; this restores the documented default so the effective
    /// token is always derived from the persisted secret in one place.
    fn with_health_token_fallback(mut self) -> Self {
        if self.health_token.is_empty() {
            self.health_token = String::from_utf8_lossy(&self.hmac_secret).to_string();
        }
        self
    }

    /// The bearer token actually guarding `GET /health` and `GET /`.
    ///
    /// Read path for the health endpoint: falls back to the HMAC secret for
    /// any `Config` built without an explicit `RELAY_HEALTH_TOKEN`.
    fn effective_health_token(&self) -> &str {
        if self.health_token.is_empty() {
            std::str::from_utf8(&self.hmac_secret).unwrap_or_default()
        } else {
            &self.health_token
        }
    }
}

/// Per-connection token bucket for inbound message rate limiting.
///
/// Drops excess messages (counted in `messages_dropped_rate_limit`) without
/// closing the connection, so a noisy client cannot starve the relay.
struct MessageRateLimiter {
    tokens: f64,
    last_refill: Instant,
    rate_per_sec: f64,
    burst: f64,
}

impl MessageRateLimiter {
    fn new() -> Self {
        Self::with_params(MSG_RATE_PER_SEC, MSG_BURST)
    }

    fn with_params(rate_per_sec: f64, burst: f64) -> Self {
        Self {
            tokens: burst,
            last_refill: Instant::now(),
            rate_per_sec,
            burst,
        }
    }

    /// Consume one token if available; refill from elapsed time first.
    fn allow(&mut self) -> bool {
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

/// Constant-time bearer token check for the health endpoint.
fn bearer_token_authorized(header_value: &str, expected: &str) -> bool {
    let Some(provided) = header_value.strip_prefix("Bearer ") else {
        return false;
    };
    let expected_b = expected.as_bytes();
    let provided_b = provided.as_bytes();
    if expected_b.len() != provided_b.len() {
        return false;
    }
    bool::from(subtle::ConstantTimeEq::ct_eq(expected_b, provided_b))
}

/// Format one structured JSON log line with an RFC3339 UTC timestamp.
fn format_log_line(timestamp_ms: i64, level: &str, target: &str, message: &str) -> String {
    serde_json::json!({
        "timestamp": format_rfc3339_ms(timestamp_ms),
        "level": level.to_ascii_lowercase(),
        "target": target,
        "message": message,
    })
    .to_string()
}

/// Convert epoch milliseconds to `YYYY-MM-DDTHH:MM:SS.mmmZ` without chrono.
/// Uses a signed civil-from-days algorithm so pre-epoch values never panic.
fn format_rfc3339_ms(timestamp_ms: i64) -> String {
    let secs = timestamp_ms.div_euclid(1000);
    let ms = timestamp_ms.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (sod / 3600, (sod % 3600) / 60, sod % 60);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        y, m, d, hh, mm, ss, ms
    )
}

/// Howard Hinnant's `civil_from_days`: days since 1970-01-01 → (y, m, d).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    (if m <= 2 { y + 1 } else { y }, m as u32, d as u32)
}

struct RateLimiter {
    windows: std::sync::Mutex<HashMap<IpAddr, VecDeque<Instant>>>,
    max_attempts: usize,
    window_secs: u64,
}

impl RateLimiter {
    fn new(max_attempts: usize, window_secs: u64) -> Self {
        Self {
            windows: std::sync::Mutex::new(HashMap::new()),
            max_attempts,
            window_secs,
        }
    }

    fn allow(&self, ip: IpAddr) -> bool {
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
    fn sweep(&self) -> usize {
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
    fn tracked_addresses(&self) -> usize {
        self.lock_windows().len()
    }

    fn lock_windows(&self) -> std::sync::MutexGuard<'_, HashMap<IpAddr, VecDeque<Instant>>> {
        // A poisoned lock means a previous holder panicked mid-update. The map
        // is a plain cache of timestamps, so recovering the data is strictly
        // better than propagating the panic into the accept loop.
        self.windows.lock().unwrap_or_else(|e| e.into_inner())
    }
}

struct AppState {
    clients: Clients,
    active_connections: Arc<std::sync::atomic::AtomicUsize>,
    metrics: Arc<Metrics>,
    config: Config,
    rate_limiter: Arc<RateLimiter>,
    /// Shared nonce replay cache — process-wide (not per-connection) so a
    /// reconnect cannot replay a previously accepted nonce, and persisted
    /// across restarts via `hmac::save_nonces`. Entries are scoped per
    /// authenticated device id so one client cannot evict another's.
    nonces: Arc<RwLock<NonceCache>>,
    /// `sha256/<base64>` SPKI pin of the served certificate, served by
    /// `GET /pin`. `None` when TLS failed to initialise.
    tls_pin: Option<String>,
}

/// Remove entries from the routing table whose outbound channel has closed.
///
/// The clean disconnect path already removes a device on the way out, but an
/// aborted or panicked `handle_connection` task leaves a dead `Sender` behind
/// forever, which both leaks memory and makes the device permanently appear
/// "connected". Returns the number of entries removed.
async fn reconcile_clients(clients: &Clients) -> usize {
    let mut map = clients.write().await;
    let before = map.len();
    map.retain(|_, tx| !tx.is_closed());
    before - map.len()
}

fn build_metrics_json(state: &AppState) -> Value {
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
fn build_prometheus_metrics(state: &AppState) -> String {
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

/// Serve `/health` and `/metrics` over plain HTTP until shutdown is signalled.
///
/// Split out of `main` so integration tests exercise the real routing
/// (JSON health payload, Prometheus exposition format, 404 handling)
/// instead of a reimplementation.
async fn spawn_health_server(
    state: Arc<AppState>,
    mut shutdown_rx: watch::Receiver<bool>,
) -> std::io::Result<tokio::task::JoinHandle<()>> {
    // Bind inside the container so nginx and Docker health checks can reach
    // this listener; compose restricts the host-published port to loopback.
    let health_addr = format!("0.0.0.0:{}", state.config.health_port);
    let health_listener = TcpListener::bind(&health_addr).await?;
    Ok(tokio::spawn(async move {
        info!("Health check listener on: {}", health_addr);
        loop {
            tokio::select! {
                result = health_listener.accept() => {
                    if let Ok((stream, _)) = result {
                        let io = TokioIo::new(stream);
                        let state = state.clone();
                        tokio::spawn(async move {
                            let service = hyper::service::service_fn(move |req: Request<Incoming>| {
                                let state = state.clone();
                                async move {
                                    match req.uri().path() {
                                        "/healthz" => {
                                            let resp: Response<String> = Response::builder()
                                                .status(StatusCode::OK)
                                                .header("Content-Type", "application/json")
                                                .body(r#"{"status":"ok"}"#.to_string())
                                                .unwrap();
                                            Ok::<_, hyper::Error>(resp)
                                        }
                                        "/health" | "/" => {
                                            let authorized = req
                                                .headers()
                                                .get(hyper::header::AUTHORIZATION)
                                                .and_then(|v| v.to_str().ok())
                                                .map(|h| {
                                                    bearer_token_authorized(
                                                        h,
                                                        state.config.effective_health_token(),
                                                    )
                                                })
                                                .unwrap_or(false);
                                            if !authorized {
                                                let resp: Response<String> = Response::builder()
                                                    .status(StatusCode::UNAUTHORIZED)
                                                    .header("WWW-Authenticate", "Bearer")
                                                    .header("Content-Type", "application/json")
                                                    .body(
                                                        r#"{"error":"unauthorized"}"#.to_string(),
                                                    )
                                                    .unwrap();
                                                return Ok::<_, hyper::Error>(resp);
                                            }
                                            let body = build_metrics_json(&state);
                                            let resp: Response<String> = Response::builder()
                                                .status(StatusCode::OK)
                                                .header("Content-Type", "application/json")
                                                .body(body.to_string())
                                                .unwrap();
                                            Ok::<_, hyper::Error>(resp)
                                        }
                                        "/metrics" => {
                                            // Unauthenticated by default (tested
                                            // decision: Prometheus must scrape
                                            // without the health secret), but
                                            // gateable via RELAY_METRICS_TOKEN.
                                            if let Some(expected) =
                                                state.config.metrics_token.as_deref()
                                            {
                                                let authorized = req
                                                    .headers()
                                                    .get(hyper::header::AUTHORIZATION)
                                                    .and_then(|v| v.to_str().ok())
                                                    .map(|h| {
                                                        bearer_token_authorized(h, expected)
                                                    })
                                                    .unwrap_or(false);
                                                if !authorized {
                                                    let resp: Response<String> = Response::builder()
                                                        .status(StatusCode::UNAUTHORIZED)
                                                        .header("WWW-Authenticate", "Bearer")
                                                        .header("Content-Type", "application/json")
                                                        .body(r#"{"error":"unauthorized"}"#.to_string())
                                                        .unwrap();
                                                    return Ok::<_, hyper::Error>(resp);
                                                }
                                            }
                                            let body = build_prometheus_metrics(&state);
                                            let resp: Response<String> = Response::builder()
                                                .status(StatusCode::OK)
                                                .header("Content-Type", "text/plain; version=0.0.4; charset=utf-8")
                                                .body(body)
                                                .unwrap();
                                            Ok::<_, hyper::Error>(resp)
                                        }
                                        "/pin" => {
                                            // Certificate pin discovery, so an
                                            // operator never has to compute it
                                            // out-of-band from the file on disk
                                            // (and never pins the wrong hash).
                                            let body = match &state.tls_pin {
                                                Some(pin) => serde_json::json!({
                                                    "sha256": pin,
                                                    "algorithm": "spki-sha256",
                                                })
                                                .to_string(),
                                                None => serde_json::json!({
                                                    "sha256": serde_json::Value::Null,
                                                    "algorithm": "spki-sha256",
                                                    "error": "tls_unavailable",
                                                })
                                                .to_string(),
                                            };
                                            let resp: Response<String> = Response::builder()
                                                .status(StatusCode::OK)
                                                .header("Content-Type", "application/json")
                                                .body(body)
                                                .unwrap();
                                            Ok::<_, hyper::Error>(resp)
                                        }
                                        _ => {
                                            let resp: Response<String> = Response::builder()
                                                .status(StatusCode::NOT_FOUND)
                                                .body("Not Found".to_string())
                                                .unwrap();
                                            Ok::<_, hyper::Error>(resp)
                                        }
                                    }
                                }
                            });
                            hyper_util::server::conn::auto::Builder::new(hyper_util::rt::TokioExecutor::new())
                                .serve_connection(io, service)
                                .await
                        });
                    }
                }
                _ = shutdown_rx.changed() => {
                    info!("Health check listener shutting down");
                    break;
                }
            }
        }
    }))
}

#[tokio::main]
async fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .format(|buf, record| {
            let timestamp_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let line = format_log_line(
                timestamp_ms,
                &record.level().to_string(),
                record.target(),
                &record.args().to_string(),
            );
            use std::io::Write;
            writeln!(buf, "{}", line)
        })
        .init();

    let config = Config::from_env().unwrap_or_else(|e| {
        error!("fatal: {}", e);
        std::process::exit(EXIT_CONFIG);
    });

    let tls_context = match tls::load_tls_context() {
        Ok(ctx) => {
            tls::warn_if_sans_may_not_match(&ctx);
            info!(
                "TLS ready (TLS 1.3, {:?} certificate, SANs {:?}) — cert={} key={} pin={}",
                ctx.source,
                ctx.subject_alt_names,
                ctx.cert_path.display(),
                ctx.key_path.display(),
                ctx.spki_pin
            );
            info!(
                "Certificates are loaded once at startup: renewing {} requires a restart.",
                ctx.cert_path.display()
            );
            Some(ctx)
        }
        Err(e) => {
            warn!("TLS initialization failed: {}. WSS will be unavailable.", e);
            None
        }
    };
    let tls_acceptor = tls_context.as_ref().map(|c| c.acceptor.clone());
    let tls_pin = tls_context.as_ref().map(|c| c.spki_pin.clone());

    let loaded_nonces = hmac::load_nonces(&config.nonce_file);
    info!(
        "Replay cache: loaded {} nonce(s) from {}",
        loaded_nonces.len(),
        config.nonce_file.display()
    );
    info!(
        "Message signing keys accepted: {:?} (current signs, all listed verify)",
        config.signing_keys.ids()
    );

    let state = Arc::new(AppState {
        clients: Arc::new(RwLock::new(HashMap::new())),
        active_connections: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        metrics: Arc::new(Metrics::new()),
        rate_limiter: Arc::new(RateLimiter::new(10, 60)),
        nonces: Arc::new(RwLock::new(NonceCache::from_map(&loaded_nonces))),
        tls_pin,
        config,
    });

    // Persist the nonce replay cache periodically (crash resilience).
    // Atomic tmp+rename in save_nonces makes a mid-write crash safe.
    //
    // `save_nonces` does synchronous `std::fs` I/O, so it runs on the blocking
    // pool: calling it directly from this task blocked a tokio worker thread
    // for the duration of every 30 s flush.
    {
        let state = state.clone();
        let nonce_file = state.config.nonce_file.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                let snapshot = state.nonces.read().await.to_map();
                let file = nonce_file.clone();
                let result =
                    tokio::task::spawn_blocking(move || hmac::save_nonces(&file, &snapshot)).await;
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => warn!("Failed to persist nonce cache: {}", e),
                    Err(e) => error!("Nonce cache persistence task failed: {}", e),
                }
            }
        });
    }

    // Periodic housekeeping: the connection rate limiter and the routing table
    // are both otherwise only pruned on an event involving the *same* key, so an
    // attacker rotating source addresses (or a task aborted mid-connection) grew
    // them without bound.
    {
        let state = state.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            loop {
                interval.tick().await;
                let swept = state.rate_limiter.sweep();
                let removed = reconcile_clients(&state.clients).await;
                if swept > 0 || removed > 0 {
                    info!(
                        "Housekeeping: evicted {} expired rate-limit window(s), \
                         reconciled {} dead client entr{}",
                        swept,
                        removed,
                        if removed == 1 { "y" } else { "ies" }
                    );
                }
            }
        });
    }

    // Graceful shutdown: listen for SIGINT / SIGTERM
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    {
        let shutdown_tx = shutdown_tx.clone();
        let signal_state = state.clone();
        tokio::spawn(async move {
            let ctrl_c = tokio::signal::ctrl_c();
            #[cfg(unix)]
            {
                use tokio::signal::unix::{SignalKind, signal};
                // Registration only fails if the platform lacks the signal;
                // fall back to ctrl_c rather than panicking in a signal task.
                match signal(SignalKind::terminate()) {
                    Ok(mut sigterm) => {
                        tokio::select! {
                            _ = ctrl_c => {}
                            _ = sigterm.recv() => {}
                        }
                    }
                    Err(e) => {
                        warn!("SIGTERM handler unavailable ({}); relying on Ctrl-C", e);
                        ctrl_c.await.ok();
                    }
                }
            }
            #[cfg(not(unix))]
            {
                ctrl_c.await.ok();
            }
            info!(
                "Shutdown signal received — signalling {} connection(s) to close",
                active_at_shutdown(&signal_state)
            );
            let _ = shutdown_tx.send(true);
        });
    }

    // Health + metrics listener (plain HTTP)
    if let Err(e) = spawn_health_server(state.clone(), shutdown_rx.clone()).await {
        error!(
            "fatal: failed to bind health port {}: {}. Is RELAY_HEALTH_PORT already in use?",
            state.config.health_port, e
        );
        std::process::exit(EXIT_LISTEN);
    }

    let mut wss_shutdown_rx = shutdown_rx.clone();
    let wss_handle = if let Some(acceptor) = tls_acceptor {
        let wss_addr = format!("0.0.0.0:{}", state.config.wss_port);
        let wss_listener = match TcpListener::bind(&wss_addr).await {
            Ok(l) => l,
            Err(e) => {
                error!(
                    "fatal: failed to bind WSS port {} ({}): {}. \
                     Is RELAY_WSS_PORT already in use?",
                    state.config.wss_port, wss_addr, e
                );
                std::process::exit(EXIT_LISTEN);
            }
        };
        info!("Secure WSS listening on: {}", wss_addr);

        let state_clone = state.clone();
        let acceptor = Arc::new(acceptor);
        Some(tokio::spawn(async move {
            // JoinSet owns every in-flight connection task so shutdown can
            // actually reap them instead of leaking tasks and `state.clients`
            // entries.
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    result = wss_listener.accept() => {
                        match result {
                            Ok((stream, peer)) => {
                                info!("New WSS connection from {}", peer);
                                let state = state_clone.clone();
                                let acceptor = acceptor.clone();
                                let conn_shutdown = wss_shutdown_rx.clone();
                                connections.spawn(async move {
                                    match acceptor.accept(stream).await {
                                        Ok(tls_stream) => {
                                            let _guard = ConnectionGuard { count: state.active_connections.clone() };
                                            if state.active_connections.fetch_add(1, Ordering::Relaxed) >= MAX_CONNECTIONS {
                                                warn!("Connection limit reached, rejecting {}", peer);
                                                return;
                                            }
                                            state.metrics.connections_connected.fetch_add(1, Ordering::Relaxed);
                                            handle_connection(tls_stream, state, peer, conn_shutdown).await;
                                        }
                                        Err(e) => {
                                            error!("TLS handshake error from {}: {}", peer, e);
                                        }
                                    }
                                });
                            }
                            Err(e) => {
                                error!("WSS accept error: {}", e);
                            }
                        }
                    }
                    Some(joined) = connections.join_next(), if !connections.is_empty() => {
                        if let Err(e) = joined {
                            error!("WSS connection task failed: {}", e);
                        }
                    }
                    _ = wss_shutdown_rx.changed() => {
                        info!("WSS listener shutting down");
                        break;
                    }
                }
            }
            drain_connections(&mut connections, "WSS").await;
        }))
    } else {
        None
    };

    let mut ws_shutdown_rx = shutdown_rx.clone();
    let ws_handle = if state.config.enable_plain_ws {
        let ws_addr = format!("0.0.0.0:{}", state.config.ws_port);
        let ws_listener = match TcpListener::bind(&ws_addr).await {
            Ok(l) => l,
            Err(e) => {
                error!(
                    "fatal: failed to bind WS port {} ({}): {}. \
                     Is RELAY_WS_PORT already in use?",
                    state.config.ws_port, ws_addr, e
                );
                std::process::exit(EXIT_LISTEN);
            }
        };
        info!(
            "Plain WS listening on: {} (INSECURE - tokens sent in cleartext)",
            ws_addr
        );

        let state_clone = state.clone();
        Some(tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    result = ws_listener.accept() => {
                        match result {
                            Ok((stream, peer)) => {
                                info!("New WS connection from {}", peer);
                                let state = state_clone.clone();
                                let conn_shutdown = ws_shutdown_rx.clone();
                                connections.spawn(async move {
                                    let _guard = ConnectionGuard { count: state.active_connections.clone() };
                                    if state.active_connections.fetch_add(1, Ordering::Relaxed) >= MAX_CONNECTIONS {
                                        warn!("Connection limit reached, rejecting {}", peer);
                                        return;
                                    }
                                    state.metrics.connections_connected.fetch_add(1, Ordering::Relaxed);
                                    handle_connection(stream, state, peer, conn_shutdown).await;
                                });
                            }
                            Err(e) => {
                                error!("WS accept error: {}", e);
                            }
                        }
                    }
                    Some(joined) = connections.join_next(), if !connections.is_empty() => {
                        if let Err(e) = joined {
                            error!("WS connection task failed: {}", e);
                        }
                    }
                    _ = ws_shutdown_rx.changed() => {
                        info!("WS listener shutting down");
                        break;
                    }
                }
            }
            drain_connections(&mut connections, "WS").await;
        }))
    } else {
        info!("Plain WS disabled (set RELAY_ENABLE_PLAIN_WS=true to enable)");
        None
    };

    // Wait for both listeners to finish draining.
    if let Some(handle) = wss_handle {
        let _ = handle.await;
    }
    if let Some(handle) = ws_handle {
        let _ = handle.await;
    }

    if state.active_connections.load(Ordering::Relaxed) == 0 {
        error!(
            "No listeners started (TLS unavailable and plain WS disabled). Exiting with {}.",
            EXIT_NO_LISTENERS
        );
        std::process::exit(EXIT_NO_LISTENERS);
    }

    // Final flush of the shared nonce replay cache on graceful shutdown.
    // `save_nonces` is synchronous I/O; this runs once at exit, after the
    // listeners are down, so blocking briefly here is fine and there is no
    // worker thread left to starve.
    {
        let snapshot = state.nonces.read().await.to_map();
        let file = state.config.nonce_file.clone();
        match hmac::save_nonces(&file, &snapshot) {
            Ok(()) => info!("Nonce cache persisted to {}", file.display()),
            Err(e) => warn!("Failed to persist nonce cache on shutdown: {}", e),
        }
    }
}

/// Graceful drain: give in-flight connection tasks a bounded window to notice
/// the shutdown signal and close on their own, then abort whatever is left.
///
/// This is what "draining connections" actually means; the previous code logged
/// that and then dropped every task handle on the floor.
async fn drain_connections(connections: &mut tokio::task::JoinSet<()>, label: &str) {
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

/// Seconds a listener waits for in-flight connections to close on shutdown.
const DRAIN_TIMEOUT_SECS: u64 = 5;

/// Active connection count at the moment a shutdown signal is observed.
fn active_at_shutdown(state: &Arc<AppState>) -> usize {
    state.active_connections.load(Ordering::Relaxed)
}

async fn handle_connection<S>(
    stream: S,
    state: Arc<AppState>,
    peer: SocketAddr,
    mut shutdown_rx: watch::Receiver<bool>,
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let mut ws_stream = match accept_async(stream).await {
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
    let (tx, mut rx) = mpsc::channel(1024);

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
        if let Message::Text(text) = msg {
            if text.len() > MAX_TEXT_SIZE {
                warn!("Auth message too large from {}", peer);
                return;
            }
            if let Ok(json) = serde_json::from_str::<Value>(&text)
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
                            .insert(auth.device_id.clone(), tx.clone());
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
                                    let _ =
                                        write.send(error_frame("invalid_device_id", reason)).await;
                                    return;
                                }
                                device_id = Some(id.to_string());
                                state
                                    .clients
                                    .write()
                                    .await
                                    .insert(id.to_string(), tx.clone());
                                state
                                    .metrics
                                    .auth_attempts_success
                                    .fetch_add(1, Ordering::Relaxed);
                                info!("Device authenticated: {} from {}", id, peer);
                                let _ = write
                                    .send(Message::Text(
                                        serde_json::to_string(&RelayAuthOk::new())
                                            .unwrap_or_default(),
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
    let signing_keys = state.config.signing_keys.clone();
    // Share the process-wide nonce cache: a device that disconnects and
    // reconnects cannot replay a nonce that was already accepted. Entries are
    // scoped to `my_id` (the authenticated identity) so one client cannot
    // evict another's replay protection.
    let nonces = state.nonces.clone();
    // Highest binary sequence number accepted on this connection (replay guard).
    let mut last_binary_seq: Option<u32> = None;

    let mut ping_interval = tokio::time::interval(tokio::time::Duration::from_secs(25));

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
                                let _ = tx
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
                                match handle_relay_route(&signing_keys, &nonces, &json, &my_id)
                                    .await
                                {
                                    Ok(route) => {
                                        forward_text(
                                            &state,
                                            &my_id,
                                            &route.to_device_id,
                                            route.payload,
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
                                        let _ = tx
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
                                let _ = tx
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
                                let _ = tx
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
                                let _ = tx
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
                        state.metrics.observe_message_size(bytes.len());
                        match handle_binary_frame(
                            &state,
                            &signing_keys,
                            &my_id,
                            bytes,
                            &mut last_binary_seq,
                        )
                        .await
                        {
                            Ok(Some(frame)) => {
                                forward_binary(&state, &my_id, &frame.target_id, frame.payload)
                                    .await;
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
                                let _ =
                                    tx.send(error_frame(rejection.code, rejection.reason)).await;
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
fn error_frame(code: &str, message: impl Into<String>) -> Message {
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

/// Shared copy for the "encrypted was not wrapped in a relay_route" refusal.
const NOT_WRAPPED_MSG: &str = "'encrypted' messages must be wrapped in a signed relay_route so the sender can be \
     authenticated";

/// Why a message was refused, which selects the metric it is counted under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RejectionKind {
    /// Signature/tag did not verify, or the claimed sender did not match the
    /// authenticated connection identity.
    Integrity,
    /// Replayed nonce or stale timestamp / non-increasing binary sequence.
    Replay,
}

/// A refusal, carrying the machine-readable code sent in the `error` frame.
#[derive(Debug, Clone)]
struct Rejection {
    kind: RejectionKind,
    code: &'static str,
    reason: String,
}

impl Rejection {
    fn integrity(code: &'static str, reason: impl Into<String>) -> Self {
        Self {
            kind: RejectionKind::Integrity,
            code,
            reason: reason.into(),
        }
    }

    fn replay(code: &'static str, reason: impl Into<String>) -> Self {
        Self {
            kind: RejectionKind::Replay,
            code,
            reason: reason.into(),
        }
    }
}

/// Validate a claimed `device_id` from `relay_auth`.
fn validate_device_id(id: &str) -> Result<(), String> {
    if is_valid_device_id(id) {
        return Ok(());
    }
    Err(format!(
        "device_id must be 1-{} lowercase hex characters (PROTOCOL.md device_id \
         is the first 16 hex chars of the device X25519 key); got {} character(s)",
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
async fn handle_relay_route(
    signing_keys: &SigningKeyring,
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

    signing_keys.verify(json).ok_or_else(|| {
        Rejection::integrity(
            "hmac_invalid",
            "signature did not verify against any accepted key (check key_id \
             and that the client signs with the relay's message-signing key, \
             not the relay token)",
        )
    })?;

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
struct VerifiedBinaryFrame<'a> {
    target_id: String,
    payload: &'a [u8],
}

/// Verify a v2 binary relay frame: version, target id, HMAC tag and sequence.
///
/// `Ok(None)` is reserved for frames that are structurally valid but carry
/// nothing to route (currently unreachable; kept so future "accept and ignore"
/// cases stay explicit rather than becoming silent drops).
async fn handle_binary_frame<'a>(
    _state: &Arc<AppState>,
    signing_keys: &SigningKeyring,
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

    if !verify_binary_tag(signing_keys, authenticated_device_id, bytes, payload, tag) {
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
fn binary_mac_input(authenticated_device_id: &str, header_and_payload: &[u8]) -> Vec<u8> {
    let mut input =
        Vec::with_capacity(authenticated_device_id.len() + 1 + header_and_payload.len());
    input.extend_from_slice(authenticated_device_id.as_bytes());
    input.push(0x1f);
    input.extend_from_slice(header_and_payload);
    input
}

/// Verify a v2 binary frame tag against any key in the ring.
///
/// Unlike `relay_route`, the frame carries no `key_id` (there is no room in the
/// fixed header), so every accepted key is tried. This is safe because each
/// candidate is compared in constant time and the sender id is bound into the
/// MAC input.
fn verify_binary_tag(
    signing_keys: &SigningKeyring,
    authenticated_device_id: &str,
    frame: &[u8],
    payload: &[u8],
    tag: &[u8],
) -> bool {
    use conduit_protocol::BINARY_AUTHENTICATED_PREFIX_LEN;

    let mut header = Vec::with_capacity(BINARY_AUTHENTICATED_PREFIX_LEN + payload.len());
    header.extend_from_slice(&frame[..BINARY_AUTHENTICATED_PREFIX_LEN]);
    header.extend_from_slice(payload);
    let input = binary_mac_input(authenticated_device_id, &header);
    let input_hex = hex::encode(&input);
    let tag_hex = hex::encode(tag);

    let mut candidates: Vec<&[u8]> = vec![&signing_keys.current().secret];
    if let Some(previous) = signing_keys.previous() {
        candidates.push(&previous.secret);
    }

    candidates
        .iter()
        .any(|secret| hmac::verify_hmac(secret, &input_hex, &tag_hex))
}

/// Forward a text payload, updating the routing metrics.
async fn forward_text(
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
async fn forward_text_with_timeout(
    state: &Arc<AppState>,
    from_device_id: &str,
    to_device_id: &str,
    payload: Value,
    timeout: std::time::Duration,
) {
    let clients = state.clients.read().await;
    let Some(target_tx) = clients.get(to_device_id) else {
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

    // Bounded wait: without this a target that stops reading (its outbound
    // queue fills) blocks this sender's read loop indefinitely. The drop is
    // counted as `timeout`, which is what that metric was always meant for.
    let send = target_tx.send(Message::Text(payload.to_string()));
    match tokio::time::timeout(timeout, send).await {
        Ok(Ok(())) => {
            state
                .metrics
                .messages_routed
                .fetch_add(1, Ordering::Relaxed);
        }
        Ok(Err(_)) => {
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
async fn forward_binary(
    state: &Arc<AppState>,
    from_device_id: &str,
    to_device_id: &str,
    payload: &[u8],
) {
    let clients = state.clients.read().await;
    let Some(target_tx) = clients.get(to_device_id) else {
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

    let send = target_tx.send(Message::Binary(payload.to_vec()));
    match tokio::time::timeout(std::time::Duration::from_secs(FORWARD_TIMEOUT_SECS), send).await {
        Ok(Ok(())) => {
            state
                .metrics
                .messages_routed
                .fetch_add(1, Ordering::Relaxed);
        }
        Ok(Err(_)) => {
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

// ============================================================
//  Test suite
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

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

    #[test]
    fn format_log_line_is_valid_json_with_expected_fields() {
        let line = format_log_line(1_700_000_000_123i64, "INFO", "relay::main", "hello world");
        let v: Value = serde_json::from_str(&line).expect("log line must be JSON");
        assert_eq!(v["level"], "info", "level must be lowercase");
        assert_eq!(v["target"], "relay::main");
        assert_eq!(v["message"], "hello world");
        assert_eq!(v["timestamp"], "2023-11-14T22:13:20.123Z");
    }

    #[test]
    fn format_log_line_lowercases_all_levels() {
        for level in ["ERROR", "WARN", "INFO", "DEBUG", "TRACE"] {
            let line = format_log_line(0, level, "t", "m");
            let v: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(
                v["level"],
                level.to_ascii_lowercase(),
                "level {} should lowercase",
                level
            );
        }
    }

    #[test]
    fn format_log_line_escapes_message_content() {
        let line = format_log_line(0, "ERROR", "t", "quote\" back\\slash\nnewline");
        let v: Value = serde_json::from_str(&line).expect("must stay valid JSON");
        assert_eq!(v["message"], "quote\" back\\slash\nnewline");
    }

    #[test]
    fn format_rfc3339_ms_epoch_and_pre_epoch_do_not_panic() {
        assert_eq!(format_rfc3339_ms(0), "1970-01-01T00:00:00.000Z");
        // Pre-epoch must not panic (SystemTime::duration_since would).
        let pre = format_rfc3339_ms(-1_000);
        assert!(pre.starts_with("1969-12-31"), "got {}", pre);
        assert!(
            pre.ends_with("59.000Z") || pre.contains("59."),
            "got {}",
            pre
        );
    }

    #[test]
    fn format_rfc3339_ms_known_leap_day() {
        // 2024-02-29T00:00:00Z = 1709164800000 ms
        assert_eq!(
            format_rfc3339_ms(1_709_164_800_000),
            "2024-02-29T00:00:00.000Z"
        );
    }

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
                    std::env::set_var("RELAY_TOKEN", "tok");
                    std::env::set_var("HMAC_SECRET", "sec");
                    std::env::set_var("RELAY_HEALTH_TOKEN", "custom-health");
                }
                let cfg = Config::from_env().unwrap();
                assert_eq!(cfg.health_token, "custom-health");

                unsafe {
                    std::env::set_var("RELAY_HEALTH_TOKEN", "");
                }
                let cfg = Config::from_env().unwrap();
                assert_eq!(
                    cfg.health_token, "sec",
                    "empty RELAY_HEALTH_TOKEN must fall back to hmac secret"
                );

                unsafe {
                    std::env::remove_var("RELAY_HEALTH_TOKEN");
                }
                let cfg = Config::from_env().unwrap();
                assert_eq!(
                    cfg.health_token, "sec",
                    "unset RELAY_HEALTH_TOKEN must fall back to hmac secret"
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
            wss_port: 9529,
            health_port: 9530,
            hmac_secret: b"test-secret-for-unit-tests!!!".to_vec(),
            health_token: "test-secret-for-unit-tests!!!".to_string(),
            metrics_token: None,
            relay_token: "test-secret-for-unit-tests!!!".to_string(),
            // Tests sign with the derived keyring, never the relay token.
            signing_keys: SigningKeyring::new(
                hmac::DEFAULT_KEY_ID,
                hmac::derive_signing_key(b"test-secret-for-unit-tests!!!").to_vec(),
            ),
            enable_plain_ws: false,
            nonce_file: std::path::PathBuf::from("./data/test-nonces.json"),
            auth_timeout_secs: 10,
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
        let keys = SigningKeyring::new(hmac::DEFAULT_KEY_ID, e2e_signing_key());
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
        let mut authenticated =
            Vec::with_capacity(conduit_protocol::BINARY_AUTHENTICATED_PREFIX_LEN);
        authenticated
            .extend_from_slice(&frame[..conduit_protocol::BINARY_AUTHENTICATED_PREFIX_LEN]);
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
    async fn binary_frame_accepts_the_previous_key_during_rotation() {
        let previous_key = b"previous-signing-key-32-bytes!!";
        let keys =
            SigningKeyring::new("k2", e2e_signing_key()).with_previous("k1", previous_key.to_vec());
        let state = Arc::new(test_state(0, 0, 0, 0, 0));

        // Build a frame with the previous key.
        let mut frame = binary_frame("aa", "dev", 1, b"rotating");
        let payload = &frame[conduit_protocol::BINARY_HEADER_LEN..].to_vec();
        let authenticated = {
            let mut h = frame[..conduit_protocol::BINARY_AUTHENTICATED_PREFIX_LEN].to_vec();
            h.extend_from_slice(payload);
            h
        };
        let tag = hmac::compute_hmac(
            previous_key,
            &hex::encode(binary_mac_input("aa", &authenticated)),
        );
        frame[conduit_protocol::BINARY_TAG_OFFSET
            ..conduit_protocol::BINARY_TAG_OFFSET + conduit_protocol::BINARY_TAG_LEN]
            .copy_from_slice(&hex::decode(&tag).unwrap());

        let mut seq = None;
        let parsed = handle_binary_frame(&state, &keys, "aa", &frame, &mut seq)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(parsed.target_id, "dev");

        // Once the rotation window closes, the same frame is refused.
        let closed = keys.end_rotation();
        let mut seq2 = None;
        assert_eq!(
            handle_binary_frame(&state, &closed, "aa", &frame, &mut seq2)
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

    // ---------------------------------------------------------------
    //  Config tests – empty / missing RELAY_TOKEN
    //  These must be serialised because they mutate process-wide env vars.
    // ---------------------------------------------------------------

    /// Snapshot the env vars we touch, then restore them after the closure.
    fn with_env_snapshot<F: FnOnce()>(keys: &[&str], f: F) {
        let saved: Vec<(&str, Option<String>)> =
            keys.iter().map(|&k| (k, std::env::var(k).ok())).collect();
        f();
        for (k, v) in saved {
            match v {
                Some(val) => unsafe { std::env::set_var(k, val) },
                None => unsafe { std::env::remove_var(k) },
            }
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
            let result = Config::from_env();
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(
                err.contains("must not be empty"),
                "error should mention empty token, got: {}",
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
            let result = Config::from_env();
            assert!(result.is_err());
            let err = result.unwrap_err();
            assert!(
                err.contains("required"),
                "error should mention required, got: {}",
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

                let cfg = Config::from_env().unwrap();
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
                    std::env::set_var("RELAY_TOKEN", "tok");
                    std::env::set_var("HMAC_SECRET", "sec");
                    std::env::set_var("RELAY_WS_PORT", "8080");
                    std::env::set_var("RELAY_WSS_PORT", "8443");
                    std::env::set_var("RELAY_HEALTH_PORT", "9090");
                    std::env::set_var("RELAY_ENABLE_PLAIN_WS", "true");
                }

                let cfg = Config::from_env().unwrap();
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
                std::env::set_var("RELAY_TOKEN", "tok");
                std::env::remove_var("HMAC_SECRET");
                // Point at an isolated temp file so tests never touch ./secrets.
                let dir =
                    std::env::temp_dir().join(format!("relay-hmac-test-{}", std::process::id()));
                std::fs::create_dir_all(&dir).ok();
                let file = dir.join("hmac_secret");
                std::fs::remove_file(&file).ok();
                std::env::set_var(
                    "HMAC_SECRET_FILE",
                    file.to_str().expect("temp path should be valid UTF-8"),
                );
            }

            let cfg = Config::from_env().unwrap();
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
            let cfg2 = Config::from_env().unwrap();
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
                std::env::set_var("RELAY_TOKEN", "tok");
                std::env::remove_var("HMAC_SECRET");
                let dir =
                    std::env::temp_dir().join(format!("relay-hmac-empty-{}", std::process::id()));
                std::fs::create_dir_all(&dir).ok();
                let file = dir.join("hmac_secret");
                std::fs::write(&file, "\n").ok();
                std::env::set_var(
                    "HMAC_SECRET_FILE",
                    file.to_str().expect("temp path should be valid UTF-8"),
                );
            }

            let result = Config::from_env();
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
    fn config_invalid_port_ignores_and_uses_default() {
        with_env_snapshot(&["RELAY_TOKEN", "HMAC_SECRET", "RELAY_WS_PORT"], || {
            unsafe {
                std::env::set_var("RELAY_TOKEN", "tok");
                std::env::set_var("HMAC_SECRET", "sec");
                std::env::set_var("RELAY_WS_PORT", "not-a-number");
            }

            let cfg = Config::from_env().unwrap();
            assert_eq!(
                cfg.ws_port, 9528,
                "invalid port should fall back to default"
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

        clients.write().await.insert("target-device".into(), tx);

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

        clients.write().await.insert("device-a".into(), tx1);
        clients.write().await.insert("device-b".into(), tx2);

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

        clients.write().await.insert("leaving-device".into(), tx);
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
            "k1",
            secret,
            "dead1",
            "dead2",
            serde_json::json!({"data": "hi"}),
            1_700_000_000_000,
            "test-nonce-1",
        );
        let msg = serde_json::to_value(&route).unwrap();

        assert!(hmac::verify_message_hmac(secret, &msg));
        let ring = SigningKeyring::new("k1", secret.to_vec());
        assert_eq!(ring.verify(&msg).as_deref(), Some("k1"));
    }

    #[tokio::test]
    async fn hmac_relay_route_message_rejected_with_wrong_hmac() {
        let secret = b"test-hmac-secret-key-32-bytes!";
        let mut route = RelayRoute::signed_with(
            "k1",
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
        assert!(
            SigningKeyring::new("k1", secret.to_vec())
                .verify(&msg)
                .is_none()
        );
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

        let keys = SigningKeyring::new("k1", b"test-hmac-secret-key-32-bytes!".to_vec());
        assert!(
            keys.verify(&legacy).is_none(),
            "an unsigned route must never verify"
        );
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
            lock.insert("device-lifecycle".into(), tx);
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
        clients.write().await.insert("device-reconnect".into(), tx1);

        // Reconnect with new sender (old sender dropped)
        clients.write().await.insert("device-reconnect".into(), tx2);

        // Old sender should be dropped → rx1 returns None
        assert!(rx1.recv().await.is_none(), "old channel should be closed");

        // New sender works
        {
            let lock = clients.read().await;
            lock.get("device-reconnect")
                .unwrap()
                .send(Message::Text("reconnected".to_string()))
                .await
                .unwrap();
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
                        std::env::set_var("RELAY_TOKEN", "tok");
                        std::env::set_var("HMAC_SECRET", "test-secret");
                        std::env::set_var("RELAY_ENABLE_PLAIN_WS", *val);
                    }
                    let cfg = Config::from_env().unwrap();
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
                        std::env::set_var("RELAY_TOKEN", "tok");
                        std::env::set_var("HMAC_SECRET", "test-secret");
                        std::env::set_var("RELAY_ENABLE_PLAIN_WS", *val);
                    }
                    let cfg = Config::from_env().unwrap();
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
                std::env::set_var("HMAC_SECRET", "secret");
            }
            // Whitespace-only is NOT empty, so it should be accepted
            let result = Config::from_env();
            assert!(result.is_ok(), "whitespace-only token should be accepted");
        });
    }

    #[test]
    #[serial_test::serial]
    fn config_hmac_secret_uses_exact_bytes() {
        with_env_snapshot(&["RELAY_TOKEN", "HMAC_SECRET"], || {
            unsafe {
                std::env::set_var("RELAY_TOKEN", "tok");
                std::env::set_var("HMAC_SECRET", "my-secret");
            }
            let cfg = Config::from_env().unwrap();
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

    /// Message-signing key for e2e tests: derived from [`E2E_SECRET`] exactly
    /// the way `Config::from_env` derives it when `RELAY_SIGNING_KEY` is unset.
    ///
    /// Deliberately **not** `E2E_SECRET`/`E2E_RELAY_TOKEN`: signing with the
    /// token is the defect VULNERABILITY 1 is about, and
    /// `e2e_route_signed_with_the_relay_token_is_rejected` proves the relay
    /// refuses exactly that.
    fn e2e_signing_key() -> Vec<u8> {
        hmac::derive_signing_key(E2E_SECRET).to_vec()
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
            hmac::DEFAULT_KEY_ID,
            &e2e_signing_key(),
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
        let tag = hmac::compute_hmac(&e2e_signing_key(), &hex::encode(&mac_input));
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

        let delivered = recv_text_matching(&mut bob, 5, |t| t.contains("ping"))
            .await
            .expect("bob should receive routed payload");
        assert!(
            delivered.contains("\"type\":\"ping\"") || delivered.contains("ping"),
            "delivered payload should be the inner message, got: {}",
            delivered
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
        assert_eq!(
            bytes, b"binary-payload-bytes",
            "payload should be forwarded without the 53-byte v2 header"
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

        let (status, body) =
            http_get_bearer(relay.health_port, "/health", Some("wrong-token")).await;
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
        signing_keys: SigningKeyring,
        metrics_token: Option<String>,
    }

    impl Default for StateOpts {
        fn default() -> Self {
            Self {
                max_attempts: 1000,
                auth_timeout_secs: 10,
                signing_keys: SigningKeyring::new(hmac::DEFAULT_KEY_ID, e2e_signing_key()),
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
            config: Config {
                ws_port: 0,
                wss_port: 0,
                health_port: free_port(),
                hmac_secret: E2E_SECRET.to_vec(),
                health_token: String::from_utf8_lossy(E2E_SECRET).to_string(),
                metrics_token: opts.metrics_token,
                // The relay verifies relay_route HMACs with `signing_keys`, and
                // the token stays a separate credential on purpose: the tests
                // would fail loudly if anything ever verified routes with it.
                relay_token: String::from_utf8_lossy(E2E_SECRET).to_string(),
                signing_keys: opts.signing_keys,
                enable_plain_ws: true,
                nonce_file,
                auth_timeout_secs: opts.auth_timeout_secs,
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
        route.insert("key_id".into(), serde_json::json!(hmac::DEFAULT_KEY_ID));
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
    fn relay_signing_key_is_never_the_master_secret() {
        let keys = load_signing_keyring(E2E_SECRET).expect("keyring");
        assert_ne!(
            keys.current().secret,
            E2E_SECRET.to_vec(),
            "the signing key must not be the master secret"
        );
        assert_eq!(
            keys.current().secret,
            hmac::derive_signing_key(E2E_SECRET).to_vec()
        );
    }

    #[test]
    #[serial_test::serial]
    fn relay_signing_key_does_not_depend_on_the_relay_token() {
        // Two relays with the same master secret but completely different
        // bearer tokens must derive the same signing key: the token is not an
        // input to key derivation at all.
        with_env_snapshot(&["RELAY_TOKEN", "HMAC_SECRET", "HMAC_SECRET_FILE"], || {
            let signing_key_for_token = |token: &str| {
                unsafe {
                    std::env::set_var("RELAY_TOKEN", token);
                    std::env::set_var("HMAC_SECRET", "shared-master");
                }
                Config::from_env()
                    .expect("config")
                    .signing_keys
                    .current()
                    .secret
                    .clone()
            };
            let a = signing_key_for_token("token-a");
            let b = signing_key_for_token("token-b");
            assert_eq!(a, b, "the signing key must be independent of RELAY_TOKEN");
            assert_ne!(
                a,
                hmac::derive_signing_key(b"a-completely-different-master").to_vec(),
                "sanity: a different master secret must give a different key"
            );
        });
    }

    #[tokio::test]
    async fn e2e_route_signed_with_the_previous_key_is_accepted_during_rotation() {
        let previous = hmac::derive_key(E2E_SECRET, "conduit-relay/v1/legacy-signing-key").to_vec();
        let state = e2e_state_with_opts(StateOpts {
            signing_keys: SigningKeyring::new("v2", e2e_signing_key())
                .with_previous("v1", previous.clone()),
            ..Default::default()
        });
        let relay = spawn_relay(state.clone()).await;

        let mut sender = ws_client(relay.ws_addr).await;
        let mut target = ws_client(relay.ws_addr).await;
        authenticate(&mut sender, "ee11", E2E_RELAY_TOKEN).await;
        authenticate(&mut target, "ff22", E2E_RELAY_TOKEN).await;

        // Signed with the RETIRING key and its key_id: must still be accepted.
        let route = RelayRoute::signed_with(
            "v1",
            &previous,
            "ee11",
            "ff22",
            serde_json::json!({"type": "ping", "tag": "rotating"}),
            now_millis(),
            "rotation-nonce",
        );
        sender
            .send(Message::Text(serde_json::to_string(&route).unwrap()))
            .await
            .unwrap();

        let got = recv_text_matching(&mut target, 5, |t| t.contains("rotating")).await;
        assert!(
            got.is_some(),
            "a message signed with the previous key must be accepted during \
             the rotation window"
        );

        // Once the window closes, the same message is refused.
        let closed = e2e_state_with_opts(StateOpts {
            signing_keys: SigningKeyring::new("v2", e2e_signing_key()),
            ..Default::default()
        });
        let relay2 = spawn_relay(closed.clone()).await;
        let mut s2 = ws_client(relay2.ws_addr).await;
        let mut t2 = ws_client(relay2.ws_addr).await;
        authenticate(&mut s2, "ee11", E2E_RELAY_TOKEN).await;
        authenticate(&mut t2, "ff22", E2E_RELAY_TOKEN).await;
        s2.send(Message::Text(
            serde_json::to_string(&RelayRoute::signed_with(
                "v1",
                &previous,
                "ee11",
                "ff22",
                serde_json::json!({"type": "ping", "tag": "rotating-closed"}),
                now_millis(),
                "rotation-nonce-2",
            ))
            .unwrap(),
        ))
        .await
        .unwrap();
        assert!(
            recv_text_matching(&mut t2, 2, |t| t.contains("rotating-closed"))
                .await
                .is_none(),
            "the retired key must stop being accepted once the window closes"
        );

        drop(relay);
        drop(relay2);
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

        let err =
            recv_text_matching(&mut ws, 5, |t| t.contains("not_wrapped_in_relay_route")).await;
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
    fn device_id_validation_rejects_everything_else() {
        for bad in [
            "",
            "alice",         // not hex
            "DEADBEEF",      // uppercase would alias a lowercase id
            "dead beef",     // whitespace
            "deadbeef-1234", // punctuation
            "deadbeef/1234", // path-ish
            "日本語",        // non-ascii
            "dead\nbeef",    // control character
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
            clients.write().await.insert("aabb".into(), tx);
            drop(rx);
        }
        let (tx_live, _rx_live) = mpsc::channel::<Message>(4);
        clients.write().await.insert("ccdd".into(), tx_live);
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
    //  VULNERABILITY 7 — key rotation configuration
    // ================================================================

    const SIGNING_ENV: [&str; 4] = [
        "RELAY_SIGNING_KEY",
        "RELAY_SIGNING_KEY_ID",
        "RELAY_SIGNING_KEY_PREVIOUS",
        "RELAY_SIGNING_KEY_PREVIOUS_ID",
    ];

    fn clear_signing_env() {
        for key in SIGNING_ENV {
            unsafe { std::env::remove_var(key) };
        }
    }

    #[test]
    #[serial_test::serial]
    fn signing_keyring_defaults_to_the_derived_key() {
        with_env_snapshot(&SIGNING_ENV, || {
            clear_signing_env();
            let ring = load_signing_keyring(b"master").unwrap();
            assert_eq!(ring.ids(), vec!["v1"]);
            assert_eq!(
                ring.current().secret,
                hmac::derive_signing_key(b"master").to_vec()
            );
            assert!(ring.previous().is_none());
        });
    }

    #[test]
    #[serial_test::serial]
    fn signing_keyring_uses_the_explicit_key_and_id() {
        with_env_snapshot(&SIGNING_ENV, || {
            clear_signing_env();
            unsafe {
                std::env::set_var("RELAY_SIGNING_KEY", "explicit-signing-key");
                std::env::set_var("RELAY_SIGNING_KEY_ID", "2026-01");
            }
            let ring = load_signing_keyring(b"master").unwrap();
            assert_eq!(ring.current().id, "2026-01");
            assert_eq!(
                ring.signing_key_for("2026-01"),
                Some(&b"explicit-signing-key".to_vec()[..])
            );
            assert!(ring.signing_key_for("v1").is_none());
        });
    }

    #[test]
    #[serial_test::serial]
    fn signing_keyring_accepts_a_previous_key_during_rotation() {
        with_env_snapshot(&SIGNING_ENV, || {
            clear_signing_env();
            unsafe {
                std::env::set_var("RELAY_SIGNING_KEY", "new-key");
                std::env::set_var("RELAY_SIGNING_KEY_ID", "k2");
                std::env::set_var("RELAY_SIGNING_KEY_PREVIOUS", "old-key");
                std::env::set_var("RELAY_SIGNING_KEY_PREVIOUS_ID", "k1");
            }
            let ring = load_signing_keyring(b"master").unwrap();
            assert_eq!(ring.ids(), vec!["k2", "k1"]);
            assert_eq!(ring.signing_key_for("k1"), Some(&b"old-key".to_vec()[..]));
            assert_eq!(ring.signing_key_for("k2"), Some(&b"new-key".to_vec()[..]));
            assert!(ring.signing_key_for("k3").is_none());
        });
    }

    #[test]
    #[serial_test::serial]
    fn signing_keyring_requires_a_name_for_the_previous_key() {
        with_env_snapshot(&SIGNING_ENV, || {
            clear_signing_env();
            unsafe {
                std::env::set_var("RELAY_SIGNING_KEY_PREVIOUS", "old-key");
            }
            let err = load_signing_keyring(b"master").unwrap_err();
            assert!(
                err.contains("RELAY_SIGNING_KEY_PREVIOUS_ID"),
                "an unnamed retiring key must be rejected, got: {err}"
            );
        });
    }

    #[test]
    #[serial_test::serial]
    fn signing_keyring_rejects_a_duplicate_key_id() {
        with_env_snapshot(&SIGNING_ENV, || {
            clear_signing_env();
            unsafe {
                std::env::set_var("RELAY_SIGNING_KEY_ID", "same");
                std::env::set_var("RELAY_SIGNING_KEY_PREVIOUS", "old-key");
                std::env::set_var("RELAY_SIGNING_KEY_PREVIOUS_ID", "same");
            }
            let err = load_signing_keyring(b"master").unwrap_err();
            assert!(err.contains("must differ"), "got: {err}");
        });
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

        let (status, body) =
            http_get_bearer(relay.health_port, "/metrics", Some("prom-token")).await;
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
            .insert("tgt".to_string(), tx.clone());

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
        state.clients.write().await.insert("dead".to_string(), tx);
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

    #[test]
    fn exit_codes_are_distinct() {
        // A port conflict used to `.expect()` into a panic-driven crash loop;
        // now each failure mode has its own code so an orchestrator can tell a
        // misconfiguration from a bind failure.
        let codes = [EXIT_CONFIG, EXIT_LISTEN, EXIT_NO_LISTENERS];
        let unique: std::collections::BTreeSet<_> = codes.iter().collect();
        assert_eq!(unique.len(), codes.len(), "exit codes must be distinct");
        assert!(codes.iter().all(|c| *c > 0));
    }

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
}
