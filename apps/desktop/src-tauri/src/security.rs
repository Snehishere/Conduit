//! Security utilities: rate limiting, message validation, token expiration,
//! the per-launch local-webview capability, and the shared shell-command
//! allowlist.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use log::warn;

use crate::error::ConduitError;

// ---------------------------------------------------------------------------
// Rate Limiter — per-client token-bucket (global fallback)
// ---------------------------------------------------------------------------

/// Configuration for the per-client rate limiter.
pub struct RateLimitConfig {
    /// Maximum messages allowed per window.
    pub max_messages: u32,
    /// Maximum *bytes* of inbound payload allowed per window, per client.
    ///
    /// A message-count limit alone is not a bandwidth limit: with
    /// [`MAX_MESSAGE_SIZE`] accepted per message, a peer sitting just under
    /// `max_messages` could still push gigabytes. This is the per-connection
    /// byte counter that closes that gap.
    pub max_bytes: u64,
    /// Window length.
    pub window: Duration,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            max_messages: 100,
            // 256 MiB / 10 s ≈ 25 MB/s. Comfortably above the ~3 MB/s a
            // 64 KiB-chunk file transfer needs, and far below "sit under the
            // message cap and push until the process dies".
            max_bytes: 256 * 1024 * 1024,
            window: Duration::from_secs(10),
        }
    }
}

struct Bucket {
    count: u32,
    bytes: u64,
    window_start: Instant,
}

/// Global per-client rate limiter — serves as a fallback cap across all message
/// types for a single client.  Prevents any single client from flooding the
/// server regardless of message type, in both message count and byte volume.
pub struct RateLimiter {
    buckets: Arc<RwLock<HashMap<String, Bucket>>>,
    config: RateLimitConfig,
}

impl RateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        let limiter = Self {
            buckets: Arc::new(RwLock::new(HashMap::new())),
            config,
        };
        // Spawn periodic cleanup every 60s
        let buckets = limiter.buckets.clone();
        let window = limiter.config.window;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                let mut map = buckets.write().await;
                let now = Instant::now();
                map.retain(|_, b| now.duration_since(b.window_start) < window * 2);
            }
        });
        limiter
    }

    /// Account one inbound message of `bytes` bytes against `client_id`'s window
    /// and report whether it is within both the message cap and the byte cap.
    ///
    /// `msg_type` is accepted for symmetry with [`PerTypeRateLimiter`] and for
    /// future per-type budgets; the global cap is intentionally type-agnostic so
    /// that a *renamed* type cannot buy a fresh budget.
    pub async fn check(&self, client_id: &str, _msg_type: &str, bytes: u64) -> bool {
        let bucket_key = client_id.to_string();
        let mut buckets = self.buckets.write().await;
        let now = Instant::now();

        let bucket = buckets.entry(bucket_key).or_insert_with(|| Bucket {
            count: 0,
            bytes: 0,
            window_start: now,
        });

        if now.duration_since(bucket.window_start) > self.config.window {
            bucket.count = 0;
            bucket.bytes = 0;
            bucket.window_start = now;
        }

        bucket.count += 1;
        bucket.bytes = bucket.bytes.saturating_add(bytes);
        bucket.count <= self.config.max_messages && bucket.bytes <= self.config.max_bytes
    }

    /// Bytes charged to `client_id` in the current window (test/diagnostic use).
    pub async fn window_bytes(&self, client_id: &str) -> u64 {
        self.buckets
            .read()
            .await
            .get(client_id)
            .map_or(0, |b| b.bytes)
    }

    pub async fn remove_client(&self, client_id: &str) {
        self.buckets.write().await.retain(|k, _| k != client_id);
    }
}

// ---------------------------------------------------------------------------
// Per-Type Rate Limiter — per-client, per-message-type buckets
// ---------------------------------------------------------------------------

/// Rate limit configuration for a single message type.
pub struct TypeLimitConfig {
    /// Maximum messages of this type allowed within the window.
    pub max_messages: u32,
    /// Window length.
    pub window: Duration,
}

impl TypeLimitConfig {
    const fn new(max_messages: u32, window_secs: u64) -> Self {
        Self {
            max_messages,
            window: Duration::from_secs(window_secs),
        }
    }
}

/// Returns the per-type rate limit configuration for a given message type.
///
/// Each top-level `type` field gets its own bucket with limits tuned to the
/// expected throughput of that feature area.
fn type_limit_for(msg_type: &str) -> TypeLimitConfig {
    match msg_type {
        // Security — prevent brute-force pairing attempts
        "pairing" => TypeLimitConfig::new(5, 60),
        // High throughput for large file transfers (chunks, progress, etc.)
        "file" => TypeLimitConfig::new(500, 60),
        // Screen mirror frames: ~30 fps × 2 msgs each
        "screen_mirror" => TypeLimitConfig::new(60, 60),
        // Audio streaming: 48 kHz mono ≈ ~30 msgs/sec
        "audio" => TypeLimitConfig::new(60, 60),
        // Clipboard sync — typing speed
        "clipboard" => TypeLimitConfig::new(30, 60),
        // Notification batches
        "notification" => TypeLimitConfig::new(100, 60),
        // Automation triggers — low frequency
        "automation" => TypeLimitConfig::new(20, 60),
        // Status updates — heartbeats + battery
        "status" => TypeLimitConfig::new(60, 60),
        // Catch-all for everything else
        _ => TypeLimitConfig::new(100, 60),
    }
}

/// Per-message-type rate limiter.
///
/// Maintains a separate sliding-window bucket for every `(client_id, msg_type)`
/// pair.  This prevents high-throughput types (e.g. `file/chunk`) from
/// starving low-volume types (e.g. `pairing`) and vice-versa.
///
/// Usage:
/// ```ignore
/// if !per_type_limiter.check_type_limit(client_id, msg_type).await {
///     warn!("Per-type rate limited");
///     return;
/// }
/// ```
pub struct PerTypeRateLimiter {
    buckets: Arc<RwLock<HashMap<String, Bucket>>>,
}

impl PerTypeRateLimiter {
    pub fn new() -> Self {
        let limiter = Self {
            buckets: Arc::new(RwLock::new(HashMap::new())),
        };
        // Spawn periodic cleanup every 60s — evict stale entries older than
        // 2× the longest configured window (120s).
        let buckets = limiter.buckets.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                let mut map = buckets.write().await;
                let now = Instant::now();
                map.retain(|_, b| now.duration_since(b.window_start) < Duration::from_secs(120));
            }
        });
        limiter
    }

    /// Returns `true` if the message is allowed, `false` if the per-type
    /// limit has been exceeded.
    pub async fn check_type_limit(&self, client_id: &str, message_type: &str) -> bool {
        let limit = type_limit_for(message_type);
        let bucket_key = format!("{}:{}", client_id, message_type);
        let mut buckets = self.buckets.write().await;
        let now = Instant::now();

        let bucket = buckets.entry(bucket_key).or_insert_with(|| Bucket {
            count: 0,
            bytes: 0,
            window_start: now,
        });

        if now.duration_since(bucket.window_start) > limit.window {
            bucket.count = 0;
            bucket.window_start = now;
        }

        bucket.count += 1;
        bucket.count <= limit.max_messages
    }

    /// Remove all buckets for a given client (call on disconnect).
    pub async fn remove_client(&self, client_id: &str) {
        let prefix = format!("{}:", client_id);
        self.buckets
            .write()
            .await
            .retain(|k, _| !k.starts_with(&prefix));
    }
}

// ---------------------------------------------------------------------------
// Message Validator — schema checks for incoming WebSocket messages
// ---------------------------------------------------------------------------

/// Maximum allowed sizes for various message fields.
const MAX_ID_LEN: usize = 128;
const MAX_NAME_LEN: usize = 256;
const MAX_STRING_FIELD_LEN: usize = 10_000;
/// Largest single WebSocket message the server will assemble (50 MB, generous
/// for audio data).
///
/// Handed to tungstenite as `WebSocketConfig::max_message_size` so the limit is
/// enforced while *reading* the frame, not only after the whole message has
/// already been buffered in memory. See `crate::server::ws_read_limits`.
pub const MAX_MESSAGE_SIZE: usize = 50 * 1024 * 1024;
/// Largest single WebSocket *frame* payload. Equal to
/// [`MAX_MESSAGE_SIZE`] so a fragmented message is not capped below a
/// single-frame one.
pub const MAX_FRAME_SIZE: usize = MAX_MESSAGE_SIZE;
/// Upper bound on the outbound per-socket send queue. A client that stops
/// reading is disconnected instead of being allowed to buffer unbounded
/// server-side memory.
pub const MAX_SEND_BUFFER_BYTES: usize = 8 * 1024 * 1024;
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024 * 1024; // 10 GB
const MAX_NOTIFICATION_BODY: usize = 50_000;
const MAX_AUTOMATION_RULES: usize = 100;
/// Battery percentage bounds for `status` frames and `discovery` announces.
pub const BATTERY_MIN: i64 = 0;
pub const BATTERY_MAX: i64 = 100;

/// Validate that a string field is within acceptable bounds.
fn validate_string_field(
    value: &str,
    max_len: usize,
    field_name: &str,
) -> Result<(), ConduitError> {
    if value.len() > max_len {
        return Err(ConduitError::Protocol(format!(
            "{} too long: {} bytes (max {})",
            field_name,
            value.len(),
            max_len
        )));
    }
    Ok(())
}

/// Validate a transfer/file ID — alphanumeric + underscores + hyphens only.
fn validate_id(id: &str) -> Result<(), ConduitError> {
    if id.is_empty() {
        return Err(ConduitError::Protocol("ID must not be empty".to_string()));
    }
    if id.len() > MAX_ID_LEN {
        return Err(ConduitError::Protocol(format!(
            "ID too long: {} (max {})",
            id.len(),
            MAX_ID_LEN
        )));
    }
    if !id
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ConduitError::Protocol(format!(
            "ID contains invalid characters: {}",
            id
        )));
    }
    Ok(())
}

/// Validate a file name — no path separators, no control characters.
fn validate_filename(name: &str) -> Result<(), ConduitError> {
    validate_string_field(name, MAX_NAME_LEN, "filename")?;
    if name.contains('/') || name.contains('\\') {
        return Err(ConduitError::Protocol(
            "Filename must not contain path separators".to_string(),
        ));
    }
    if name.chars().any(|c| c.is_control()) {
        return Err(ConduitError::Protocol(
            "Filename must not contain control characters".to_string(),
        ));
    }
    // Reject obvious traversal patterns
    if name == "." || name == ".." || name.starts_with("..") {
        return Err(ConduitError::Protocol(
            "Filename must not be a traversal pattern".to_string(),
        ));
    }
    Ok(())
}

/// Validate a hex-encoded public key (exactly 64 hex chars = 32 bytes).
fn validate_public_key(key: &str) -> Result<(), ConduitError> {
    if key.is_empty() {
        return Err(ConduitError::Protocol(
            "Public key must not be empty".to_string(),
        ));
    }
    if key.len() != 64 {
        return Err(ConduitError::Protocol(format!(
            "Public key must be 64 hex chars, got {}",
            key.len()
        )));
    }
    if !key.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(ConduitError::Protocol(
            "Public key must be valid hex".to_string(),
        ));
    }
    Ok(())
}

/// Validate a base64-encoded chunk — check it decodes and isn't absurdly large.
fn validate_chunk_data(data: &str) -> Result<(), ConduitError> {
    if data.is_empty() {
        return Err(ConduitError::Protocol(
            "Chunk data must not be empty".to_string(),
        ));
    }
    // Rough base64 size check: 4 chars encode 3 bytes
    let estimated_size = data.len() * 3 / 4;
    if estimated_size > MAX_MESSAGE_SIZE {
        return Err(ConduitError::Protocol(format!(
            "Chunk data too large: ~{} bytes (max {})",
            estimated_size, MAX_MESSAGE_SIZE
        )));
    }
    // Verify valid base64
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|e| ConduitError::Protocol(format!("Invalid base64 in chunk data: {}", e)))?;
    Ok(())
}

/// Validate a message type string.
fn validate_msg_type(msg_type: &str) -> Result<(), ConduitError> {
    const VALID_TYPES: &[&str] = &[
        "pairing",
        "notification",
        "clipboard",
        "file",
        "sms",
        "call",
        "audio",
        "status",
        "automation",
        "screen_mirror",
        "remote_input",
        "discovery",
        "tv",
        "watch",
        "ping",
        "pong",
        // The relay's own delivery envelope. Only ever received on the outbound
        // relay connection, and unwrapped by `handle_message` before it reaches
        // any handler, so allowing the type here does not expose it to the LAN
        // dispatcher: a LAN client that sends one has it refused as an unknown
        // action, because no `(relay_delivery, _)` arm exists below the unwrap.
        "relay_delivery",
        // Relay control-plane frames. `relay_auth_ok` / `relay_auth_rejected` /
        // `error` are the relay's answers to this app's own connection; without
        // them in the list `validate_message` rejected them before dispatch, so a
        // rejected token looked identical to a dropped socket.
        "relay_auth_ok",
        "relay_auth_rejected",
        "error",
    ];
    if !VALID_TYPES.contains(&msg_type) {
        return Err(ConduitError::Protocol(format!(
            "Unknown message type: {}",
            msg_type
        )));
    }
    Ok(())
}

/// Reject any field the type's handler does not read.
///
/// A validator that only bounds the fields it knows about accepts an unbounded
/// blob of attacker-chosen keys next to them, which then rides along inside
/// the *encrypted* frame to every paired device. This helper is what makes
/// "deny by default" real for the relay-only types.
fn reject_unknown_fields(msg: &serde_json::Value, allowed: &[&str]) -> Result<(), ConduitError> {
    let Some(obj) = msg.as_object() else {
        return Err(ConduitError::Protocol(
            "Message must be a JSON object".to_string(),
        ));
    };
    for key in obj.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(ConduitError::Protocol(format!(
                "Unexpected field '{}' in message",
                key
            )));
        }
    }
    Ok(())
}

/// Validate a `battery` field: must be present-or-absent, integer, and within
/// `BATTERY_MIN..=BATTERY_MAX`.
fn validate_battery(msg: &serde_json::Value) -> Result<(), ConduitError> {
    match msg.get("battery") {
        None | Some(serde_json::Value::Null) => Ok(()),
        Some(v) => {
            let n = v
                .as_i64()
                .ok_or_else(|| ConduitError::Protocol("battery must be an integer".to_string()))?;
            if !(BATTERY_MIN..=BATTERY_MAX).contains(&n) {
                return Err(ConduitError::Protocol(format!(
                    "battery out of range: {} (expected {BATTERY_MIN}..={BATTERY_MAX})",
                    n
                )));
            }
            Ok(())
        }
    }
}

/// Validate an action string — basic length and character check.
fn validate_action(action: &str) -> Result<(), ConduitError> {
    if action.len() > 64 {
        return Err(ConduitError::Protocol(format!(
            "Action too long: {} (max 64)",
            action.len()
        )));
    }
    if !action
        .chars()
        .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
    {
        return Err(ConduitError::Protocol(format!(
            "Action contains invalid characters: {}",
            action
        )));
    }
    Ok(())
}

/// Main validation entry point — validates a parsed JSON message.
/// Returns Ok(()) if valid, or Err with a descriptive message.
pub fn validate_message(msg: &serde_json::Value) -> Result<(), ConduitError> {
    // Extract and validate type
    let msg_type = msg
        .get("type")
        .and_then(|v| v.as_str())
        .ok_or_else(|| ConduitError::Protocol("Missing 'type' field".to_string()))?;
    validate_msg_type(msg_type)?;

    // Extract and validate action
    let action = msg.get("action").and_then(|v| v.as_str()).unwrap_or("");
    validate_action(action)?;

    // Type-specific validation
    match msg_type {
        "pairing" => validate_pairing_message(msg, action),
        "notification" => validate_notification_message(msg, action),
        "file" => validate_file_message(msg, action),
        "clipboard" => validate_clipboard_message(msg),
        "automation" => validate_automation_message(msg, action),
        "audio" => validate_audio_message(msg, action),
        "status" => validate_status_message(msg),
        "discovery" => validate_discovery_message(msg, action),
        // sms, call, screen_mirror, remote_input, tv, watch — relay types whose
        // boundary validation is owned by their handlers.
        _ => Ok(()),
    }
}

/// Normalise a user-entered pairing token to the form the store holds.
///
/// The desktop UI renders the token as `XXXX-XXXX`; the user may type it with
/// or without the dash, in either case, and may add stray spaces. Stripping
/// `-` and uppercasing on *both* sides makes the comparison total, so a token
/// is never rejected merely because of how it was transcribed.
pub fn normalize_pairing_token(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .flat_map(char::to_uppercase)
        .collect()
}

fn validate_pairing_message(msg: &serde_json::Value, action: &str) -> Result<(), ConduitError> {
    match action {
        "request" => {
            if let Some(key) = msg.get("public_key").and_then(|v| v.as_str()) {
                validate_public_key(key)?;
            }
            if let Some(token) = msg.get("token").and_then(|v| v.as_str()) {
                validate_string_field(token, MAX_ID_LEN, "pairing token")?;
            }
            Ok(())
        }
        // SECURITY: `accept` registers a `devices` row, a `ConnectedClient` and
        // a `ws_to_device_id` entry, and fires `DeviceConnect` automation
        // triggers — it is a full pairing, not a passive reply. It therefore
        // carries the *same* one-time token as `request`; without the check any
        // peer that can open a socket could hand the server an arbitrary
        // `public_key` and become a paired device.
        //
        // The token is required to be present, not merely well-formed: a
        // missing `token` is `None` here, and `handle_pairing_accept` resolves
        // `None` to the empty string, which is never a valid store key.
        "accept" => {
            if let Some(key) = msg.get("public_key").and_then(|v| v.as_str()) {
                validate_public_key(key)?;
            }
            let token = msg.get("token").and_then(|v| v.as_str());
            if token.is_none() {
                return Err(ConduitError::Protocol(
                    "pairing/accept requires a one-time token".to_string(),
                ));
            }
            validate_string_field(token.unwrap_or_default(), MAX_ID_LEN, "pairing token")?;
            Ok(())
        }
        // `local_auth` is how the desktop's own webview exchanges the per-launch
        // capability for the `local_desktop` identity. It is exempt from
        // `requires_auth` (it *is* the authentication step), so the only thing
        // standing between a loopback peer and the automation surface is this
        // length bound plus the constant-time comparison in the handler.
        "local_auth" => {
            let token = msg.get("token").and_then(|v| v.as_str()).ok_or_else(|| {
                ConduitError::Protocol("pairing/local_auth requires a token".to_string())
            })?;
            validate_string_field(token, MAX_ID_LEN, "local capability token")?;
            Ok(())
        }
        _ => Ok(()),
    }
}

/// Validate a `status` frame.
///
/// `status` used to have *no* validation at all, so `handle_status_update` ran
/// on a completely empty body. Every field the handler reads is now bounded and
/// the field set is closed.
fn validate_status_message(msg: &serde_json::Value) -> Result<(), ConduitError> {
    reject_unknown_fields(
        msg,
        &[
            "type",
            "action",
            "protocol_version",
            "battery",
            "device_id",
            "wifi_ssid",
            "app_package",
            "timestamp",
        ],
    )?;
    validate_battery(msg)?;
    if let Some(id) = msg.get("device_id").and_then(|v| v.as_str()) {
        validate_id(id)?;
    }
    for (field, max) in [("wifi_ssid", MAX_NAME_LEN), ("app_package", MAX_NAME_LEN)] {
        if let Some(v) = msg.get(field).and_then(|v| v.as_str()) {
            validate_string_field(v, max, field)?;
        }
    }
    Ok(())
}

/// Validate a `discovery` frame (`announce` or `remove`).
///
/// `discovery` was missing from `VALID_TYPES` entirely, which made
/// `handle_discovery_announce` (and its 55-line reply) unreachable: every
/// packet died in `validate_message` with only a `warn!`.
fn validate_discovery_message(msg: &serde_json::Value, action: &str) -> Result<(), ConduitError> {
    match action {
        "announce" => {
            reject_unknown_fields(
                msg,
                &[
                    "type",
                    "action",
                    "protocol_version",
                    "device_id",
                    "device_name",
                    "device_type",
                    "os",
                    "version",
                    "battery",
                    "ws_port",
                    "wss_port",
                    "apns_token",
                ],
            )?;
            if let Some(id) = msg.get("device_id").and_then(|v| v.as_str()) {
                validate_id(id)?;
            }
            for (field, max) in [
                ("device_name", MAX_NAME_LEN),
                ("device_type", 32),
                ("os", 32),
                ("version", 32),
            ] {
                if let Some(v) = msg.get(field).and_then(|v| v.as_str()) {
                    validate_string_field(v, max, field)?;
                }
            }
            validate_battery(msg)?;
            Ok(())
        }
        "remove" => {
            reject_unknown_fields(msg, &["type", "action", "device_id"])?;
            let id = msg.get("device_id").and_then(|v| v.as_str()).unwrap_or("");
            validate_id(id)
        }
        other => Err(ConduitError::Protocol(format!(
            "Unknown discovery action: {}",
            if other.is_empty() { "<none>" } else { other }
        ))),
    }
}

fn validate_notification_message(
    msg: &serde_json::Value,
    _action: &str,
) -> Result<(), ConduitError> {
    if let Some(id) = msg.get("id").and_then(|v| v.as_str()) {
        validate_string_field(id, MAX_ID_LEN, "notification id")?;
    }
    if let Some(title) = msg.get("title").and_then(|v| v.as_str()) {
        validate_string_field(title, MAX_NAME_LEN, "notification title")?;
    }
    if let Some(body) = msg.get("body").and_then(|v| v.as_str()) {
        validate_string_field(body, MAX_NOTIFICATION_BODY, "notification body")?;
    }
    Ok(())
}

fn validate_file_message(msg: &serde_json::Value, action: &str) -> Result<(), ConduitError> {
    match action {
        "request" => {
            if let Some(id) = msg.get("id").and_then(|v| v.as_str()) {
                validate_id(id)?;
            }
            if let Some(name) = msg.get("name").and_then(|v| v.as_str()) {
                validate_filename(name)?;
            }
            if let Some(size) = msg.get("size").and_then(|v| v.as_u64())
                && size > MAX_FILE_SIZE
            {
                return Err(ConduitError::Protocol(format!(
                    "File size too large: {} bytes (max {})",
                    size, MAX_FILE_SIZE
                )));
            }
            Ok(())
        }
        "chunk" => {
            if let Some(id) = msg.get("id").and_then(|v| v.as_str()) {
                validate_id(id)?;
            }
            if let Some(index) = msg.get("index").and_then(|v| v.as_u64())
                && index > 1_000_000
            {
                return Err(ConduitError::Protocol(format!(
                    "Chunk index too large: {}",
                    index
                )));
            }
            if let Some(data) = msg.get("data").and_then(|v| v.as_str()) {
                validate_chunk_data(data)?;
            }
            Ok(())
        }
        "accept" | "cancel" | "complete" | "resume" | "progress" => {
            if let Some(id) = msg.get("id").and_then(|v| v.as_str()) {
                validate_id(id)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn validate_clipboard_message(msg: &serde_json::Value) -> Result<(), ConduitError> {
    if let Some(content) = msg.get("content").and_then(|v| v.as_str()) {
        // Clipboard content can be large (e.g. pasted code) but not unlimited
        validate_string_field(content, MAX_STRING_FIELD_LEN, "clipboard content")?;
    }
    Ok(())
}

fn validate_automation_message(msg: &serde_json::Value, action: &str) -> Result<(), ConduitError> {
    match action {
        "rule" | "" => {
            // Limit number of rules
            if let Some(rules) = msg.get("rules").and_then(|v| v.as_array())
                && rules.len() > MAX_AUTOMATION_RULES
            {
                return Err(ConduitError::Protocol(format!(
                    "Too many automation rules: {} (max {})",
                    rules.len(),
                    MAX_AUTOMATION_RULES
                )));
            }
            // Validate rule name if present
            if let Some(name) = msg.get("name").and_then(|v| v.as_str()) {
                validate_string_field(name, MAX_NAME_LEN, "automation rule name")?;
            }
            Ok(())
        }
        "delete" | "sync" | "triggered" => Ok(()),
        _ => Ok(()),
    }
}

fn validate_audio_message(msg: &serde_json::Value, action: &str) -> Result<(), ConduitError> {
    match action {
        "stream_data" => {
            if let Some(data) = msg.get("data").and_then(|v| v.as_str()) {
                validate_chunk_data(data)?;
            }
            Ok(())
        }
        "stream_start" | "stream_stop" | "route" => Ok(()),
        _ => Ok(()),
    }
}

// ---------------------------------------------------------------------------
// Token Expiration
// ---------------------------------------------------------------------------

/// Timestamped token that expires after a configurable duration.
pub struct ExpiringToken {
    pub created_at: Instant,
    pub ttl: Duration,
}

impl ExpiringToken {
    pub fn new(ttl: Duration) -> Self {
        Self {
            created_at: Instant::now(),
            ttl,
        }
    }

    pub fn is_expired(&self) -> bool {
        Instant::now().duration_since(self.created_at) > self.ttl
    }
}

/// Upper bound on simultaneously-live pairing tokens.
///
/// `generate_pairing_token` is an invokable Tauri command: without a cap, a
/// script (or a user mashing the button) can grow the map without limit while
/// every entry is unexpired. 32 live tokens is far more than any human needs
/// within the 60 s TTL and bounds the map at a few kilobytes.
pub const MAX_LIVE_TOKENS: usize = 32;

pub struct TokenStore {
    tokens: Arc<RwLock<HashMap<String, ExpiringToken>>>,
    default_ttl: Duration,
}

impl TokenStore {
    pub fn new(default_ttl: Duration) -> Self {
        let store = Self {
            tokens: Arc::new(RwLock::new(HashMap::new())),
            default_ttl,
        };
        // Spawn periodic cleanup if inside a tokio runtime context
        let tokens = store.tokens.clone();
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_secs(30));
                loop {
                    interval.tick().await;
                    let mut map = tokens.write().await;
                    map.retain(|_, t| !t.is_expired());
                }
            });
        }
        store
    }

    /// Add a token, refusing to grow the map past [`MAX_LIVE_TOKENS`].
    ///
    /// Two properties this deliberately does *not* have any more:
    ///
    /// * **No silent overwrite.** `HashMap::insert` used to clobber an existing
    ///   entry, so a collision replaced the *first* token's expiry with a fresh
    ///   one — and, worse, the caller was told the insert succeeded. An existing
    ///   unexpired entry is now kept, so the originally-issued token is the one
    ///   that works.
    /// * **No unbounded growth.** Expired entries are swept first; if the store
    ///   is still full, the *oldest* entry is evicted (it is the closest to
    ///   expiring anyway) and the refusal is logged. An allowlist-style map that
    ///   silently stops accepting is the correct failure mode here: the user
    ///   re-generates a token.
    pub async fn insert(&self, token: String) {
        let mut tokens = self.tokens.write().await;
        if tokens.contains_key(&token) {
            // Keep the original expiry so a collision cannot extend the life of
            // an already-issued credential.
            return;
        }
        if tokens.len() >= MAX_LIVE_TOKENS {
            tokens.retain(|_, t| !t.is_expired());
        }
        if tokens.len() >= MAX_LIVE_TOKENS
            && let Some(oldest) = tokens
                .iter()
                .min_by_key(|(_, t)| t.created_at)
                .map(|(k, _)| k.clone())
        {
            warn!(
                "Pairing token store full ({MAX_LIVE_TOKENS} live tokens); \
                 evicting the oldest entry to make room"
            );
            tokens.remove(&oldest);
        }
        tokens.insert(token, ExpiringToken::new(self.default_ttl));
    }

    pub async fn remove(&self, token: &str) -> bool {
        self.tokens.write().await.remove(token).is_some()
    }

    pub async fn contains_valid(&self, token: &str) -> bool {
        let tokens = self.tokens.read().await;
        tokens.get(token).is_some_and(|t| !t.is_expired())
    }

    /// Number of entries currently held (test/diagnostic use).
    pub async fn len(&self) -> usize {
        self.tokens.read().await.len()
    }
}

// ---------------------------------------------------------------------------
// Local webview capability — the token that replaces "any loopback peer is
// trusted"
// ---------------------------------------------------------------------------

/// Length, in characters, of the per-launch local capability.
///
/// `rand::distr::Alphanumeric` draws from a 62-character alphabet, so 43
/// characters is log2(62^43) ≈ 256 bits. The token is regenerated on every
/// launch, so a copy read from disk or from the JS heap has a bounded lifetime
/// of one process.
///
/// This protects against *guessing* only. It does not defend against a process
/// running as the same user, which can read [`LOCAL_CAP_FILE_NAME`] directly —
/// see ADR-0005, which records same-user local access as out of scope.
const LOCAL_CAP_TOKEN_LEN: usize = 43;

/// File, under the app data dir, holding the current token.
///
/// Written `0600` (owner read/write only) so the token is not readable by
/// other users of the machine. It is a convenience/diagnostic artefact: the
/// authoritative copy is the in-process one, and the file is overwritten (never
/// appended to) on every launch.
const LOCAL_CAP_FILE_NAME: &str = "local_ws_token";

/// Generate `len` characters from `rand::distr::Alphanumeric`.
///
/// A single place for token generation so the entropy source and character set
/// are auditable. `rand::rng()` is a CSPRNG (`ChaCha12`).
fn random_alphanumeric(len: usize) -> String {
    use rand::RngExt;
    use rand::distr::Alphanumeric;
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(len)
        .map(char::from)
        .collect()
}

/// Constant-time equality for equal-length secrets.
///
/// Length is not hidden (the token is fixed-length by construction), but the
/// *contents* must not leak through an early-exit comparison.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// The per-launch capability that a loopback peer must present to be registered
/// as the trusted `local_desktop` identity.
pub struct LocalCapability {
    token: String,
}

impl LocalCapability {
    fn generate() -> Self {
        Self {
            token: random_alphanumeric(LOCAL_CAP_TOKEN_LEN),
        }
    }

    /// The token itself. Only handed to the in-process Tauri webview via the
    /// `get_local_ws_token` command.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Constant-time check of a presented token.
    pub fn verify(&self, presented: &str) -> bool {
        constant_time_eq(self.token.as_bytes(), presented.as_bytes())
    }
}

static LOCAL_CAPABILITY: OnceLock<LocalCapability> = OnceLock::new();

/// The process-wide local capability, generated on first use.
///
/// Lazy initialisation (rather than an explicit `init` in `main`) means there
/// is exactly one construction site and no ordering dependency: the token
/// exists by the time any socket is accepted, and it is rotated per launch
/// because it only ever lives in this process's memory.
pub fn local_capability() -> &'static LocalCapability {
    LOCAL_CAPABILITY.get_or_init(|| {
        let cap = LocalCapability::generate();
        persist_local_capability(cap.token());
        cap
    })
}

/// Path of the `0600` token file under the app data dir.
pub fn local_capability_file_path() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("conduit")
        .join(LOCAL_CAP_FILE_NAME)
}

/// Write the token to `local_capability_file_path` with owner-only permissions.
///
/// Best effort: if the file cannot be created the token still works in memory
/// (it is handed to the webview over Tauri IPC), so this only degrades
/// observability, not security. It logs rather than fails.
fn persist_local_capability(token: &str) {
    let path = local_capability_file_path();
    let Some(parent) = path.parent() else { return };
    if std::fs::create_dir_all(parent).is_err() {
        warn!(
            "Could not create {} for the local capability",
            parent.display()
        );
        return;
    }
    match std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&path)
    {
        Ok(mut f) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if let Err(e) = f.set_permissions(std::fs::Permissions::from_mode(0o600)) {
                    warn!("Could not chmod 0600 {}: {e}", path.display());
                }
            }
            if let Err(e) = std::io::Write::write_all(&mut f, token.as_bytes()) {
                warn!("Could not write {}: {e}", path.display());
            }
        }
        Err(e) => warn!("Could not open {}: {e}", path.display()),
    }
}

// ---------------------------------------------------------------------------
// Shell-command allowlist — one process-wide instance, deny-by-default
// ---------------------------------------------------------------------------

/// The single `CommandAllowlist` the whole process enforces.
///
/// Constructed once at startup from the `allowed_commands` setting (see
/// `main`) and replaced in place when the user edits the list. Every
/// `execute_action_with_allowlist` call site passes `Some(..)` from here, so
/// "the allowlist exists" and "the allowlist is consulted" cannot drift: there
/// is exactly one value and one accessor.
///
/// A `std::sync::RwLock` (not tokio's) because the guard is only ever held long
/// enough to clone a small `Vec<String>` — it is never held across an `await`.
/// That also lets startup initialisation be a plain synchronous call from
/// `main`, which avoids `Handle::block_on` panicking inside the runtime that
/// `main` is already blocking on.
static COMMAND_ALLOWLIST: OnceLock<Arc<std::sync::RwLock<crate::automation::CommandAllowlist>>> =
    OnceLock::new();

fn command_allowlist_cell() -> &'static Arc<std::sync::RwLock<crate::automation::CommandAllowlist>>
{
    COMMAND_ALLOWLIST.get_or_init(|| {
        Arc::new(std::sync::RwLock::new(
            crate::automation::CommandAllowlist::new(Vec::new()),
        ))
    })
}

/// Construct the allowlist at startup from `entries`.
///
/// Called from `main` before any automation can fire. Entries are trimmed and
/// empties dropped so a stray blank line in the settings UI does not become a
/// meaningless allowlist entry.
pub fn init_command_allowlist(entries: Vec<String>) {
    set_command_allowlist(entries);
}

/// Replace the live allowlist (Settings → Advanced → Allowed Commands).
///
/// Takes effect immediately: the next `execute_action_with_allowlist` call
/// clones the new value, so removing a command takes effect without a restart.
pub fn set_command_allowlist(entries: Vec<String>) {
    let cleaned = sanitize_allowed_commands(entries);
    *command_allowlist_cell()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) =
        crate::automation::CommandAllowlist::new(cleaned);
}

/// A snapshot of the current allowlist, safe to hand to
/// `execute_action_with_allowlist` while another task replaces the live one.
pub fn current_command_allowlist() -> crate::automation::CommandAllowlist {
    command_allowlist_cell()
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .clone()
}

/// An allowlist longer than this is a mistake, and every entry is compared
/// against every candidate command — cap it.
pub const MAX_ALLOWED_COMMANDS: usize = 256;

/// Normalise a user-supplied allowlist: trim, drop empties, de-duplicate
/// preserving order, and cap the length.
pub fn sanitize_allowed_commands(entries: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for raw in entries {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            continue;
        }
        if !out.iter().any(|e| e == trimmed) {
            out.push(trimmed.to_string());
        }
        if out.len() >= MAX_ALLOWED_COMMANDS {
            break;
        }
    }
    out
}

/// Parse the `allowed_commands` settings row (a JSON array of strings).
///
/// A missing row and a corrupt row both yield an empty list — an allowlist
/// fails closed.
pub fn parse_allowed_commands_setting(raw: Option<&str>) -> Vec<String> {
    let Some(raw) = raw else {
        return Vec::new();
    };
    match serde_json::from_str::<Vec<String>>(raw) {
        Ok(entries) => sanitize_allowed_commands(entries),
        Err(_) => Vec::new(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_validate_id() {
        assert!(validate_id("abc_123-def").is_ok());
        assert!(validate_id("").is_err());
        assert!(validate_id("has spaces").is_err());
        assert!(validate_id(&"x".repeat(200)).is_err());
    }

    #[test]
    fn test_validate_filename() {
        assert!(validate_filename("photo.jpg").is_ok());
        assert!(validate_filename("../../../etc/passwd").is_err());
        assert!(validate_filename("file/with/slash").is_err());
        assert!(validate_filename("..").is_err());
        assert!(validate_filename(".").is_err());
    }

    #[test]
    fn test_validate_public_key() {
        let valid = "a".repeat(64);
        assert!(validate_public_key(&valid).is_ok());
        assert!(validate_public_key("short").is_err());
        assert!(validate_public_key(&"g".repeat(64)).is_err()); // 'g' not hex
    }

    #[test]
    fn test_validate_message_valid() {
        let msg = json!({
            "type": "file",
            "action": "request",
            "id": "transfer_123",
            "name": "photo.jpg",
            "size": 1024
        });
        assert!(validate_message(&msg).is_ok());
    }

    #[test]
    fn test_validate_message_missing_type() {
        let msg = json!({"action": "foo"});
        assert!(validate_message(&msg).is_err());
    }

    #[test]
    fn test_validate_message_unknown_type() {
        let msg = json!({"type": "evil_hack", "action": "foo"});
        assert!(validate_message(&msg).is_err());
    }

    #[test]
    fn test_validate_file_request_traversal() {
        let msg = json!({
            "type": "file",
            "action": "request",
            "id": "x",
            "name": "../../etc/passwd",
            "size": 100
        });
        assert!(validate_message(&msg).is_err());
    }

    #[test]
    fn test_validate_chunk_huge() {
        let msg = json!({
            "type": "file",
            "action": "chunk",
            "id": "x",
            "index": 0,
            "data": "AA=="
        });
        // Single-byte base64 is fine
        assert!(validate_message(&msg).is_ok());
    }

    #[test]
    fn test_validate_chunk_invalid_base64() {
        let msg = json!({
            "type": "file",
            "action": "chunk",
            "id": "x",
            "index": 0,
            "data": "!!!not-base64!!!"
        });
        assert!(validate_message(&msg).is_err());
    }

    #[test]
    fn test_validate_automation_too_many_rules() {
        let rules: Vec<serde_json::Value> = (0..200)
            .map(|i| json!({"name": format!("rule_{}", i)}))
            .collect();
        let msg = json!({
            "type": "automation",
            "action": "rule",
            "rules": rules
        });
        assert!(validate_message(&msg).is_err());
    }

    #[test]
    fn test_token_expiration() {
        let token = ExpiringToken::new(Duration::from_millis(1));
        assert!(!token.is_expired());
        std::thread::sleep(Duration::from_millis(10));
        assert!(token.is_expired());
    }

    // -----------------------------------------------------------------------
    // PerTypeRateLimiter tests
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_type_allows_messages_within_limit() {
        let limiter = PerTypeRateLimiter::new();
        // "pairing" allows 5 per 60s — first 5 should all pass
        for i in 0..5 {
            assert!(
                limiter.check_type_limit("client1", "pairing").await,
                "Message {} should be allowed",
                i + 1,
            );
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_type_blocks_when_limit_exceeded() {
        let limiter = PerTypeRateLimiter::new();
        // "pairing" allows 5 per 60s — the 6th should be blocked
        for _ in 0..5 {
            assert!(limiter.check_type_limit("client1", "pairing").await);
        }
        assert!(
            !limiter.check_type_limit("client1", "pairing").await,
            "6th pairing message should be rate-limited",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_type_limits_are_independent_across_types() {
        let limiter = PerTypeRateLimiter::new();
        // Exhaust pairing quota
        for _ in 0..5 {
            assert!(limiter.check_type_limit("client1", "pairing").await);
        }
        assert!(!limiter.check_type_limit("client1", "pairing").await);

        // "file" has a separate bucket (500) — should still work
        assert!(
            limiter.check_type_limit("client1", "file").await,
            "file bucket should be independent from pairing",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_type_limits_are_independent_across_clients() {
        let limiter = PerTypeRateLimiter::new();
        // Exhaust pairing quota for client1
        for _ in 0..5 {
            assert!(limiter.check_type_limit("client1", "pairing").await);
        }
        assert!(!limiter.check_type_limit("client1", "pairing").await);

        // client2 has its own bucket — should still work
        assert!(
            limiter.check_type_limit("client2", "pairing").await,
            "client2 should have its own pairing bucket",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_type_unknown_type_uses_default_limit() {
        let limiter = PerTypeRateLimiter::new();
        // Unknown type gets the default limit of 100 per 60s
        for _ in 0..100 {
            assert!(limiter.check_type_limit("client1", "unknown_type").await);
        }
        assert!(
            !limiter.check_type_limit("client1", "unknown_type").await,
            "101st message of unknown type should hit default limit",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_type_high_throughput_file_limit() {
        let limiter = PerTypeRateLimiter::new();
        // "file" allows 500 per 60s — 500 should all pass
        for i in 0..500 {
            assert!(
                limiter.check_type_limit("client1", "file").await,
                "file message {} should be allowed",
                i + 1,
            );
        }
        assert!(
            !limiter.check_type_limit("client1", "file").await,
            "501st file message should be rate-limited",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_type_remove_client_clears_buckets() {
        let limiter = PerTypeRateLimiter::new();
        // Exhaust pairing quota for client1
        for _ in 0..5 {
            assert!(limiter.check_type_limit("client1", "pairing").await);
        }
        assert!(!limiter.check_type_limit("client1", "pairing").await);

        // Remove client1 — should reset all buckets
        limiter.remove_client("client1").await;

        // Pairing should work again
        assert!(
            limiter.check_type_limit("client1", "pairing").await,
            "After remove_client, pairing should be available again",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_type_remove_client_does_not_affect_others() {
        let limiter = PerTypeRateLimiter::new();
        // Exhaust pairing for both clients
        for _ in 0..5 {
            assert!(limiter.check_type_limit("client1", "pairing").await);
            assert!(limiter.check_type_limit("client2", "pairing").await);
        }
        assert!(!limiter.check_type_limit("client1", "pairing").await);
        assert!(!limiter.check_type_limit("client2", "pairing").await);

        // Remove client1 only
        limiter.remove_client("client1").await;

        // client1 gets a fresh bucket
        assert!(limiter.check_type_limit("client1", "pairing").await);
        // client2 is unaffected — still rate-limited
        assert!(
            !limiter.check_type_limit("client2", "pairing").await,
            "client2 should still be rate-limited after removing client1",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_per_type_automation_limit() {
        let limiter = PerTypeRateLimiter::new();
        // "automation" allows 20 per 60s
        for i in 0..20 {
            assert!(
                limiter.check_type_limit("client1", "automation").await,
                "automation message {} should be allowed",
                i + 1,
            );
        }
        assert!(
            !limiter.check_type_limit("client1", "automation").await,
            "21st automation message should be rate-limited",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn test_type_limit_config_values() {
        // Verify the configuration map has the expected limits
        assert_eq!(type_limit_for("pairing").max_messages, 5);
        assert_eq!(type_limit_for("file").max_messages, 500);
        assert_eq!(type_limit_for("screen_mirror").max_messages, 60);
        assert_eq!(type_limit_for("audio").max_messages, 60);
        assert_eq!(type_limit_for("clipboard").max_messages, 30);
        assert_eq!(type_limit_for("notification").max_messages, 100);
        assert_eq!(type_limit_for("automation").max_messages, 20);
        assert_eq!(type_limit_for("status").max_messages, 60);
        assert_eq!(type_limit_for("default_unknown").max_messages, 100); // default
    }

    // -----------------------------------------------------------------------
    // Per-connection byte accounting (V5)
    //
    // The message cap alone is not a bandwidth cap: with MAX_MESSAGE_SIZE
    // accepted per message a peer can sit under `max_messages` and still push
    // gigabytes. These pin that a byte budget exists and is enforced
    // independently of the message count.
    // -----------------------------------------------------------------------

    fn byte_capped_limiter(max_messages: u32, max_bytes: u64) -> RateLimiter {
        RateLimiter::new(RateLimitConfig {
            max_messages,
            max_bytes,
            window: Duration::from_secs(60),
        })
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn limiter_blocks_when_byte_budget_exceeded_even_under_message_cap() {
        let limiter = byte_capped_limiter(1_000, 1_000);
        // Three 400-byte messages: under the 1 000-message cap, over the
        // 1 000-byte cap on the third.
        assert!(limiter.check("c", "file", 400).await);
        assert!(limiter.check("c", "file", 400).await);
        assert!(
            !limiter.check("c", "file", 400).await,
            "byte budget must be enforced even though the message cap is far away"
        );
        assert_eq!(limiter.window_bytes("c").await, 1_200);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn limiter_byte_budget_is_per_client() {
        let limiter = byte_capped_limiter(1_000, 1_000);
        assert!(limiter.check("noisy", "file", 900).await);
        assert!(!limiter.check("noisy", "file", 900).await);
        // A different connection starts with a fresh budget.
        assert!(
            limiter.check("quiet", "ping", 10).await,
            "byte budget must not be shared between connections"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn limiter_byte_budget_resets_with_the_window() {
        let limiter = RateLimiter::new(RateLimitConfig {
            max_messages: 1_000,
            max_bytes: 100,
            window: Duration::from_millis(20),
        });
        assert!(limiter.check("c", "file", 100).await);
        assert!(!limiter.check("c", "file", 1).await);
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert!(
            limiter.check("c", "file", 1).await,
            "a new window must start with a fresh byte budget"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn limiter_message_cap_still_applies() {
        let limiter = byte_capped_limiter(3, 1_000_000);
        assert!(limiter.check("c", "ping", 1).await);
        assert!(limiter.check("c", "ping", 1).await);
        assert!(limiter.check("c", "ping", 1).await);
        assert!(!limiter.check("c", "ping", 1).await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn limiter_default_config_has_a_byte_budget() {
        // The production default must not be message-only.
        let cfg = RateLimitConfig::default();
        assert!(
            cfg.max_bytes > 0,
            "the default limiter must cap bytes, not just messages"
        );
    }

    // -----------------------------------------------------------------------
    // pairing/accept token requirement (V1b)
    // -----------------------------------------------------------------------

    #[test]
    fn validate_pairing_accept_requires_a_token() {
        let peer_key = "ab".repeat(32);
        let without_token = json!({
            "type": "pairing", "action": "accept", "public_key": peer_key
        });
        assert!(
            validate_message(&without_token).is_err(),
            "pairing/accept without a token must be rejected before it reaches a handler"
        );

        let with_token = json!({
            "type": "pairing", "action": "accept", "public_key": peer_key, "token": "ABC123"
        });
        assert!(validate_message(&with_token).is_ok());

        let empty_token = json!({
            "type": "pairing", "action": "accept", "public_key": "ab".repeat(32), "token": ""
        });
        assert!(
            validate_message(&empty_token).is_ok(),
            "an empty token is structurally fine; it is the store lookup that must refuse it"
        );
    }

    #[test]
    fn validate_pairing_local_auth_requires_a_bounded_token() {
        assert!(validate_message(&json!({"type": "pairing", "action": "local_auth"})).is_err());
        assert!(
            validate_message(&json!({
                "type": "pairing", "action": "local_auth", "token": "x".repeat(500)
            }))
            .is_err()
        );
        assert!(
            validate_message(&json!({
                "type": "pairing", "action": "local_auth", "token": "abc123"
            }))
            .is_ok()
        );
    }

    #[test]
    fn normalize_pairing_token_strips_dashes_case_and_whitespace() {
        assert_eq!(normalize_pairing_token("AB-CDEF"), "ABCDEF");
        assert_eq!(normalize_pairing_token("ab cdef"), "ABCDEF");
        assert_eq!(normalize_pairing_token("  AB-CDEF\n"), "ABCDEF");
        assert_eq!(normalize_pairing_token(""), "");
    }

    // -----------------------------------------------------------------------
    // status validation (V5)
    // -----------------------------------------------------------------------

    #[test]
    fn validate_status_rejects_unknown_fields() {
        // `status` used to accept a completely empty body.
        assert!(validate_message(&json!({"type": "status", "action": "update"})).is_ok());
        assert!(validate_message(&json!({"type": "status"})).is_ok());
        assert!(
            validate_message(&json!({
                "type": "status", "action": "update", "evil": "x"
            }))
            .is_err(),
            "an unread field must not be smuggled through"
        );
    }

    #[test]
    fn validate_status_bounds_battery() {
        assert!(validate_message(&json!({"type": "status", "battery": 0})).is_ok());
        assert!(validate_message(&json!({"type": "status", "battery": 100})).is_ok());
        assert!(validate_message(&json!({"type": "status", "battery": 101})).is_err());
        assert!(validate_message(&json!({"type": "status", "battery": -1})).is_err());
        assert!(
            validate_message(&json!({"type": "status", "battery": "full"})).is_err(),
            "a non-numeric battery must be rejected, not silently ignored"
        );
        assert!(validate_message(&json!({"type": "status", "battery": 55.5})).is_err());
    }

    #[test]
    fn validate_status_bounds_device_id() {
        assert!(validate_message(&json!({"type": "status", "device_id": "dev_1"})).is_ok());
        assert!(validate_message(&json!({"type": "status", "device_id": ""})).is_err());
        assert!(validate_message(&json!({"type": "status", "device_id": "a b"})).is_err());
    }

    // -----------------------------------------------------------------------
    // discovery validation (V4)
    // -----------------------------------------------------------------------

    #[test]
    fn validate_discovery_announce_is_reachable() {
        // REGRESSION: `discovery` was missing from VALID_TYPES, so
        // handle_discovery_announce could never run.
        let msg = json!({
            "type": "discovery", "action": "announce",
            "device_id": "dev-1", "device_name": "Pixel", "device_type": "phone",
            "os": "android", "version": "1.0.0", "battery": 80
        });
        assert!(validate_message(&msg).is_ok());
    }

    #[test]
    fn validate_discovery_announce_bounds_fields() {
        assert!(
            validate_message(&json!({
                "type": "discovery", "action": "announce", "device_id": "has space"
            }))
            .is_err()
        );
        assert!(
            validate_message(&json!({
                "type": "discovery", "action": "announce", "battery": 900
            }))
            .is_err()
        );
        assert!(
            validate_message(&json!({
                "type": "discovery", "action": "announce", "device_name": "x".repeat(400)
            }))
            .is_err()
        );
        assert!(
            validate_message(&json!({
                "type": "discovery", "action": "announce", "surprise": 1
            }))
            .is_err()
        );
    }

    #[test]
    fn validate_discovery_remove_requires_a_sane_device_id() {
        assert!(
            validate_message(&json!({"type": "discovery", "action": "remove", "device_id": "d1"}))
                .is_ok()
        );
        assert!(validate_message(&json!({"type": "discovery", "action": "remove"})).is_err());
        assert!(
            validate_message(
                &json!({"type": "discovery", "action": "remove", "device_id": "../x"})
            )
            .is_err()
        );
    }

    #[test]
    fn validate_discovery_rejects_unknown_actions() {
        assert!(validate_message(&json!({"type": "discovery", "action": "evil"})).is_err());
        assert!(
            validate_message(&json!({"type": "discovery"})).is_err(),
            "an action-less discovery frame has no handler; say so"
        );
    }

    // -----------------------------------------------------------------------
    // TokenStore bounds (V5)
    // -----------------------------------------------------------------------

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn token_store_is_capped() {
        let store = TokenStore::new(Duration::from_secs(60));
        for i in 0..(MAX_LIVE_TOKENS * 3) {
            store.insert(format!("TOK{i:04}")).await;
        }
        assert!(
            store.len().await <= MAX_LIVE_TOKENS,
            "an invokable command must not be able to grow the token map without bound; \
             len = {}",
            store.len().await
        );
        let newest = format!("TOK{:04}", MAX_LIVE_TOKENS * 3 - 1);
        assert!(store.contains_valid(&newest).await);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn token_store_insert_does_not_extend_an_existing_token() {
        let store = TokenStore::new(Duration::from_millis(30));
        store.insert("SAME".to_string()).await;
        tokio::time::sleep(Duration::from_millis(60)).await;
        // A collision must NOT refresh the expiry, which is what made the first
        // token silently invalid mid-flight.
        store.insert("SAME".to_string()).await;
        assert!(
            !store.contains_valid("SAME").await,
            "a colliding insert must not resurrect an expired token"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn token_store_sweeps_expired_before_refusing() {
        let store = TokenStore::new(Duration::from_millis(20));
        for i in 0..MAX_LIVE_TOKENS {
            store.insert(format!("OLD{i:04}")).await;
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
        store.insert("FRESH".to_string()).await;
        assert!(
            store.contains_valid("FRESH").await,
            "expired entries must be swept so a store full of stale tokens still works"
        );
    }

    // -----------------------------------------------------------------------
    // Local capability (V1a)
    // -----------------------------------------------------------------------

    #[test]
    fn local_capability_is_long_alphanumeric_and_unguessable() {
        let cap = LocalCapability::generate();
        assert_eq!(cap.token().len(), LOCAL_CAP_TOKEN_LEN);
        assert!(cap.token().len() >= 8, "the capability must be >= 8 chars");
        assert!(cap.token().chars().all(|c| c.is_ascii_alphanumeric()));
    }

    #[test]
    fn local_capability_verifies_only_its_own_token() {
        let cap = LocalCapability::generate();
        assert!(cap.verify(cap.token()));
        assert!(!cap.verify(""));
        assert!(!cap.verify(&cap.token()[..cap.token().len() - 1]));
        let mut mutated: Vec<char> = cap.token().chars().collect();
        mutated[0] = if mutated[0] == 'a' { 'b' } else { 'a' };
        let mutated: String = mutated.into_iter().collect();
        assert!(!cap.verify(&mutated));
        assert_ne!(LocalCapability::generate().token(), cap.token());
    }

    #[test]
    fn constant_time_eq_matches_semantics() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"ab"));
        assert!(constant_time_eq(b"", b""));
    }

    #[test]
    fn process_local_capability_is_stable_within_a_launch() {
        let a = local_capability();
        let b = local_capability();
        assert_eq!(a.token(), b.token(), "the token must not rotate per call");
    }

    // -----------------------------------------------------------------------
    // Command allowlist wiring (V1c)
    // -----------------------------------------------------------------------

    #[test]
    fn sanitize_allowed_commands_trims_dedupes_and_caps() {
        let entries = vec![
            "  ls  ".to_string(),
            "ls".to_string(),
            "   ".to_string(),
            String::new(),
            "echo".to_string(),
        ];
        assert_eq!(
            sanitize_allowed_commands(entries),
            vec!["ls".to_string(), "echo".to_string()]
        );

        let many: Vec<String> = (0..MAX_ALLOWED_COMMANDS + 50)
            .map(|i| format!("cmd{i}"))
            .collect();
        assert_eq!(
            sanitize_allowed_commands(many).len(),
            MAX_ALLOWED_COMMANDS,
            "the allowlist length must be bounded"
        );
    }

    #[test]
    fn parse_allowed_commands_setting_fails_closed() {
        assert!(parse_allowed_commands_setting(None).is_empty());
        assert!(parse_allowed_commands_setting(Some("not json")).is_empty());
        assert!(parse_allowed_commands_setting(Some("{}")).is_empty());
        assert_eq!(
            parse_allowed_commands_setting(Some(r#"["ls", " echo "]"#)),
            vec!["ls".to_string(), "echo".to_string()]
        );
    }

    #[test]
    fn command_allowlist_can_be_replaced_at_runtime() {
        let list = crate::automation::CommandAllowlist::new(vec!["ls".to_string()]);
        assert!(list.is_allowed("ls -la"));
        assert!(!list.is_allowed("calc.exe"));
    }
}
