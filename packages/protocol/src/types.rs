//! Canonical wire-protocol message types for the Conduit ecosystem.
//!
//! Every JSON message exchanged between desktop, mobile, and relay has a
//! corresponding Rust struct or enum here. The `#[serde(tag = "type",
//! content = "action")]` or `#[serde(tag = "action")]` attributes match
//! the `(type, action)` dispatch used in `server/mod.rs`.
//!
//! # Versioning
//!
//! All messages carry an optional `protocol_version` field. The current
//! version is [`PROTOCOL_VERSION`]. Clients MUST reject messages with a
//! higher version than they support.

use serde::{Deserialize, Serialize};

/// Current protocol version. Bump when the wire format changes incompatibly.
pub const PROTOCOL_VERSION: u32 = 1;

// ───────────────────────────────────────────────────────────────────────────
//  LAN transport ports
// ───────────────────────────────────────────────────────────────────────────
//
//  The desktop binds **two** WebSocket listeners. They are not
//  interchangeable, and mixing them up breaks pairing in a way that looks like
//  a certificate problem rather than a port problem:
//
//  * [`LAN_WS_PORT`] is **plaintext** (`ws://`). Nothing on it is encrypted in
//    transit and no certificate is presented.
//  * [`LAN_WSS_PORT`] is **TLS** (`wss://`), self-signed. This is the only
//    port the mobile app dials for a LAN peer, because the certificate-pin
//    trust bootstrap (`WebSocketService._verifyCertificatePin`) only ever runs
//    on the `wss://` code path.
//
//  A `wss://` handshake against [`LAN_WS_PORT`] throws, so the pin is never
//  captured and every later connection fails closed. That was the mobile
//  pairing bug; see PROTOCOL.md §2 and §2.2.
//
//  These are the *only* definitions of these port numbers in the Rust
//  workspace. `main.rs` re-exports them as `WS_PORT` / `WSS_PORT`, and the
//  Dart client mirrors them as `kLanWsPort` / `kLanWssPort` in
//  `apps/mobile/lib/services/websocket_service.dart`. Tests in this module
//  assert all three representations (Rust, PROTOCOL.md, Dart) agree.

/// Desktop's **plaintext** (`ws://`) LAN WebSocket listener.
///
/// Documented in PROTOCOL.md §2 as "WS (LAN)". Never dial this with `wss://`.
pub const LAN_WS_PORT: u16 = 9527;

/// Desktop's **TLS** (`wss://`) LAN WebSocket listener.
///
/// Documented in PROTOCOL.md §2 as "WSS (LAN)" and §2.2 as the only port the
/// mobile app dials for a LAN peer.
pub const LAN_WSS_PORT: u16 = 9531;

// ───────────────────────────────────────────────────────────────────────────
//  Envelope
// ───────────────────────────────────────────────────────────────────────────

/// Top-level enum covering every message type on the wire.
///
/// Discriminated by `type` (and `action` where applicable).
/// Deserialization uses an untagged approach so each variant is a clean struct;
/// dispatch logic matches `type` + `action` strings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum WireMessage {
    /// Encrypted wrapper carrying ciphertext instead of plaintext.
    Encrypted(EncryptedEnvelope),
    /// All plaintext messages are represented as raw JSON values so that
    /// the relay can forward them without needing every variant.
    Plaintext(serde_json::Value),
}

// ───────────────────────────────────────────────────────────────────────────
//  Discovery
// ───────────────────────────────────────────────────────────────────────────

/// `type: "discovery", action: "announce"`
///
/// Sent by both mobile and desktop immediately after connecting.
/// The receiver replies with its own announce, giving both sides device info.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryAnnounce {
    #[serde(rename = "type")]
    pub msg_type: String, // "discovery"
    #[serde(rename = "action")]
    pub action: String, // "announce"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u32>,
    pub device_id: String,
    pub device_name: String,
    /// "phone" | "tablet" | "desktop"
    pub device_type: String,
    /// "ios" | "android" | "windows" | "macos" | "linux"
    pub os: String,
    pub version: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ws_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wss_port: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apns_token: Option<String>,
}

/// `type: "discovery", action: "remove"`
///
/// Notifies peers that a device is no longer available.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoveryRemove {
    #[serde(rename = "type")]
    pub msg_type: String, // "discovery"
    #[serde(rename = "action")]
    pub action: String, // "remove"
    pub device_id: String,
}

// ───────────────────────────────────────────────────────────────────────────
//  Pairing
// ───────────────────────────────────────────────────────────────────────────

/// `type: "pairing", action: "request"`
///
/// Mobile sends this after scanning the desktop's QR code.
/// Contains a one-time token and the mobile's X25519 public key.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairingRequest {
    #[serde(rename = "type")]
    pub msg_type: String, // "pairing"
    #[serde(rename = "action")]
    pub action: String, // "request"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u32>,
    /// One-time pairing token (dashes may be stripped by the receiver).
    pub token: String,
    /// X25519 public key, hex-encoded.
    pub public_key: String,
    /// Device metadata for the desktop to store.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_info: Option<DeviceInfo>,
}

/// `type: "pairing", action: "accept"`
///
/// Desktop replies with its X25519 public key after validating the token.
/// The shared secret is derived from the local private key + peer public key.
///
/// [`PairingAccept::device_id`] is the identifier the desktop has assigned to
/// the *pairing* peer. The peer cannot choose it and cannot derive it: the
/// desktop picks a `devices.id` (a per-connection UUID the first time, the
/// stored `devices.id` on every re-pair with the same public key) and this
/// field is the only way the peer learns it.
///
/// It matters because the peer must stamp that value into `source_device` on
/// every `encrypted` envelope it sends — the desktop resolves the shared
/// secret by that field when it opens the envelope. A peer that does not know
/// its own id cannot send a single encrypted frame. This is optional on the
/// wire so a pre-existing peer keeps parsing; the desktop always sets it.
///
/// [`PairingAccept::relay_url`], [`PairingAccept::relay_token`] and
/// [`PairingAccept::relay_cert_pin`] describe the relay the *sender* hosts, for
/// a peer that is on a different network and cannot reach the desktop directly.
/// They are the desktop's own knowledge — the address it listens on, the bearer
/// token it generated, and the pin of the certificate the relay itself serves —
/// so the peer cannot derive any of them and has to be told. All three are
/// optional and **absence is normal**: a sender that does not host a relay, or
/// whose relay is not running, omits them, and the peer keeps whatever relay
/// configuration it already had.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairingAccept {
    #[serde(rename = "type")]
    pub msg_type: String, // "pairing"
    #[serde(rename = "action")]
    pub action: String, // "accept"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u32>,
    /// Desktop's X25519 public key, hex-encoded.
    pub public_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_info: Option<DeviceInfo>,
    /// The `devices.id` the desktop has assigned to the peer.
    ///
    /// The peer must store this and send it as `source_device` on every
    /// `encrypted` envelope. `None` means the sender is a desktop too old to
    /// assign ids, in which case the receiver falls back to the
    /// connection-derived identity.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// The desktop's own `device_id`.
    ///
    /// The peer needs it for the same reason the desktop needs one: a relayed
    /// v2 frame is verified under the *sender's* route key, and that key is
    /// derived with the sender's id bound into the label. A phone that does not
    /// know the hub's id cannot derive the key that verifies a relayed file
    /// chunk, so it would have to drop every one of them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hub_device_id: Option<String>,
    /// `wss://` URL of the relay this desktop hosts, for a peer that cannot
    /// reach the desktop directly.
    ///
    /// It names the **TLS** listener on the **relay** port, never the
    /// loopback-only plaintext listener the desktop uses to join its own relay:
    /// a peer is on another machine, so the plaintext listener is unreachable
    /// and carries the bearer token in the clear. The host is the sender's
    /// configured relay hostname when it has one, and its LAN address
    /// otherwise — never a loopback address, which a peer can never dial.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_url: Option<String>,
    /// Bearer token this relay's clients present during `relay_auth`.
    ///
    /// Generated and stored by the desktop, so the peer cannot derive it. It is
    /// only ever sent to a peer that has just presented a valid one-time pairing
    /// token, i.e. one that completed pairing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_token: Option<String>,
    /// `sha256/<base64>` SPKI pin of the certificate **the relay** serves.
    ///
    /// The relay generates its own TLS material, so this is not the hub's pin
    /// and must not be stored in the slot a receiver keeps for the hub's: a
    /// peer that accepted it for the hub would pin the wrong certificate
    /// everywhere else. `None` when the relay serves no certificate.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub relay_cert_pin: Option<String>,
}

/// `type: "pairing", action: "revoke"`
///
/// Sent by the desktop when it revokes a paired device.
/// The mobile must disconnect and clear stored credentials.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PairingRevoke {
    #[serde(rename = "type")]
    pub msg_type: String, // "pairing"
    #[serde(rename = "action")]
    pub action: String, // "revoke"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

// ───────────────────────────────────────────────────────────────────────────
//  Clipboard
// ───────────────────────────────────────────────────────────────────────────

/// `type: "clipboard", action: "sync"`
///
/// Bidirectional clipboard content sharing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipboardSync {
    #[serde(rename = "type")]
    pub msg_type: String, // "clipboard"
    #[serde(rename = "action")]
    pub action: String, // "sync"
    pub content: String,
    /// MIME type: "text/plain", "text/html", "image/png", etc.
    pub mime: String,
    pub source_device: String,
    /// Unix timestamp (seconds).
    pub timestamp: i64,
}

/// `type: "clipboard", action: "request"`
///
/// Request clipboard content from a peer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClipboardRequest {
    #[serde(rename = "type")]
    pub msg_type: String, // "clipboard"
    #[serde(rename = "action")]
    pub action: String, // "request"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
}

// ───────────────────────────────────────────────────────────────────────────
//  Notification
// ───────────────────────────────────────────────────────────────────────────

/// `type: "notification", action: "post"`
///
/// Mobile pushes a notification to the desktop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotificationPost {
    #[serde(rename = "type")]
    pub msg_type: String, // "notification"
    #[serde(rename = "action")]
    pub action: String, // "post"
    pub id: String,
    pub device_id: String,
    pub app: String,
    pub title: String,
    pub body: String,
    pub timestamp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<String>>,
}

/// `type: "notification", action: "dismiss"`
///
/// User dismissed a notification (bidirectional).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotificationDismiss {
    #[serde(rename = "type")]
    pub msg_type: String, // "notification"
    #[serde(rename = "action")]
    pub action: String, // "dismiss"
    pub id: String,
}

/// `type: "notification", action: "mark_read"`
///
/// User marked a notification as read (bidirectional).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotificationMarkRead {
    #[serde(rename = "type")]
    pub msg_type: String, // "notification"
    #[serde(rename = "action")]
    pub action: String, // "mark_read"
    pub id: String,
}

/// `type: "notification", action: "reply"`
///
/// User replied to a notification from the desktop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotificationReply {
    #[serde(rename = "type")]
    pub msg_type: String, // "notification"
    #[serde(rename = "action")]
    pub action: String, // "reply"
    pub id: String,
    pub text: String,
}

// ───────────────────────────────────────────────────────────────────────────
//  File Transfer
// ───────────────────────────────────────────────────────────────────────────

/// `type: "file", action: "request"`
///
/// Initiates a file transfer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileRequest {
    #[serde(rename = "type")]
    pub msg_type: String, // "file"
    #[serde(rename = "action")]
    pub action: String, // "request"
    pub id: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub from: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

/// `type: "file", action: "accept"`
///
/// Receiver accepts the incoming file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileAccept {
    #[serde(rename = "type")]
    pub msg_type: String, // "file"
    #[serde(rename = "action")]
    pub action: String, // "accept"
    pub id: String,
}

/// `type: "file", action: "chunk"`
///
/// Base64-encoded file chunk (JSON path, for smaller files).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileChunk {
    #[serde(rename = "type")]
    pub msg_type: String, // "file"
    #[serde(rename = "action")]
    pub action: String, // "chunk"
    pub id: String,
    pub index: u32,
    /// Base64-encoded chunk data.
    pub data: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,
}

/// `type: "file", action: "progress"`
///
/// Transfer progress update (broadcast to other clients).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileProgress {
    #[serde(rename = "type")]
    pub msg_type: String, // "file"
    #[serde(rename = "action")]
    pub action: String, // "progress"
    pub id: String,
    /// 0..100
    pub percent: u32,
}

/// `type: "file", action: "complete"`
///
/// File transfer finished.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileComplete {
    #[serde(rename = "type")]
    pub msg_type: String, // "file"
    #[serde(rename = "action")]
    pub action: String, // "complete"
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// `type: "file", action: "cancel"`
///
/// Either side cancels an in-progress transfer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileCancel {
    #[serde(rename = "type")]
    pub msg_type: String, // "file"
    #[serde(rename = "action")]
    pub action: String, // "cancel"
    pub id: String,
}

/// `type: "file", action: "resume"`
///
/// Request to resume an interrupted transfer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileResume {
    #[serde(rename = "type")]
    pub msg_type: String, // "file"
    #[serde(rename = "action")]
    pub action: String, // "resume"
    pub id: String,
    pub name: String,
    pub size: u64,
    pub mime: String,
    pub from: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checksum: Option<String>,
}

/// `type: "file", action: "resume_ack"`
///
/// Acknowledges a resume request with how many chunks were loaded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileResumeAck {
    #[serde(rename = "type")]
    pub msg_type: String, // "file"
    #[serde(rename = "action")]
    pub action: String, // "resume_ack"
    pub id: String,
    pub chunks_loaded: u32,
}

/// Metadata header inside the binary file-chunk frame (v1, direct LAN).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BinaryFileMetadata {
    pub id: String,
    pub index: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<u32>,
}

// ───────────────────────────────────────────────────────────────────────────
//  Audio
// ───────────────────────────────────────────────────────────────────────────

/// `type: "audio", action: "stream_start"`
///
/// Mobile asks the desktop to begin capturing and streaming system audio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioStreamStart {
    #[serde(rename = "type")]
    pub msg_type: String, // "audio"
    #[serde(rename = "action")]
    pub action: String, // "stream_start"
}

/// `type: "audio", action: "stream_stop"`
///
/// Stops the audio capture stream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioStreamStop {
    #[serde(rename = "type")]
    pub msg_type: String, // "audio"
    #[serde(rename = "action")]
    pub action: String, // "stream_stop"
}

/// `type: "audio", action: "stream_started"`
///
/// Desktop confirms capture is active.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioStreamStarted {
    #[serde(rename = "type")]
    pub msg_type: String, // "audio"
    #[serde(rename = "action")]
    pub action: String, // "stream_started"
    pub from: String,
}

/// `type: "audio", action: "stream_data"`
///
/// PCM-16 audio data, base64-encoded.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioStreamData {
    #[serde(rename = "type")]
    pub msg_type: String, // "audio"
    #[serde(rename = "action")]
    pub action: String, // "stream_data"
    /// Base64-encoded PCM-16 LE samples.
    pub data: String,
    /// Always "pcm16".
    pub format: String,
    /// Sample rate in Hz (e.g. 16000).
    pub sample_rate: u32,
    /// Channel count (1 = mono, 2 = stereo).
    pub channels: u32,
    pub from: String,
}

/// `type: "audio", action: "playback_start"`
///
/// Mobile requests desktop to start receiving audio for duplex playback.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioPlaybackStart {
    #[serde(rename = "type")]
    pub msg_type: String, // "audio"
    #[serde(rename = "action")]
    pub action: String, // "playback_start"
}

/// `type: "audio", action: "playback_stop"`
///
/// Stops duplex playback.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioPlaybackStop {
    #[serde(rename = "type")]
    pub msg_type: String, // "audio"
    #[serde(rename = "action")]
    pub action: String, // "playback_stop"
}

/// `type: "audio", action: "playback_started"`
///
/// Desktop confirms playback is active.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioPlaybackStarted {
    #[serde(rename = "type")]
    pub msg_type: String, // "audio"
    #[serde(rename = "action")]
    pub action: String, // "playback_started"
    pub from: String,
}

/// `type: "audio", action: "playback_stopped"`
///
/// Desktop confirms playback has stopped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioPlaybackStopped {
    #[serde(rename = "type")]
    pub msg_type: String, // "audio"
    #[serde(rename = "action")]
    pub action: String, // "playback_stopped"
    pub from: String,
}

/// `type: "audio", action: "playback_data"`
///
/// PCM-16 data destined for the desktop speaker output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioPlaybackData {
    #[serde(rename = "type")]
    pub msg_type: String, // "audio"
    #[serde(rename = "action")]
    pub action: String, // "playback_data"
    pub data: String,
    pub format: String,
    pub sample_rate: u32,
    pub channels: u32,
}

// ───────────────────────────────────────────────────────────────────────────
//  Screen Mirror
// ───────────────────────────────────────────────────────────────────────────

/// `type: "screen_mirror", action: "start"`
///
/// Requests that the *capture* side begin streaming frames to the requester.
///
/// Direction is decided by the session table, not by this message: whoever
/// sends `start` is the viewer, and the device named in the `device_id`
/// routing envelope is the capture side. The Android capture pipeline clamps
/// `fps` to 1..=30.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenMirrorStart {
    #[serde(rename = "type")]
    pub msg_type: String, // "screen_mirror"
    #[serde(rename = "action")]
    pub action: String, // "start"
    /// "low" | "medium" | "high"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    /// Requested capture frame rate. Clamped by the capture side.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<u32>,
}

/// `type: "screen_mirror", action: "stop"`
///
/// Stops screen capture.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenMirrorStop {
    #[serde(rename = "type")]
    pub msg_type: String, // "screen_mirror"
    #[serde(rename = "action")]
    pub action: String, // "stop"
}

/// `type: "screen_mirror", action: "frame"`
///
/// A single JPEG frame from the desktop screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenMirrorFrame {
    #[serde(rename = "type")]
    pub msg_type: String, // "screen_mirror"
    #[serde(rename = "action")]
    pub action: String, // "frame"
    /// Base64-encoded JPEG image.
    pub data: String,
    pub format: String, // "jpeg"
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from_desktop: Option<bool>,
}

/// `type: "screen_mirror", action: "capture_stopped"`
///
/// Desktop confirms capture has stopped.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenMirrorCaptureStopped {
    #[serde(rename = "type")]
    pub msg_type: String, // "screen_mirror"
    #[serde(rename = "action")]
    pub action: String, // "capture_stopped"
}

/// `type: "screen_mirror", action: "touch"`
///
/// Touch/click input injection on the desktop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenMirrorTouch {
    #[serde(rename = "type")]
    pub msg_type: String, // "screen_mirror"
    #[serde(rename = "action")]
    pub action: String, // "touch"
    pub x: f64,
    pub y: f64,
    /// "tap" | "double_tap" | "long_press" | "right_click" | "move"
    ///
    /// `x` / `y` are relative (0.0..=1.0) screen coordinates; the injection
    /// side scales them by its own display, so out-of-range values are
    /// clamped there rather than rejected here.
    #[serde(
        rename = "actionType",
        default,
        skip_serializing_if = "Option::is_none",
        with = "validate_touch_action_type"
    )]
    pub action_type: Option<String>,
}

/// `type: "screen_mirror", action: "key"`
///
/// Keyboard input injection on the desktop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenMirrorKey {
    #[serde(rename = "type")]
    pub msg_type: String, // "screen_mirror"
    #[serde(rename = "action")]
    pub action: String, // "key"
    pub key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifiers: Option<Vec<String>>,
}

/// `type: "screen_mirror", action: "scroll"`
///
/// Scroll input injection on the desktop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScreenMirrorScroll {
    #[serde(rename = "type")]
    pub msg_type: String, // "screen_mirror"
    #[serde(rename = "action")]
    pub action: String, // "scroll"
    pub dx: f64,
    pub dy: f64,
}

// ───────────────────────────────────────────────────────────────────────────
//  Remote Input (standalone mouse control)
// ───────────────────────────────────────────────────────────────────────────

/// `type: "remote_input", action: "move"`
///
/// Relative mouse movement.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteInputMove {
    #[serde(rename = "type")]
    pub msg_type: String, // "remote_input"
    #[serde(rename = "action")]
    pub action: String, // "move"
    pub dx: f64,
    pub dy: f64,
}

/// `type: "remote_input", action: "click"`
///
/// Mouse button click.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteInputClick {
    #[serde(rename = "type")]
    pub msg_type: String, // "remote_input"
    #[serde(rename = "action")]
    pub action: String, // "click"
    /// "left" | "right" | "double_left" | "middle"
    pub button: String,
}

/// `type: "remote_input", action: "scroll"`
///
/// Scroll wheel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteInputScroll {
    #[serde(rename = "type")]
    pub msg_type: String, // "remote_input"
    #[serde(rename = "action")]
    pub action: String, // "scroll"
    pub dx: f64,
    pub dy: f64,
}

/// `type: "remote_input", action: "key"`
///
/// Keyboard input: one complete key chord (modifiers + key), pressed and
/// released as a unit.
///
/// There is deliberately no `key_down` / `key_up` pair and no free-text
/// message: the receiver is a machine, not a text field, and a `key` chord is
/// the smallest unit that can be injected safely. Text entry is expressed as a
/// series of `key` chords.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RemoteInputKey {
    #[serde(rename = "type")]
    pub msg_type: String, // "remote_input"
    #[serde(rename = "action")]
    pub action: String, // "key"
    /// DOM `KeyboardEvent.key` value: a single character, or a key name
    /// (`"Enter"`, `"ArrowLeft"`, `"F5"`, ...).
    ///
    /// Receivers cap the accepted length; a longer string is a protocol
    /// violation, not text to type.
    pub key: String,
    /// "shift" | "control" | "alt" | "meta" (schema `modifiers` enum).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub modifiers: Option<Vec<String>>,
}

// ───────────────────────────────────────────────────────────────────────────
//  Automation
// ───────────────────────────────────────────────────────────────────────────

/// `type: "automation", action: "rule"`
///
/// Upserts a single automation rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationRuleMessage {
    #[serde(rename = "type")]
    pub msg_type: String, // "automation"
    #[serde(rename = "action")]
    pub action: String, // "rule"
    pub id: String,
    pub name: String,
    pub trigger: AutomationTrigger,
    #[serde(rename = "rule_action")]
    pub rule_action: AutomationActionPayload,
    pub enabled: bool,
}

/// `type: "automation", action: "delete"`
///
/// Deletes an automation rule by ID.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationDelete {
    #[serde(rename = "type")]
    pub msg_type: String, // "automation"
    #[serde(rename = "action")]
    pub action: String, // "delete"
    #[serde(rename = "rule_id", alias = "id")]
    pub rule_id: String,
}

/// `type: "automation", action: "sync"`
///
/// Bulk sync of automation rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationSync {
    #[serde(rename = "type")]
    pub msg_type: String, // "automation"
    #[serde(rename = "action")]
    pub action: String, // "sync"
    pub rules: Vec<serde_json::Value>,
    /// When true, rules not present in `rules` are deleted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_sync: Option<bool>,
}

/// `type: "automation", action: "triggered"`
///
/// Notification that a rule was triggered.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationTriggered {
    #[serde(rename = "type")]
    pub msg_type: String, // "automation"
    #[serde(rename = "action")]
    pub action: String, // "triggered"
    pub id: String,
    pub trigger_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

// ───────────────────────────────────────────────────────────────────────────
//  Call
// ───────────────────────────────────────────────────────────────────────────

/// `type: "call"`
///
/// Call signaling (incoming, outgoing, missed, accept, decline, end).
///
/// The `action` field is an open string rather than a closed enum because the
/// two clients disagree on the verb set: the phone emits `incoming` /
/// `forward` (`call_service.dart`) while the desktop emits `answer` /
/// `reject` (`useCalls.ts`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CallMessage {
    #[serde(rename = "type")]
    pub msg_type: String, // "call"
    pub action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub to_device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub route: Option<String>,
    /// Subscriber number of the far end, E.164 formatted.
    ///
    /// Sent by the phone for `action: "incoming"`; read by both receivers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub number: Option<String>,
    /// Address-book display name for `number`. Empty string when unresolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Originating device ID. Read by the desktop for `action: "incoming"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
}

// ───────────────────────────────────────────────────────────────────────────
//  SMS
// ───────────────────────────────────────────────────────────────────────────

/// `type: "sms", action: "send"`
///
/// Desktop requests mobile to send an SMS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmsSend {
    #[serde(rename = "type")]
    pub msg_type: String, // "sms"
    #[serde(rename = "action")]
    pub action: String, // "send"
    pub to: String,
    pub body: String,
}

/// `type: "sms", action: "sync"`
///
/// Full SMS thread snapshot pushed by the phone to its peers.
///
/// Emitted by `SmsService.syncToDesktop()` and consumed by the desktop
/// (`useSms.ts`, `action: "sync"`) and by other phones
/// (`SmsService.handleSync`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmsSync {
    #[serde(rename = "type")]
    pub msg_type: String, // "sms"
    #[serde(rename = "action")]
    pub action: String, // "sync"
    pub threads: Vec<SmsThread>,
}

/// `type: "sms", action: "new"`
///
/// A single newly received SMS, forwarded by the phone to its peers.
///
/// # Shape divergence (unresolved — see PROTOCOL.md)
///
/// The two ends of this message currently disagree, and the schema accepts
/// both variants rather than picking a winner:
///
/// * **Sent as** `{from, body, timestamp}` by `SmsService._startIncomingSmsListener`.
/// * **Read as** `{thread_id, message}` by the desktop (`useSms.ts`) and by
///   `main.dart`'s `sms` handler calling `handleNewMessage`.
///
/// Because the desktop blindly relays `("sms", _)`, a `sms/new` from one phone
/// is delivered to every other peer, so both variants must stay valid.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmsNew {
    #[serde(rename = "type")]
    pub msg_type: String, // "sms"
    #[serde(rename = "action")]
    pub action: String, // "new"
    /// Sender address, as emitted by the phone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// Message body, as emitted by the phone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Unix timestamp (seconds) at which the phone received the SMS.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
    /// Thread the message belongs to, as expected by the receiving peers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Nested message record, as expected by the receiving peers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<SmsMessage>,
}

/// `type: "sms", action: "sent"`
///
/// Confirmation that the phone successfully sent an SMS, so the desktop can
/// reconcile its own outbox.
///
/// Emitted by `SmsService.sendSms()` on the native `sendSms` success path.
/// Carries the same sender/receiver shape divergence as [`SmsNew`]: sent as
/// `{to, body, timestamp}`, read as `{thread_id, message}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmsSent {
    #[serde(rename = "type")]
    pub msg_type: String, // "sms"
    #[serde(rename = "action")]
    pub action: String, // "sent"
    /// Destination number, as emitted by the phone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to: Option<String>,
    /// Message body, as emitted by the phone.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Unix timestamp (seconds) at which the phone handed the SMS to the radio.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
    /// Thread the message belongs to, as expected by the receiving peers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    /// Nested message record, as expected by the receiving peers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<SmsMessage>,
}

// ───────────────────────────────────────────────────────────────────────────
//  Status / Heartbeat
// ───────────────────────────────────────────────────────────────────────────

/// `type: "status", action: "update"`
///
/// Periodic status update (battery, wifi, etc.).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StatusUpdate {
    #[serde(rename = "type")]
    pub msg_type: String, // "status"
    #[serde(rename = "action")]
    pub action: String, // "update"
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "validate_battery_pct"
    )]
    pub battery: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wifi_ssid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_info: Option<DeviceInfo>,
}

/// `type: "ping"` / `type: "pong"`
///
/// Connection keep-alive. Sent every 25 s; timeout after 60 s.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ping {
    #[serde(rename = "type")]
    pub msg_type: String, // "ping"
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Pong {
    #[serde(rename = "type")]
    pub msg_type: String, // "pong"
}

// ───────────────────────────────────────────────────────────────────────────
//  Error
// ───────────────────────────────────────────────────────────────────────────

/// `type: "error"`
///
/// Server-side error response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ErrorMessage {
    #[serde(rename = "type")]
    pub msg_type: String, // "error"
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub server_version: Option<u32>,
}

// ───────────────────────────────────────────────────────────────────────────
//  Relay
// ───────────────────────────────────────────────────────────────────────────

/// `type: "relay_auth"`
///
/// First message a client sends to the relay to authenticate.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelayAuth {
    #[serde(rename = "type")]
    pub msg_type: String, // "relay_auth"
    pub device_id: String,
    pub relay_token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub apns_token: Option<String>,
}

/// `type: "relay_auth_ok"`
///
/// Relay confirms successful authentication.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelayAuthOk {
    #[serde(rename = "type")]
    pub msg_type: String, // "relay_auth_ok"
}

impl RelayAuthOk {
    pub fn new() -> Self {
        Self {
            msg_type: "relay_auth_ok".into(),
        }
    }
}

impl Default for RelayAuthOk {
    fn default() -> Self {
        Self::new()
    }
}

/// `type: "relay_auth_rejected"`
///
/// Relay rejects authentication.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelayAuthRejected {
    #[serde(rename = "type")]
    pub msg_type: String, // "relay_auth_rejected"
    #[serde(with = "validate_relay_reject_reason")]
    pub reason: String, // "invalid_token" | "missing_token"
}

impl RelayAuthRejected {
    pub fn new(reason: impl Into<String>) -> Self {
        Self {
            msg_type: "relay_auth_rejected".into(),
            reason: reason.into(),
        }
    }

    pub fn invalid_token() -> Self {
        Self::new("invalid_token")
    }

    pub fn missing_token() -> Self {
        Self::new("missing_token")
    }
}

/// `type: "relay_route"`
///
/// Envelope used to route a message through the relay to a specific device.
///
/// # Signed form
///
/// The five routing fields (`type`, `from_device_id`, `to_device_id`,
/// `payload`, `timestamp`, `nonce`, `key_id`) are the ones covered by the
/// HMAC — see [`crate::hmac::SIGNED_FIELDS`], which is the single source of
/// truth for the canonical signing order.
///
/// * `from_device_id` MUST be the authenticated sender's own device id. The
///   relay compares it against the identity established by `relay_auth` and
///   rejects any mismatch, so one authenticated client cannot forge a route on
///   behalf of another.
/// * `key_id` names the signing key (see [`crate::hmac::SigningKeyring`]) and
///   makes key rotation observable on the wire.
/// * `timestamp` + `nonce` drive replay rejection, scoped per device id.
///
/// All four optional fields are `#[serde(default)]` so an older, minimal
/// `{type, to_device_id, payload}` message still *deserialises*; the relay
/// rejects it at verification time rather than silently forwarding it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelayRoute {
    #[serde(rename = "type")]
    pub msg_type: String, // "relay_route"
    /// Authenticated sender identity. Signed, and checked against the
    /// connection identity by the relay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_device_id: Option<String>,
    pub to_device_id: String,
    /// The inner payload (a complete JSON message) to forward.
    pub payload: serde_json::Value,
    /// Unix timestamp in milliseconds. Replay window: -5 s … +30 s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<i64>,
    /// Unique-per-sender replay nonce.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nonce: Option<String>,
    /// Which signing key produced `hmac`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_id: Option<String>,
    /// Lowercase hex HMAC-SHA256 over the canonical signed-field subset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hmac: Option<String>,
}

/// `type: "relay_delivery"`
///
/// What the relay sends to the recipient once it has verified a `relay_route`.
///
/// # Why this exists
///
/// The relay used to forward `route.payload` on its own, which threw away the
/// one piece of information it had just authenticated: *who* the sender was. It
/// checks `from_device_id` against the identity the connection presented during
/// `relay_auth` and refuses any mismatch, so by the time it forwards anything it
/// knows the sender with certainty. Forwarding the bare payload threw that away,
/// and the consequence was not theoretical:
///
///   * the receiver had no way to authenticate the sender, so a message that
///     arrived over the relay could not be told apart from one that arrived over
///     the LAN; and
///   * receivers that require their own pairing registry to authorise a message
///     had no identity to check, so the message was refused outright.
///
/// This type is the envelope that carries the verified attribution across. It is
/// produced **only** by the relay, after verification, and it is not signed: the
/// trust is the relay's, and a client cannot produce one that says anything the
/// relay did not verify. `from_device_id` is the identity the relay authenticated
/// on the sending connection, not a claim the sender chose.
///
/// The recipient still authenticates the *content* end to end through the inner
/// `encrypted` envelope, exactly as it does on the LAN. This type adds
/// attribution, not confidentiality, and it does not let an unpaired device send.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RelayDelivery {
    #[serde(rename = "type")]
    pub msg_type: String, // "relay_delivery"
    /// The device the relay authenticated as the sender.
    pub from_device_id: String,
    /// The device this delivery is addressed to. Carried so a recipient can
    /// reject a message the relay misrouted before parsing the payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub to_device_id: Option<String>,
    /// The forwarded message, verbatim. Normally an `encrypted` envelope.
    pub payload: serde_json::Value,
}

impl RelayDelivery {
    /// Wrap `payload` as having been sent by `from_device_id`.
    ///
    /// Called by the relay only, after `handle_relay_route` has returned `Ok`.
    pub fn new(
        from_device_id: impl Into<String>,
        to_device_id: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        Self {
            msg_type: "relay_delivery".into(),
            from_device_id: from_device_id.into(),
            to_device_id: Some(to_device_id.into()),
            payload,
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
//  Encrypted Envelope
// ───────────────────────────────────────────────────────────────────────────

/// `type: "encrypted"`
///
/// Wraps any plaintext message after encryption. The `data` field contains
/// the hex-encoded ciphertext. The original message type is NOT visible
/// to the relay or any intermediary.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EncryptedEnvelope {
    #[serde(rename = "type")]
    pub msg_type: String, // "encrypted"
    /// Hex-encoded nonce used for XChaCha20-Poly1305.
    pub nonce: String,
    /// Hex-encoded HMAC-SHA256 over the ciphertext.
    pub hmac: String,
    /// Hex-encoded ciphertext (XChaCha20-Poly1305).
    pub data: String,
    /// Device ID of the sender (for relay routing).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u32>,
}

// ───────────────────────────────────────────────────────────────────────────
//  Shared Value Objects
// ───────────────────────────────────────────────────────────────────────────

/// Device metadata exchanged during pairing and discovery.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub name: String,
    /// "phone" | "tablet" | "desktop"
    #[serde(rename = "type")]
    pub device_type: String,
    /// "ios" | "android" | "windows" | "macos" | "linux"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battery: Option<i32>,
}

/// A single SMS record, as carried inside an [`SmsThread`] and inside the
/// nested `message` field of [`SmsNew`] / [`SmsSent`].
///
/// Field set mirrors `SmsMessage.toJson()` in the mobile `SmsService`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmsMessage {
    /// Stable record ID. Prefixed `in_` for received and `out_` for sent.
    pub id: String,
    /// Sender address for received messages, recipient for sent ones.
    pub address: String,
    pub body: String,
    /// Unix timestamp (seconds).
    pub timestamp: i64,
    /// True once the message has been read on the originating device.
    pub read: bool,
    /// True for messages sent from the phone, false for received ones.
    pub is_outgoing: bool,
}

/// One conversation thread in an [`SmsSync`] snapshot.
///
/// Field set mirrors `SmsThread` / `threadsToJson()` in the mobile
/// `SmsService`. Threads are keyed by `thread_id`; the phone generates it as
/// `t_<millis>` when it first sees an address.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SmsThread {
    /// Stable thread ID. Prefixed `t_` on the originating phone.
    pub thread_id: String,
    /// Party address, used to group records into threads.
    pub address: String,
    /// Address-book display name, `null` when unresolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Most recent message body, for list rendering.
    pub snippet: String,
    /// Number of unread messages in the thread.
    pub unread_count: i64,
    /// Unix timestamp (seconds) of the most recent message.
    pub timestamp: i64,
    /// Chronologically ordered messages, oldest first.
    pub messages: Vec<SmsMessage>,
}

// ───────────────────────────────────────────────────────────────────────────
//  Automation Inner Types
// ───────────────────────────────────────────────────────────────────────────

/// Discriminated trigger for automation rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationTrigger {
    /// One of the trigger type wire tags:
    /// "device_connect", "device_disconnect", "time", "battery_level",
    /// "wifi_change", "app_open", "audio_device_connect", "audio_device_disconnect"
    #[serde(rename = "type")]
    pub trigger_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    /// "HH:MM" 24h string for time-based triggers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<String>,
    /// Battery threshold (0..100) for battery_level triggers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub below: Option<i32>,
    /// WiFi SSID for wifi_change triggers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ssid: Option<String>,
    /// Package name for app_open triggers.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_package: Option<String>,
}

/// Discriminated action payload for automation rules.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AutomationActionPayload {
    /// One of the action type wire tags:
    /// "send_notification", "set_phone_profile", "route_audio",
    /// "run_shell_command", "toggle_wifi", "toggle_bluetooth",
    /// "open_url", "open_app", "set_window_state"
    #[serde(rename = "type")]
    pub action_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// "silent" | "vibrate" | "ring"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub app_package: Option<String>,
    /// "minimize" | "maximize" | "restore" | "close"
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

// ───────────────────────────────────────────────────────────────────────────
//  Binary Frame Formats
// ───────────────────────────────────────────────────────────────────────────

/// Relay binary frame format.
///
/// # Version 1 (0x01) — **RETIRED, rejected by the relay**
///
/// ```text
/// Byte 0:        version (0x01)
/// Bytes 1..=16:  target device ID, ASCII, zero-padded to 16 bytes
/// Bytes 17..:    payload
/// ```
///
/// v1 carried no integrity protection at all: any authenticated client could
/// flood any other device with arbitrary bytes. The relay now rejects 0x01.
///
/// # Version 2 (0x02) — current
///
/// ```text
/// Byte  0:              version (0x02)
/// Bytes 1..=16:         target device ID: the id's first 16 bytes, NUL-padded
///                        — exactly [`binary_target_field`], nothing else
/// Bytes 17..=20:        sequence number, u32 big-endian
///                        (strictly increasing per sender connection)
/// Bytes 21..=52:        HMAC-SHA256 tag (32 raw bytes) — see below
/// Bytes 53..:           payload
/// ```
///
/// Minimum length: [`BINARY_HEADER_LEN`] = 53 bytes (empty payload).
///
/// # MAC input
///
/// The tag is **not** a MAC over the frame alone — the sender's authenticated
/// device id is bound in, so a captured frame cannot be re-presented by a
/// different authenticated client:
///
/// ```text
/// tag = HMAC-SHA256(
///         relay_signing_key,
///         from_device_id_utf8 || 0x1F || frame[0..21] || frame[53..] )
/// ```
///
/// i.e. the header (version, target id, sequence) concatenated with the
/// payload, prefixed by `from_device_id` and a `0x1F` separator. `from_device_id`
/// is taken from the *authenticated* connection identity, never from the frame.
///
/// The relay rejects a frame whose tag does not verify, whose version is not
/// `0x02`, whose target id is empty/invalid, or whose sequence number is not
/// strictly greater than the previous accepted one on that connection.
///
/// The target id is a *prefix* of the full device id, so "whose target id is
/// invalid" means "whose 16-byte field is not a UTF-8, non-empty id prefix", and
/// a frame that verifies is not yet deliverable: the receiver of the bytes still
/// has to resolve that prefix. See [`binary_target_field`].
pub const BINARY_FRAME_VERSION: u8 = 0x02;
/// Width of the target device id field, in bytes.
///
/// Sixteen, which is **smaller than every device id this protocol produces** — a
/// `device_id` is a 36-character UUID. The field therefore holds the id's first
/// sixteen bytes, and what that means for routing is defined by
/// [`binary_target_field`], not by this constant.
pub const BINARY_DEVICE_ID_LEN: usize = 16;
/// Sequence-number field length (u32 big-endian).
pub const BINARY_SEQ_LEN: usize = 4;
/// HMAC-SHA256 tag length in raw bytes.
pub const BINARY_TAG_LEN: usize = 32;
/// Offset of the HMAC tag within a v2 frame.
pub const BINARY_TAG_OFFSET: usize = 1 + BINARY_DEVICE_ID_LEN + BINARY_SEQ_LEN; // 21
/// Length of the authenticated (tagged) region that precedes the tag.
pub const BINARY_AUTHENTICATED_PREFIX_LEN: usize = BINARY_TAG_OFFSET; // 21
/// Total fixed header: version + target id + sequence + tag.
pub const BINARY_HEADER_LEN: usize = BINARY_TAG_OFFSET + BINARY_TAG_LEN; // 53

/// Why a v2 frame's 16-byte target field could not be read back as a device id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryTargetFieldError {
    /// The field is not valid UTF-8 — the producer split a multi-byte character.
    NotUtf8,
    /// The field was nothing but NUL padding.
    Empty,
}

/// The 16 wire bytes that name `device_id` in a v2 binary frame.
///
/// **The rule, in one sentence:** the first [`BINARY_DEVICE_ID_LEN`] bytes of the
/// id's UTF-8 encoding, NUL-padded, with no hashing, case folding, separator
/// removal or length field.
///
/// This is the *only* definition of that field. The producer
/// ([`build_binary_frame`]), the relay's router ([`binary_target_matches`]) and
/// every receiver's identity check call it, so the four parties cannot disagree
/// about what those 16 bytes mean. Truncation is by **byte**, not by character,
/// matching the Dart implementation (`relay_route.dart`), which truncates
/// `utf8.encode(targetDeviceId)`.
///
/// # Why a prefix at all
///
/// A `device_id` is a 36-character UUID (`Uuid::new_v4().to_string()` on the
/// desktop; the per-connection UUID the pairing handshake falls back to on a
/// phone) and 36 does not fit in 16. Widening the field is not available: the
/// layout is already deployed and is pinned by tests in three languages, and
/// those bytes are inside the MAC input, so a wider field would invalidate every
/// tag in flight. The alternative considered and rejected — a truncated hash of
/// the id — is a wire break in all but name, and is argued against below.
///
/// The prefix is not an authentication or routing decision on its own. The tag
/// covers these exact 16 bytes (`frame[0..21]`), and the relay resolves the
/// prefix to exactly one connected device before forwarding anything. A prefix
/// that matches no connected device is dropped; one that matches two is dropped
/// as ambiguous. Neither is ever delivered to a guess.
///
/// # What happens when two ids share a prefix
///
/// Nothing, and that is the point: two ids agreeing on their first 16 bytes are
/// **identical on the wire**, so no encoding of 16 bytes could tell them apart.
/// The consequences are therefore all in the resolver, not here:
///
/// * A frame naming a shared prefix resolves to *both* devices, so the relay
///   must fail closed on ambiguity — count it and drop it — rather than pick one.
///   Delivering it would hand a file to the wrong device.
/// * A receiver whose own id is one of the two accepts the frame, so the relay
///   is the only place that can enforce "exactly one"; that is why the drop has
///   to happen before the re-frame, not after.
/// * An id of 16 bytes or fewer is *never* ambiguous against a different id: the
///   field is then the whole id, and NUL padding distinguishes `device-1` from
///   `device-10`. Ambiguity needs ids longer than 16 bytes.
///
/// For a hyphenated UUIDv4 the first 16 characters carry 14 hex digits — 56 bits
/// of the version's randomness — so a collision is not expected in practice
/// (roughly `5e-11` across 10 000 devices, from the birthday bound), which is
/// why resolution can be exact rather than merely likely.
///
/// # Why a prefix and not a hash of the id
///
/// A 128-bit digest of the id would make collisions astronomically unlikely
/// rather than merely unlikely. It buys nothing here and costs three things:
///
/// * **Compatibility.** Truncation is what every deployed producer already
///   writes — [`build_binary_frame`] on the relay's re-frame path,
///   `buildBinaryFrame` on the phone. Redefining those same 16 bytes to be a
///   digest would leave every in-flight and already-built frame unresolvable: a
///   wire break dressed as a semantic one, with no version byte to announce it.
/// * **The field's type.** It is NUL-padded ASCII and is read back as UTF-8 with
///   trailing NULs stripped ([`parse_binary_target_field`]); a raw digest would
///   change what the field *is*, not just what it means.
/// * **Legibility.** In a hex dump the recipient's id is simply there — the first
///   16 characters, readable. A digest identifies nobody without recomputing it
///   against every registered device, which is exactly what an operator reading
///   a packet capture at 3am does not want.
///
/// And the security case for a hash does not apply: ambiguity is handled fail
/// closed by the resolver, and the tag authenticates the bytes regardless. The
/// leading 14 hex digits of a UUIDv4 are not a secret either — the full id is
/// already in the clear in `relay_auth`.
pub fn binary_target_field(device_id: &str) -> [u8; BINARY_DEVICE_ID_LEN] {
    let mut field = [0u8; BINARY_DEVICE_ID_LEN];
    let bytes = device_id.as_bytes();
    let n = bytes.len().min(BINARY_DEVICE_ID_LEN);
    field[..n].copy_from_slice(&bytes[..n]);
    field
}

/// Read a v2 frame's 16-byte target field back as the id prefix it carries:
/// trailing NUL padding removed, nothing else.
///
/// The inverse of [`binary_target_field`] up to the loss of the padding, and the
/// only place the padding is stripped. Both rejection modes fail closed: a field
/// that is not UTF-8 cannot have come from a device id (see the doc comment on
/// [`binary_target_field`]), and an all-NUL field names nothing.
///
/// Takes a slice rather than a fixed-size array so the result borrows from the
/// caller's *frame*, not from a local copy of the field, which is what lets
/// [`VerifiedBinaryFrame`](crate::VerifiedBinaryFrame)-style verifiers hand the
/// prefix back with the frame's lifetime.
pub fn parse_binary_target_field(field: &[u8]) -> Result<&str, BinaryTargetFieldError> {
    let text = std::str::from_utf8(field).map_err(|_| BinaryTargetFieldError::NotUtf8)?;
    let trimmed = text.trim_end_matches('\0');
    if trimmed.is_empty() {
        return Err(BinaryTargetFieldError::Empty);
    }
    Ok(trimmed)
}

/// Whether a v2 frame's target field names `device_id`.
///
/// The receiver-side identity check, and the same predicate the relay's router
/// applies to every connected device: the field is a device's name if and only if
/// it is byte-for-byte that device's [`binary_target_field`]. Because the
/// comparison is equality on the canonical field rather than a substring test, a
/// short id is not a wildcard — `device-1` does not match `device-10`, and a
/// frame naming `device-1` does not match a longer id that begins with it.
///
/// Compared without early exit. Nothing secret is on either side of this (the
/// target of a frame is public to everyone the relay routes between), so the
/// timing is not the reason; it is one line either way and this way cannot grow
/// one.
pub fn binary_target_matches(device_id: &str, field: &[u8; BINARY_DEVICE_ID_LEN]) -> bool {
    let expected = binary_target_field(device_id);
    let mut diff = 0u8;
    for (want, got) in expected.iter().zip(field.iter()) {
        diff |= want ^ got;
    }
    diff == 0
}

/// Build a v2 binary frame addressed to `target_device_id` and tagged for
/// `from_device_id`.
///
/// The single place a v2 frame is *produced*, so the sender id, the header and
/// the tag cannot drift apart. The mobile client has a byte-compatible
/// implementation in `relay_route.dart`; `conduit-protocol`'s tests pin the two
/// against a shared vector.
///
/// `sequence` is the per-connection frame counter and must strictly increase.
pub fn build_binary_frame(
    route_key: &[u8],
    from_device_id: &str,
    target_device_id: &str,
    sequence: u32,
    payload: &[u8],
) -> Vec<u8> {
    let mut frame = vec![0u8; BINARY_HEADER_LEN + payload.len()];
    frame[0] = BINARY_FRAME_VERSION;

    // The target field is the canonical prefix of the full id, so an over-long
    // id is *canonicalised* rather than rejected: the field holds its first 16
    // bytes, the tag covers exactly those bytes, and the relay resolves them back
    // to one connected device. See [`binary_target_field`] for why truncation is
    // the rule and what two ids sharing a prefix mean.
    frame[1..1 + BINARY_DEVICE_ID_LEN].copy_from_slice(&binary_target_field(target_device_id));
    frame[1 + BINARY_DEVICE_ID_LEN..1 + BINARY_DEVICE_ID_LEN + BINARY_SEQ_LEN]
        .copy_from_slice(&sequence.to_be_bytes());

    // MAC input: sender id, 0x1F, then the header and payload as they will be
    // transmitted. Computed over the *hex encoding* of those bytes, which is
    // what both implementations do and is pinned by the interop vector.
    let authenticated_len = BINARY_AUTHENTICATED_PREFIX_LEN + payload.len();
    let mut authenticated = Vec::with_capacity(authenticated_len);
    authenticated.extend_from_slice(&frame[..BINARY_AUTHENTICATED_PREFIX_LEN]);
    authenticated.extend_from_slice(payload);

    let mut mac_input = Vec::with_capacity(from_device_id.len() + 1 + authenticated_len);
    mac_input.extend_from_slice(from_device_id.as_bytes());
    mac_input.push(0x1f);
    mac_input.extend_from_slice(&authenticated);

    let tag = crate::hmac::compute_hmac(route_key, &hex::encode(&mac_input));
    let tag_bytes = hex::decode(&tag).expect("compute_hmac returns lowercase hex");
    frame[BINARY_TAG_OFFSET..BINARY_HEADER_LEN].copy_from_slice(&tag_bytes);
    frame[BINARY_HEADER_LEN..].copy_from_slice(payload);
    frame
}

/// The byte string a v2 frame's tag is computed over.
///
/// `from_device_id || 0x1F || frame[..21] || payload`. Exposed because the relay
/// verifies the tag itself and must build the identical input.
pub fn binary_mac_input(from_device_id: &str, header_and_payload: &[u8]) -> Vec<u8> {
    let mut input = Vec::with_capacity(from_device_id.len() + 1 + header_and_payload.len());
    input.extend_from_slice(from_device_id.as_bytes());
    input.push(0x1f);
    input.extend_from_slice(header_and_payload);
    input
}

/// LAN direct-connection binary chunk format (version 0x01 implied, no header).
///
/// Used for file transfer chunks over direct LAN WebSocket connections.
///
/// ```text
/// Bytes 0..=23:     XChaCha20-Poly1305 nonce (24 bytes)
/// Bytes 24..=27:    metadata length as u32 little-endian
/// Bytes 28..(28+len): JSON metadata (BinaryFileMetadata)
/// Remaining bytes:  XChaCha20-Poly1305 ciphertext (includes 16-byte MAC)
/// ```
///
/// Minimum length: 28 bytes (nonce + length, no metadata or ciphertext).
pub const CHUNK_NONCE_LEN: usize = 24;
pub const CHUNK_LEN_FIELD: usize = 4;
pub const CHUNK_HEADER_LEN: usize = CHUNK_NONCE_LEN + CHUNK_LEN_FIELD; // 28

// ───────────────────────────────────────────────────────────────────────────
//  Convenience Constructors
// ───────────────────────────────────────────────────────────────────────────

// Serde helpers that enforce `schema.json` constraints at the Rust boundary
// so serialized payloads always validate (and invalid wire input fails fast).

/// `RelayAuthRejected.reason` enum: `"invalid_token" | "missing_token"`.
mod validate_relay_reject_reason {
    use serde::{de, ser, Deserialize, Deserializer, Serializer};

    const VALID: [&str; 2] = ["invalid_token", "missing_token"];

    pub fn serialize<S: Serializer>(v: &String, s: S) -> Result<S::Ok, S::Error> {
        if VALID.contains(&v.as_str()) {
            s.serialize_str(v)
        } else {
            Err(ser::Error::custom(format!(
                "relay_auth_rejected reason must be one of {VALID:?}, got {v}"
            )))
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
        let s = String::deserialize(d)?;
        if VALID.contains(&s.as_str()) {
            Ok(s)
        } else {
            Err(de::Error::custom(format!(
                "relay_auth_rejected reason must be one of {VALID:?}, got {s}"
            )))
        }
    }
}

/// `ScreenMirrorTouch.actionType` enum (schema `actionType`).
mod validate_touch_action_type {
    use serde::{de, ser, Deserialize, Deserializer, Serializer};

    // `move` is the hover / drag-update gesture: the Android accessibility
    // injector treats it as "move the pointer, do not click"
    // (`ConduitAccessibilityService.injectTouch`), so it is a first-class
    // wire value rather than a client-side detail.
    const VALID: [&str; 5] = ["tap", "double_tap", "long_press", "right_click", "move"];

    pub fn serialize<S: Serializer>(v: &Option<String>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            None => s.serialize_none(),
            Some(val) if VALID.contains(&val.as_str()) => s.serialize_some(val),
            Some(val) => Err(ser::Error::custom(format!(
                "actionType must be one of {VALID:?}, got {val}"
            ))),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
        let v = Option::<String>::deserialize(d)?;
        match v {
            None => Ok(None),
            Some(val) if VALID.contains(&val.as_str()) => Ok(Some(val)),
            Some(val) => Err(de::Error::custom(format!(
                "actionType must be one of {VALID:?}, got {val}"
            ))),
        }
    }
}

/// `StatusUpdate.battery` integer percentage: schema `minimum: 0, maximum: 100`.
mod validate_battery_pct {
    use serde::{de, ser, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(v: &Option<i32>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            None => s.serialize_none(),
            Some(n) if (0..=100).contains(n) => s.serialize_some(n),
            Some(n) => Err(ser::Error::custom(format!(
                "battery must be 0..=100, got {n}"
            ))),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i32>, D::Error> {
        let v = Option::<i32>::deserialize(d)?;
        match v {
            None => Ok(None),
            Some(n) if (0..=100).contains(&n) => Ok(Some(n)),
            Some(n) => Err(de::Error::custom(format!(
                "battery must be 0..=100, got {n}"
            ))),
        }
    }
}

impl Ping {
    pub fn new() -> Self {
        Self {
            msg_type: "ping".into(),
        }
    }
}

impl Default for Ping {
    fn default() -> Self {
        Self::new()
    }
}

impl Pong {
    pub fn new() -> Self {
        Self {
            msg_type: "pong".into(),
        }
    }
}

impl Default for Pong {
    fn default() -> Self {
        Self::new()
    }
}

impl RelayRoute {
    /// Unsigned route. Convenience for local/LAN use only — the relay rejects
    /// any `relay_route` that arrives without a valid `hmac`.
    pub fn new(to_device_id: impl Into<String>, payload: serde_json::Value) -> Self {
        Self {
            msg_type: "relay_route".into(),
            from_device_id: None,
            to_device_id: to_device_id.into(),
            payload,
            timestamp: None,
            nonce: None,
            key_id: None,
            hmac: None,
        }
    }

    /// Build a fully signed `relay_route` with a generated timestamp and nonce.
    ///
    /// `route_key` MUST be this device's own key, from
    /// [`crate::hmac::derive_route_key`] applied to the pairing secret the hub
    /// holds for it. Never the relay token: every client holds that, so a route
    /// signed with it could claim to be from any device.
    ///
    /// `from_device_id` MUST be the sender's own authenticated device id. It is
    /// also used as the `key_id`, so the receiver can look the key up and check
    /// the two agree.
    pub fn signed(
        route_key: &[u8],
        from_device_id: impl Into<String>,
        to_device_id: impl Into<String>,
        payload: serde_json::Value,
    ) -> Self {
        let from = from_device_id.into();
        Self::signed_with(
            &from.clone(),
            route_key,
            from,
            to_device_id,
            payload,
            crate::hmac::now_millis(),
            crate::hmac::new_nonce(),
        )
    }

    /// [`RelayRoute::signed`] with every signed field supplied explicitly.
    ///
    /// Use this when the caller owns the timestamp/nonce (clients SHOULD use a
    /// cryptographically random nonce and a locally-synchronised clock) or when
    /// testing a specific field value.
    ///
    /// `key_id` must equal `from_device_id`: a receiver resolves the key by
    /// `key_id` and then requires the attributed sender to match, so a
    /// mismatched pair is rejected rather than mis-attributed.
    pub fn signed_with(
        key_id: &str,
        route_key: &[u8],
        from_device_id: impl Into<String>,
        to_device_id: impl Into<String>,
        payload: serde_json::Value,
        timestamp: i64,
        nonce: impl Into<String>,
    ) -> Self {
        let mut route = Self {
            msg_type: "relay_route".into(),
            from_device_id: Some(from_device_id.into()),
            to_device_id: to_device_id.into(),
            payload,
            timestamp: Some(timestamp),
            nonce: Some(nonce.into()),
            key_id: Some(key_id.to_string()),
            hmac: None,
        };
        route.sign_with(key_id, route_key);
        route
    }

    /// (Re)compute `hmac` over the canonical signed-field subset.
    ///
    /// Serializes through `serde_json::Value` so the bytes hashed are exactly
    /// what goes on the wire — the verifier reconstructs the same canonical
    /// string from the received JSON.
    ///
    /// The canonical string is built by inserting into a `serde_json::Map` and
    /// serializing the result, so it is only byte-identical to the verifier's
    /// output while `serde_json` is not built with the `preserve_order` feature.
    /// Under `preserve_order` both sides preserve insertion order, and because
    /// `canonical_signing_string` inserts in [`crate::hmac::SIGNED_FIELDS`]
    /// order on both sides they still agree; a signer that reorders the fields
    /// would not. Callers MUST NOT reorder the map before signing.
    ///
    /// If serialisation fails the value degrades to `Value::Null` and the
    /// resulting `hmac` will not verify — this function does not return an
    /// error, so a caller that needs to distinguish that case must verify
    /// afterwards with [`crate::hmac::SigningKeyring::verify`].
    pub fn sign_with(&mut self, key_id: &str, signing_key: &[u8]) {
        let mut value = serde_json::to_value(&*self).unwrap_or(serde_json::Value::Null);
        self.key_id = Some(key_id.to_string());
        if let serde_json::Value::Object(ref mut map) = value {
            map.insert(
                "key_id".to_string(),
                serde_json::Value::String(key_id.to_string()),
            );
        }
        let canonical = crate::hmac::canonical_signing_string(&value);
        self.hmac = Some(crate::hmac::compute_hmac(signing_key, &canonical));
    }

    /// `from_device_id` if present.
    pub fn from_device(&self) -> Option<&str> {
        self.from_device_id.as_deref()
    }

    /// Whether every signed field the relay requires is present.
    ///
    /// `hmac` and `key_id` are deliberately excluded: they are verified, not
    /// merely required to exist.
    pub fn has_required_signed_fields(&self) -> bool {
        self.from_device_id.is_some()
            && self.timestamp.is_some()
            && self.nonce.as_deref().is_some_and(|n| !n.is_empty())
    }
}

impl RelayAuth {
    pub fn new(device_id: impl Into<String>, relay_token: impl Into<String>) -> Self {
        Self {
            msg_type: "relay_auth".into(),
            device_id: device_id.into(),
            relay_token: relay_token.into(),
            apns_token: None,
        }
    }
}

impl EncryptedEnvelope {
    pub fn new(
        nonce_hex: impl Into<String>,
        hmac_hex: impl Into<String>,
        data_hex: impl Into<String>,
    ) -> Self {
        Self {
            msg_type: "encrypted".into(),
            nonce: nonce_hex.into(),
            hmac: hmac_hex.into(),
            data: data_hex.into(),
            source_device: None,
            protocol_version: Some(PROTOCOL_VERSION),
        }
    }
}

// ============================================================
//  Test suite
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    // ---------------------------------------------------------------
    //  Helper: roundtrip serialize then deserialize
    // ---------------------------------------------------------------

    fn roundtrip<T: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(
        val: &T,
    ) -> T {
        let json_str = serde_json::to_string(val).expect("serialize");
        serde_json::from_str(&json_str).expect("deserialize")
    }

    // ---------------------------------------------------------------
    //  Protocol version constant
    // ---------------------------------------------------------------

    #[test]
    fn protocol_version_is_one() {
        assert_eq!(PROTOCOL_VERSION, 1);
    }

    // ---------------------------------------------------------------
    //  LAN port constants — the three representations must agree:
    //    1. these Rust constants,
    //    2. PROTOCOL.md (the spec the desktop is audited against),
    //    3. the Dart client that actually dials them.
    //  A drift between any two of these is the exact bug that made the mobile
    //  app unpairable, so each is pinned by a test.
    // ---------------------------------------------------------------

    /// PROTOCOL.md lives one directory up from `src/`.
    const PROTOCOL_MD: &str = include_str!("../PROTOCOL.md");
    /// The mobile transport layer that mirrors these constants on the Dart side.
    const DART_WS_SERVICE: &str =
        include_str!("../../../apps/mobile/lib/services/websocket_service.dart");
    const DART_PAIRING_SERVICE: &str =
        include_str!("../../../apps/mobile/lib/services/pairing_service.dart");
    const DART_DISCOVERY_SERVICE: &str =
        include_str!("../../../apps/mobile/lib/services/discovery_service.dart");

    #[test]
    fn lan_ports_have_the_documented_values() {
        assert_eq!(LAN_WS_PORT, 9527, "plaintext LAN WS port");
        assert_eq!(LAN_WSS_PORT, 9531, "TLS LAN WSS port");
    }

    #[test]
    fn lan_ports_are_distinct() {
        // A single port cannot serve both a plaintext and a TLS socket; if
        // these ever collide the two listeners could not both bind.
        assert_ne!(LAN_WS_PORT, LAN_WSS_PORT);
    }

    #[test]
    fn protocol_md_transport_table_documents_both_lan_ports() {
        // Row must name the transport, the port, and say which is TLS.
        assert!(
            PROTOCOL_MD.contains("| WS (LAN) | 9527 |"),
            "PROTOCOL.md must document the plaintext LAN listener as 9527; \
             the §2 transport table has drifted from the code"
        );
        assert!(
            PROTOCOL_MD.contains("| WSS (LAN) | 9531 |"),
            "PROTOCOL.md must document the TLS LAN listener as 9531; \
             the §2 transport table has drifted from the code"
        );
    }

    #[test]
    fn protocol_md_announce_example_advertises_the_wss_port() {
        // The `discovery/announce` example must show the real wss_port so a
        // reader cannot copy 9527 into a `wss://` URL.
        assert!(
            PROTOCOL_MD.contains("\"wss_port\": 9531"),
            "PROTOCOL.md's discovery/announce example must advertise wss_port 9531"
        );
    }

    #[test]
    fn protocol_md_forbids_wss_on_the_plaintext_lan_port() {
        assert!(
            PROTOCOL_MD.to_lowercase().contains("never dial"),
            "PROTOCOL.md §2.2 must state that `wss://` must not be used on the \
             plaintext LAN port; that misuse is what broke pairing"
        );
    }

    /// Strip `//` line comments (Dart has no block comments) and blank lines so
    /// prose that legitimately names a port is not mistaken for a literal.
    fn dart_code_lines(source: &str) -> impl Iterator<Item = &str> {
        source
            .lines()
            .map(|l| l.split("//").next().unwrap_or(""))
            .filter(|l| !l.trim().is_empty())
    }

    #[test]
    fn dart_client_mirrors_the_lan_port_constants() {
        assert!(
            DART_WS_SERVICE.contains("kLanWssPort = 9531"),
            "apps/mobile/lib/services/websocket_service.dart must define \
             `kLanWssPort = 9531` to mirror conduit_protocol::types::LAN_WSS_PORT"
        );
        assert!(
            DART_WS_SERVICE.contains("kLanWsPort = 9527"),
            "apps/mobile/lib/services/websocket_service.dart must define \
             `kLanWsPort = 9527` to mirror conduit_protocol::types::LAN_WS_PORT"
        );
    }

    #[test]
    fn dart_pairing_never_hardcodes_a_port_literal() {
        // The original bug: `wss://$ip:9527` inlined into three call sites.
        // Pairing must reference the shared constant instead.
        for line in dart_code_lines(DART_PAIRING_SERVICE) {
            assert!(
                !line.contains("9527") && !line.contains("9531"),
                "pairing_service.dart must not hardcode a LAN port; found: {line}"
            );
        }
    }

    #[test]
    fn dart_discovery_does_not_force_the_plaintext_port() {
        // The original bug: `port: 9527, // Force port 9527` discarded the SRV
        // record and re-pointed TLS traffic at the plaintext listener.
        for line in dart_code_lines(DART_DISCOVERY_SERVICE) {
            assert!(
                !line.contains("9527") && !line.contains("9531"),
                "discovery_service.dart must not hardcode a LAN port; found: {line}"
            );
        }
    }

    #[test]
    fn binary_frame_version_constant() {
        // Pinned the retired (unauthenticated) v1; the relay's binary path is
        // now v2, which authenticates the frame and its sender.
        assert_eq!(BINARY_FRAME_VERSION, 0x02);
    }

    #[test]
    fn binary_device_id_len_is_16() {
        assert_eq!(BINARY_DEVICE_ID_LEN, 16);
    }

    #[test]
    fn binary_header_len_matches_v2_layout() {
        // v2 = version(1) + target id(16) + sequence(4) + HMAC tag(32).
        // Both ends of the relay binary path derive every offset from this
        // constant, so a change here is a wire-format change.
        assert_eq!(BINARY_HEADER_LEN, 53);
        assert_eq!(
            BINARY_HEADER_LEN,
            1 + BINARY_DEVICE_ID_LEN + BINARY_SEQ_LEN + BINARY_TAG_LEN
        );
        assert_eq!(BINARY_TAG_OFFSET, 21);
        assert_eq!(BINARY_AUTHENTICATED_PREFIX_LEN, BINARY_TAG_OFFSET);
    }

    #[test]
    fn binary_frame_version_is_v2() {
        assert_eq!(
            BINARY_FRAME_VERSION, 0x02,
            "the relay rejects the unauthenticated v1 (0x01) frame"
        );
    }

    #[test]
    fn binary_authenticated_region_covers_prefix_and_payload() {
        // The tagged region is frame[0..21] ++ payload — everything except the
        // tag itself. The tag sits immediately before the payload.
        let payload_len = 4;
        let mut frame = vec![0u8; BINARY_HEADER_LEN + payload_len];
        frame[0] = BINARY_FRAME_VERSION;
        let payload_start = BINARY_HEADER_LEN;
        assert_eq!(
            payload_start,
            BINARY_AUTHENTICATED_PREFIX_LEN + BINARY_TAG_LEN,
            "the payload begins immediately after the HMAC tag"
        );
        assert_eq!(frame.len(), payload_start + payload_len);
    }

    #[test]
    fn the_dart_client_builds_the_same_frames() {
        // The other half of the Dart suite's
        // `binary frame interop with the Rust producer` group: those constants
        // live in `apps/mobile/test/services/relay_route_test.dart` and were, until
        // this test, a one-way pin — the Dart side could only prove it matched a
        // value nobody on this side ever checked, so a change to the producer or to
        // the target-field rule would have broken the phone with a green build here.
        //
        // `cafebabe`/`0123456789abcdef` fit the field, the 36-character UUID does
        // not, and the last two are the branches that matter for the target rule:
        // an id too long for the field, and a real device id.
        let secret: Vec<u8> = (0u8..32).collect();
        let key = crate::hmac::derive_route_key(&secret, "deadbeef");
        let built = |target: &str, seq: u32, payload: &[u8]| {
            hex::encode(build_binary_frame(&key, "deadbeef", target, seq, payload))
        };

        assert_eq!(
            built("cafebabe", 7, b"hello"),
            "026361666562616265000000000000000000000007b721e258479b9f7c7b4b0ca343acde53f\
             1a495b884cb568297114c7dca056c5f68656c6c6f"
                .replace(' ', ""),
            "a short target id is NUL-padded to 16 bytes"
        );
        assert_eq!(
            built("cafebabe", 1, b""),
            "026361666562616265000000000000000000000001138f68922f828ce72b7573213db3e\
             2f798245c651512f1155623ce3aeb7ee865"
                .replace(' ', ""),
            "an empty payload is the 53-byte minimum"
        );
        assert_eq!(
            built("0123456789abcdef", 0x01020304, b"the quick brown fox"),
            "02303132333435363738396162636465660102030475dcdfbaa081e2127b1d6df93df6de\
             99614b28792e84b5e40baedfa4a1d826b874686520717569636b2062726f776e20666f78"
                .replace(' ', ""),
            "a full 16-byte target id is not padded"
        );
        assert_eq!(
            built("0123456789abcdefEXTRA", 2, b"x"),
            "0230313233343536373839616263646566000000022129374e159f29fd2130c0bd1f17b\
             616dee807e3994120f91bcbb0e35fcffb6178"
                .replace(' ', ""),
            "an over-long id is canonicalised to its first 16 bytes"
        );
        // The vector that made this a defect rather than an edge case: a real
        // device id, 36 characters, addressed by a 16-byte field. `35353065383430
        // 302d653239622d3431` is "550e8400-e29b-41" — the id's first 16 bytes and
        // nothing else.
        assert_eq!(
            built("550e8400-e29b-41d4-a716-446655440000", 9, b"file-chunk"),
            "0235353065383430302d653239622d343100000009032b8e9c8245ecb70eac249da51aad2\
             851efb926aec7bd389496aa1ccd308c9066696c652d6368756e6b"
                .replace(' ', ""),
            "a 36-character UUID is carried as its 16-byte prefix"
        );
    }

    // ---------------------------------------------------------------
    //  The 16-byte target field
    //
    //  One definition, four parties: the producer below, the relay's router
    //  (`resolve_binary_target`), and both receivers' identity checks. These
    //  tests use 36-character UUIDs, because an id short enough to fit the field
    //  cannot tell any of this apart.
    // ---------------------------------------------------------------

    const DESKTOP: &str = "550e8400-e29b-41d4-a716-446655440000";
    const PHONE: &str = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";

    #[test]
    fn the_target_field_is_the_first_sixteen_bytes_nul_padded() {
        assert_eq!(
            binary_target_field(PHONE).to_vec(),
            b"3f2504e0-4f89-11".to_vec(),
            "a 36-character UUID fills the field exactly: 16 characters, 16 bytes"
        );
        assert_eq!(
            binary_target_field("b145d"),
            [b'b', b'1', b'4', b'5', b'd', 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            "a short id is the whole field, NUL-padded"
        );
        assert_eq!(
            binary_target_field(""),
            [0u8; BINARY_DEVICE_ID_LEN],
            "an empty id is all padding, which no reader accepts"
        );
    }

    #[test]
    fn the_target_field_is_the_same_every_time() {
        // It has to be: the relay resolves the field against a routing table it
        // reads whenever a device connects, and the sender computes it whenever a
        // chunk is built. A rule that depended on time, length or ordering would
        // make those two disagree.
        assert_eq!(binary_target_field(DESKTOP), binary_target_field(DESKTOP));
    }

    #[test]
    fn truncating_the_field_again_is_a_no_op() {
        // The relay re-frames with the *resolved full* id, so the field it writes
        // must come out byte-identical to the field it verified — otherwise the
        // tag it stamps covers bytes the recipient never sees.
        let field = binary_target_field(DESKTOP);
        let prefix = parse_binary_target_field(&field).expect("a UUID prefix is UTF-8");
        assert_eq!(binary_target_field(prefix), field);
    }

    #[test]
    fn two_ids_sharing_a_prefix_produce_the_same_field() {
        // The case the resolver has to fail closed on. Nothing here can separate
        // them: they are the same 16 bytes.
        let other = "550e8400-e29b-41d4-b716-446655440000";
        assert_ne!(DESKTOP, other);
        assert_eq!(&DESKTOP[..16], &other[..16]);
        assert_eq!(binary_target_field(DESKTOP), binary_target_field(other));
        assert!(binary_target_matches(DESKTOP, &binary_target_field(other)));
        assert!(binary_target_matches(other, &binary_target_field(DESKTOP)));
    }

    #[test]
    fn a_target_field_matches_only_its_own_device() {
        assert!(binary_target_matches(
            DESKTOP,
            &binary_target_field(DESKTOP)
        ));
        assert!(binary_target_matches(PHONE, &binary_target_field(PHONE)));
        assert!(
            !binary_target_matches(PHONE, &binary_target_field(DESKTOP)),
            "a frame addressed to the desktop is not this phone's"
        );
        // NUL padding is part of the comparison, so a short id is a device and not
        // a prefix: `device-1` does not also claim `device-10`.
        assert!(binary_target_matches(
            "device-1",
            &binary_target_field("device-1")
        ));
        assert!(!binary_target_matches(
            "device-10",
            &binary_target_field("device-1")
        ));
        assert!(!binary_target_matches(
            "device-1",
            &binary_target_field("device-10")
        ));
    }

    #[test]
    fn reading_a_field_back_strips_only_the_padding() {
        assert_eq!(
            parse_binary_target_field(&binary_target_field(PHONE)).expect("UTF-8"),
            "3f2504e0-4f89-11"
        );
        assert_eq!(
            parse_binary_target_field(&binary_target_field("b145d")).expect("UTF-8"),
            "b145d"
        );
        assert_eq!(
            parse_binary_target_field(&[0u8; BINARY_DEVICE_ID_LEN]),
            Err(BinaryTargetFieldError::Empty)
        );
        assert_eq!(
            parse_binary_target_field(&binary_target_field(DESKTOP)),
            Ok("550e8400-e29b-41"),
            "36 characters do not fit; the field reads back as the prefix"
        );
    }

    #[test]
    fn a_non_ascii_id_is_not_silently_mangled() {
        // Byte truncation can split a multi-byte character, and the resulting field
        // is not UTF-8. Every reader refuses it, which is the point: such a device
        // cannot be named, and the frame fails closed instead of naming something
        // else. (A `device_id` is hex and hyphens, so this cannot arise in
        // practice — it is here so the failure mode is pinned rather than assumed.)
        let id = "日本語のデバイス名です";
        let bytes = id.as_bytes();
        assert!(
            bytes.len() > BINARY_DEVICE_ID_LEN && BINARY_DEVICE_ID_LEN % 3 == 1,
            "byte {} of this id must land inside a 3-byte character for the test to \
             mean anything",
            BINARY_DEVICE_ID_LEN
        );
        let field = binary_target_field(id);
        assert_eq!(field.to_vec(), bytes[..BINARY_DEVICE_ID_LEN].to_vec());
        assert_eq!(
            parse_binary_target_field(&field),
            Err(BinaryTargetFieldError::NotUtf8),
            "a split code point must not decode to a shorter id"
        );
    }

    #[test]
    fn the_field_is_covered_by_the_tag() {
        // Retargeting a frame without re-signing it must fail, and the reason it
        // does is that these bytes are inside the MAC input.
        let secret: Vec<u8> = (0u8..32).collect();
        let key = crate::hmac::derive_route_key(&secret, "deadbeef");
        let mut frame = build_binary_frame(&key, "deadbeef", DESKTOP, 1, b"payload");
        assert!(binary_target_matches(
            DESKTOP,
            frame[1..1 + BINARY_DEVICE_ID_LEN]
                .try_into()
                .expect("16 bytes")
        ));
        frame[1..1 + BINARY_DEVICE_ID_LEN].copy_from_slice(&binary_target_field(PHONE));

        let payload = &frame[BINARY_HEADER_LEN..];
        let tag = hex::encode(&frame[BINARY_TAG_OFFSET..BINARY_HEADER_LEN]);
        let mut authenticated = frame[..BINARY_AUTHENTICATED_PREFIX_LEN].to_vec();
        authenticated.extend_from_slice(payload);
        let expected = crate::hmac::compute_hmac(
            &key,
            &hex::encode(binary_mac_input("deadbeef", &authenticated)),
        );
        assert_ne!(tag, expected, "a retargeted frame must not verify");
    }

    // ---------------------------------------------------------------
    //  Convenience constructors
    // ---------------------------------------------------------------

    #[test]
    fn ping_new_has_correct_type() {
        let p = Ping::new();
        assert_eq!(p.msg_type, "ping");
    }

    #[test]
    fn pong_new_has_correct_type() {
        let p = Pong::new();
        assert_eq!(p.msg_type, "pong");
    }

    #[test]
    fn relay_route_new_has_correct_type_and_fields() {
        let r = RelayRoute::new("target-1", json!({"key": "val"}));
        assert_eq!(r.msg_type, "relay_route");
        assert_eq!(r.to_device_id, "target-1");
        assert_eq!(r.payload, json!({"key": "val"}));
    }

    #[test]
    fn relay_auth_new_has_correct_type_and_defaults() {
        let a = RelayAuth::new("dev-1", "token-abc");
        assert_eq!(a.msg_type, "relay_auth");
        assert_eq!(a.device_id, "dev-1");
        assert_eq!(a.relay_token, "token-abc");
        assert!(a.apns_token.is_none());
    }

    #[test]
    fn encrypted_envelope_new_has_correct_type_and_protocol_version() {
        let e = EncryptedEnvelope::new("aaa", "bbb", "ccc");
        assert_eq!(e.msg_type, "encrypted");
        assert_eq!(e.nonce, "aaa");
        assert_eq!(e.hmac, "bbb");
        assert_eq!(e.data, "ccc");
        assert!(e.source_device.is_none());
        assert_eq!(e.protocol_version, Some(PROTOCOL_VERSION));
    }

    // ---------------------------------------------------------------
    //  DiscoveryAnnounce – serde
    // ---------------------------------------------------------------

    #[test]
    fn discovery_announce_roundtrip_minimal() {
        let msg = DiscoveryAnnounce {
            msg_type: "discovery".into(),
            action: "announce".into(),
            protocol_version: None,
            device_id: "dev-1".into(),
            device_name: "My Phone".into(),
            device_type: "phone".into(),
            os: "ios".into(),
            version: "1.0.0".into(),
            battery: None,
            ws_port: None,
            wss_port: None,
            apns_token: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.device_id, "dev-1");
        assert_eq!(restored.device_name, "My Phone");
        assert_eq!(restored.device_type, "phone");
        assert!(restored.battery.is_none());
    }

    #[test]
    fn discovery_announce_roundtrip_all_fields() {
        let msg = DiscoveryAnnounce {
            msg_type: "discovery".into(),
            action: "announce".into(),
            protocol_version: Some(1),
            device_id: "dev-2".into(),
            device_name: "Desktop".into(),
            device_type: "desktop".into(),
            os: "windows".into(),
            version: "2.0.0".into(),
            battery: None,
            ws_port: Some(9528),
            wss_port: Some(9529),
            apns_token: Some("apns-xyz".into()),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.protocol_version, Some(1));
        assert_eq!(restored.ws_port, Some(9528));
        assert_eq!(restored.wss_port, Some(9529));
        assert_eq!(restored.apns_token.as_deref(), Some("apns-xyz"));
    }

    #[test]
    fn discovery_announce_from_json_with_extra_fields_ignored() {
        let json_str = r#"{
            "type": "discovery",
            "action": "announce",
            "device_id": "d1",
            "device_name": "Phone",
            "device_type": "phone",
            "os": "android",
            "version": "1.0",
            "unknown_field": "should be ignored"
        }"#;
        let msg: DiscoveryAnnounce = serde_json::from_str(json_str).unwrap();
        assert_eq!(msg.device_id, "d1");
    }

    #[test]
    fn discovery_announce_missing_required_field_fails() {
        let json_str = r#"{"type": "discovery", "action": "announce"}"#;
        let result = serde_json::from_str::<DiscoveryAnnounce>(json_str);
        assert!(result.is_err());
    }

    #[test]
    fn discovery_announce_battery_value_preserved() {
        let msg = DiscoveryAnnounce {
            msg_type: "discovery".into(),
            action: "announce".into(),
            protocol_version: None,
            device_id: "d1".into(),
            device_name: "Phone".into(),
            device_type: "phone".into(),
            os: "ios".into(),
            version: "1.0".into(),
            battery: Some(85),
            ws_port: None,
            wss_port: None,
            apns_token: None,
        };
        let json_str = serde_json::to_string(&msg).unwrap();
        assert!(json_str.contains(r#""battery":85"#));
        let restored: DiscoveryAnnounce = serde_json::from_str(&json_str).unwrap();
        assert_eq!(restored.battery, Some(85));
    }

    // ---------------------------------------------------------------
    //  DiscoveryRemove
    // ---------------------------------------------------------------

    #[test]
    fn discovery_remove_roundtrip() {
        let msg = DiscoveryRemove {
            msg_type: "discovery".into(),
            action: "remove".into(),
            device_id: "dev-gone".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.device_id, "dev-gone");
        assert_eq!(restored.msg_type, "discovery");
        assert_eq!(restored.action, "remove");
    }

    #[test]
    fn discovery_remove_from_json() {
        let json_str = r#"{"type":"discovery","action":"remove","device_id":"x"}"#;
        let msg: DiscoveryRemove = serde_json::from_str(json_str).unwrap();
        assert_eq!(msg.device_id, "x");
    }

    // ---------------------------------------------------------------
    //  PairingRequest
    // ---------------------------------------------------------------

    #[test]
    fn pairing_request_roundtrip_minimal() {
        let msg = PairingRequest {
            msg_type: "pairing".into(),
            action: "request".into(),
            protocol_version: None,
            token: "abc-123".into(),
            public_key: "aabb".into(),
            device_info: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.token, "abc-123");
        assert!(restored.device_info.is_none());
    }

    #[test]
    fn pairing_request_roundtrip_with_device_info() {
        let msg = PairingRequest {
            msg_type: "pairing".into(),
            action: "request".into(),
            protocol_version: Some(1),
            token: "tok".into(),
            public_key: "pk".into(),
            device_info: Some(DeviceInfo {
                name: "My Phone".into(),
                device_type: "phone".into(),
                os: Some("ios".into()),
                battery: Some(50),
            }),
        };
        let restored = roundtrip(&msg);
        let di = restored.device_info.unwrap();
        assert_eq!(di.name, "My Phone");
        assert_eq!(di.os, Some("ios".into()));
        assert_eq!(di.battery, Some(50));
    }

    #[test]
    fn pairing_request_missing_required_field_fails() {
        let json_str = r#"{"type":"pairing","action":"request"}"#;
        let result = serde_json::from_str::<PairingRequest>(json_str);
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------
    //  PairingAccept
    // ---------------------------------------------------------------

    #[test]
    fn pairing_accept_roundtrip() {
        let msg = PairingAccept {
            msg_type: "pairing".into(),
            action: "accept".into(),
            protocol_version: None,
            public_key: "ccdd".into(),
            device_info: None,
            device_id: None,
            hub_device_id: None,
            relay_url: None,
            relay_token: None,
            relay_cert_pin: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.public_key, "ccdd");
    }

    #[test]
    fn pairing_accept_with_device_info() {
        let msg = PairingAccept {
            msg_type: "pairing".into(),
            action: "accept".into(),
            protocol_version: Some(1),
            public_key: "eeff".into(),
            device_info: Some(DeviceInfo {
                name: "Desktop".into(),
                device_type: "desktop".into(),
                os: Some("macos".into()),
                battery: None,
            }),
            device_id: Some("assigned-1".into()),
            hub_device_id: Some("hub-1".into()),
            relay_url: None,
            relay_token: None,
            relay_cert_pin: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.device_info.unwrap().os, Some("macos".into()));
        // The assigned id has to survive the round trip, or the peer learns
        // nothing and every envelope it sends names it wrong.
        assert_eq!(restored.device_id.as_deref(), Some("assigned-1"));
        // So does the hub's own id: without it the peer cannot derive the key
        // that verifies a relayed frame, and drops every one of them.
        assert_eq!(restored.hub_device_id.as_deref(), Some("hub-1"));
    }

    /// The three relay fields are new, and an already-shipped peer deserialises
    /// `PairingAccept` with a fixed field list. Absence — not an empty string —
    /// has to be a valid frame, or every pairing with an older peer breaks.
    #[test]
    fn pairing_accept_without_the_relay_fields_still_deserialises() {
        let restored: PairingAccept =
            serde_json::from_str(r#"{"type":"pairing","action":"accept","public_key":"aabb"}"#)
                .expect("an accept from a peer that predates the relay fields must still parse");

        assert_eq!(restored.public_key, "aabb");
        assert_eq!(restored.relay_url, None);
        assert_eq!(restored.relay_token, None);
        assert_eq!(restored.relay_cert_pin, None);
    }

    /// …and they must not appear at all when they are absent. `null` is not the
    /// same thing: a client that reads `message['relay_url']` and hands it
    /// straight to `Uri.parse` gets a different failure than one that finds the
    /// key missing and keeps what it had.
    #[test]
    fn the_relay_fields_are_omitted_rather_than_null() {
        let json = serde_json::to_value(PairingAccept {
            msg_type: "pairing".into(),
            action: "accept".into(),
            protocol_version: None,
            public_key: "aabb".into(),
            device_info: None,
            device_id: None,
            hub_device_id: None,
            relay_url: None,
            relay_token: None,
            relay_cert_pin: None,
        })
        .expect("serializes");
        let object = json.as_object().expect("an accept frame is an object");
        for field in ["relay_url", "relay_token", "relay_cert_pin"] {
            assert!(
                !object.contains_key(field),
                "{field} must be omitted when absent, not serialised as null: {json}"
            );
        }
    }

    /// The other direction: a desktop that *does* host a relay has to be able to
    /// say so, and all three values have to survive the wire intact. Dropping
    /// the token or the pin here is exactly the bug this feature exists to fix —
    /// the peer would have a URL it cannot authenticate to.
    #[test]
    fn pairing_accept_roundtrips_the_relay_fields() {
        let msg = PairingAccept {
            msg_type: "pairing".into(),
            action: "accept".into(),
            protocol_version: Some(PROTOCOL_VERSION),
            public_key: "aabb".into(),
            device_info: None,
            device_id: Some("assigned-1".into()),
            hub_device_id: Some("hub-1".into()),
            relay_url: Some("wss://192.168.1.50:9529".into()),
            relay_token: Some("0123456789abcdef".into()),
            relay_cert_pin: Some("sha256/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=".into()),
        };

        let restored = roundtrip(&msg);
        assert_eq!(
            restored.relay_url.as_deref(),
            Some("wss://192.168.1.50:9529")
        );
        assert_eq!(restored.relay_token.as_deref(), Some("0123456789abcdef"));
        assert_eq!(
            restored.relay_cert_pin.as_deref(),
            Some("sha256/AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=")
        );

        // The URL must not have absorbed the hub's own port or been rewritten
        // to the plaintext listener: it is the one a peer on another network
        // dials.
        let json = serde_json::to_value(&msg).expect("serializes");
        assert_eq!(json["relay_url"], "wss://192.168.1.50:9529");
        assert!(
            json.get("relay_token").and_then(|v| v.as_str()) == Some("0123456789abcdef"),
            "the token must be present when it is Some"
        );
    }

    // ---------------------------------------------------------------
    //  PairingRevoke
    // ---------------------------------------------------------------

    #[test]
    fn pairing_revoke_roundtrip_minimal() {
        let msg = PairingRevoke {
            msg_type: "pairing".into(),
            action: "revoke".into(),
            protocol_version: None,
            reason: None,
        };
        let restored = roundtrip(&msg);
        assert!(restored.reason.is_none());
    }

    #[test]
    fn pairing_revoke_with_reason() {
        let msg = PairingRevoke {
            msg_type: "pairing".into(),
            action: "revoke".into(),
            protocol_version: Some(1),
            reason: Some("user requested".into()),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.reason.as_deref(), Some("user requested"));
    }

    // ---------------------------------------------------------------
    //  ClipboardSync
    // ---------------------------------------------------------------

    #[test]
    fn clipboard_sync_roundtrip() {
        let msg = ClipboardSync {
            msg_type: "clipboard".into(),
            action: "sync".into(),
            content: "hello world".into(),
            mime: "text/plain".into(),
            source_device: "dev-1".into(),
            timestamp: 1_700_000_000,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.content, "hello world");
        assert_eq!(restored.mime, "text/plain");
        assert_eq!(restored.timestamp, 1_700_000_000);
    }

    #[test]
    fn clipboard_sync_from_json() {
        let json_str = r#"{
            "type":"clipboard","action":"sync",
            "content":"test","mime":"text/html",
            "source_device":"d1","timestamp":123
        }"#;
        let msg: ClipboardSync = serde_json::from_str(json_str).unwrap();
        assert_eq!(msg.mime, "text/html");
    }

    // ---------------------------------------------------------------
    //  ClipboardRequest
    // ---------------------------------------------------------------

    #[test]
    fn clipboard_request_roundtrip_minimal() {
        let msg = ClipboardRequest {
            msg_type: "clipboard".into(),
            action: "request".into(),
            mime: None,
        };
        let restored = roundtrip(&msg);
        assert!(restored.mime.is_none());
    }

    #[test]
    fn clipboard_request_with_mime() {
        let msg = ClipboardRequest {
            msg_type: "clipboard".into(),
            action: "request".into(),
            mime: Some("image/png".into()),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.mime.as_deref(), Some("image/png"));
    }

    // ---------------------------------------------------------------
    //  NotificationPost
    // ---------------------------------------------------------------

    #[test]
    fn notification_post_roundtrip_minimal() {
        let msg = NotificationPost {
            msg_type: "notification".into(),
            action: "post".into(),
            id: "n-1".into(),
            device_id: "dev-1".into(),
            app: "Messages".into(),
            title: "New Message".into(),
            body: "Hello!".into(),
            timestamp: 1_700_000_000,
            actions: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.id, "n-1");
        assert_eq!(restored.title, "New Message");
        assert!(restored.actions.is_none());
    }

    #[test]
    fn notification_post_with_actions() {
        let msg = NotificationPost {
            msg_type: "notification".into(),
            action: "post".into(),
            id: "n-2".into(),
            device_id: "dev-1".into(),
            app: "Email".into(),
            title: "Mail".into(),
            body: "You've got mail".into(),
            timestamp: 1_700_000_000,
            actions: Some(vec!["Reply".into(), "Delete".into()]),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.actions.unwrap(), vec!["Reply", "Delete"]);
    }

    // ---------------------------------------------------------------
    //  NotificationDismiss
    // ---------------------------------------------------------------

    #[test]
    fn notification_dismiss_roundtrip() {
        let msg = NotificationDismiss {
            msg_type: "notification".into(),
            action: "dismiss".into(),
            id: "n-1".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.id, "n-1");
    }

    #[test]
    fn notification_mark_read_roundtrip() {
        let msg = NotificationMarkRead {
            msg_type: "notification".into(),
            action: "mark_read".into(),
            id: "n-1".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.id, "n-1");
    }

    // ---------------------------------------------------------------
    //  NotificationReply
    // ---------------------------------------------------------------

    #[test]
    fn notification_reply_roundtrip() {
        let msg = NotificationReply {
            msg_type: "notification".into(),
            action: "reply".into(),
            id: "n-1".into(),
            text: "Got it!".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.text, "Got it!");
    }

    // ---------------------------------------------------------------
    //  FileRequest
    // ---------------------------------------------------------------

    #[test]
    fn file_request_roundtrip_minimal() {
        let msg = FileRequest {
            msg_type: "file".into(),
            action: "request".into(),
            id: "f-1".into(),
            name: "photo.jpg".into(),
            size: 1024,
            mime: "image/jpeg".into(),
            from: "dev-a".into(),
            to: None,
            checksum: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.name, "photo.jpg");
        assert_eq!(restored.size, 1024);
        assert!(restored.to.is_none());
    }

    #[test]
    fn file_request_with_all_optional_fields() {
        let msg = FileRequest {
            msg_type: "file".into(),
            action: "request".into(),
            id: "f-2".into(),
            name: "doc.pdf".into(),
            size: 2_048_000,
            mime: "application/pdf".into(),
            from: "dev-a".into(),
            to: Some("dev-b".into()),
            checksum: Some("sha256:abc".into()),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.to.as_deref(), Some("dev-b"));
        assert_eq!(restored.checksum.as_deref(), Some("sha256:abc"));
    }

    // ---------------------------------------------------------------
    //  FileAccept
    // ---------------------------------------------------------------

    #[test]
    fn file_accept_roundtrip() {
        let msg = FileAccept {
            msg_type: "file".into(),
            action: "accept".into(),
            id: "f-1".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.id, "f-1");
    }

    // ---------------------------------------------------------------
    //  FileChunk
    // ---------------------------------------------------------------

    #[test]
    fn file_chunk_roundtrip_minimal() {
        let msg = FileChunk {
            msg_type: "file".into(),
            action: "chunk".into(),
            id: "f-1".into(),
            index: 0,
            data: "base64data".into(),
            total: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.index, 0);
        assert!(restored.total.is_none());
    }

    #[test]
    fn file_chunk_with_total() {
        let msg = FileChunk {
            msg_type: "file".into(),
            action: "chunk".into(),
            id: "f-1".into(),
            index: 5,
            data: "chunk5data".into(),
            total: Some(10),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.index, 5);
        assert_eq!(restored.total, Some(10));
    }

    // ---------------------------------------------------------------
    //  FileProgress
    // ---------------------------------------------------------------

    #[test]
    fn file_progress_roundtrip() {
        let msg = FileProgress {
            msg_type: "file".into(),
            action: "progress".into(),
            id: "f-1".into(),
            percent: 75,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.percent, 75);
    }

    #[test]
    fn file_progress_at_zero() {
        let msg = FileProgress {
            msg_type: "file".into(),
            action: "progress".into(),
            id: "f".into(),
            percent: 0,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.percent, 0);
    }

    #[test]
    fn file_progress_at_100() {
        let msg = FileProgress {
            msg_type: "file".into(),
            action: "progress".into(),
            id: "f".into(),
            percent: 100,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.percent, 100);
    }

    // ---------------------------------------------------------------
    //  FileComplete
    // ---------------------------------------------------------------

    #[test]
    fn file_complete_roundtrip_minimal() {
        let msg = FileComplete {
            msg_type: "file".into(),
            action: "complete".into(),
            id: "f-1".into(),
            path: None,
        };
        let restored = roundtrip(&msg);
        assert!(restored.path.is_none());
    }

    #[test]
    fn file_complete_with_path() {
        let msg = FileComplete {
            msg_type: "file".into(),
            action: "complete".into(),
            id: "f-1".into(),
            path: Some("/downloads/photo.jpg".into()),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.path.as_deref(), Some("/downloads/photo.jpg"));
    }

    // ---------------------------------------------------------------
    //  FileCancel
    // ---------------------------------------------------------------

    #[test]
    fn file_cancel_roundtrip() {
        let msg = FileCancel {
            msg_type: "file".into(),
            action: "cancel".into(),
            id: "f-1".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.id, "f-1");
    }

    // ---------------------------------------------------------------
    //  FileResume
    // ---------------------------------------------------------------

    #[test]
    fn file_resume_roundtrip_minimal() {
        let msg = FileResume {
            msg_type: "file".into(),
            action: "resume".into(),
            id: "f-1".into(),
            name: "doc.pdf".into(),
            size: 1024,
            mime: "application/pdf".into(),
            from: "dev-a".into(),
            checksum: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.name, "doc.pdf");
    }

    #[test]
    fn file_resume_with_checksum() {
        let msg = FileResume {
            msg_type: "file".into(),
            action: "resume".into(),
            id: "f-1".into(),
            name: "video.mp4".into(),
            size: 50_000_000,
            mime: "video/mp4".into(),
            from: "dev-a".into(),
            checksum: Some("sha256:abc123".into()),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.checksum.as_deref(), Some("sha256:abc123"));
    }

    // ---------------------------------------------------------------
    //  FileResumeAck
    // ---------------------------------------------------------------

    #[test]
    fn file_resume_ack_roundtrip() {
        let msg = FileResumeAck {
            msg_type: "file".into(),
            action: "resume_ack".into(),
            id: "f-1".into(),
            chunks_loaded: 42,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.chunks_loaded, 42);
    }

    // ---------------------------------------------------------------
    //  BinaryFileMetadata
    // ---------------------------------------------------------------

    #[test]
    fn binary_file_metadata_roundtrip_minimal() {
        let msg = BinaryFileMetadata {
            id: "f-1".into(),
            index: 0,
            total: None,
        };
        let restored = roundtrip(&msg);
        assert!(restored.total.is_none());
    }

    #[test]
    fn binary_file_metadata_with_total() {
        let msg = BinaryFileMetadata {
            id: "f-1".into(),
            index: 10,
            total: Some(100),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.total, Some(100));
    }

    // ---------------------------------------------------------------
    //  AudioStreamStart
    // ---------------------------------------------------------------

    #[test]
    fn audio_stream_start_roundtrip() {
        let msg = AudioStreamStart {
            msg_type: "audio".into(),
            action: "stream_start".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.msg_type, "audio");
        assert_eq!(restored.action, "stream_start");
    }

    // ---------------------------------------------------------------
    //  AudioStreamStop
    // ---------------------------------------------------------------

    #[test]
    fn audio_stream_stop_roundtrip() {
        let msg = AudioStreamStop {
            msg_type: "audio".into(),
            action: "stream_stop".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.action, "stream_stop");
    }

    // ---------------------------------------------------------------
    //  AudioStreamStarted
    // ---------------------------------------------------------------

    #[test]
    fn audio_stream_started_roundtrip() {
        let msg = AudioStreamStarted {
            msg_type: "audio".into(),
            action: "stream_started".into(),
            from: "dev-audio".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.from, "dev-audio");
    }

    // ---------------------------------------------------------------
    //  AudioStreamData
    // ---------------------------------------------------------------

    #[test]
    fn audio_stream_data_roundtrip() {
        let msg = AudioStreamData {
            msg_type: "audio".into(),
            action: "stream_data".into(),
            data: "base64pcmdata".into(),
            format: "pcm16".into(),
            sample_rate: 16000,
            channels: 1,
            from: "dev-audio".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.sample_rate, 16000);
        assert_eq!(restored.channels, 1);
        assert_eq!(restored.format, "pcm16");
    }

    // ---------------------------------------------------------------
    //  AudioPlaybackStart
    // ---------------------------------------------------------------

    #[test]
    fn audio_playback_start_roundtrip() {
        let msg = AudioPlaybackStart {
            msg_type: "audio".into(),
            action: "playback_start".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.action, "playback_start");
    }

    // ---------------------------------------------------------------
    //  AudioPlaybackStop
    // ---------------------------------------------------------------

    #[test]
    fn audio_playback_stop_roundtrip() {
        let msg = AudioPlaybackStop {
            msg_type: "audio".into(),
            action: "playback_stop".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.action, "playback_stop");
    }

    // ---------------------------------------------------------------
    //  AudioPlaybackStarted
    // ---------------------------------------------------------------

    #[test]
    fn audio_playback_started_roundtrip() {
        let msg = AudioPlaybackStarted {
            msg_type: "audio".into(),
            action: "playback_started".into(),
            from: "dev-speaker".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.from, "dev-speaker");
    }

    // ---------------------------------------------------------------
    //  AudioPlaybackStopped
    // ---------------------------------------------------------------

    #[test]
    fn audio_playback_stopped_roundtrip() {
        let msg = AudioPlaybackStopped {
            msg_type: "audio".into(),
            action: "playback_stopped".into(),
            from: "dev-speaker".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.action, "playback_stopped");
        assert_eq!(restored.from, "dev-speaker");
    }

    // ---------------------------------------------------------------
    //  AudioPlaybackData
    // ---------------------------------------------------------------

    #[test]
    fn audio_playback_data_roundtrip() {
        let msg = AudioPlaybackData {
            msg_type: "audio".into(),
            action: "playback_data".into(),
            data: "pcmdata".into(),
            format: "pcm16".into(),
            sample_rate: 44100,
            channels: 2,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.sample_rate, 44100);
        assert_eq!(restored.channels, 2);
    }

    // ---------------------------------------------------------------
    //  ScreenMirrorStart
    // ---------------------------------------------------------------

    #[test]
    fn screen_mirror_start_roundtrip_minimal() {
        let msg = ScreenMirrorStart {
            msg_type: "screen_mirror".into(),
            action: "start".into(),
            quality: None,
            fps: None,
        };
        let restored = roundtrip(&msg);
        assert!(restored.quality.is_none());
        assert!(restored.fps.is_none());
    }

    #[test]
    fn screen_mirror_start_with_quality() {
        let msg = ScreenMirrorStart {
            msg_type: "screen_mirror".into(),
            action: "start".into(),
            quality: Some("high".into()),
            fps: Some(30),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.quality.as_deref(), Some("high"));
        assert_eq!(restored.fps, Some(30));
    }

    /// The `fps` field was added because the capture side hardcoded 15 fps
    /// while both clients already sent an `fps` value that the protocol
    /// silently dropped. Pin the wire spelling so it cannot drift again.
    #[test]
    fn screen_mirror_start_serializes_fps_on_the_wire() {
        let msg = ScreenMirrorStart {
            msg_type: "screen_mirror".into(),
            action: "start".into(),
            quality: Some("medium".into()),
            fps: Some(15),
        };
        let value = serde_json::to_value(&msg).unwrap();
        assert_eq!(value["fps"], serde_json::json!(15));
        assert_eq!(value["quality"], serde_json::json!("medium"));
    }

    // ---------------------------------------------------------------
    //  ScreenMirrorStop
    // ---------------------------------------------------------------

    #[test]
    fn screen_mirror_stop_roundtrip() {
        let msg = ScreenMirrorStop {
            msg_type: "screen_mirror".into(),
            action: "stop".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.action, "stop");
    }

    // ---------------------------------------------------------------
    //  ScreenMirrorFrame
    // ---------------------------------------------------------------

    #[test]
    fn screen_mirror_frame_roundtrip_minimal() {
        let msg = ScreenMirrorFrame {
            msg_type: "screen_mirror".into(),
            action: "frame".into(),
            data: "base64jpeg".into(),
            format: "jpeg".into(),
            width: 1920,
            height: 1080,
            from_desktop: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.width, 1920);
        assert_eq!(restored.height, 1080);
    }

    #[test]
    fn screen_mirror_frame_with_from_desktop() {
        let msg = ScreenMirrorFrame {
            msg_type: "screen_mirror".into(),
            action: "frame".into(),
            data: "jpeg".into(),
            format: "jpeg".into(),
            width: 800,
            height: 600,
            from_desktop: Some(true),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.from_desktop, Some(true));
    }

    // ---------------------------------------------------------------
    //  ScreenMirrorCaptureStopped
    // ---------------------------------------------------------------

    #[test]
    fn screen_mirror_capture_stopped_roundtrip() {
        let msg = ScreenMirrorCaptureStopped {
            msg_type: "screen_mirror".into(),
            action: "capture_stopped".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.action, "capture_stopped");
    }

    // ---------------------------------------------------------------
    //  ScreenMirrorTouch
    // ---------------------------------------------------------------

    #[test]
    fn screen_mirror_touch_roundtrip_minimal() {
        let msg = ScreenMirrorTouch {
            msg_type: "screen_mirror".into(),
            action: "touch".into(),
            x: 100.5,
            y: 200.3,
            action_type: None,
        };
        let restored = roundtrip(&msg);
        assert!((restored.x - 100.5).abs() < f64::EPSILON);
        assert!((restored.y - 200.3).abs() < f64::EPSILON);
    }

    #[test]
    fn screen_mirror_touch_with_action_type() {
        let msg = ScreenMirrorTouch {
            msg_type: "screen_mirror".into(),
            action: "touch".into(),
            x: 0.0,
            y: 0.0,
            action_type: Some("double_tap".into()),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.action_type.as_deref(), Some("double_tap"));
    }

    /// `move` (hover / drag update) is a legal wire value: the Android
    /// accessibility injector handles it, so the desktop can drive drags.
    #[test]
    fn screen_mirror_touch_with_move_action_type() {
        let msg = ScreenMirrorTouch {
            msg_type: "screen_mirror".into(),
            action: "touch".into(),
            x: 0.5,
            y: 0.5,
            action_type: Some("move".into()),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.action_type.as_deref(), Some("move"));
    }

    /// An actionType outside the enum must be rejected at the Rust boundary so
    /// it can never reach an injector as a default tap.
    #[test]
    fn screen_mirror_touch_rejects_unknown_action_type() {
        let bad =
            r#"{"type":"screen_mirror","action":"touch","x":0.5,"y":0.5,"actionType":"explode"}"#;
        assert!(
            serde_json::from_str::<ScreenMirrorTouch>(bad).is_err(),
            "unknown actionType must fail deserialization"
        );
    }

    // ---------------------------------------------------------------
    //  ScreenMirrorKey
    // ---------------------------------------------------------------

    #[test]
    fn screen_mirror_key_roundtrip_minimal() {
        let msg = ScreenMirrorKey {
            msg_type: "screen_mirror".into(),
            action: "key".into(),
            key: "Enter".into(),
            modifiers: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.key, "Enter");
    }

    #[test]
    fn screen_mirror_key_with_modifiers() {
        let msg = ScreenMirrorKey {
            msg_type: "screen_mirror".into(),
            action: "key".into(),
            key: "C".into(),
            // Schema `modifiers` enum: shift | control | alt | meta.
            modifiers: Some(vec!["control".into(), "shift".into()]),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.modifiers.unwrap(), vec!["control", "shift"]);
    }

    /// `"ctrl"` is *not* a wire value — the enum spells it `"control"`. Pin
    /// the legal set so a client cannot quietly send a modifier the receiving
    /// injector has to guess at.
    #[test]
    fn screen_mirror_key_modifiers_must_be_schema_names() {
        let good = r#"{"type":"screen_mirror","action":"key","key":"c","modifiers":["control","shift","alt","meta"]}"#;
        let parsed: ScreenMirrorKey = serde_json::from_str(good).expect("schema modifier names");
        assert_eq!(parsed.modifiers.unwrap().len(), 4);
    }

    // ---------------------------------------------------------------
    //  ScreenMirrorScroll
    // ---------------------------------------------------------------

    #[test]
    fn screen_mirror_scroll_roundtrip() {
        let msg = ScreenMirrorScroll {
            msg_type: "screen_mirror".into(),
            action: "scroll".into(),
            dx: 0.0,
            dy: -50.0,
        };
        let restored = roundtrip(&msg);
        assert!((restored.dy - (-50.0)).abs() < f64::EPSILON);
    }

    // ---------------------------------------------------------------
    //  RemoteInputMove
    // ---------------------------------------------------------------

    #[test]
    fn remote_input_move_roundtrip() {
        let msg = RemoteInputMove {
            msg_type: "remote_input".into(),
            action: "move".into(),
            dx: 10.5,
            dy: -3.2,
        };
        let restored = roundtrip(&msg);
        assert!((restored.dx - 10.5).abs() < f64::EPSILON);
    }

    // ---------------------------------------------------------------
    //  RemoteInputClick
    // ---------------------------------------------------------------

    #[test]
    fn remote_input_click_roundtrip() {
        let msg = RemoteInputClick {
            msg_type: "remote_input".into(),
            action: "click".into(),
            button: "left".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.button, "left");
    }

    // ---------------------------------------------------------------
    //  RemoteInputScroll
    // ---------------------------------------------------------------

    #[test]
    fn remote_input_scroll_roundtrip() {
        let msg = RemoteInputScroll {
            msg_type: "remote_input".into(),
            action: "scroll".into(),
            dx: 0.0,
            dy: 120.0,
        };
        let restored = roundtrip(&msg);
        assert!((restored.dy - 120.0).abs() < f64::EPSILON);
    }

    // ---------------------------------------------------------------
    //  RemoteInputKey
    // ---------------------------------------------------------------

    /// `remote_input` had no keyboard message at all: the desktop UI sent
    /// `action: "key_press"` and there was nothing on the wire to receive it.
    /// This pins the shape that now carries a key chord.
    #[test]
    fn remote_input_key_roundtrip() {
        let msg = RemoteInputKey {
            msg_type: "remote_input".into(),
            action: "key".into(),
            key: "Enter".into(),
            modifiers: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.msg_type, "remote_input");
        assert_eq!(restored.action, "key");
        assert_eq!(restored.key, "Enter");
        assert!(restored.modifiers.is_none());
    }

    #[test]
    fn remote_input_key_with_modifiers() {
        let msg = RemoteInputKey {
            msg_type: "remote_input".into(),
            action: "key".into(),
            key: "c".into(),
            modifiers: Some(vec!["control".into()]),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.modifiers.unwrap(), vec!["control"]);
    }

    /// `action` is the dispatch key, so a legacy `action: "event"` frame is
    /// rejected by routing (see the `remote_input` handler's action dispatch),
    /// not by this struct — the struct only guarantees the payload shape.
    #[test]
    fn remote_input_key_dispatches_only_on_action_key() {
        let legacy =
            r#"{"type":"remote_input","action":"event","event_type":"key_press","key":"a"}"#;
        let value: serde_json::Value = serde_json::from_str(legacy).unwrap();
        assert_ne!(
            value["action"].as_str(),
            Some("key"),
            "the legacy event envelope must not be routed to the key branch"
        );

        let good = r#"{"type":"remote_input","action":"key","key":"a"}"#;
        let parsed: RemoteInputKey = serde_json::from_str(good).expect("canonical key frame");
        assert_eq!(parsed.action, "key");
    }

    /// `key` is required: a frame without it is not a chord.
    #[test]
    fn remote_input_key_requires_key_field() {
        let missing = r#"{"type":"remote_input","action":"key"}"#;
        assert!(serde_json::from_str::<RemoteInputKey>(missing).is_err());
    }

    // ---------------------------------------------------------------
    //  AutomationRuleMessage
    // ---------------------------------------------------------------

    #[test]
    fn automation_rule_message_roundtrip() {
        let msg = AutomationRuleMessage {
            msg_type: "automation".into(),
            action: "rule".into(),
            id: "rule-1".into(),
            name: "Auto Reply".into(),
            trigger: AutomationTrigger {
                trigger_type: "notification".into(),
                device_id: None,
                time: None,
                below: None,
                ssid: None,
                app_package: None,
            },
            rule_action: AutomationActionPayload {
                action_type: "send_notification".into(),
                title: Some("Auto Reply".into()),
                body: Some("I'm busy".into()),
                profile: None,
                device_id: None,
                command: None,
                enabled: None,
                url: None,
                app_package: None,
                state: None,
            },
            enabled: true,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.name, "Auto Reply");
        assert!(restored.enabled);
        assert_eq!(restored.trigger.trigger_type, "notification");
        assert_eq!(restored.rule_action.action_type, "send_notification");
    }

    #[test]
    fn automation_rule_message_disabled() {
        let msg = AutomationRuleMessage {
            msg_type: "automation".into(),
            action: "rule".into(),
            id: "rule-2".into(),
            name: "Disabled Rule".into(),
            trigger: AutomationTrigger {
                trigger_type: "time".into(),
                device_id: None,
                time: Some("09:00".into()),
                below: None,
                ssid: None,
                app_package: None,
            },
            rule_action: AutomationActionPayload {
                action_type: "toggle_wifi".into(),
                title: None,
                body: None,
                profile: None,
                device_id: None,
                command: None,
                enabled: Some(true),
                url: None,
                app_package: None,
                state: None,
            },
            enabled: false,
        };
        let restored = roundtrip(&msg);
        assert!(!restored.enabled);
        assert_eq!(restored.trigger.time.as_deref(), Some("09:00"));
    }

    // ---------------------------------------------------------------
    //  AutomationDelete
    // ---------------------------------------------------------------

    #[test]
    fn automation_delete_roundtrip() {
        let msg = AutomationDelete {
            msg_type: "automation".into(),
            action: "delete".into(),
            rule_id: "rule-1".into(),
        };
        let json_str = serde_json::to_string(&msg).unwrap();
        let restored: AutomationDelete = serde_json::from_str(&json_str).unwrap();
        assert_eq!(restored.rule_id, "rule-1");
    }

    #[test]
    fn automation_delete_deserializes_with_id_alias() {
        // The serde alias allows "id" or "rule_id" as field name
        let json_str = r#"{"type":"automation","action":"delete","id":"rule-alias"}"#;
        let msg: AutomationDelete = serde_json::from_str(json_str).unwrap();
        assert_eq!(msg.rule_id, "rule-alias");
    }

    // ---------------------------------------------------------------
    //  AutomationSync
    // ---------------------------------------------------------------

    #[test]
    fn automation_sync_roundtrip_minimal() {
        let msg = AutomationSync {
            msg_type: "automation".into(),
            action: "sync".into(),
            rules: vec![],
            full_sync: None,
        };
        let restored = roundtrip(&msg);
        assert!(restored.rules.is_empty());
        assert!(restored.full_sync.is_none());
    }

    #[test]
    fn automation_sync_with_rules_and_full_sync() {
        let msg = AutomationSync {
            msg_type: "automation".into(),
            action: "sync".into(),
            rules: vec![json!({"id": "r1", "name": "test"})],
            full_sync: Some(true),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.rules.len(), 1);
        assert_eq!(restored.full_sync, Some(true));
    }

    // ---------------------------------------------------------------
    //  AutomationTriggered
    // ---------------------------------------------------------------

    #[test]
    fn automation_triggered_roundtrip_minimal() {
        let msg = AutomationTriggered {
            msg_type: "automation".into(),
            action: "triggered".into(),
            id: "rule-1".into(),
            trigger_type: "battery_level".into(),
            device_id: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.id, "rule-1");
        assert!(restored.device_id.is_none());
    }

    #[test]
    fn automation_triggered_with_device_id() {
        let msg = AutomationTriggered {
            msg_type: "automation".into(),
            action: "triggered".into(),
            id: "rule-1".into(),
            trigger_type: "wifi_change".into(),
            device_id: Some("dev-1".into()),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.device_id.as_deref(), Some("dev-1"));
    }

    // ---------------------------------------------------------------
    //  CallMessage
    // ---------------------------------------------------------------

    #[test]
    fn call_message_roundtrip_minimal() {
        let msg = CallMessage {
            msg_type: "call".into(),
            action: "incoming".into(),
            call_id: None,
            to_device_id: None,
            route: None,
            number: None,
            name: None,
            device_id: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.action, "incoming");
        assert!(restored.call_id.is_none());
    }

    #[test]
    fn call_message_all_fields() {
        let msg = CallMessage {
            msg_type: "call".into(),
            action: "accept".into(),
            call_id: Some("call-abc".into()),
            to_device_id: Some("dev-1".into()),
            route: Some("wss://relay.example.com".into()),
            number: Some("+15551234567".into()),
            name: Some("Ada Lovelace".into()),
            device_id: Some("phone".into()),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.call_id.as_deref(), Some("call-abc"));
        assert_eq!(restored.to_device_id.as_deref(), Some("dev-1"));
        assert_eq!(restored.route.as_deref(), Some("wss://relay.example.com"));
        assert_eq!(restored.number.as_deref(), Some("+15551234567"));
        assert_eq!(restored.name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(restored.device_id.as_deref(), Some("phone"));
    }

    /// The phone sends `number`/`name` for `action: "incoming"`; both must
    /// survive a round-trip and validate against `schema.json`.
    #[test]
    fn call_message_incoming_payload_matches_phone_sender() {
        // Mirrors apps/mobile/lib/services/call_service.dart:184-190.
        let raw = r#"{
            "type": "call",
            "action": "incoming",
            "call_id": "call_1700000000000",
            "number": "+15551234567",
            "name": "Ada Lovelace"
        }"#;
        let msg: CallMessage = serde_json::from_str(raw).expect("call/incoming must deserialize");
        assert_eq!(msg.number.as_deref(), Some("+15551234567"));
        assert_eq!(msg.name.as_deref(), Some("Ada Lovelace"));

        let v = serde_json::to_value(&msg).unwrap();
        let validator = validator();
        assert!(
            is_valid(&validator, &v),
            "phone-sent call/incoming must satisfy schema.json: {}",
            v
        );
    }

    // ---------------------------------------------------------------
    //  SmsSend
    // ---------------------------------------------------------------

    #[test]
    fn sms_send_roundtrip() {
        let msg = SmsSend {
            msg_type: "sms".into(),
            action: "send".into(),
            to: "+15551234567".into(),
            body: "Hello from desktop!".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.to, "+15551234567");
        assert_eq!(restored.body, "Hello from desktop!");
    }

    // ---------------------------------------------------------------
    //  SmsSync / SmsNew / SmsSent
    // ---------------------------------------------------------------

    /// Every real payload shape emitted by `apps/mobile/lib/services/sms_service.dart`
    /// must be representable in Rust *and* satisfy `schema.json`. The Rust type
    /// is the source of truth, so a new sender shape is a compile error here.
    #[test]
    fn sms_phone_sender_payloads_all_validate_against_schema() {
        // sms_service.dart:426-430 (syncToDesktop) with one thread from
        // threadsToJson() (sms_service.dart:411-421) and one SmsMessage.
        let sync = json!({
            "type": "sms", "action": "sync",
            "threads": [{
                "thread_id": "t_1700000000000",
                "address": "+15551234567",
                "name": "Ada Lovelace",
                "snippet": "hello",
                "unread_count": 1,
                "timestamp": 1700000000,
                "messages": [{
                    "id": "in_1700000000000",
                    "address": "+15551234567",
                    "body": "hello",
                    "timestamp": 1700000000,
                    "read": false,
                    "is_outgoing": false
                }]
            }]
        });
        // sms_service.dart:233-239 (incoming SMS forwarded to the desktop).
        let new = json!({
            "type": "sms", "action": "new",
            "from": "+15551234567", "body": "hello", "timestamp": 1700000000
        });
        // sms_service.dart:366-372 (sendSms success relay).
        let sent = json!({
            "type": "sms", "action": "sent",
            "to": "+15551234567", "body": "hello", "timestamp": 1700000000
        });

        let validator = validator();
        for (label, sample) in [("sync", &sync), ("new", &new), ("sent", &sent)] {
            assert!(
                is_valid(&validator, sample),
                "phone-sent sms/{} must satisfy schema.json, got: {}",
                label,
                sample
            );
        }
    }

    #[test]
    fn sms_sync_roundtrip_with_nested_thread() {
        let msg = SmsSync {
            msg_type: "sms".into(),
            action: "sync".into(),
            threads: vec![SmsThread {
                thread_id: "t_1700000000000".into(),
                address: "+15551234567".into(),
                name: Some("Ada Lovelace".into()),
                snippet: "hello".into(),
                unread_count: 1,
                timestamp: 1_700_000_000,
                messages: vec![SmsMessage {
                    id: "in_1700000000000".into(),
                    address: "+15551234567".into(),
                    body: "hello".into(),
                    timestamp: 1_700_000_000,
                    read: false,
                    is_outgoing: false,
                }],
            }],
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.threads.len(), 1);
        let thread = &restored.threads[0];
        assert_eq!(thread.thread_id, "t_1700000000000");
        assert_eq!(thread.address, "+15551234567");
        assert_eq!(thread.name.as_deref(), Some("Ada Lovelace"));
        assert_eq!(thread.unread_count, 1);
        assert_eq!(thread.messages.len(), 1);
        assert_eq!(thread.messages[0].body, "hello");
        assert!(!thread.messages[0].is_outgoing);
    }

    /// `SmsThread.name` is emitted as JSON `null` by the phone when the
    /// address book has no match, so it must deserialize from `null`.
    #[test]
    fn sms_thread_name_accepts_null() {
        let raw = r#"{
            "thread_id": "t_1", "address": "+1555", "name": null,
            "snippet": "s", "unread_count": 0, "timestamp": 1, "messages": []
        }"#;
        let thread: SmsThread = serde_json::from_str(raw).expect("null name must deserialize");
        assert!(thread.name.is_none());
        // Omitted entirely is equally valid.
        let raw_omitted = r#"{
            "thread_id": "t_1", "address": "+1555",
            "snippet": "s", "unread_count": 0, "timestamp": 1, "messages": []
        }"#;
        let thread: SmsThread = serde_json::from_str(raw_omitted).expect("omitted name must work");
        assert!(thread.name.is_none());
    }

    /// `sms/new` is sent as `{from, body, timestamp}` but read as
    /// `{thread_id, message}`. Both variants must round-trip.
    #[test]
    fn sms_new_accepts_both_wire_shapes() {
        let sender = SmsNew {
            msg_type: "sms".into(),
            action: "new".into(),
            from: Some("+15551234567".into()),
            body: Some("hello".into()),
            timestamp: Some(1_700_000_000),
            thread_id: None,
            message: None,
        };
        let restored = roundtrip(&sender);
        assert_eq!(restored.from.as_deref(), Some("+15551234567"));
        assert!(restored.thread_id.is_none());

        let receiver = SmsNew {
            msg_type: "sms".into(),
            action: "new".into(),
            from: None,
            body: None,
            timestamp: None,
            thread_id: Some("t_1".into()),
            message: Some(SmsMessage {
                id: "in_1".into(),
                address: "+15551234567".into(),
                body: "hello".into(),
                timestamp: 1_700_000_000,
                read: false,
                is_outgoing: false,
            }),
        };
        let restored = roundtrip(&receiver);
        assert_eq!(restored.thread_id.as_deref(), Some("t_1"));
        assert_eq!(restored.message.unwrap().body, "hello");
    }

    /// `sms/sent` carries the same dual shape as `sms/new`.
    #[test]
    fn sms_sent_accepts_both_wire_shapes() {
        let sender = SmsSent {
            msg_type: "sms".into(),
            action: "sent".into(),
            to: Some("+15551234567".into()),
            body: Some("hello".into()),
            timestamp: Some(1_700_000_000),
            thread_id: None,
            message: None,
        };
        let restored = roundtrip(&sender);
        assert_eq!(restored.to.as_deref(), Some("+15551234567"));
        assert_eq!(restored.timestamp, Some(1_700_000_000));

        let receiver = SmsSent {
            msg_type: "sms".into(),
            action: "sent".into(),
            to: None,
            body: None,
            timestamp: None,
            thread_id: Some("t_1".into()),
            message: Some(SmsMessage {
                id: "out_1".into(),
                address: "+15551234567".into(),
                body: "hello".into(),
                timestamp: 1_700_000_000,
                read: true,
                is_outgoing: true,
            }),
        };
        let restored = roundtrip(&receiver);
        assert_eq!(restored.thread_id.as_deref(), Some("t_1"));
        assert!(restored.message.unwrap().is_outgoing);
    }

    /// A bare `sms/new` carrying neither shape's required fields is invalid.
    #[test]
    fn schema_rejects_sms_new_with_either_shape_missing() {
        let validator = validator();
        let bare = json!({"type": "sms", "action": "new"});
        assert!(
            !is_valid(&validator, &bare),
            "sms/new with neither {{from,body,timestamp}} nor {{thread_id,message}} must be rejected"
        );
        let bare_sent = json!({"type": "sms", "action": "sent"});
        assert!(
            !is_valid(&validator, &bare_sent),
            "sms/sent with neither {{to,body,timestamp}} nor {{thread_id,message}} must be rejected"
        );
    }

    // ---------------------------------------------------------------
    //  StatusUpdate
    // ---------------------------------------------------------------

    #[test]
    fn status_update_roundtrip_minimal() {
        let msg = StatusUpdate {
            msg_type: "status".into(),
            action: "update".into(),
            battery: None,
            wifi_ssid: None,
            device_info: None,
        };
        let restored = roundtrip(&msg);
        assert!(restored.battery.is_none());
    }

    #[test]
    fn status_update_all_fields() {
        let msg = StatusUpdate {
            msg_type: "status".into(),
            action: "update".into(),
            battery: Some(75),
            wifi_ssid: Some("HomeWiFi".into()),
            device_info: Some(DeviceInfo {
                name: "Phone".into(),
                device_type: "phone".into(),
                os: Some("ios".into()),
                battery: Some(75),
            }),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.battery, Some(75));
        assert_eq!(restored.wifi_ssid.as_deref(), Some("HomeWiFi"));
        let di = restored.device_info.unwrap();
        assert_eq!(di.name, "Phone");
    }

    // ---------------------------------------------------------------
    //  Ping / Pong
    // ---------------------------------------------------------------

    #[test]
    fn ping_roundtrip() {
        let msg = Ping::new();
        let restored = roundtrip(&msg);
        assert_eq!(restored.msg_type, "ping");
    }

    #[test]
    fn pong_roundtrip() {
        let msg = Pong::new();
        let restored = roundtrip(&msg);
        assert_eq!(restored.msg_type, "pong");
    }

    #[test]
    fn ping_from_json() {
        let msg: Ping = serde_json::from_str(r#"{"type":"ping"}"#).unwrap();
        assert_eq!(msg.msg_type, "ping");
    }

    #[test]
    fn pong_from_json() {
        let msg: Pong = serde_json::from_str(r#"{"type":"pong"}"#).unwrap();
        assert_eq!(msg.msg_type, "pong");
    }

    // ---------------------------------------------------------------
    //  ErrorMessage
    // ---------------------------------------------------------------

    #[test]
    fn error_message_roundtrip_minimal() {
        let msg = ErrorMessage {
            msg_type: "error".into(),
            code: "AUTH_FAILED".into(),
            message: "Invalid token".into(),
            server_version: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.code, "AUTH_FAILED");
        assert!(restored.server_version.is_none());
    }

    #[test]
    fn error_message_with_server_version() {
        let msg = ErrorMessage {
            msg_type: "error".into(),
            code: "RATE_LIMIT".into(),
            message: "Too many requests".into(),
            server_version: Some(1),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.server_version, Some(1));
    }

    // ---------------------------------------------------------------
    //  RelayAuth / RelayAuthOk / RelayAuthRejected
    // ---------------------------------------------------------------

    #[test]
    fn relay_auth_roundtrip_minimal() {
        let msg = RelayAuth::new("device-1", "secret-token");
        let restored = roundtrip(&msg);
        assert_eq!(restored.device_id, "device-1");
        assert_eq!(restored.relay_token, "secret-token");
        assert!(restored.apns_token.is_none());
    }

    #[test]
    fn relay_auth_with_apns_token() {
        let mut msg = RelayAuth::new("device-1", "token");
        msg.apns_token = Some("apns-123".into());
        let restored = roundtrip(&msg);
        assert_eq!(restored.apns_token.as_deref(), Some("apns-123"));
    }

    #[test]
    fn relay_auth_ok_roundtrip() {
        let msg = RelayAuthOk {
            msg_type: "relay_auth_ok".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.msg_type, "relay_auth_ok");
    }

    #[test]
    fn relay_auth_rejected_roundtrip_invalid_token() {
        let msg = RelayAuthRejected {
            msg_type: "relay_auth_rejected".into(),
            reason: "invalid_token".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.reason, "invalid_token");
    }

    #[test]
    fn relay_auth_rejected_roundtrip_missing_token() {
        let msg = RelayAuthRejected {
            msg_type: "relay_auth_rejected".into(),
            reason: "missing_token".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.reason, "missing_token");
    }

    // ---------------------------------------------------------------
    //  RelayRoute
    // ---------------------------------------------------------------

    #[test]
    fn relay_route_roundtrip() {
        let msg = RelayRoute::new("target-device", json!({"action": "sync"}));
        let restored = roundtrip(&msg);
        assert_eq!(restored.to_device_id, "target-device");
        assert_eq!(restored.payload, json!({"action": "sync"}));
        assert!(restored.from_device_id.is_none());
        assert!(restored.hmac.is_none());
    }

    #[test]
    fn relay_route_signed_roundtrip_and_verifies() {
        let key = b"relay-signing-key-32-bytes-long!";
        let msg = RelayRoute::signed(key, "dev-a", "dev-b", json!({"type": "ping"}));
        assert!(msg.has_required_signed_fields());
        assert_eq!(msg.from_device(), Some("dev-a"));
        assert_eq!(
            msg.key_id.as_deref(),
            Some("dev-a"),
            "key_id names the signer, so the receiver can look the key up"
        );

        let restored = roundtrip(&msg);
        assert_eq!(restored, msg);

        let wire = serde_json::to_value(&msg).unwrap();
        assert!(crate::hmac::verify_message_hmac(key, &wire));
    }

    #[test]
    fn relay_route_signed_binds_from_device_id() {
        let key = b"relay-signing-key-32-bytes-long!";
        let msg = RelayRoute::signed(key, "dev-a", "dev-b", json!({"type": "ping"}));

        // Rewriting the claimed sender must invalidate the signature.
        let mut forged = msg.clone();
        forged.from_device_id = Some("dev-victim".into());
        let wire = serde_json::to_value(&forged).unwrap();
        assert!(
            !crate::hmac::verify_message_hmac(key, &wire),
            "a client must not be able to forge a route claiming another from_device_id"
        );
    }

    #[test]
    fn relay_route_signed_binds_key_id_and_payload() {
        let key = b"relay-signing-key-32-bytes-long!";
        let msg = RelayRoute::signed(key, "dev-a", "dev-b", json!({"type": "ping"}));

        let mut wrong_key_id = msg.clone();
        wrong_key_id.key_id = Some("other".into());
        assert!(!crate::hmac::verify_message_hmac(
            key,
            &serde_json::to_value(&wrong_key_id).unwrap()
        ));

        let mut wrong_payload = msg.clone();
        wrong_payload.payload = json!({"type": "tampered"});
        assert!(!crate::hmac::verify_message_hmac(
            key,
            &serde_json::to_value(&wrong_payload).unwrap()
        ));
    }

    #[test]
    fn relay_route_signed_with_honours_explicit_fields() {
        let key = b"k";
        let msg = RelayRoute::signed_with(
            "dev-a",
            key,
            "dev-a",
            "dev-b",
            json!({"x": 1}),
            1_700_000_000_000,
            "fixed-nonce",
        );
        assert_eq!(msg.key_id.as_deref(), Some("dev-a"));
        assert_eq!(msg.timestamp, Some(1_700_000_000_000));
        assert_eq!(msg.nonce.as_deref(), Some("fixed-nonce"));

        let mut ring = crate::hmac::RouteKeyring::new();
        ring.insert("dev-a", key.to_vec());
        let wire = serde_json::to_value(&msg).unwrap();
        assert_eq!(ring.verify(&wire).as_deref(), Some("dev-a"));
    }

    /// `signed_with` accepts a mismatched pair because it does not know better
    /// than its caller — but the receiver refuses it, so the mistake can never
    /// reach a device. This is the reason the keyring checks both names.
    #[test]
    fn relay_route_signed_with_a_mismatched_key_id_is_rejected_downstream() {
        let key = b"k";
        let msg = RelayRoute::signed_with(
            "someone-else",
            key,
            "dev-a",
            "dev-b",
            json!({"x": 1}),
            1_700_000_000_000,
            "fixed-nonce",
        );

        let mut ring = crate::hmac::RouteKeyring::new();
        ring.insert("someone-else", key.to_vec());
        let wire = serde_json::to_value(&msg).unwrap();
        assert_eq!(
            ring.verify(&wire),
            None,
            "a key id that disagrees with the attributed sender must not verify"
        );
    }

    /// A route whose `key_id` names one device but whose payload is attributed
    /// to another must not verify.
    ///
    /// The key is looked up by `key_id` and the attributed sender is then
    /// required to match, so a valid signature cannot be re-labelled. Without
    /// that second check, a compromised device holding its own key could sign a
    /// route the receiver believes came from the desktop.
    #[test]
    fn relay_route_rejects_a_key_id_that_disagrees_with_the_sender() {
        let key = b"relay-signing-key-32-bytes-long!";
        // Sign honestly, then rewrite the attributed sender. `from_device_id`
        // is signed, so this must not verify.
        let msg = RelayRoute::signed(key, "dev-a", "dev-b", json!({"type": "ping"}));
        let mut wire = serde_json::to_value(&msg).unwrap();
        wire["from_device_id"] = json!("dev-b");

        let mut ring = crate::hmac::RouteKeyring::new();
        ring.insert("dev-a", key.to_vec());
        ring.insert("dev-b", b"dev-b-route-key-32-bytes-long!!".to_vec());
        assert_eq!(
            ring.verify(&wire),
            None,
            "a signature from dev-a must not be accepted as dev-b"
        );
    }

    /// A device that holds only its own key cannot produce a route that verifies
    /// as some other device, even if it writes that device's id.
    #[test]
    fn a_device_cannot_sign_as_another_device() {
        let dev_a_key = b"dev-a-route-key-32-bytes-long!!!!";

        // dev-a forges a route claiming to be dev-b, signing with its own key.
        let mut forged = RelayRoute::signed(
            dev_a_key,
            "dev-b",
            "dev-c",
            json!({"type": "sms", "body": "not mine"}),
        );
        forged.key_id = Some("dev-b".into());
        let wire = serde_json::to_value(&forged).unwrap();

        let mut ring = crate::hmac::RouteKeyring::new();
        ring.insert("dev-a", dev_a_key.to_vec());
        ring.insert("dev-b", b"dev-b-route-key-32-bytes-long!!".to_vec());

        assert_eq!(
            ring.verify(&wire),
            None,
            "dev-a holds only its own key, so nothing it signs may verify as dev-b"
        );
    }

    /// Two devices that somehow shared a pairing secret still get unrelated
    /// route keys, because the device id is bound into the derivation label.
    #[test]
    fn route_keys_are_domain_separated_by_device_id() {
        let shared = b"a-pairing-secret";
        let a = crate::hmac::derive_route_key(shared, "dev-a");
        let b = crate::hmac::derive_route_key(shared, "dev-b");
        assert_ne!(a, b, "one secret under two ids must not yield one key");
        assert_ne!(
            a,
            crate::hmac::derive_key(shared, crate::hmac::ROUTE_KEY_LABEL)
        );
    }

    #[test]
    fn relay_route_unsigned_constructor_lacks_required_fields() {
        let msg = RelayRoute::new("target-device", json!({"action": "sync"}));
        assert!(
            !msg.has_required_signed_fields(),
            "the unsigned form is structurally incapable of being routed"
        );
    }

    #[test]
    fn relay_route_legacy_minimal_json_still_deserialises() {
        // Backwards-compatible parse: the relay, not serde, rejects it.
        let legacy = json!({
            "type": "relay_route",
            "to_device_id": "dev-b",
            "payload": {"type": "ping"},
        });
        let parsed: RelayRoute = serde_json::from_value(legacy).unwrap();
        assert_eq!(parsed.to_device_id, "dev-b");
        assert!(!parsed.has_required_signed_fields());
    }

    // ---------------------------------------------------------------
    //  EncryptedEnvelope
    // ---------------------------------------------------------------

    #[test]
    fn encrypted_envelope_roundtrip() {
        let msg = EncryptedEnvelope {
            msg_type: "encrypted".into(),
            nonce: "aabbccdd".into(),
            hmac: "11223344".into(),
            data: "deadbeef".into(),
            source_device: Some("dev-1".into()),
            protocol_version: Some(1),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.nonce, "aabbccdd");
        assert_eq!(restored.source_device.as_deref(), Some("dev-1"));
    }

    #[test]
    fn encrypted_envelope_from_json_with_source_device() {
        let json_str = r#"{
            "type": "encrypted",
            "nonce": "aa",
            "hmac": "bb",
            "data": "cc",
            "source_device": "dev-x"
        }"#;
        let msg: EncryptedEnvelope = serde_json::from_str(json_str).unwrap();
        assert_eq!(msg.source_device.as_deref(), Some("dev-x"));
    }

    // ---------------------------------------------------------------
    //  DeviceInfo
    // ---------------------------------------------------------------

    #[test]
    fn device_info_roundtrip_minimal() {
        let msg = DeviceInfo {
            name: "Phone".into(),
            device_type: "phone".into(),
            os: None,
            battery: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.name, "Phone");
        assert!(restored.os.is_none());
    }

    #[test]
    fn device_info_all_fields() {
        let msg = DeviceInfo {
            name: "Desktop".into(),
            device_type: "desktop".into(),
            os: Some("windows".into()),
            battery: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.os.as_deref(), Some("windows"));
    }

    // ---------------------------------------------------------------
    //  AutomationTrigger
    // ---------------------------------------------------------------

    #[test]
    fn automation_trigger_device_connect() {
        let msg = AutomationTrigger {
            trigger_type: "device_connect".into(),
            device_id: Some("dev-1".into()),
            time: None,
            below: None,
            ssid: None,
            app_package: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.trigger_type, "device_connect");
        assert_eq!(restored.device_id.as_deref(), Some("dev-1"));
    }

    #[test]
    fn automation_trigger_time() {
        let msg = AutomationTrigger {
            trigger_type: "time".into(),
            device_id: None,
            time: Some("08:30".into()),
            below: None,
            ssid: None,
            app_package: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.time.as_deref(), Some("08:30"));
    }

    #[test]
    fn automation_trigger_battery_level() {
        let msg = AutomationTrigger {
            trigger_type: "battery_level".into(),
            device_id: None,
            time: None,
            below: Some(20),
            ssid: None,
            app_package: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.below, Some(20));
    }

    #[test]
    fn automation_trigger_wifi_change() {
        let msg = AutomationTrigger {
            trigger_type: "wifi_change".into(),
            device_id: None,
            time: None,
            below: None,
            ssid: Some("OfficeWiFi".into()),
            app_package: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.ssid.as_deref(), Some("OfficeWiFi"));
    }

    #[test]
    fn automation_trigger_app_open() {
        let msg = AutomationTrigger {
            trigger_type: "app_open".into(),
            device_id: None,
            time: None,
            below: None,
            ssid: None,
            app_package: Some("com.example.app".into()),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.app_package.as_deref(), Some("com.example.app"));
    }

    // ---------------------------------------------------------------
    //  AutomationActionPayload
    // ---------------------------------------------------------------

    #[test]
    fn automation_action_send_notification() {
        let msg = AutomationActionPayload {
            action_type: "send_notification".into(),
            title: Some("Alert".into()),
            body: Some("Something happened".into()),
            profile: None,
            device_id: None,
            command: None,
            enabled: None,
            url: None,
            app_package: None,
            state: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.title.as_deref(), Some("Alert"));
    }

    #[test]
    fn automation_action_set_phone_profile() {
        let msg = AutomationActionPayload {
            action_type: "set_phone_profile".into(),
            title: None,
            body: None,
            profile: Some("silent".into()),
            device_id: None,
            command: None,
            enabled: None,
            url: None,
            app_package: None,
            state: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.profile.as_deref(), Some("silent"));
    }

    #[test]
    fn automation_action_run_shell_command() {
        let msg = AutomationActionPayload {
            action_type: "run_shell_command".into(),
            title: None,
            body: None,
            profile: None,
            device_id: None,
            command: Some("echo hello".into()),
            enabled: None,
            url: None,
            app_package: None,
            state: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.command.as_deref(), Some("echo hello"));
    }

    #[test]
    fn automation_action_toggle_wifi() {
        let msg = AutomationActionPayload {
            action_type: "toggle_wifi".into(),
            title: None,
            body: None,
            profile: None,
            device_id: None,
            command: None,
            enabled: Some(true),
            url: None,
            app_package: None,
            state: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.enabled, Some(true));
    }

    #[test]
    fn automation_action_open_url() {
        let msg = AutomationActionPayload {
            action_type: "open_url".into(),
            title: None,
            body: None,
            profile: None,
            device_id: None,
            command: None,
            enabled: None,
            url: Some("https://example.com".into()),
            app_package: None,
            state: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.url.as_deref(), Some("https://example.com"));
    }

    #[test]
    fn automation_action_open_app() {
        let msg = AutomationActionPayload {
            action_type: "open_app".into(),
            title: None,
            body: None,
            profile: None,
            device_id: None,
            command: None,
            enabled: None,
            url: None,
            app_package: Some("com.example.app".into()),
            state: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.app_package.as_deref(), Some("com.example.app"));
    }

    #[test]
    fn automation_action_set_window_state() {
        let msg = AutomationActionPayload {
            action_type: "set_window_state".into(),
            title: None,
            body: None,
            profile: None,
            device_id: None,
            command: None,
            enabled: None,
            url: None,
            app_package: None,
            state: Some("maximize".into()),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.state.as_deref(), Some("maximize"));
    }

    // ---------------------------------------------------------------
    //  WireMessage – untagged enum
    // ---------------------------------------------------------------

    #[test]
    fn wire_message_encrypted_variant() {
        let msg = WireMessage::Encrypted(EncryptedEnvelope::new("aa", "bb", "cc"));
        let json_str = serde_json::to_string(&msg).unwrap();
        let restored: WireMessage = serde_json::from_str(&json_str).unwrap();
        match restored {
            WireMessage::Encrypted(e) => assert_eq!(e.nonce, "aa"),
            _ => panic!("expected Encrypted variant"),
        }
    }

    #[test]
    fn wire_message_plaintext_variant() {
        let msg = WireMessage::Plaintext(json!({"type": "ping"}));
        let json_str = serde_json::to_string(&msg).unwrap();
        let restored: WireMessage = serde_json::from_str(&json_str).unwrap();
        match restored {
            WireMessage::Plaintext(v) => assert_eq!(v["type"], "ping"),
            _ => panic!("expected Plaintext variant"),
        }
    }

    // ---------------------------------------------------------------
    //  Edge cases – empty strings, zero values, special characters
    // ---------------------------------------------------------------

    #[test]
    fn discovery_announce_empty_strings() {
        let msg = DiscoveryAnnounce {
            msg_type: "".into(),
            action: "".into(),
            protocol_version: None,
            device_id: "".into(),
            device_name: "".into(),
            device_type: "".into(),
            os: "".into(),
            version: "".into(),
            battery: Some(0),
            ws_port: Some(0),
            wss_port: Some(0),
            apns_token: None,
        };
        let restored = roundtrip(&msg);
        assert!(restored.device_id.is_empty());
        assert_eq!(restored.battery, Some(0));
        assert_eq!(restored.ws_port, Some(0));
    }

    #[test]
    fn clipboard_sync_unicode_content() {
        let msg = ClipboardSync {
            msg_type: "clipboard".into(),
            action: "sync".into(),
            content: "Hello \u{1F600} \u{4E16}\u{754C}".into(),
            mime: "text/plain".into(),
            source_device: "dev-1".into(),
            timestamp: 0,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.content, "Hello \u{1F600} \u{4E16}\u{754C}");
    }

    #[test]
    fn notification_post_long_body() {
        let long_body = "a".repeat(10_000);
        let msg = NotificationPost {
            msg_type: "notification".into(),
            action: "post".into(),
            id: "n-long".into(),
            device_id: "dev-1".into(),
            app: "Test".into(),
            title: "Title".into(),
            body: long_body.clone(),
            timestamp: 0,
            actions: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.body.len(), 10_000);
    }

    #[test]
    fn file_request_max_u64_size() {
        let msg = FileRequest {
            msg_type: "file".into(),
            action: "request".into(),
            id: "f-max".into(),
            name: "huge.bin".into(),
            size: u64::MAX,
            mime: "application/octet-stream".into(),
            from: "dev-1".into(),
            to: None,
            checksum: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.size, u64::MAX);
    }

    #[test]
    fn call_message_empty_action() {
        let msg = CallMessage {
            msg_type: "call".into(),
            action: "".into(),
            call_id: None,
            to_device_id: None,
            route: None,
            number: None,
            name: None,
            device_id: None,
        };
        let restored = roundtrip(&msg);
        assert!(restored.action.is_empty());
    }

    // ---------------------------------------------------------------
    //  Protocol version compatibility
    // ---------------------------------------------------------------

    #[test]
    fn discovery_announce_with_higher_protocol_version() {
        let json_str = r#"{
            "type": "discovery",
            "action": "announce",
            "protocol_version": 999,
            "device_id": "d1",
            "device_name": "Phone",
            "device_type": "phone",
            "os": "ios",
            "version": "1.0"
        }"#;
        let msg: DiscoveryAnnounce = serde_json::from_str(json_str).unwrap();
        assert_eq!(msg.protocol_version, Some(999));
    }

    #[test]
    fn encrypted_envelope_protocol_version_optional() {
        let json_str = r#"{
            "type": "encrypted",
            "nonce": "aa",
            "hmac": "bb",
            "data": "cc"
        }"#;
        let msg: EncryptedEnvelope = serde_json::from_str(json_str).unwrap();
        assert!(msg.protocol_version.is_none());
    }

    #[test]
    fn pairing_request_protocol_version_roundtrip() {
        let msg = PairingRequest {
            msg_type: "pairing".into(),
            action: "request".into(),
            protocol_version: Some(42),
            token: "tok".into(),
            public_key: "pk".into(),
            device_info: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.protocol_version, Some(42));
    }

    // ---------------------------------------------------------------
    //  Unknown fields are ignored on deserialization
    // ---------------------------------------------------------------

    #[test]
    fn relay_auth_ignores_unknown_fields() {
        let json_str = r#"{
            "type": "relay_auth",
            "device_id": "d1",
            "relay_token": "tok",
            "future_field": "should be ignored",
            "another_unknown": 42
        }"#;
        let msg: RelayAuth = serde_json::from_str(json_str).unwrap();
        assert_eq!(msg.device_id, "d1");
        assert_eq!(msg.relay_token, "tok");
    }

    #[test]
    fn error_message_ignores_unknown_fields() {
        let json_str = r#"{
            "type": "error",
            "code": "ERR",
            "message": "msg",
            "extra_data": {"nested": true}
        }"#;
        let msg: ErrorMessage = serde_json::from_str(json_str).unwrap();
        assert_eq!(msg.code, "ERR");
    }

    #[test]
    fn screen_mirror_frame_ignores_unknown_fields() {
        let json_str = r#"{
            "type": "screen_mirror",
            "action": "frame",
            "data": "jpeg",
            "format": "jpeg",
            "width": 100,
            "height": 100,
            "deprecated_field": true
        }"#;
        let msg: ScreenMirrorFrame = serde_json::from_str(json_str).unwrap();
        assert_eq!(msg.width, 100);
    }

    // ---------------------------------------------------------------
    //  Missing required fields return errors
    // ---------------------------------------------------------------

    #[test]
    fn clipboard_sync_missing_content_fails() {
        let json_str = r#"{"type":"clipboard","action":"sync","mime":"text/plain","source_device":"d1","timestamp":0}"#;
        let result = serde_json::from_str::<ClipboardSync>(json_str);
        assert!(result.is_err());
    }

    #[test]
    fn file_chunk_missing_data_fails() {
        let json_str = r#"{"type":"file","action":"chunk","id":"f1","index":0}"#;
        let result = serde_json::from_str::<FileChunk>(json_str);
        assert!(result.is_err());
    }

    #[test]
    fn notification_post_missing_title_fails() {
        let json_str = r#"{"type":"notification","action":"post","id":"n1","device_id":"d1","app":"App","body":"b","timestamp":0}"#;
        let result = serde_json::from_str::<NotificationPost>(json_str);
        assert!(result.is_err());
    }

    #[test]
    fn sms_send_missing_to_fails() {
        let json_str = r#"{"type":"sms","action":"send","body":"hi"}"#;
        let result = serde_json::from_str::<SmsSend>(json_str);
        assert!(result.is_err());
    }

    // ---------------------------------------------------------------
    //  JSON roundtrip for complex nested structures
    // ---------------------------------------------------------------

    #[test]
    fn automation_rule_full_json_roundtrip() {
        let json_str = r#"{
            "type": "automation",
            "action": "rule",
            "id": "r1",
            "name": "Smart Rule",
            "trigger": {
                "type": "battery_level",
                "below": 15
            },
            "rule_action": {
                "type": "set_phone_profile",
                "profile": "silent"
            },
            "enabled": true
        }"#;
        let msg: AutomationRuleMessage = serde_json::from_str(json_str).unwrap();
        assert_eq!(msg.name, "Smart Rule");
        assert_eq!(msg.trigger.trigger_type, "battery_level");
        assert_eq!(msg.trigger.below, Some(15));
        assert_eq!(msg.rule_action.action_type, "set_phone_profile");
        assert_eq!(msg.rule_action.profile.as_deref(), Some("silent"));

        // Roundtrip back
        let json_out = serde_json::to_string(&msg).unwrap();
        let restored: AutomationRuleMessage = serde_json::from_str(&json_out).unwrap();
        assert_eq!(restored.name, msg.name);
        assert_eq!(restored.enabled, msg.enabled);
    }

    #[test]
    fn discovery_announce_all_device_types() {
        for dt in &["phone", "tablet", "desktop"] {
            let msg = DiscoveryAnnounce {
                msg_type: "discovery".into(),
                action: "announce".into(),
                protocol_version: None,
                device_id: "d1".into(),
                device_name: "Test".into(),
                device_type: dt.to_string(),
                os: "windows".into(),
                version: "1.0".into(),
                battery: None,
                ws_port: None,
                wss_port: None,
                apns_token: None,
            };
            let restored = roundtrip(&msg);
            assert_eq!(restored.device_type, *dt);
        }
    }

    #[test]
    fn discovery_announce_all_os_types() {
        for os in &["ios", "android", "windows", "macos", "linux"] {
            let msg = DiscoveryAnnounce {
                msg_type: "discovery".into(),
                action: "announce".into(),
                protocol_version: None,
                device_id: "d1".into(),
                device_name: "Test".into(),
                device_type: "phone".into(),
                os: os.to_string(),
                version: "1.0".into(),
                battery: None,
                ws_port: None,
                wss_port: None,
                apns_token: None,
            };
            let restored = roundtrip(&msg);
            assert_eq!(restored.os, *os);
        }
    }

    // ---------------------------------------------------------------
    //  Negative and boundary numeric values
    // ---------------------------------------------------------------

    #[test]
    fn clipboard_sync_negative_timestamp() {
        let msg = ClipboardSync {
            msg_type: "clipboard".into(),
            action: "sync".into(),
            content: "test".into(),
            mime: "text/plain".into(),
            source_device: "d1".into(),
            timestamp: -1,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.timestamp, -1);
    }

    #[test]
    fn notification_post_zero_timestamp() {
        let msg = NotificationPost {
            msg_type: "notification".into(),
            action: "post".into(),
            id: "n1".into(),
            device_id: "d1".into(),
            app: "App".into(),
            title: "Title".into(),
            body: "Body".into(),
            timestamp: 0,
            actions: None,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.timestamp, 0);
    }

    #[test]
    fn screen_mirror_touch_negative_coordinates() {
        let msg = ScreenMirrorTouch {
            msg_type: "screen_mirror".into(),
            action: "touch".into(),
            x: -10.0,
            y: -20.0,
            action_type: None,
        };
        let restored = roundtrip(&msg);
        assert!((restored.x - (-10.0)).abs() < f64::EPSILON);
        assert!((restored.y - (-20.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn file_chunk_zero_index() {
        let msg = FileChunk {
            msg_type: "file".into(),
            action: "chunk".into(),
            id: "f1".into(),
            index: 0,
            data: "data".into(),
            total: Some(0),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.index, 0);
        assert_eq!(restored.total, Some(0));
    }

    // ---------------------------------------------------------------
    //  deserialize_strict – correct type/action dispatch
    // ---------------------------------------------------------------

    #[test]
    fn deserialize_relay_auth_ok_from_json() {
        let msg: RelayAuthOk = serde_json::from_str(r#"{"type":"relay_auth_ok"}"#).unwrap();
        assert_eq!(msg.msg_type, "relay_auth_ok");
    }

    #[test]
    fn deserialize_automation_sync_with_empty_rules() {
        let msg: AutomationSync =
            serde_json::from_str(r#"{"type":"automation","action":"sync","rules":[]}"#).unwrap();
        assert!(msg.rules.is_empty());
    }

    #[test]
    fn deserialize_call_message_minimal_json() {
        let msg: CallMessage = serde_json::from_str(r#"{"type":"call","action":"end"}"#).unwrap();
        assert_eq!(msg.action, "end");
    }

    // ---------------------------------------------------------------
    //  serialize produces correct type tags
    // ---------------------------------------------------------------

    #[test]
    fn serialize_produces_correct_type_tag_for_all_categories() {
        // Discovery
        let da = DiscoveryAnnounce {
            msg_type: "discovery".into(),
            action: "announce".into(),
            protocol_version: None,
            device_id: "".into(),
            device_name: "".into(),
            device_type: "".into(),
            os: "".into(),
            version: "".into(),
            battery: None,
            ws_port: None,
            wss_port: None,
            apns_token: None,
        };
        let v: Value = serde_json::from_str(&serde_json::to_string(&da).unwrap()).unwrap();
        assert_eq!(v["type"], "discovery");

        // Pairing
        let pr = PairingRequest {
            msg_type: "pairing".into(),
            action: "request".into(),
            protocol_version: None,
            token: "".into(),
            public_key: "".into(),
            device_info: None,
        };
        let v: Value = serde_json::from_str(&serde_json::to_string(&pr).unwrap()).unwrap();
        assert_eq!(v["type"], "pairing");

        // Clipboard
        let cs = ClipboardSync {
            msg_type: "clipboard".into(),
            action: "sync".into(),
            content: "".into(),
            mime: "".into(),
            source_device: "".into(),
            timestamp: 0,
        };
        let v: Value = serde_json::from_str(&serde_json::to_string(&cs).unwrap()).unwrap();
        assert_eq!(v["type"], "clipboard");

        // Notification
        let np = NotificationPost {
            msg_type: "notification".into(),
            action: "post".into(),
            id: "".into(),
            device_id: "".into(),
            app: "".into(),
            title: "".into(),
            body: "".into(),
            timestamp: 0,
            actions: None,
        };
        let v: Value = serde_json::from_str(&serde_json::to_string(&np).unwrap()).unwrap();
        assert_eq!(v["type"], "notification");

        // File
        let fa = FileAccept {
            msg_type: "file".into(),
            action: "accept".into(),
            id: "".into(),
        };
        let v: Value = serde_json::from_str(&serde_json::to_string(&fa).unwrap()).unwrap();
        assert_eq!(v["type"], "file");

        // Audio
        let as_start = AudioStreamStart {
            msg_type: "audio".into(),
            action: "stream_start".into(),
        };
        let v: Value = serde_json::from_str(&serde_json::to_string(&as_start).unwrap()).unwrap();
        assert_eq!(v["type"], "audio");

        // Screen Mirror
        let sm = ScreenMirrorStop {
            msg_type: "screen_mirror".into(),
            action: "stop".into(),
        };
        let v: Value = serde_json::from_str(&serde_json::to_string(&sm).unwrap()).unwrap();
        assert_eq!(v["type"], "screen_mirror");

        // Remote Input
        let ri = RemoteInputMove {
            msg_type: "remote_input".into(),
            action: "move".into(),
            dx: 0.0,
            dy: 0.0,
        };
        let v: Value = serde_json::from_str(&serde_json::to_string(&ri).unwrap()).unwrap();
        assert_eq!(v["type"], "remote_input");

        // Call
        let cm = CallMessage {
            msg_type: "call".into(),
            action: "end".into(),
            call_id: None,
            to_device_id: None,
            route: None,
            number: None,
            name: None,
            device_id: None,
        };
        let v: Value = serde_json::from_str(&serde_json::to_string(&cm).unwrap()).unwrap();
        assert_eq!(v["type"], "call");

        // SMS
        let ss = SmsSend {
            msg_type: "sms".into(),
            action: "send".into(),
            to: "".into(),
            body: "".into(),
        };
        let v: Value = serde_json::from_str(&serde_json::to_string(&ss).unwrap()).unwrap();
        assert_eq!(v["type"], "sms");

        // Status
        let su = StatusUpdate {
            msg_type: "status".into(),
            action: "update".into(),
            battery: None,
            wifi_ssid: None,
            device_info: None,
        };
        let v: Value = serde_json::from_str(&serde_json::to_string(&su).unwrap()).unwrap();
        assert_eq!(v["type"], "status");

        // Error
        let em = ErrorMessage {
            msg_type: "error".into(),
            code: "".into(),
            message: "".into(),
            server_version: None,
        };
        let v: Value = serde_json::from_str(&serde_json::to_string(&em).unwrap()).unwrap();
        assert_eq!(v["type"], "error");

        // Relay
        let ra = RelayAuth::new("d", "t");
        let v: Value = serde_json::from_str(&serde_json::to_string(&ra).unwrap()).unwrap();
        assert_eq!(v["type"], "relay_auth");

        // Encrypted
        let ee = EncryptedEnvelope::new("n", "h", "d");
        let v: Value = serde_json::from_str(&serde_json::to_string(&ee).unwrap()).unwrap();
        assert_eq!(v["type"], "encrypted");
    }

    // ---------------------------------------------------------------
    //  Verify skip_serializing_if behavior
    // ---------------------------------------------------------------

    #[test]
    fn skip_serializing_none_optional_fields() {
        let msg = PairingRequest {
            msg_type: "pairing".into(),
            action: "request".into(),
            protocol_version: None,
            token: "tok".into(),
            public_key: "pk".into(),
            device_info: None,
        };
        let json_str = serde_json::to_string(&msg).unwrap();
        assert!(
            !json_str.contains("protocol_version"),
            "None optional should be skipped"
        );
        assert!(
            !json_str.contains("device_info"),
            "None optional should be skipped"
        );
    }

    #[test]
    fn serialize_some_optional_fields() {
        let msg = PairingRequest {
            msg_type: "pairing".into(),
            action: "request".into(),
            protocol_version: Some(1),
            token: "tok".into(),
            public_key: "pk".into(),
            device_info: Some(DeviceInfo {
                name: "Phone".into(),
                device_type: "phone".into(),
                os: None,
                battery: None,
            }),
        };
        let json_str = serde_json::to_string(&msg).unwrap();
        assert!(
            json_str.contains("protocol_version"),
            "Some optional should be serialized"
        );
        assert!(
            json_str.contains("device_info"),
            "Some optional should be serialized"
        );
        // DeviceInfo inner None fields should be skipped
        assert!(!json_str.contains("\"os\""), "inner None should be skipped");
        assert!(
            !json_str.contains("\"battery\""),
            "inner None should be skipped"
        );
    }

    // ================================================================
    //  JSON Schema validation (schema.json vs serde types)
    //
    //  schema.json is the wire contract for non-Rust clients. These tests
    //  pin branch coverage and catch serde/schema divergence.
    // ================================================================

    /// Load the canonical schema once per test binary.
    fn schema() -> serde_json::Value {
        serde_json::from_str(include_str!("../schema.json")).expect("schema.json must parse")
    }

    /// Build a jsonschema validator for the canonical schema.
    fn validator() -> jsonschema::Validator {
        jsonschema::validator_for(&schema()).expect("schema must be valid JSON Schema")
    }

    fn is_valid(v: &jsonschema::Validator, instance: &serde_json::Value) -> bool {
        v.validate(instance).is_ok()
    }

    /// Titles of every oneOf branch (for coverage assertions).
    fn oneof_titles(schema: &serde_json::Value) -> Vec<String> {
        schema["oneOf"]
            .as_array()
            .expect("schema must have oneOf")
            .iter()
            .map(|b| b["title"].as_str().unwrap_or("<untitled>").to_string())
            .collect()
    }

    /// One representative JSON sample per oneOf branch, keyed by branch title.
    ///
    /// The sample set is intentionally one-per-branch so
    /// `schema_sample_set_covers_every_oneof_branch` can assert coverage.
    fn schema_samples() -> Vec<(&'static str, serde_json::Value)> {
        vec![
            (
                "Discovery Announce",
                json!({
                    "type": "discovery", "action": "announce",
                    "device_id": "d1", "device_name": "Phone",
                    "device_type": "phone", "os": "ios", "version": "1.0"
                }),
            ),
            (
                "Discovery Remove",
                json!({"type": "discovery", "action": "remove", "device_id": "d1"}),
            ),
            (
                "Pairing Request",
                json!({
                    "type": "pairing", "action": "request",
                    "token": "abc", "public_key": "aabb"
                }),
            ),
            (
                "Pairing Accept",
                json!({"type": "pairing", "action": "accept", "public_key": "ccdd"}),
            ),
            (
                "Pairing Revoke",
                json!({"type": "pairing", "action": "revoke"}),
            ),
            (
                "Clipboard Sync",
                json!({
                    "type": "clipboard", "action": "sync",
                    "content": "hi", "mime": "text/plain",
                    "source_device": "d1", "timestamp": 1
                }),
            ),
            (
                "Clipboard Request",
                json!({"type": "clipboard", "action": "request"}),
            ),
            (
                "Notification Post",
                json!({
                    "type": "notification", "action": "post",
                    "id": "n1", "device_id": "d1", "app": "App",
                    "title": "T", "body": "B", "timestamp": 1
                }),
            ),
            (
                "Notification Dismiss",
                json!({"type": "notification", "action": "dismiss", "id": "n1"}),
            ),
            (
                "Notification Reply",
                json!({
                    "type": "notification", "action": "reply",
                    "id": "n1", "text": "ok"
                }),
            ),
            (
                "Notification Mark Read",
                json!({"type": "notification", "action": "mark_read", "id": "n1"}),
            ),
            (
                "File Request",
                json!({
                    "type": "file", "action": "request",
                    "id": "f1", "name": "a.txt", "size": 10,
                    "mime": "text/plain", "from": "d1"
                }),
            ),
            (
                "File Accept",
                json!({"type": "file", "action": "accept", "id": "f1"}),
            ),
            (
                "File Chunk",
                json!({
                    "type": "file", "action": "chunk",
                    "id": "f1", "index": 0, "data": "AAAA"
                }),
            ),
            (
                "File Progress",
                json!({
                    "type": "file", "action": "progress",
                    "id": "f1", "percent": 50
                }),
            ),
            (
                "File Complete",
                json!({"type": "file", "action": "complete", "id": "f1"}),
            ),
            (
                "File Cancel",
                json!({"type": "file", "action": "cancel", "id": "f1"}),
            ),
            (
                "File Resume",
                json!({
                    "type": "file", "action": "resume",
                    "id": "f1", "name": "a.txt", "size": 10,
                    "mime": "text/plain", "from": "d1"
                }),
            ),
            (
                "File Resume Ack",
                json!({
                    "type": "file", "action": "resume_ack",
                    "id": "f1", "chunks_loaded": 3
                }),
            ),
            (
                "Audio Stream Start",
                json!({"type": "audio", "action": "stream_start"}),
            ),
            (
                "Audio Stream Stop",
                json!({"type": "audio", "action": "stream_stop"}),
            ),
            (
                "Audio Stream Started",
                json!({"type": "audio", "action": "stream_started", "from": "d1"}),
            ),
            (
                "Audio Stream Data",
                json!({
                    "type": "audio", "action": "stream_data",
                    "data": "AAAA", "format": "pcm16",
                    "sample_rate": 16000, "channels": 1, "from": "d1"
                }),
            ),
            (
                "Audio Playback Start",
                json!({"type": "audio", "action": "playback_start"}),
            ),
            (
                "Audio Playback Stop",
                json!({"type": "audio", "action": "playback_stop"}),
            ),
            (
                "Audio Playback Started",
                json!({"type": "audio", "action": "playback_started", "from": "d1"}),
            ),
            (
                "Audio Playback Data",
                json!({
                    "type": "audio", "action": "playback_data",
                    "data": "AAAA", "format": "pcm16",
                    "sample_rate": 16000, "channels": 1
                }),
            ),
            (
                "Screen Mirror Start",
                json!({"type": "screen_mirror", "action": "start", "quality": "high"}),
            ),
            (
                "Screen Mirror Stop",
                json!({"type": "screen_mirror", "action": "stop"}),
            ),
            (
                "Screen Mirror Frame",
                json!({
                    "type": "screen_mirror", "action": "frame",
                    "data": "AAAA", "format": "jpeg",
                    "width": 100, "height": 50
                }),
            ),
            (
                "Screen Mirror Capture Stopped",
                json!({"type": "screen_mirror", "action": "capture_stopped"}),
            ),
            (
                "Screen Mirror Touch",
                json!({
                    "type": "screen_mirror", "action": "touch",
                    "x": 10.0, "y": 20.0, "actionType": "tap"
                }),
            ),
            (
                "Screen Mirror Key",
                json!({
                    "type": "screen_mirror", "action": "key",
                    "key": "a", "modifiers": ["shift"]
                }),
            ),
            (
                "Screen Mirror Scroll",
                json!({
                    "type": "screen_mirror", "action": "scroll",
                    "dx": 1.0, "dy": -1.0
                }),
            ),
            (
                "Remote Input Move",
                json!({
                    "type": "remote_input", "action": "move",
                    "dx": 1.0, "dy": 2.0
                }),
            ),
            (
                "Remote Input Click",
                json!({
                    "type": "remote_input", "action": "click",
                    "button": "left"
                }),
            ),
            (
                "Remote Input Scroll",
                json!({
                    "type": "remote_input", "action": "scroll",
                    "dx": 0.0, "dy": 1.0
                }),
            ),
            (
                "Automation Rule",
                json!({
                    "type": "automation", "action": "rule",
                    "id": "r1", "name": "Rule",
                    "trigger": {"type": "battery_level", "below": 20},
                    "rule_action": {"type": "send_notification", "title": "Low"},
                    "enabled": true
                }),
            ),
            (
                "Automation Delete",
                json!({"type": "automation", "action": "delete", "rule_id": "r1"}),
            ),
            (
                "Automation Sync",
                json!({"type": "automation", "action": "sync", "rules": []}),
            ),
            (
                "Automation Triggered",
                json!({
                    "type": "automation", "action": "triggered",
                    "id": "r1", "trigger_type": "battery_level"
                }),
            ),
            (
                "Call Message",
                json!({"type": "call", "action": "incoming", "call_id": "c1"}),
            ),
            (
                "SMS Send",
                json!({
                    "type": "sms", "action": "send",
                    "to": "+15551234567", "body": "hello"
                }),
            ),
            (
                "SMS Sync",
                json!({
                    "type": "sms", "action": "sync",
                    "threads": [{
                        "thread_id": "t_1700000000000",
                        "address": "+15551234567",
                        "name": "Ada Lovelace",
                        "snippet": "hello",
                        "unread_count": 1,
                        "timestamp": 1700000000,
                        "messages": [{
                            "id": "in_1700000000000",
                            "address": "+15551234567",
                            "body": "hello",
                            "timestamp": 1700000000,
                            "read": false,
                            "is_outgoing": false
                        }]
                    }]
                }),
            ),
            (
                "SMS New",
                json!({
                    "type": "sms", "action": "new",
                    "from": "+15551234567", "body": "hello",
                    "timestamp": 1700000000
                }),
            ),
            (
                "SMS Sent",
                json!({
                    "type": "sms", "action": "sent",
                    "to": "+15551234567", "body": "hello",
                    "timestamp": 1700000000
                }),
            ),
            (
                "Status Update",
                json!({"type": "status", "action": "update", "battery": 80}),
            ),
            ("Ping", json!({"type": "ping"})),
            ("Pong", json!({"type": "pong"})),
            (
                "Error",
                json!({"type": "error", "code": "E1", "message": "boom"}),
            ),
            (
                "Relay Auth",
                json!({
                    "type": "relay_auth",
                    "device_id": "d1", "relay_token": "tok"
                }),
            ),
            ("Relay Auth OK", json!({"type": "relay_auth_ok"})),
            (
                "Relay Auth Rejected",
                json!({
                    "type": "relay_auth_rejected",
                    "reason": "invalid_token"
                }),
            ),
            (
                "Relay Route",
                json!({
                    "type": "relay_route",
                    "to_device_id": "d2",
                    "payload": {"type": "ping"}
                }),
            ),
            (
                "Relay Delivery",
                json!({
                    "type": "relay_delivery",
                    "from_device_id": "d1",
                    "to_device_id": "d2",
                    "payload": {"type": "ping"}
                }),
            ),
            (
                "Encrypted Envelope",
                json!({
                    "type": "encrypted",
                    "nonce": "aa", "hmac": "bb", "data": "cc"
                }),
            ),
        ]
    }

    /// Guards the root cause of the historical `schema_accepts_every_sample_in
    /// _sample_set` failure: a duplicated branch in `oneOf` makes the union
    /// ambiguous, so a *valid* message matches more than one branch and every
    /// conforming validator rejects it.
    ///
    /// The check is on the `(type, action)` discriminator pair, not just the
    /// title, because that is what actually makes two branches overlap.
    #[test]
    fn schema_oneof_branches_are_unambiguous() {
        let s = schema();
        let mut seen: Vec<(String, String, String)> = Vec::new();
        for (i, branch) in s["oneOf"].as_array().expect("oneOf").iter().enumerate() {
            let title = branch["title"].as_str().unwrap_or("<untitled>").to_string();
            let ty = branch["properties"]["type"]["const"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| "<none>".into());
            let action = branch["properties"]["action"]["const"]
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| "<open>".into());
            for (prev_ty, prev_action, prev_title) in &seen {
                assert!(
                    !(*prev_ty == ty && *prev_action == action),
                    "schema.json oneOf branches #{} and #{} both discriminate on \
                     (type={:?}, action={:?}) (titles {:?} and {:?}). A valid message \
                     would satisfy both, which is an error for any conforming validator.",
                    i,
                    seen.len(),
                    ty,
                    action,
                    prev_title,
                    title
                );
            }
            seen.push((ty, action, title));
        }
    }

    /// Branch titles are the identity used to map schema branches onto Rust
    /// types and generated TypeScript interfaces, so they must be unique.
    #[test]
    fn schema_oneof_branch_titles_are_unique() {
        let titles = oneof_titles(&schema());
        let unique: std::collections::BTreeSet<&String> = titles.iter().collect();
        assert_eq!(
            unique.len(),
            titles.len(),
            "schema.json oneOf has duplicate titles: {:?}",
            titles
                .iter()
                .filter(|t| titles.iter().filter(|x| *x == *t).count() > 1)
                .collect::<std::collections::BTreeSet<_>>()
        );
    }

    #[test]
    fn schema_sample_set_covers_every_oneof_branch() {
        let titles = oneof_titles(&schema());
        let samples = schema_samples();
        assert_eq!(
            samples.len(),
            titles.len(),
            "sample set size ({}) must equal oneOf branch count ({})",
            samples.len(),
            titles.len()
        );
        for title in &titles {
            assert!(
                samples.iter().any(|(t, _)| t == title),
                "missing schema sample for oneOf branch: {}",
                title
            );
        }
    }

    #[test]
    fn schema_accepts_every_sample_in_sample_set() {
        let v = validator();
        let mut failures = Vec::new();
        for (title, sample) in schema_samples() {
            if let Err(errors) = v.validate(&sample) {
                failures.push(format!("{}: {}", title, errors));
            }
        }
        assert!(
            failures.is_empty(),
            "schema rejected samples:\n{}",
            failures.join("\n")
        );
    }

    #[test]
    fn schema_accepts_every_serialized_rust_type() {
        // Serialize one instance of every public wire struct and validate.
        let v = validator();

        let samples: Vec<(String, serde_json::Value)> = vec![
            (
                "DiscoveryAnnounce".into(),
                serde_json::to_value(DiscoveryAnnounce {
                    msg_type: "discovery".into(),
                    action: "announce".into(),
                    protocol_version: Some(PROTOCOL_VERSION),
                    device_id: "d1".into(),
                    device_name: "Desktop".into(),
                    device_type: "desktop".into(),
                    os: "windows".into(),
                    version: "1.0".into(),
                    battery: Some(99),
                    ws_port: Some(1),
                    wss_port: Some(2),
                    apns_token: None,
                })
                .unwrap(),
            ),
            (
                "DiscoveryRemove".into(),
                serde_json::to_value(DiscoveryRemove {
                    msg_type: "discovery".into(),
                    action: "remove".into(),
                    device_id: "d1".into(),
                })
                .unwrap(),
            ),
            (
                "PairingRequest".into(),
                serde_json::to_value(PairingRequest {
                    msg_type: "pairing".into(),
                    action: "request".into(),
                    protocol_version: None,
                    token: "t".into(),
                    public_key: "pk".into(),
                    device_info: Some(DeviceInfo {
                        name: "N".into(),
                        device_type: "phone".into(),
                        os: Some("ios".into()),
                        battery: Some(50),
                    }),
                })
                .unwrap(),
            ),
            (
                "PairingAccept".into(),
                serde_json::to_value(PairingAccept {
                    msg_type: "pairing".into(),
                    action: "accept".into(),
                    protocol_version: None,
                    public_key: "pk".into(),
                    device_info: None,
                    device_id: None,
                    hub_device_id: None,
                    relay_url: None,
                    relay_token: None,
                    relay_cert_pin: None,
                })
                .unwrap(),
            ),
            (
                "PairingRevoke".into(),
                serde_json::to_value(PairingRevoke {
                    msg_type: "pairing".into(),
                    action: "revoke".into(),
                    protocol_version: None,
                    reason: Some("gone".into()),
                })
                .unwrap(),
            ),
            (
                "ClipboardSync".into(),
                serde_json::to_value(ClipboardSync {
                    msg_type: "clipboard".into(),
                    action: "sync".into(),
                    content: "c".into(),
                    mime: "text/plain".into(),
                    source_device: "d".into(),
                    timestamp: 1,
                })
                .unwrap(),
            ),
            (
                "ClipboardRequest".into(),
                serde_json::to_value(ClipboardRequest {
                    msg_type: "clipboard".into(),
                    action: "request".into(),
                    mime: None,
                })
                .unwrap(),
            ),
            (
                "NotificationPost".into(),
                serde_json::to_value(NotificationPost {
                    msg_type: "notification".into(),
                    action: "post".into(),
                    id: "n".into(),
                    device_id: "d".into(),
                    app: "A".into(),
                    title: "T".into(),
                    body: "B".into(),
                    timestamp: 1,
                    actions: None,
                })
                .unwrap(),
            ),
            (
                "NotificationDismiss".into(),
                serde_json::to_value(NotificationDismiss {
                    msg_type: "notification".into(),
                    action: "dismiss".into(),
                    id: "n".into(),
                })
                .unwrap(),
            ),
            (
                "NotificationMarkRead".into(),
                serde_json::to_value(NotificationMarkRead {
                    msg_type: "notification".into(),
                    action: "mark_read".into(),
                    id: "n".into(),
                })
                .unwrap(),
            ),
            (
                "NotificationReply".into(),
                serde_json::to_value(NotificationReply {
                    msg_type: "notification".into(),
                    action: "reply".into(),
                    id: "n".into(),
                    text: "ok".into(),
                })
                .unwrap(),
            ),
            (
                "FileRequest".into(),
                serde_json::to_value(FileRequest {
                    msg_type: "file".into(),
                    action: "request".into(),
                    id: "f".into(),
                    name: "n".into(),
                    size: 1,
                    mime: "text/plain".into(),
                    from: "d".into(),
                    to: None,
                    checksum: None,
                })
                .unwrap(),
            ),
            (
                "FileAccept".into(),
                serde_json::to_value(FileAccept {
                    msg_type: "file".into(),
                    action: "accept".into(),
                    id: "f".into(),
                })
                .unwrap(),
            ),
            (
                "FileChunk".into(),
                serde_json::to_value(FileChunk {
                    msg_type: "file".into(),
                    action: "chunk".into(),
                    id: "f".into(),
                    index: 0,
                    data: "AA".into(),
                    total: None,
                })
                .unwrap(),
            ),
            (
                "FileProgress".into(),
                serde_json::to_value(FileProgress {
                    msg_type: "file".into(),
                    action: "progress".into(),
                    id: "f".into(),
                    percent: 100,
                })
                .unwrap(),
            ),
            (
                "FileComplete".into(),
                serde_json::to_value(FileComplete {
                    msg_type: "file".into(),
                    action: "complete".into(),
                    id: "f".into(),
                    path: None,
                })
                .unwrap(),
            ),
            (
                "FileCancel".into(),
                serde_json::to_value(FileCancel {
                    msg_type: "file".into(),
                    action: "cancel".into(),
                    id: "f".into(),
                })
                .unwrap(),
            ),
            (
                "FileResume".into(),
                serde_json::to_value(FileResume {
                    msg_type: "file".into(),
                    action: "resume".into(),
                    id: "f".into(),
                    name: "n".into(),
                    size: 1,
                    mime: "text/plain".into(),
                    from: "d".into(),
                    checksum: None,
                })
                .unwrap(),
            ),
            (
                "FileResumeAck".into(),
                serde_json::to_value(FileResumeAck {
                    msg_type: "file".into(),
                    action: "resume_ack".into(),
                    id: "f".into(),
                    chunks_loaded: 0,
                })
                .unwrap(),
            ),
            (
                "AudioStreamStart".into(),
                serde_json::to_value(AudioStreamStart {
                    msg_type: "audio".into(),
                    action: "stream_start".into(),
                })
                .unwrap(),
            ),
            (
                "AudioStreamStop".into(),
                serde_json::to_value(AudioStreamStop {
                    msg_type: "audio".into(),
                    action: "stream_stop".into(),
                })
                .unwrap(),
            ),
            (
                "AudioStreamStarted".into(),
                serde_json::to_value(AudioStreamStarted {
                    msg_type: "audio".into(),
                    action: "stream_started".into(),
                    from: "d".into(),
                })
                .unwrap(),
            ),
            (
                "AudioStreamData".into(),
                serde_json::to_value(AudioStreamData {
                    msg_type: "audio".into(),
                    action: "stream_data".into(),
                    data: "AA".into(),
                    format: "pcm16".into(),
                    sample_rate: 48000,
                    channels: 2,
                    from: "d".into(),
                })
                .unwrap(),
            ),
            (
                "AudioPlaybackStart".into(),
                serde_json::to_value(AudioPlaybackStart {
                    msg_type: "audio".into(),
                    action: "playback_start".into(),
                })
                .unwrap(),
            ),
            (
                "AudioPlaybackStop".into(),
                serde_json::to_value(AudioPlaybackStop {
                    msg_type: "audio".into(),
                    action: "playback_stop".into(),
                })
                .unwrap(),
            ),
            (
                "AudioPlaybackStarted".into(),
                serde_json::to_value(AudioPlaybackStarted {
                    msg_type: "audio".into(),
                    action: "playback_started".into(),
                    from: "d".into(),
                })
                .unwrap(),
            ),
            (
                "AudioPlaybackData".into(),
                serde_json::to_value(AudioPlaybackData {
                    msg_type: "audio".into(),
                    action: "playback_data".into(),
                    data: "AA".into(),
                    format: "pcm16".into(),
                    sample_rate: 48000,
                    channels: 2,
                })
                .unwrap(),
            ),
            (
                "ScreenMirrorStart".into(),
                serde_json::to_value(ScreenMirrorStart {
                    msg_type: "screen_mirror".into(),
                    action: "start".into(),
                    quality: Some("high".into()),
                    fps: None,
                })
                .unwrap(),
            ),
            (
                "ScreenMirrorStop".into(),
                serde_json::to_value(ScreenMirrorStop {
                    msg_type: "screen_mirror".into(),
                    action: "stop".into(),
                })
                .unwrap(),
            ),
            (
                "ScreenMirrorFrame".into(),
                serde_json::to_value(ScreenMirrorFrame {
                    msg_type: "screen_mirror".into(),
                    action: "frame".into(),
                    data: "AA".into(),
                    format: "jpeg".into(),
                    width: 100,
                    height: 50,
                    from_desktop: Some(true),
                })
                .unwrap(),
            ),
            (
                "ScreenMirrorCaptureStopped".into(),
                serde_json::to_value(ScreenMirrorCaptureStopped {
                    msg_type: "screen_mirror".into(),
                    action: "capture_stopped".into(),
                })
                .unwrap(),
            ),
            (
                "ScreenMirrorTouch".into(),
                serde_json::to_value(ScreenMirrorTouch {
                    msg_type: "screen_mirror".into(),
                    action: "touch".into(),
                    x: 1.0,
                    y: 2.0,
                    action_type: Some("tap".into()),
                })
                .unwrap(),
            ),
            (
                "ScreenMirrorKey".into(),
                serde_json::to_value(ScreenMirrorKey {
                    msg_type: "screen_mirror".into(),
                    action: "key".into(),
                    key: "a".into(),
                    modifiers: None,
                })
                .unwrap(),
            ),
            (
                "ScreenMirrorScroll".into(),
                serde_json::to_value(ScreenMirrorScroll {
                    msg_type: "screen_mirror".into(),
                    action: "scroll".into(),
                    dx: 1.0,
                    dy: 1.0,
                })
                .unwrap(),
            ),
            (
                "RemoteInputMove".into(),
                serde_json::to_value(RemoteInputMove {
                    msg_type: "remote_input".into(),
                    action: "move".into(),
                    dx: 1.0,
                    dy: 1.0,
                })
                .unwrap(),
            ),
            (
                "RemoteInputClick".into(),
                serde_json::to_value(RemoteInputClick {
                    msg_type: "remote_input".into(),
                    action: "click".into(),
                    button: "left".into(),
                })
                .unwrap(),
            ),
            (
                "RemoteInputScroll".into(),
                serde_json::to_value(RemoteInputScroll {
                    msg_type: "remote_input".into(),
                    action: "scroll".into(),
                    dx: 1.0,
                    dy: 1.0,
                })
                .unwrap(),
            ),
            (
                "AutomationRuleMessage".into(),
                serde_json::to_value(AutomationRuleMessage {
                    msg_type: "automation".into(),
                    action: "rule".into(),
                    id: "r".into(),
                    name: "n".into(),
                    trigger: AutomationTrigger {
                        trigger_type: "time".into(),
                        device_id: None,
                        time: Some("12:00".into()),
                        below: None,
                        ssid: None,
                        app_package: None,
                    },
                    rule_action: AutomationActionPayload {
                        action_type: "send_notification".into(),
                        title: Some("T".into()),
                        body: None,
                        profile: None,
                        device_id: None,
                        command: None,
                        enabled: None,
                        url: None,
                        app_package: None,
                        state: None,
                    },
                    enabled: true,
                })
                .unwrap(),
            ),
            (
                "AutomationDelete".into(),
                serde_json::to_value(AutomationDelete {
                    msg_type: "automation".into(),
                    action: "delete".into(),
                    rule_id: "r".into(),
                })
                .unwrap(),
            ),
            (
                "AutomationSync".into(),
                serde_json::to_value(AutomationSync {
                    msg_type: "automation".into(),
                    action: "sync".into(),
                    rules: vec![],
                    full_sync: None,
                })
                .unwrap(),
            ),
            (
                "AutomationTriggered".into(),
                serde_json::to_value(AutomationTriggered {
                    msg_type: "automation".into(),
                    action: "triggered".into(),
                    id: "r".into(),
                    trigger_type: "time".into(),
                    device_id: None,
                })
                .unwrap(),
            ),
            (
                "CallMessage".into(),
                serde_json::to_value(CallMessage {
                    msg_type: "call".into(),
                    action: "end".into(),
                    call_id: None,
                    to_device_id: None,
                    route: None,
                    number: None,
                    name: None,
                    device_id: None,
                })
                .unwrap(),
            ),
            (
                "SmsSend".into(),
                serde_json::to_value(SmsSend {
                    msg_type: "sms".into(),
                    action: "send".into(),
                    to: "+1".into(),
                    body: "b".into(),
                })
                .unwrap(),
            ),
            (
                "SmsSync".into(),
                serde_json::to_value(SmsSync {
                    msg_type: "sms".into(),
                    action: "sync".into(),
                    threads: vec![SmsThread {
                        thread_id: "t_1700000000000".into(),
                        address: "+15551234567".into(),
                        name: Some("Ada Lovelace".into()),
                        snippet: "hello".into(),
                        unread_count: 1,
                        timestamp: 1_700_000_000,
                        messages: vec![SmsMessage {
                            id: "in_1700000000000".into(),
                            address: "+15551234567".into(),
                            body: "hello".into(),
                            timestamp: 1_700_000_000,
                            read: false,
                            is_outgoing: false,
                        }],
                    }],
                })
                .unwrap(),
            ),
            (
                "SmsNew (sender shape)".into(),
                serde_json::to_value(SmsNew {
                    msg_type: "sms".into(),
                    action: "new".into(),
                    from: Some("+15551234567".into()),
                    body: Some("hello".into()),
                    timestamp: Some(1_700_000_000),
                    thread_id: None,
                    message: None,
                })
                .unwrap(),
            ),
            (
                "SmsNew (receiver shape)".into(),
                serde_json::to_value(SmsNew {
                    msg_type: "sms".into(),
                    action: "new".into(),
                    from: None,
                    body: None,
                    timestamp: None,
                    thread_id: Some("t_1700000000000".into()),
                    message: Some(SmsMessage {
                        id: "in_1700000000000".into(),
                        address: "+15551234567".into(),
                        body: "hello".into(),
                        timestamp: 1_700_000_000,
                        read: false,
                        is_outgoing: false,
                    }),
                })
                .unwrap(),
            ),
            (
                "SmsSent (sender shape)".into(),
                serde_json::to_value(SmsSent {
                    msg_type: "sms".into(),
                    action: "sent".into(),
                    to: Some("+15551234567".into()),
                    body: Some("hello".into()),
                    timestamp: Some(1_700_000_000),
                    thread_id: None,
                    message: None,
                })
                .unwrap(),
            ),
            (
                "SmsSent (receiver shape)".into(),
                serde_json::to_value(SmsSent {
                    msg_type: "sms".into(),
                    action: "sent".into(),
                    to: None,
                    body: None,
                    timestamp: None,
                    thread_id: Some("t_1700000000000".into()),
                    message: Some(SmsMessage {
                        id: "out_1700000000000".into(),
                        address: "+15551234567".into(),
                        body: "hello".into(),
                        timestamp: 1_700_000_000,
                        read: true,
                        is_outgoing: true,
                    }),
                })
                .unwrap(),
            ),
            (
                "StatusUpdate".into(),
                serde_json::to_value(StatusUpdate {
                    msg_type: "status".into(),
                    action: "update".into(),
                    battery: None,
                    wifi_ssid: None,
                    device_info: None,
                })
                .unwrap(),
            ),
            ("Ping".into(), serde_json::to_value(Ping::new()).unwrap()),
            ("Pong".into(), serde_json::to_value(Pong::new()).unwrap()),
            (
                "ErrorMessage".into(),
                serde_json::to_value(ErrorMessage {
                    msg_type: "error".into(),
                    code: "E".into(),
                    message: "m".into(),
                    server_version: None,
                })
                .unwrap(),
            ),
            (
                "RelayAuth".into(),
                serde_json::to_value(RelayAuth::new("d", "t")).unwrap(),
            ),
            (
                "RelayAuthOk".into(),
                serde_json::to_value(RelayAuthOk {
                    msg_type: "relay_auth_ok".into(),
                })
                .unwrap(),
            ),
            (
                "RelayAuthRejected".into(),
                serde_json::to_value(RelayAuthRejected {
                    msg_type: "relay_auth_rejected".into(),
                    reason: "missing_token".into(),
                })
                .unwrap(),
            ),
            (
                "RelayRoute".into(),
                serde_json::to_value(RelayRoute::new("d2", json!({"type": "ping"}))).unwrap(),
            ),
            (
                "EncryptedEnvelope".into(),
                serde_json::to_value(EncryptedEnvelope::new("n", "h", "d")).unwrap(),
            ),
        ];

        let mut failures = Vec::new();
        for (name, sample) in &samples {
            if let Err(errors) = v.validate(sample) {
                failures.push(format!("{}: {}", name, errors));
            }
        }
        assert!(
            failures.is_empty(),
            "schema rejected serialized Rust types:\n{}",
            failures.join("\n")
        );
        // Sanity: we covered a broad set of structs.
        assert!(
            samples.len() >= 50,
            "expected >=50 serialized type samples, got {}",
            samples.len()
        );
    }

    // ---------------------------------------------------------------
    //  Schema negative cases
    // ---------------------------------------------------------------

    #[test]
    fn schema_rejects_missing_required_fields_relay_auth() {
        let v = validator();
        // Missing relay_token
        let bad = json!({"type": "relay_auth", "device_id": "d1"});
        assert!(!is_valid(&v, &bad), "missing relay_token must be rejected");

        // Missing device_id
        let bad2 = json!({"type": "relay_auth", "relay_token": "t"});
        assert!(!is_valid(&v, &bad2), "missing device_id must be rejected");
    }

    #[test]
    fn schema_rejects_missing_required_fields_encrypted() {
        let v = validator();
        let bad = json!({"type": "encrypted", "nonce": "aa", "hmac": "bb"});
        assert!(!is_valid(&v, &bad), "missing data must be rejected");
    }

    #[test]
    fn schema_rejects_missing_required_fields_audio_stream_data() {
        let v = validator();
        let bad = json!({
            "type": "audio", "action": "stream_data",
            "data": "AA", "format": "pcm16",
            "channels": 1, "from": "d"
        });
        assert!(!is_valid(&v, &bad), "missing sample_rate must be rejected");
    }

    #[test]
    fn schema_rejects_missing_required_fields_screen_frame() {
        let v = validator();
        let bad = json!({
            "type": "screen_mirror", "action": "frame",
            "data": "AA", "format": "jpeg",
            "height": 50
        });
        assert!(!is_valid(&v, &bad), "missing width must be rejected");
    }

    #[test]
    fn schema_rejects_missing_required_fields_automation_rule() {
        let v = validator();
        let bad = json!({
            "type": "automation", "action": "rule",
            "id": "r1", "name": "n",
            "trigger": {"type": "time"},
            "enabled": true
        });
        assert!(!is_valid(&v, &bad), "missing rule_action must be rejected");
    }

    #[test]
    fn schema_rejects_missing_required_fields_file_request() {
        let v = validator();
        let bad = json!({
            "type": "file", "action": "request",
            "id": "f", "name": "n", "mime": "text/plain", "from": "d"
        });
        assert!(!is_valid(&v, &bad), "missing size must be rejected");
    }

    #[test]
    fn schema_rejects_missing_required_fields_status_no_action() {
        let v = validator();
        let bad = json!({"type": "status", "battery": 50});
        assert!(!is_valid(&v, &bad), "missing action must be rejected");
    }

    #[test]
    fn schema_rejects_missing_required_fields_call_no_action() {
        let v = validator();
        let bad = json!({"type": "call"});
        assert!(!is_valid(&v, &bad), "missing action must be rejected");
    }

    #[test]
    fn schema_rejects_missing_required_fields_error() {
        let v = validator();
        let bad = json!({"type": "error", "code": "E"});
        assert!(!is_valid(&v, &bad), "missing message must be rejected");
    }

    #[test]
    fn schema_rejects_missing_required_fields_relay_route() {
        let v = validator();
        let bad = json!({"type": "relay_route", "to_device_id": "d"});
        assert!(!is_valid(&v, &bad), "missing payload must be rejected");
    }

    // ---------------------------------------------------------------
    //  Schema enum / bound / type violations
    // ---------------------------------------------------------------

    #[test]
    fn schema_rejects_invalid_device_type_enum() {
        let v = validator();
        let bad = json!({
            "type": "discovery", "action": "announce",
            "device_id": "d", "device_name": "n",
            "device_type": "smartfridge", "os": "ios", "version": "1"
        });
        assert!(!is_valid(&v, &bad), "unknown device_type must be rejected");
    }

    #[test]
    fn schema_rejects_file_progress_percent_over_100() {
        let v = validator();
        let bad = json!({
            "type": "file", "action": "progress",
            "id": "f", "percent": 101
        });
        assert!(!is_valid(&v, &bad), "percent > 100 must be rejected");
    }

    #[test]
    fn schema_rejects_negative_file_size() {
        let v = validator();
        let bad = json!({
            "type": "file", "action": "request",
            "id": "f", "name": "n", "size": -1,
            "mime": "text/plain", "from": "d"
        });
        assert!(!is_valid(&v, &bad), "negative size must be rejected");
    }

    #[test]
    fn schema_rejects_empty_pairing_token() {
        let v = validator();
        let bad = json!({
            "type": "pairing", "action": "request",
            "token": "", "public_key": "pk"
        });
        assert!(
            !is_valid(&v, &bad),
            "empty token (minLength 1) must be rejected"
        );
    }

    #[test]
    fn schema_rejects_protocol_version_zero() {
        let v = validator();
        let bad = json!({
            "type": "discovery", "action": "announce",
            "protocol_version": 0,
            "device_id": "d", "device_name": "n",
            "device_type": "phone", "os": "ios", "version": "1"
        });
        assert!(
            !is_valid(&v, &bad),
            "protocol_version minimum is 1; 0 must be rejected"
        );
    }

    #[test]
    fn schema_rejects_device_info_additional_properties() {
        let v = validator();
        let bad = json!({
            "type": "pairing", "action": "request",
            "token": "t", "public_key": "pk",
            "device_info": {
                "name": "N", "type": "phone",
                "evil": "extra"
            }
        });
        assert!(
            !is_valid(&v, &bad),
            "DeviceInfo additionalProperties:false must reject extra keys"
        );
    }

    #[test]
    fn schema_rejects_negative_battery_in_device_info() {
        let v = validator();
        let bad = json!({
            "type": "pairing", "action": "request",
            "token": "t", "public_key": "pk",
            "device_info": {
                "name": "N", "type": "phone",
                "battery": -5
            }
        });
        assert!(!is_valid(&v, &bad), "battery minimum 0 must reject -5");
    }

    #[test]
    fn schema_rejects_non_object_relay_route_payload() {
        let v = validator();
        let bad = json!({
            "type": "relay_route",
            "to_device_id": "d",
            "payload": "not-an-object"
        });
        assert!(!is_valid(&v, &bad), "payload must be an object per schema");
    }

    #[test]
    fn schema_rejects_wrong_audio_format_const() {
        let v = validator();
        let bad = json!({
            "type": "audio", "action": "stream_data",
            "data": "AA", "format": "mp3",
            "sample_rate": 16000, "channels": 1, "from": "d"
        });
        assert!(
            !is_valid(&v, &bad),
            "format must be const pcm16 for stream_data"
        );
    }

    #[test]
    fn schema_rejects_unknown_type_tag() {
        let v = validator();
        let bad = json!({"type": "wormhole", "action": "open"});
        assert!(
            !is_valid(&v, &bad),
            "unknown type must match no oneOf branch"
        );
    }

    #[test]
    fn schema_allows_unknown_extra_fields_on_top_level_messages() {
        // Top-level message branches do NOT set additionalProperties:false
        // (unlike DeviceInfo). Document that intentionally.
        let v = validator();
        let with_extra = json!({
            "type": "ping",
            "vendor_extension": true
        });
        assert!(
            is_valid(&v, &with_extra),
            "top-level messages currently allow extra fields"
        );
    }

    // ---------------------------------------------------------------
    //  Serde vs schema alignment (formerly divergences — now enforced)
    // ---------------------------------------------------------------

    #[test]
    fn serde_schema_divergence_automation_delete_emits_rule_id() {
        // Schema accepts rule_id (or id alias); serde emits rule_id.
        let msg = AutomationDelete {
            msg_type: "automation".into(),
            action: "delete".into(),
            rule_id: "abc".into(),
        };
        let v_serde = serde_json::to_value(&msg).unwrap();
        assert_eq!(
            v_serde["rule_id"], "abc",
            "serde must emit rule_id (schema required field)"
        );
        assert!(
            v_serde.get("id").is_none(),
            "serde should not emit the id alias by default"
        );
        let v = validator();
        assert!(
            is_valid(&v, &v_serde),
            "serialized AutomationDelete must validate"
        );
    }

    #[test]
    fn serde_schema_divergence_automation_delete_accepts_id_alias() {
        // Deserialization accepts `id` as alias for rule_id (serde alias),
        // and the schema now accepts either rule_id or id via anyOf.
        let from_id: AutomationDelete =
            serde_json::from_str(r#"{"type":"automation","action":"delete","id":"x"}"#)
                .expect("serde alias id → rule_id");
        assert_eq!(from_id.rule_id, "x");

        let v = validator();
        let wire = json!({"type": "automation", "action": "delete", "id": "x"});
        assert!(
            is_valid(&v, &wire),
            "schema anyOf must accept id-only AutomationDelete payloads"
        );
    }

    #[test]
    fn serde_schema_divergence_screen_touch_uses_action_type_rename() {
        // Rust field action_type serializes as "actionType" matching schema.
        let msg = ScreenMirrorTouch {
            msg_type: "screen_mirror".into(),
            action: "touch".into(),
            x: 1.0,
            y: 2.0,
            action_type: Some("long_press".into()),
        };
        let v_serde = serde_json::to_value(&msg).unwrap();
        assert_eq!(v_serde["actionType"], "long_press");
        assert!(
            v_serde.get("action_type").is_none(),
            "snake_case must not leak"
        );
        let v = validator();
        assert!(is_valid(&v, &v_serde));

        // Invalid actionType values now fail at the serde boundary.
        let bad = ScreenMirrorTouch {
            msg_type: "screen_mirror".into(),
            action: "touch".into(),
            x: 1.0,
            y: 2.0,
            action_type: Some("swipe".into()),
        };
        assert!(
            serde_json::to_value(&bad).is_err(),
            "serde must reject actionType outside the schema enum"
        );
        assert!(
            serde_json::from_str::<ScreenMirrorTouch>(
                r#"{"type":"screen_mirror","action":"touch","x":1.0,"y":1.0,"actionType":"swipe"}"#
            )
            .is_err(),
            "deserialization must reject actionType outside the schema enum"
        );
    }

    #[test]
    fn serde_schema_divergence_relay_auth_rejected_reason_enum() {
        // Rust reason is constrained to the schema enum on both ser and de.
        let ok = RelayAuthRejected {
            msg_type: "relay_auth_rejected".into(),
            reason: "invalid_token".into(),
        };
        let v = validator();
        assert!(is_valid(&v, &serde_json::to_value(&ok).unwrap()));

        let loose = RelayAuthRejected {
            msg_type: "relay_auth_rejected".into(),
            reason: "because_i_said_so".into(),
        };
        assert!(
            serde_json::to_value(&loose).is_err(),
            "serde must reject reason outside the schema enum"
        );
        assert!(
            serde_json::from_str::<RelayAuthRejected>(
                r#"{"type":"relay_auth_rejected","reason":"because_i_said_so"}"#
            )
            .is_err(),
            "deserialization must reject reason outside the schema enum"
        );
        // Schema still rejects hand-built invalid payloads.
        let wire = json!({"type": "relay_auth_rejected", "reason": "because_i_said_so"});
        assert!(
            !is_valid(&v, &wire),
            "schema enums reason; free-form string must fail validation"
        );
    }

    #[test]
    fn serde_schema_divergence_status_update_battery_unbounded_in_serde() {
        // Rust StatusUpdate.battery is constrained to 0..=100 on ser and de.
        let out_of_range = StatusUpdate {
            msg_type: "status".into(),
            action: "update".into(),
            battery: Some(150),
            wifi_ssid: None,
            device_info: None,
        };
        assert!(
            serde_json::to_value(&out_of_range).is_err(),
            "serde must reject battery outside 0..=100"
        );
        assert!(
            serde_json::from_str::<StatusUpdate>(
                r#"{"type":"status","action":"update","battery":150}"#
            )
            .is_err(),
            "deserialization must reject battery outside 0..=100"
        );
        let v = validator();
        let wire = json!({"type": "status", "action": "update", "battery": 150});
        assert!(
            !is_valid(&v, &wire),
            "schema maximum:100 must reject battery 150"
        );
        // In-range values still work.
        let ok = StatusUpdate {
            msg_type: "status".into(),
            action: "update".into(),
            battery: Some(100),
            wifi_ssid: None,
            device_info: None,
        };
        assert!(is_valid(&v, &serde_json::to_value(&ok).unwrap()));
    }

    // ---------------------------------------------------------------
    //  Edge-case roundtrips
    // ---------------------------------------------------------------

    #[test]
    fn roundtrip_special_characters_and_nul() {
        let msg = ClipboardSync {
            msg_type: "clipboard".into(),
            action: "sync".into(),
            content: "emoji \u{1F600} nul\u{0} quote\" back\\slash\nnewline\ttab".into(),
            mime: "text/plain".into(),
            source_device: "dev\u{0}-1".into(),
            timestamp: i64::MAX,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.content, msg.content);
        assert_eq!(restored.source_device, msg.source_device);
        assert_eq!(restored.timestamp, i64::MAX);
    }

    #[test]
    fn roundtrip_file_chunk_u32_max_index() {
        let msg = FileChunk {
            msg_type: "file".into(),
            action: "chunk".into(),
            id: "f".into(),
            index: u32::MAX,
            data: "AA".into(),
            total: Some(u32::MAX),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.index, u32::MAX);
        assert_eq!(restored.total, Some(u32::MAX));
    }

    #[test]
    fn roundtrip_timestamp_i64_min() {
        let msg = ClipboardSync {
            msg_type: "clipboard".into(),
            action: "sync".into(),
            content: "x".into(),
            mime: "text/plain".into(),
            source_device: "d".into(),
            timestamp: i64::MIN,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.timestamp, i64::MIN);
    }

    #[test]
    fn roundtrip_empty_strings_everywhere() {
        let msg = SmsSend {
            msg_type: "sms".into(),
            action: "send".into(),
            to: "".into(),
            body: "".into(),
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.to, "");
        assert_eq!(restored.body, "");
    }

    #[test]
    fn roundtrip_large_string_payload() {
        let big = "A".repeat(256 * 1024);
        let msg = ClipboardSync {
            msg_type: "clipboard".into(),
            action: "sync".into(),
            content: big.clone(),
            mime: "text/plain".into(),
            source_device: "d".into(),
            timestamp: 0,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.content.len(), big.len());
        assert_eq!(restored.content, big);
    }

    #[test]
    fn roundtrip_control_characters_preserved() {
        let content: String = (0u8..32).map(|c| c as char).collect();
        let msg = ClipboardSync {
            msg_type: "clipboard".into(),
            action: "sync".into(),
            content: content.clone(),
            mime: "text/plain".into(),
            source_device: "d".into(),
            timestamp: 1,
        };
        let restored = roundtrip(&msg);
        assert_eq!(restored.content, content);
    }
}
