# Conduit Wire Protocol Specification

**Version:** 1
**Status:** Living document — tracks the deployed protocol.

Everything below is asserted against the code in
`packages/protocol/src/types.rs` and `packages/protocol/src/lib.rs` (the
authoritative definitions), plus the two implementations that consume them:
`services/relay` and `apps/desktop/src-tauri`. Where a statement could not be
verified in code it is marked **[unverified]** rather than guessed.

---

## 1. Overview

Conduit uses a **JSON-over-WebSocket** protocol for all control messages and
a **binary framing** format for high-throughput file transfers. Every device
(desktop, mobile, relay) speaks the same wire format, preventing protocol drift.

Every device speaks the same protocol, so there is one wire format to
implement rather than three drifting ones.

Two things in this document are *not* symmetric, and a client that assumes they
are will break:

* **`relay_route` is signed.** Since the v1 relay envelope the route carries an
  HMAC-SHA256 tag over an ordered field subset, plus `from_device_id`, `key_id`
  and a replay nonce. The relay rejects an unsigned or wrongly-signed route
  outright. See §4.15.2 and §9.
* **The relay's binary frame is v2.** The 17-byte, unauthenticated v1 layout is
  retired and the relay rejects version `0x01`. See §5.1 and §9.

> **Encryption is per-peer, and it is terminated by the desktop — it is not
> end-to-end.** The `encrypted` envelope (§4.16, §6) is keyed to a secret shared
> between the desktop hub and one peer. The hub decrypts inbound envelopes and
> dispatches the inner JSON, and it cannot open an envelope addressed to a peer
> it does not share a secret with. A relay therefore sees the outer envelope,
> and either endpoint of a LAN hop sees the plaintext. No message type in this
> build is end-to-end encrypted; see ADR-0007 and §6.4 for which types are
> wrapped at all.

### 1.1 Protocol Version

`PROTOCOL_VERSION = 1` (`packages/protocol/src/types.rs`). A message MAY carry
an integer `protocol_version`.

> **Known gap — enforcement is asymmetric.** Only the **desktop hub** enforces
> it: `apps/desktop/src-tauri/src/server/mod.rs` rejects any inbound frame whose
> `protocol_version` is greater than `PROTOCOL_VERSION` and answers
> `unsupported_protocol_version`. The **relay does not check
> `protocol_version` at all**, and neither does the mobile app. A client MUST
> NOT assume that a peer has rejected a too-new version on its behalf.

```json
{
  "type": "error",
  "code": "unsupported_protocol_version",
  "message": "Server supports protocol_version 1, got 2",
  "server_version": 1
}
```

---

## 2. Transport

### 2.1 Desktop LAN listeners

The desktop binds **two** WebSocket listeners. They are not interchangeable.

| Transport | Port | TLS | Use |
|-----------|------|-----|-----|
| WS (LAN) | 9527 | No | Desktop's plaintext LAN listener. No encryption in transit, no certificate presented. |
| WSS (LAN) | 9531 | TLS, self-signed | Desktop ↔ Mobile on LAN. **The only port a mobile client dials.** |

These two numbers are defined exactly once, as
`conduit_protocol::types::LAN_WS_PORT` and `conduit_protocol::types::LAN_WSS_PORT`
(`packages/protocol/src/types.rs`). The desktop re-exports them as `WS_PORT` /
`WSS_PORT` (`apps/desktop/src-tauri/src/main.rs`); the mobile app mirrors them
as `kLanWsPort` / `kLanWssPort`
(`apps/mobile/lib/services/websocket_service.dart`). Unit tests assert that the
Rust constants, this document and the Dart constants all agree.

### 2.2 Which port a client must dial

> **Never dial `wss://` on port 9527, and never silently downgrade a LAN peer
> to `ws://` on 9527.**

9527 is a plaintext socket. A `wss://` handshake against it throws. Because
the mobile app's certificate-pin bootstrap only runs on the `wss://` code path
(`WebSocketService._verifyCertificatePin`), a failed handshake there means the
pin is never captured and every subsequent connection fails closed — the app
can never reach an authenticated state, and the desktop's `not_authenticated`
path rejects every non-pairing message. This is not a recoverable error; the
mobile app is simply non-functional over LAN.

Consequently:

* A LAN peer is **always** dialled as `wss://<ip>:9531`, or on the TLS port
  the peer advertises in its mDNS TXT record (`wss_port`).
* The `ws://` listener on 9527 exists for plaintext clients and diagnostics.
  A client that wants it must ask for it explicitly; it is never a fallback.
* Peers advertise **both** ports: the mDNS **SRV** record carries the plaintext
  port (9527) and the mDNS **TXT** record carries `ws_port` and `wss_port`.
  A client needing TLS must read `wss_port` from the TXT record, not the SRV
  port.

### 2.3 Relay listeners

Separate service (`services/relay`), every port environment-overridable:

| Transport | Default port | TLS | Use |
|-----------|--------------|-----|-----|
| WSS (relay) | 9529 | TLS 1.3 | Relay server. Always bound. |
| WS (relay) | 9528 | No | Relay development. **Off by default** — set `RELAY_ENABLE_PLAIN_WS=true` to bind it. |
| Health HTTP (relay) | 9530 | No | `/healthz`, `/health`, `/metrics`, `/pin` |

The relay serves exactly these HTTP routes on the health port:

| Route | Auth | Body |
|-------|------|------|
| `GET /healthz` | none | `{"status":"ok"}` — liveness only, no counters. |
| `GET /health` | `Authorization: Bearer <RELAY_HEALTH_TOKEN>` | The same JSON document `/metrics` renders, as an object. |
| `GET /` | same as `/health` | Alias of `/health`. |
| `GET /metrics` | none unless `RELAY_METRICS_TOKEN` is set | Prometheus text exposition (`text/plain; version=0.0.4`). |
| `GET /pin` | none | `{"sha256": "<pin>", "algorithm": "spki-sha256"}`, or the same with `"sha256": null` and `"error": "tls_unavailable"` when TLS did not initialise. |
| anything else | — | `404 Not Found`. |

`RELAY_HEALTH_TOKEN` falls back to the resolved HMAC secret when unset or
empty. `/metrics` is deliberately unauthenticated by default so a Prometheus
scrape does not need the health secret; set `RELAY_METRICS_TOKEN` to gate it.

#### 2.3.1 Relay configuration

All relay configuration is environment variables; there is no config file.
Startup is fail-closed — a missing or empty `RELAY_TOKEN` aborts with
`EX_CONFIG` (78), a port bind failure with `EX_IOERR` (74), and a process that
ends up with no listeners at all with `EX_UNAVAILABLE` (69).

| Variable | Required | Default | Purpose |
|----------|----------|---------|---------|
| `RELAY_TOKEN` | **yes** | — | Shared bearer token every client sends in `relay_auth`. Empty is refused. |
| `RELAY_WS_PORT` | no | `9528` | Plaintext relay listener. Only bound when `RELAY_ENABLE_PLAIN_WS` is set. |
| `RELAY_WSS_PORT` | no | `9529` | TLS relay listener. |
| `RELAY_HEALTH_PORT` | no | `9530` | Plain HTTP health/metrics/pin listener. |
| `RELAY_ENABLE_PLAIN_WS` | no | `false` | `true` or `1` binds the plaintext listener. |
| `RELAY_AUTH_TIMEOUT_SECS` | no | `10` | How long a fresh socket may take to send a valid `relay_auth` before it is dropped. |
| `HMAC_SECRET` | no | — | Master secret. If unset, resolved from `HMAC_SECRET_FILE` or bootstrapped. |
| `HMAC_SECRET_FILE` | no | `./secrets/hmac_secret` | Read the master secret from here; a missing file is generated (32 random bytes, hex, persisted `0600`). Fail-closed: an unreadable non-empty file aborts startup. |
| `RELAY_SIGNING_KEY` | no | derived | The `relay_route`/binary-frame message-signing key. Defaults to `derive_signing_key(HMAC_SECRET)`. **Never set this to `RELAY_TOKEN`.** |
| `RELAY_SIGNING_KEY_ID` | no | `v1` | The `key_id` the relay publishes for `RELAY_SIGNING_KEY`. |
| `RELAY_SIGNING_KEY_PREVIOUS` | no | — | The retiring key, accepted for verification during a rotation window. |
| `RELAY_SIGNING_KEY_PREVIOUS_ID` | with the above | — | Its `key_id`. Required whenever `RELAY_SIGNING_KEY_PREVIOUS` is set, and must differ from `RELAY_SIGNING_KEY_ID`. |
| `RELAY_HEALTH_TOKEN` | no | `HMAC_SECRET` | Bearer token for `/health` and `/`. |
| `RELAY_METRICS_TOKEN` | no | — | When set, `/metrics` requires it. |
| `RELAY_NONCE_FILE` | no | `./data/nonces.json` | Persisted replay-nonce cache. |
| `RELAY_CERT_DIR` | no | — | Directory holding the TLS certificate and key. |
| `RELAY_TLS_HOSTNAME` | no | — | Hostname placed in the generated certificate. |
| `RELAY_TLS_EXTRA_SANS` | no | — | Extra subject alternative names for the generated certificate. |

### 2.4 Connection Lifecycle

1. Client connects via WS or WSS.
2. Client sends `relay_auth` (if connecting through relay), within
   `RELAY_AUTH_TIMEOUT_SECS` of the socket opening. The relay replies
   `relay_auth_ok` or `relay_auth_rejected` (§4.15.1) and drops the connection
   on failure.
3. Client sends `discovery/announce` with device info. **This is not exempt from
   authentication**: the desktop answers `not_authenticated` until the peer has
   paired (§4.1.1).
4. Peer responds with its own `discovery/announce`.
5. Pairing handshake, if not already paired (§4.2).
6. Normal message exchange with `ping`/`pong` keep-alive every 25 s.
7. Disconnect after 60 s of silence (no pong/message received).

### 2.5 Message-size limits

There is no single global limit; **each hop enforces its own** and they differ by
a factor of 50.

| Where | Constant | Value | Enforcement |
|-------|----------|-------|-------------|
| Relay, inbound text frames | `MAX_TEXT_SIZE` (`services/relay/src/main.rs`) | `1024 * 1024` (1 MiB) | Checked during auth and after it, before parsing. Over the limit the relay logs and **closes the connection** — it does *not* send an `error` frame, so `message_too_large` is a documented code that is not currently emitted. |
| Relay, tungstenite read limits | tungstenite defaults | 64 MiB message / 16 MiB frame | The relay calls `accept_async` with no `WebSocketConfig`, so the defaults stand; the 1 MiB check above is what actually bounds memory. |
| Desktop, WebSocket read limits | `security::MAX_MESSAGE_SIZE` / `MAX_FRAME_SIZE` | `50 * 1024 * 1024` (50 MiB) each | Passed to `WebSocketConfig::max_message_size` / `max_frame_size` by `ws_read_limits()` (`server/mod.rs`), so a too-large frame is refused *while it is being read*. |
| Desktop, outbound queue | `security::MAX_SEND_BUFFER_BYTES` | `8 * 1024 * 1024` (8 MiB) | `max_write_buffer_size`; a client that stops reading is disconnected instead of growing server memory. |

Practical consequence: a control message larger than 1 MiB is accepted on a
direct LAN/desktop connection and rejected at the relay. Clients that route
through the relay must keep frames under 1 MiB; large payloads (file chunks,
audio, mirror frames) belong in the binary paths of §5.

---

## 3. Message Structure

Every JSON message MUST have a `"type"` string field. Most messages also
have an `"action"` field. The dispatch key is `(type, action)`.

```
{
  "type": "<domain>",
  "action": "<verb>",
  "protocol_version": 1,
  ...domain-specific fields...
}
```

The desktop hub closes its field set for the types it validates
(`security::reject_unknown_fields`): for `pairing`, `status` and `discovery`, a
frame carrying a field the handler does not read is refused with
`invalid_message` rather than forwarded inside the encrypted envelope.

---

## 4. Message Types

### 4.1 Discovery

`discovery` is **not** exempt from the authentication gate. A device must be
paired before the desktop will answer an announce, because the reply discloses
the desktop's identity, name, OS, version and both LAN ports.

#### `discovery/announce`

Sent immediately after connecting. Both sides exchange device metadata.

```json
{
  "type": "discovery",
  "action": "announce",
  "protocol_version": 1,
  "device_id": "a1b2c3d4e5f6a7b8",
  "device_name": "John's iPhone",
  "device_type": "phone",
  "os": "ios",
  "version": "17.4",
  "battery": 85,
  "ws_port": 9527,
  "wss_port": 9531,
  "apns_token": "optional-apns-token"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `device_id` | string | yes | Stable device identifier. The relay requires 1–64 lowercase hex characters; PROTOCOL.md describes it as the first 16 hex characters of the X25519 public key. |
| `device_name` | string | yes | Human-readable name. Bounded to 256 bytes at the desktop. |
| `device_type` | string | yes | `"phone"` \| `"tablet"` \| `"desktop"` |
| `os` | string | yes | `"ios"` \| `"android"` \| `"windows"` \| `"macos"` \| `"linux"` |
| `version` | string | yes | App or OS version string. Bounded to 32 bytes at the desktop. |
| `battery` | int | no | Battery percentage 0–100 (enforced at the Rust boundary by `validate_battery_pct`) |
| `ws_port` | int | no | Plain WS port (§2.1). 9527 on the desktop. |
| `wss_port` | int | no | TLS WSS port (§2.1). 9531 on the desktop. **This is the port to dial.** |
| `apns_token` | string | no | Apple Push Notification token |

There is **no** `name`, `address` or `port` field on an announce. A client that
receives `wss_port` MUST prefer it over `ws_port` and over any default it
carries compiled in. The same two keys are mirrored in the mDNS TXT
record (`ws_port`, `wss_port`).

#### `discovery/remove`

Notifies peers that a device is no longer available. It mutates peer state and
is therefore also behind the authentication gate.

```json
{
  "type": "discovery",
  "action": "remove",
  "device_id": "a1b2c3d4e5f6a7b8"
}
```

---

### 4.2 Pairing

`pairing` is exempt from the authentication gate — it *is* the authentication
step. `request` and `accept` both carry the one-time token; `local_auth` carries
the per-launch local capability (§4.2.4).

#### `pairing/request`

Mobile sends this after scanning the desktop's QR code.

```json
{
  "type": "pairing",
  "action": "request",
  "protocol_version": 1,
  "token": "ABCD-1234-EFGH-5678",
  "public_key": "hex-encoded-x25519-public-key",
  "device_info": {
    "name": "John's iPhone",
    "type": "phone",
    "os": "ios",
    "battery": 85
  }
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `token` | string | yes | One-time pairing token. Dashes, whitespace and case are normalised on both sides by the desktop before comparison. |
| `public_key` | string | yes | X25519 public key, hex-encoded |
| `device_info` | object | no | Device metadata for the desktop to store |

#### `pairing/accept`

Desktop replies with its public key after validating the token.

```json
{
  "type": "pairing",
  "action": "accept",
  "protocol_version": 1,
  "token": "ABCD-1234-EFGH-5678",
  "public_key": "hex-encoded-x25519-public-key",
  "device_info": {
    "name": "Conduit Desktop",
    "type": "desktop",
    "os": "windows"
  }
}
```

`PairingAccept` in `types.rs` does not declare a `token` field, but the desktop
**requires** one: `security::validate_pairing_message` refuses an `accept`
without a token, because `accept` registers a device and fires
`DeviceConnect` automation triggers — it is a full pairing, not a passive
reply.

The shared secret is derived via X25519 ECDH from:
- Sender's private key + Receiver's public key

Both sides compute the same 32-byte shared secret independently.

#### `pairing/revoke`

Desktop → client. The desktop revokes a paired device; the recipient must
disconnect and clear its stored credentials. There is **no inbound `revoke`
handler** on the desktop: a peer that sends one is answered
`unsupported_message` (§8).

```json
{
  "type": "pairing",
  "action": "revoke",
  "protocol_version": 1,
  "reason": "Device revoked by desktop"
}
```

#### `pairing/local_auth`

Loopback-only handshake by which the desktop's own Tauri webview obtains the
`local_desktop` identity. It replaces the old "any loopback peer is
auto-paired" rule: a connection is unpaired from the instant it is accepted, and
**being on loopback grants nothing**.

```json
{ "type": "pairing", "action": "local_auth", "token": "<43-char alphanumeric>" }
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `token` | string | yes | The per-launch local capability. 43 random alphanumeric characters, generated in-process, constant-time compared, bounded to 128 bytes by the validator. |

* The token is regenerated on every launch, so it cannot be replayed from a
  previous run, and it is handed to the webview over Tauri IPC
  (`get_local_ws_token`) rather than over the network.
* On success the connection is registered as `local_desktop`
  (`handlers::mark_local_desktop`). It has no key pair and therefore no shared
  secret, so it is deliberately absent from the `SyncEngine`; it is still
  eligible for broadcasts, which is what renders the desktop's own UI.
* A failed or absent token answers `invalid_local_capability` and grants
  nothing; such a connection may only `ping`/`pong`, pair, and receive unicast
  replies to its own requests.
* This message has **no** Rust struct in `types.rs` — it is dispatched from raw
  JSON by `server/mod.rs`. Treat the shape above as the contract.

---

### 4.3 Clipboard

#### `clipboard/sync`

Bidirectional clipboard content sharing.

```json
{
  "type": "clipboard",
  "action": "sync",
  "content": "Hello, world!",
  "mime": "text/plain",
  "source_device": "a1b2c3d4e5f6a7b8",
  "timestamp": 1700000000
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `content` | string | yes | Clipboard content (text or base64 for binary) |
| `mime` | string | yes | `"text/plain"` \| `"text/html"` \| `"image/png"` \| etc. |
| `source_device` | string | yes | Device ID of the sender |
| `timestamp` | int | yes | Unix timestamp (seconds) |

Gated by the desktop's `sync_clipboard` setting; refused with
`clipboard_sync_disabled` when off (§8).

#### `clipboard/request`

Request clipboard content from a peer.

```json
{
  "type": "clipboard",
  "action": "request",
  "mime": "text/plain"
}
```

---

### 4.4 Notification

#### `notification/post`

Mobile pushes a notification to the desktop.

```json
{
  "type": "notification",
  "action": "post",
  "id": "notif-uuid-1234",
  "device_id": "a1b2c3d4e5f6a7b8",
  "app": "com.apple.MobileSMS",
  "title": "John",
  "body": "Hey, are you free tonight?",
  "timestamp": 1700000000,
  "actions": ["reply", "dismiss"]
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | yes | Unique notification ID |
| `device_id` | string | yes | Source device ID |
| `app` | string | yes | App package/bundle identifier |
| `title` | string | yes | Notification title |
| `body` | string | yes | Notification body text (bounded to 50 000 bytes at the desktop) |
| `timestamp` | int | yes | Unix timestamp (seconds) |
| `actions` | string[] | no | Available actions |

`post` is behind the three notification settings, in order:
`sync_notifications` → `notifications_enabled` → the `notification_apps`
allowlist. Each failure has its own error code (§8).

#### `notification/dismiss`

```json
{
  "type": "notification",
  "action": "dismiss",
  "id": "notif-uuid-1234"
}
```

#### `notification/mark_read`

The user marked the notification as read. **Not** the same as `dismiss`: the
desktop relays it and mutates no local state.

```json
{
  "type": "notification",
  "action": "mark_read",
  "id": "notif-uuid-1234"
}
```

#### `notification/reply`

```json
{
  "type": "notification",
  "action": "reply",
  "id": "notif-uuid-1234",
  "text": "Sure, let's meet at 7!"
}
```

---

### 4.5 File Transfer

#### `file/request`

Initiates a file transfer.

```json
{
  "type": "file",
  "action": "request",
  "id": "file-uuid-5678",
  "name": "photo.jpg",
  "size": 2048576,
  "mime": "image/jpeg",
  "from": "a1b2c3d4e5f6a7b8",
  "to": "b2c3d4e5f6a7b8c9",
  "checksum": "sha256:abcdef..."
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `id` | string | yes | Unique transfer ID |
| `name` | string | yes | Filename |
| `size` | int | yes | Total file size in bytes (max 10 GB at the desktop) |
| `mime` | string | yes | MIME type |
| `from` | string | yes | Sender device ID |
| `to` | string | no | Target device ID (broadcast if absent) |
| `checksum` | string | no | SHA-256 checksum for integrity verification |

#### `file/accept`

```json
{
  "type": "file",
  "action": "accept",
  "id": "file-uuid-5678"
}
```

The desktop answers a request it cannot auto-accept with `file_accept_disabled`
rather than leaving the sender waiting for an ack (§8).

#### `file/chunk`

Base64-encoded chunk (JSON path, for smaller files).

```json
{
  "type": "file",
  "action": "chunk",
  "id": "file-uuid-5678",
  "index": 42,
  "data": "base64-encoded-chunk-data...",
  "total": 100
}
```

#### `file/progress`

```json
{
  "type": "file",
  "action": "progress",
  "id": "file-uuid-5678",
  "percent": 42
}
```

#### `file/complete`

```json
{
  "type": "file",
  "action": "complete",
  "id": "file-uuid-5678",
  "path": "/Users/john/Downloads/photo.jpg"
}
```

#### `file/cancel`

```json
{
  "type": "file",
  "action": "cancel",
  "id": "file-uuid-5678"
}
```

#### `file/resume`

Resume an interrupted transfer.

```json
{
  "type": "file",
  "action": "resume",
  "id": "file-uuid-5678",
  "name": "photo.jpg",
  "size": 2048576,
  "mime": "image/jpeg",
  "from": "a1b2c3d4e5f6a7b8",
  "checksum": "sha256:abcdef..."
}
```

#### `file/resume_ack`

```json
{
  "type": "file",
  "action": "resume_ack",
  "id": "file-uuid-5678",
  "chunks_loaded": 42
}
```

All `file/*` actions are gated by the desktop's `sync_files` setting
(`file_sync_disabled`, §8).

---

### 4.6 Audio

#### `audio/stream_start`

Mobile asks desktop to begin capturing system audio.

```json
{
  "type": "audio",
  "action": "stream_start"
}
```

#### `audio/stream_stop`

```json
{
  "type": "audio",
  "action": "stream_stop"
}
```

#### `audio/stream_started`

Desktop confirms capture is active.

```json
{
  "type": "audio",
  "action": "stream_started",
  "from": "desktop-device-id"
}
```

#### `audio/stream_data`

PCM-16 audio data, base64-encoded.

```json
{
  "type": "audio",
  "action": "stream_data",
  "data": "base64-encoded-pcm16...",
  "format": "pcm16",
  "sample_rate": 16000,
  "channels": 1,
  "from": "desktop-device-id"
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `data` | string | yes | Base64-encoded PCM-16 LE samples |
| `format` | string | yes | Always `"pcm16"` |
| `sample_rate` | int | yes | Sample rate in Hz |
| `channels` | int | yes | 1 = mono, 2 = stereo |

#### `audio/playback_start` / `audio/playback_stop`

```json
{ "type": "audio", "action": "playback_start" }
{ "type": "audio", "action": "playback_stop" }
```

#### `audio/playback_started` / `audio/playback_stopped`

Desktop → client acknowledgements. Both carry the desktop's device id in
`from`. The desktop emits `playback_stopped` in normal production operation, on
the `playback_stop` path; a client that has no arm for it will simply see an
unrecognised `(type, action)` pair.

```json
{ "type": "audio", "action": "playback_started", "from": "device-id" }
{ "type": "audio", "action": "playback_stopped",  "from": "device-id" }
```

#### `audio/playback_data`

```json
{
  "type": "audio",
  "action": "playback_data",
  "data": "base64-encoded-pcm16...",
  "format": "pcm16",
  "sample_rate": 16000,
  "channels": 1
}
```

Note that `playback_data` has **no** `from` field, unlike `stream_data`.

---

### 4.7 Screen Mirror

**Direction: the desktop is the viewer, the phone captures.** The desktop's own
loopback connection is the only client that ever asks for *another* device's
screen; a remote requester is asking about the machine it is talking to, and that
device captures locally. Direction is decided by the session table, not by a
field in the message: whoever sends `start` is the viewer, and the device named
in the `device_id` routing envelope is the capture side.

#### `screen_mirror/start`

```json
{
  "type": "screen_mirror",
  "action": "start",
  "quality": "medium",
  "fps": 15
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `quality` | string | no | `"low"` \| `"medium"` \| `"high"`. The desktop's own capture maps these to JPEG quality 30 / 60 / 85. |
| `fps` | int | no | Requested capture frame rate. The desktop defaults to 15 and **clamps to 1..=30**; the Android capture pipeline applies the same clamp. |

| `quality` | JPEG Quality | Approx Size |
|-----------|-------------|-------------|
| `"low"` | 30 | ~15 KB/frame |
| `"medium"` | 60 | ~40 KB/frame |
| `"high"` | 85 | ~80 KB/frame |

An optional `device_id` on the `start` frame names the capture side; it is
absent from the `ScreenMirrorStart` struct and is read from the raw message by
the desktop.

#### `screen_mirror/stop`

```json
{ "type": "screen_mirror", "action": "stop" }
```

#### `screen_mirror/frame`

```json
{
  "type": "screen_mirror",
  "action": "frame",
  "data": "base64-encoded-jpeg...",
  "format": "jpeg",
  "width": 1920,
  "height": 1080,
  "from_desktop": true
}
```

#### `screen_mirror/capture_stopped`

```json
{ "type": "screen_mirror", "action": "capture_stopped" }
```

#### `screen_mirror/touch`

```json
{
  "type": "screen_mirror",
  "action": "touch",
  "x": 0.5,
  "y": 0.25,
  "actionType": "tap"
}
```

The field is camelCase: **`actionType`**, never `action_type`. The Rust field is
`action_type` and `ScreenMirrorTouch` renames it, so the wire spelling is
fixed.

`x` and `y` are **relative** screen coordinates in `0.0..=1.0`, scaled by the
injecting side against its own display. Out-of-range values are clamped there
rather than rejected.

| `actionType` | Description |
|-------------|-------------|
| `"tap"` | Single click |
| `"double_tap"` | Double click |
| `"long_press"` | Press and hold 500 ms |
| `"right_click"` | Right-click |
| `"move"` | Hover / drag-update: move the pointer **without clicking**. A first-class wire value, not a client-side detail. |

`actionType` is validated at the Rust boundary (`validate_touch_action_type`):
any other string fails to deserialise. The field is optional and omitted when
absent.

#### `screen_mirror/key`

```json
{
  "type": "screen_mirror",
  "action": "key",
  "key": "Enter",
  "modifiers": ["control"]
}
```

`key` is a DOM `KeyboardEvent.key` value. Modifier names are the same set as
`remote_input/key` (§4.8.4).

#### `screen_mirror/scroll`

```json
{
  "type": "screen_mirror",
  "action": "scroll",
  "dx": 0.0,
  "dy": -3.0
}
```

---

### 4.8 Remote Input

Standalone mouse/keyboard control (outside screen mirror context).

#### `remote_input/move`

Relative mouse movement. This is the `action: "move"` touch gesture's
pointer-level counterpart to `screen_mirror/touch` with `actionType: "move"`.

```json
{ "type": "remote_input", "action": "move", "dx": 10.0, "dy": -5.0 }
```

`dx`/`dy` are relative deltas; the desktop clamps them to `±4096` and rounds to
whole pixels.

#### `remote_input/click`

```json
{ "type": "remote_input", "action": "click", "button": "left" }
```

| `button` | Description |
|---------|-------------|
| `"left"` | Left click |
| `"right"` | Right click |
| `"double_left"` | Double left click |
| `"middle"` | Middle click |

An unrecognised `button` is rejected; it never silently becomes a left click.

#### `remote_input/scroll`

```json
{ "type": "remote_input", "action": "scroll", "dx": 0.0, "dy": -3.0 }
```

#### `remote_input/key`

Keyboard input: **one complete key chord**, pressed and released as a unit.

```json
{
  "type": "remote_input",
  "action": "key",
  "key": "ArrowLeft",
  "modifiers": ["control", "shift"]
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `key` | string | yes | DOM `KeyboardEvent.key`: a single character, or a key name (`"Enter"`, `"ArrowLeft"`, `"F5"`, …). Capped at 32 characters by the receiver. |
| `modifiers` | string[] | no | See below. |

There is deliberately **no** `key_down` / `key_up` pair and **no** free-text
message: the receiver is a machine, not a text field, and a chord is the
smallest unit that can be injected safely. Text entry is a series of `key`
chords. A multi-character string that is not a recognised key name is
rejected, never typed.

**Modifier names are `control`, `shift`, `alt`, `meta` — not `ctrl`.** These are
the four values the schema `modifiers` enum defines. For interoperability the
desktop additionally *accepts* the `*Left` / `*Right` spellings that real
keyboards report (`controlLeft`, `metaRight`, `command`, `ctrl`, …), but a
client MUST emit the four canonical names.

The desktop also accepts `remote_input` `action: "start"` and `"stop"`. They
carry no state — the protocol has no `remote_input` session message — and are
ignored, because an older mobile build still sends them on every open/close.

---

### 4.9 Automation

#### `automation/rule`

Upserts a single automation rule.

```json
{
  "type": "automation",
  "action": "rule",
  "id": "rule-uuid-0001",
  "name": "Silence phone on connect",
  "trigger": {
    "type": "device_connect",
    "device_id": "optional-device-filter"
  },
  "rule_action": {
    "type": "set_phone_profile",
    "profile": "silent"
  },
  "enabled": true
}
```

**Trigger types:** `device_connect`, `device_disconnect`, `time`, `battery_level`, `wifi_change`, `app_open`, `audio_device_connect`, `audio_device_disconnect`

**Action types:** `send_notification`, `set_phone_profile`, `route_audio`, `run_shell_command`, `toggle_wifi`, `toggle_bluetooth`, `open_url`, `open_app`, `set_window_state`

`run_shell_command` is refused with `command_not_allowed` unless the command is
on the desktop's allowlist — on write *and* on `automation/triggered`, so a
rule persisted by an older build cannot fire later (§8).

#### `automation/delete`

```json
{
  "type": "automation",
  "action": "delete",
  "rule_id": "rule-uuid-0001"
}
```

`AutomationDelete` also accepts `id` as an alias for `rule_id` when
deserialising, and always emits `rule_id`.

#### `automation/sync`

Bulk sync of all rules. Carries the same shell-command gate as `rule`.

```json
{
  "type": "automation",
  "action": "sync",
  "rules": [ ... ],
  "full_sync": true
}
```

#### `automation/triggered`

Notification that a rule was triggered.

```json
{
  "type": "automation",
  "action": "triggered",
  "id": "rule-uuid-0001",
  "trigger_type": "device_connect",
  "device_id": "a1b2c3d4e5f6a7b8"
}
```

---

### 4.10 Call

```json
{
  "type": "call",
  "action": "incoming",
  "call_id": "call-uuid-9999",
  "to_device_id": "b2c3d4e5f6a7b8c9",
  "route": "webrtc"
}
```

Actions: `incoming`, `outgoing`, `missed`, `accept`, `decline`, `end`

`action` is an open string. The two clients do not agree on the verb set: the
phone emits `incoming` / `forward`, the desktop emits `answer` / `reject`.

Optional fields: `number` (E.164 far-end number), `name` (address-book display
name, empty string when unresolved), `device_id` (originating device).

The desktop relays every `(call, _)` frame verbatim and does not inspect it.

---

### 4.11 SMS

#### `sms/send`

Desktop requests mobile to send an SMS.

```json
{
  "type": "sms",
  "action": "send",
  "to": "+1-555-0123",
  "body": "Running late, be there in 10!"
}
```

#### `sms/sync`

Phone pushes its full SMS thread snapshot to its peers. Produced by
`SmsService.syncToDesktop()`, consumed by the desktop (`action: "sync"`) and by
other phones (`SmsService.handleSync`).

```json
{
  "type": "sms",
  "action": "sync",
  "threads": [
    {
      "thread_id": "t_1700000000000",
      "address": "+1-555-0123",
      "name": "Ada Lovelace",
      "snippet": "Running late",
      "unread_count": 1,
      "timestamp": 1700000000,
      "messages": [
        {
          "id": "in_1700000000000",
          "address": "+1-555-0123",
          "body": "Running late",
          "timestamp": 1700000000,
          "read": false,
          "is_outgoing": false
        }
      ]
    }
  ]
}
```

`SmsThread.name` may be JSON `null` when the address book has no match; the
field may also be omitted entirely. `SmsMessage.id` is prefixed `in_` for
received and `out_` for sent records. The phone generates `thread_id` as
`t_<millis>` when it first sees an address.

#### `sms/new`

A single newly received SMS, forwarded by the phone to its peers.

> **⚠️ Known divergence — unresolved.** The sender and the receivers
> currently use two different payloads for `sms/new`:
>
> | End | Shape | Source |
> |-----|-------|--------|
> | Sends | `{from, body, timestamp}` | `SmsService._startIncomingSmsListener` |
> | Reads | `{thread_id, message}` | desktop `useSms.ts`, `main.dart` `handleNewMessage` |
>
> Because the desktop relays `("sms", _)` blindly, a `sms/new` from one phone
> reaches every other peer, so **both** shapes occur on the wire. `schema.json`
> therefore encodes them as `anyOf` and accepts either, and every field of both
> shapes is optional in the Rust struct. Until one side is fixed, treat `SmsNew`
> as having two mutually exclusive variants.

```json
{ "type": "sms", "action": "new", "from": "+1-555-0123", "body": "hi", "timestamp": 1700000000 }
```

```json
{
  "type": "sms",
  "action": "new",
  "thread_id": "t_1700000000000",
  "message": {
    "id": "in_1700000000000",
    "address": "+1-555-0123",
    "body": "hi",
    "timestamp": 1700000000,
    "read": false,
    "is_outgoing": false
  }
}
```

#### `sms/sent`

Confirmation that the phone successfully sent an SMS, so the desktop can
reconcile its outbox. Emitted by `SmsService.sendSms()` on the native
`sendSms` success path. Carries the **same** sender/receiver divergence as
`sms/new`: sent as `{to, body, timestamp}`, read as `{thread_id, message}`.

```json
{ "type": "sms", "action": "sent", "to": "+1-555-0123", "body": "on my way", "timestamp": 1700000000 }
```

---

### 4.12 Status

#### `status/update`

Periodic status update (battery, wifi, etc.).

```json
{
  "type": "status",
  "action": "update",
  "battery": 72,
  "wifi_ssid": "HomeNetwork"
}
```

`battery` must be an integer in `0..=100`. The desktop closes the field set for
`status`, so `app_package`, `device_id` and `timestamp` are accepted but
anything else is refused with `invalid_message`.

---

### 4.13 Keep-Alive

#### `ping` / `pong`

```json
{ "type": "ping" }
{ "type": "pong" }
```

Sent every 25 seconds. If no message or pong is received within 60 seconds,
the connection is considered dead and closed. `ping` and `pong` are the only
non-`pairing` types exempt from the authentication gate, so an unpaired client
can probe reachability.

Both the relay and the desktop answer a `ping` with a `pong`.

---

### 4.14 Error

```json
{
  "type": "error",
  "code": "unsupported_protocol_version",
  "message": "Server supports protocol_version 1, got 2",
  "server_version": 1
}
```

`server_version` is the responder's `PROTOCOL_VERSION`; the desktop's
rate-limit error leaves it unset (`null`). The full code list, and which
component emits which, is §8.

---

### 4.15 Relay

#### `relay_auth`

First message a client sends to the relay. It must arrive within
`RELAY_AUTH_TIMEOUT_SECS` (default 10) of the socket opening.

```json
{
  "type": "relay_auth",
  "device_id": "a1b2c3d4e5f6a7b8",
  "relay_token": "shared-relay-token",
  "apns_token": "optional-apns-token"
}
```

`device_id` must be 1–64 lowercase hex characters; anything else is refused
with `invalid_device_id` and the connection is closed. The token is compared in
constant time.

#### `relay_auth_ok`

```json
{ "type": "relay_auth_ok" }
```

#### `relay_auth_rejected`

```json
{ "type": "relay_auth_rejected", "reason": "invalid_token" }
```

`reason` is `"invalid_token"` or `"missing_token"`; the value is validated at
the Rust boundary. This is a distinct message type, not an `error` frame, and
the connection is closed after it.

#### `relay_route`

The envelope for routing a message through the relay to one specific device.
This is the relay's whole routing API — the relay routes nothing else, and an
`encrypted` envelope sent unwrapped is refused with
`not_wrapped_in_relay_route` (§8).

```json
{
  "type": "relay_route",
  "from_device_id": "a1b2c3d4e5f6a7b8",
  "to_device_id": "b2c3d4e5f6a7b8c9",
  "payload": {
    "type": "clipboard",
    "action": "sync",
    "content": "..."
  },
  "timestamp": 1700000000000,
  "nonce": "6f1e...",
  "key_id": "v1",
  "hmac": "9a2c..."
}
```

| Field | Type | Required | Description |
|-------|------|----------|-------------|
| `type` | string | yes | Always `"relay_route"`. Signed. |
| `from_device_id` | string | **yes** | The sender's own device id. Signed, and compared against the identity established by `relay_auth` on *this* connection; a mismatch is `sender_mismatch`. |
| `to_device_id` | string | yes | Non-empty destination. Signed. |
| `payload` | object | yes | A complete JSON message to forward. Signed. |
| `timestamp` | int | **yes** | Unix timestamp in **milliseconds**. Signed. Accepted window is −5 s … +30 s. |
| `nonce` | string | **yes** | Unique-per-sender, non-empty. Signed. |
| `key_id` | string | **yes** | Which signing key produced `hmac`. Signed. |
| `hmac` | string | **yes** | Lowercase hex HMAC-SHA256 over the canonical signed-field subset. Not itself part of the signed input. |

The four optional fields are `#[serde(default)]`, so a minimal
`{type, to_device_id, payload}` message still *deserialises* — the relay
rejects it at verification time with `incomplete_relay_route` rather than
forwarding it. A client MUST NOT rely on that: it is a fail-closed detail, not
a compatibility mode.

##### 4.15.2.1 Canonical signing string

`hmac::SIGNED_FIELDS` is the authoritative, ordered list of what the HMAC
covers:

```
type, from_device_id, to_device_id, payload, timestamp, nonce, key_id
```

`hmac::canonical_signing_string(json)` builds a JSON object containing **only**
those fields, **in that order**, omitting any that are absent, and serialises it
compactly; that string is the HMAC message. A field not in this list is
*unauthenticated* — adding a security-relevant field to a signed message
without adding it here is the exact class of bug that once let `relay_route`
carry an unauthenticated sender, so any new signed field MUST be added to
`SIGNED_FIELDS` **and** to the client signers.

The signer and the verifier MUST both go through the same function
(`RelayRoute::signed` / `signed_with` / `sign_with` on the sender side,
`SigningKeyring::verify` on the receiver's) rather than hand-serialising the
subset, so the bytes hashed are exactly the bytes the verifier reconstructs.

```rust
// Sign
let route = RelayRoute::signed_with(key_id, &signing_key, from, to, payload, ts, nonce);
// Verify (relay side)
let accepted = signing_keys.verify(&json);
```

##### 4.15.2.2 Key derivation

The signing key is **not** the relay token and **not** the master secret.

* Default: `derive_signing_key(HMAC_SECRET)`, a labelled PRF-based KDF —
  `HMAC-SHA256(hmac_secret, "conduit-protocol/v1/derive:" + "conduit-relay/v1/message-signing-key")`.
* Override: `RELAY_SIGNING_KEY` / `RELAY_SIGNING_KEY_ID`.

The label is mandatory (`derive_key` panics on an empty one) and is part of the
KDF input, so a derived key can never equal the master secret nor any other
derived key. `RELAY_TOKEN` is a *bearer credential* held by every authenticated
client; before the separation it doubled as the route MAC key, which meant any
client could forge a route claiming to be any other device. Clients MUST
therefore never sign with the token.

##### 4.15.2.3 Rotation window

`SigningKeyring` holds a `current` key plus, during rotation, exactly one
`previous` key.

* Signing MUST use `current`. The default `key_id` is `v1`
  (`hmac::DEFAULT_KEY_ID`).
* Verification accepts either key, but only when the message's `key_id` names
  it — the overlap window is explicit and auditable on the wire, not a
  "try every key we know" oracle.
* `key_id` is **mandatory**. A message without it is rejected
  (`SigningKeyring::verify` returns `None`), because a missing id would make
  the rotation state invisible.
* Because `key_id` is itself in `SIGNED_FIELDS`, an attacker cannot rewrite a
  message onto the retiring key.
* The relay opens the window with `RELAY_SIGNING_KEY_PREVIOUS` +
  `RELAY_SIGNING_KEY_PREVIOUS_ID`, and closes it by unsetting them.

##### 4.15.2.4 Replay protection

`timestamp` + `nonce` drive rejection, scoped **per authenticated device id**:

* The window is `now - timestamp ∈ [-5 s, +30 s]`.
* An empty `nonce` is refused.
* `NonceCache` keeps an insertion-ordered queue per device. Entries older than
  60 s are pruned; when a device exceeds its `MAX_NONCES_PER_DEVICE` (4096)
  quota the cache evicts **oldest-first** — it never `clear()`s — and a
  process-wide ceiling of `MAX_NONCES` (10 000) is applied the same way. One
  high-volume client can therefore only ever evict its own oldest nonces.
* The scope is always the *authenticated connection identity*, never a value
  taken from the message body, so a client cannot burn another client's replay
  protection by naming them.
* The cache is persisted to `RELAY_NONCE_FILE` and reloaded, so a reconnect
  cannot replay a nonce accepted before the restart. A corrupt cache file is
  treated as empty and logged rather than aborting startup.
* A refusal is answered with `replay_detected`.

---

### 4.16 Encrypted Envelope

When a message type is wrapped (§6.4 lists which ones are), the plaintext is
encrypted and the result carried in this envelope:

```json
{
  "type": "encrypted",
  "source_device": "a1b2c3d4e5f6a7b8",
  "protocol_version": 1,
  "nonce": "hex-encoded-24-byte-nonce",
  "hmac": "hex-encoded-hmac-sha256",
  "data": "hex-encoded-ciphertext"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `nonce` | string | 24-byte XChaCha20 nonce, hex-encoded |
| `hmac` | string | HMAC-SHA256 over `data`, hex-encoded |
| `data` | string | XChaCha20-Poly1305 ciphertext, hex-encoded |
| `source_device` | string | Sender device ID (for relay routing) |
| `protocol_version` | int | Set by `EncryptedEnvelope::new`; the desktop writes `1` by default. |

The decrypted plaintext is a complete JSON message (any type from sections
4.1–4.14). The envelope is **not** end-to-end: the desktop hub decrypts it and
dispatches the inner message, so the hub is a party to the plaintext rather than
an opaque forwarder. See ADR-0007.

**Routing an `encrypted` envelope through the relay is not optional.** The
relay cannot attribute an `encrypted` frame — `source_device` is
unauthenticated, so blind forwarding would let any client impersonate any
sender. An `encrypted` message that arrives unwrapped is refused with
`not_wrapped_in_relay_route`. The correct form is:

```json
{
  "type": "relay_route",
  "from_device_id": "...",
  "to_device_id": "...",
  "payload": { "type": "encrypted", "source_device": "...", "nonce": "...", "hmac": "...", "data": "..." },
  "timestamp": 1700000000000,
  "nonce": "...",
  "key_id": "v1",
  "hmac": "..."
}
```

---

## 5. Binary Frame Formats

### 5.1 Relay Binary Frame

**Version 2 (`0x02`) is the only version the relay accepts.**
`BINARY_FRAME_VERSION = 0x02`, `BINARY_HEADER_LEN = 53`.

```
Byte  0:              version (0x02)
Bytes 1..=16:         target device ID, ASCII, zero-padded to 16 bytes
Bytes 17..=20:        sequence number, u32 big-endian
                      (strictly increasing per sender connection)
Bytes 21..=52:        HMAC-SHA256 tag (32 raw bytes) — see below
Bytes 53..:           payload
```

```
Offset  Length  Description
0       1       Version byte (0x02)
1       16      Target device ID (ASCII, zero-padded, NUL-trimmed on read)
17      4       Sequence number (u32 big-endian)
21      32      HMAC-SHA256 tag (raw bytes)
53      N       Payload bytes
```

Minimum frame size: **53 bytes** (empty payload). The payload begins
immediately after the tag.

#### 5.1.1 MAC input

The tag is **not** a MAC over the frame alone — the sender's authenticated
device id is bound in, so a captured frame cannot be re-presented by a
different authenticated client:

```
tag = HMAC-SHA256(
        relay_signing_key,
        hex( from_device_id_utf8 || 0x1F || frame[0..21] || frame[53..] ) )
```

That is: the authenticated `from_device_id`, an ASCII unit separator (`0x1F`),
then the header (version, target id, sequence) concatenated with the payload.
`from_device_id` is taken from the *authenticated* connection identity, never
from the frame — the 16-byte header names only the recipient.

A **v1** frame (`0x01`, 17 bytes: version + target id + payload) is **retired
and rejected** with `binary_version_unsupported`. v1 carried no integrity
protection at all, so any authenticated client could flood any other device
with arbitrary bytes. A frame shorter than 53 bytes is `binary_frame_too_short`.

#### 5.1.2 Sequence and replay

The sequence number must be **strictly greater** than the previous accepted one
on that connection; anything else is `binary_replay_detected`. The check is per
connection, so a reconnect resets the floor.

#### 5.1.3 Key selection

Unlike `relay_route`, a binary frame has no room for a `key_id` in its fixed
header, so the relay tries every key in the ring (`current`, then `previous`).
This is safe because each candidate is compared in constant time and the sender
id is bound into the MAC input.

**Mobile and desktop both send:**

```
[0x02][target_id_16][seq_u32_be][hmac_tag_32][payload]
```

### 5.2 LAN Direct Binary Chunk (file transfer)

Used for encrypted file transfer chunks over direct LAN WebSocket connections.
This format has no version byte; it is not affected by the v1 → v2 change above
and is never seen by the relay.

```
Offset  Length       Description
0       24           XChaCha20-Poly1305 nonce
24      4            Metadata length (u32, little-endian)
28      N            JSON metadata (BinaryFileMetadata)
28+N    M            XChaCha20-Poly1305 ciphertext (includes 16-byte MAC tag)
```

**Metadata JSON:**

```json
{
  "id": "file-uuid-5678",
  "index": 42,
  "total": 100
}
```

Minimum frame size: 28 bytes (nonce + length field, empty metadata + ciphertext).

---

## 6. Encryption Scheme

### 6.1 Key Exchange

1. Each device generates an **X25519** key pair on first launch.
2. During pairing, both sides exchange public keys (hex-encoded, 64 chars).
3. Each side computes the **shared secret** via X25519 ECDH.
4. The 32-byte shared secret is used directly as the encryption key.

### 6.2 Encryption

- Algorithm: **XChaCha20-Poly1305** (extended nonce variant)
- Nonce: 24 bytes, randomly generated per message
- Output: ciphertext + 16-byte Poly1305 MAC tag (appended to ciphertext)

### 6.3 HMAC Verification

Before decryption, each side verifies an HMAC-SHA256 over the hex-encoded
ciphertext using the shared secret as the key. This prevents ciphertext
tampering without needing to attempt decryption.

```
hmac = HMAC-SHA256(shared_secret, hex(ciphertext))
```

This is the `hmac` field **inside** the `encrypted` envelope, and is unrelated
to the `hmac` field of a `relay_route` (§4.15.2), which is keyed by the relay's
message-signing key instead of the peer shared secret.

### 6.4 What Is Encrypted

The envelope is opt-in per message type, and in this build almost nothing opts
in. `seal_for_peer` (`apps/desktop/src-tauri/src/server/mod.rs:321-361`) wraps
what the *hub* fans out, and it can only select a per-peer secret when the
frame names a single recipient. Types the frontend sends straight through the
hub, and types with no destination device id, go out unwrapped.

| Message Type | Wrapped by the desktop hub? |
|-------------|-----------|
| `pairing/*` | No — establishes the secret |
| `relay_auth`, `relay_auth_ok`, `relay_auth_rejected` | No |
| `call` (answer, reject, forward) | Yes, via `send_encrypted_message` |
| `sms`, `file`, `notification`, `clipboard`, `screen_mirror`, `remote_input`, `audio` | **No** — broadcast, or forwarded verbatim |

This is hop encryption in any case, not end-to-end: the hub terminates the
envelope either way. The desktop keeps the authoritative per-type table, with
the deciding code cited on each row, in `MESSAGE_PROTECTION`
(`apps/desktop/src/hooks/useEncryption.ts:58`).

A failed envelope HMAC, a bad nonce/data hex string, or a decryption failure
causes the desktop to drop the frame with a log line and **no** `error` reply —
so `decryption_failed` is a documented code that is not currently emitted.

### 6.5 Binary Chunk Encryption

Binary file chunks use the same XChaCha20-Poly1305 scheme but transmit
the nonce as raw 24 bytes (not hex-encoded) at the start of the frame.
The ciphertext includes the 16-byte Poly1305 MAC tag at the end.

---

## 7. Versioning Strategy

| Change Type | Action |
|-------------|--------|
| New optional field | Bump minor, no version change |
| New message type | Bump minor, no version change |
| Changed required field semantics | Bump `protocol_version` |
| Removed field | Bump `protocol_version` |
| Changed encryption algorithm | Bump `protocol_version` + new key exchange |
| Relay binary frame layout | Bump `BINARY_FRAME_VERSION`; the relay rejects the old byte |
| `SIGNED_FIELDS` membership or order | Bump `protocol_version` — it changes what a valid signature is |

Note that the two breaking changes in §9 did **not** bump `PROTOCOL_VERSION`.
`PROTOCOL_VERSION` covers the JSON message shapes; the binary frame version and
the route-signing scheme are versioned independently. That is a real gap — a
v1 client and a v2 client both claim `protocol_version: 1` — and it is why §9
exists.

---

## 8. Error Codes

A refusal is reported as an `error` message with a stable `code`. Codes are
grouped by the component that emits them; a client MUST treat an unrecognised
code as a generic failure rather than a protocol violation.

### 8.1 Emitted by the desktop hub (`apps/desktop/src-tauri`)

| Code | Condition |
|------|-----------|
| `unsupported_protocol_version` | `protocol_version` greater than 1 |
| `invalid_message` | Failed `security::validate_message`: unknown `type`, bad field bounds, or a field the handler does not read |
| `not_authenticated` | Any non-`pairing`/`ping`/`pong` message from a connection that is not a trusted peer — including `discovery` |
| `rate_limited` | Per-message-type rate limit exceeded for this connection |
| `command_not_allowed` | `automation/rule`, `automation/sync` or `automation/triggered` would run a shell command that is not on the allowlist |
| `unknown_action` | Unrecognised `automation` action |
| `unsupported_message` | `type` is protocol-valid but has no handler on the desktop — includes `tv`, `watch`, and an inbound `pairing/revoke` |
| `delivery_failed` | The desktop could not encrypt an outbound frame for the intended device; substituted for the frame it refused to send in the clear |
| `invalid_pairing_token` | `pairing/request` or `pairing/accept` presented an unknown or expired one-time token |
| `invalid_local_capability` | `pairing/local_auth` presented the wrong per-launch capability |
| `file_accept_disabled` | The desktop cannot accept the transfer (destination unavailable or auto-accept off) |
| `notification_sync_disabled` | `sync_notifications` is off |
| `notifications_disabled` | `notifications_enabled` is off |
| `notification_app_not_allowed` | The posting app is not in the `notification_apps` allowlist (an empty list mirrors nothing) |
| `clipboard_sync_disabled` | `sync_clipboard` is off |
| `file_sync_disabled` | `sync_files` is off — applies to **every** `file/*` action, including `accept` |

The five settings codes are evaluated once for the whole message, in the order
listed above, before any handler runs.

### 8.2 Emitted by the relay (`services/relay`)

| Code | Condition |
|------|-----------|
| `malformed_json` | Inbound text frame was not valid JSON |
| `invalid_device_id` | `relay_auth.device_id` was not 1–64 lowercase hex characters |
| `malformed_relay_route` | The frame did not deserialise into `RelayRoute` |
| `incomplete_relay_route` | `from_device_id`, `timestamp` or a non-empty `nonce` was missing |
| `sender_mismatch` | The signed `from_device_id` is not the identity `relay_auth` established on this connection |
| `hmac_invalid` | The signature did not verify under any accepted key — usually a wrong `key_id`, or signing with the relay token instead of the message-signing key |
| `missing_target` | `to_device_id` was empty |
| `replay_detected` | Nonce already used, or the timestamp was outside −5 s … +30 s |
| `not_wrapped_in_relay_route` | An `encrypted` envelope arrived unwrapped |
| `unknown_message_type` | Any `type` the relay does not route |
| `binary_frame_too_short` | Binary frame shorter than 53 bytes |
| `binary_version_unsupported` | Binary frame version was not `0x02` |
| `binary_target_id_invalid` | The 16-byte target id is not valid UTF-8 |
| `binary_target_id_empty` | The target id was empty after NUL-trimming |
| `binary_replay_detected` | Sequence number not strictly greater than the last accepted one |
| `binary_hmac_invalid` | The frame tag did not verify |

Auth failures use a different message type entirely: `relay_auth_rejected` with
`reason: "invalid_token" | "missing_token"`.

### 8.3 Documented but not currently emitted

| Code | Status |
|------|--------|
| `message_too_large` | The relay's 1 MiB check logs and closes the connection; it does not answer. The desktop's 50 MiB tungstenite limit surfaces as a read error, which is also not answered. |
| `decryption_failed` | The desktop drops a bad `encrypted` envelope silently. |
| `invalid_public_key` | A malformed `public_key` fails `validate_message` and is reported as `invalid_message` instead. |

Rate limiting on the relay is also silent: an over-limit frame is dropped and
counted in `conduit_relay_messages_dropped_total`, and an over-limit IP has its
connection closed, without an `error` frame. `rate_limited` is a **desktop**
code only.

---

## 9. Compatibility

### 9.1 This release is a breaking change for deployed clients

Two wire formats changed incompatibly, and the relay **rejects the old forms
outright** — there is no negotiation, no version field on either construct, and
no shim.

| Change | Old | New | Relay's answer to the old form |
|--------|-----|-----|--------------------------------|
| `relay_route` signing | Unsigned, or HMAC keyed with the shared `RELAY_TOKEN`, over a 5-field subset (`type`, `to_device_id`, `payload`, `timestamp`, `nonce`) | HMAC keyed with the domain-separated message-signing key, over the 7-field `SIGNED_FIELDS` subset, with a mandatory `key_id` and a signed `from_device_id` checked against the connection identity | `incomplete_relay_route`, `hmac_invalid`, or `sender_mismatch` |
| Relay binary frame | 17 bytes: `0x01` + 16-byte target id + payload. No integrity protection. | 53 bytes: `0x02` + 16-byte target id + u32 sequence + 32-byte HMAC tag + payload | `binary_version_unsupported` |

A client that predates these changes cannot route a single message through a
current relay, and cannot send a single binary frame it will accept. Deployed
clients must be upgraded; there is no server-side compatibility path, and none
is intended — the old forms are the vulnerability the change fixed.

### 9.2 What a client must implement

1. **Sign every `relay_route`.** Include `from_device_id` (your own
   authenticated device id), `timestamp` in **milliseconds**, a non-empty
   cryptographically random `nonce`, and `key_id`. Compute the HMAC over the
   canonical signed-field subset (§4.15.2.1) with the relay's
   message-signing key — **never** with `RELAY_TOKEN`, and never with the
   master secret. Prefer `RelayRoute::signed_with` / `sign_with` so the bytes
   match the verifier's.
2. **Wrap `encrypted` envelopes in a signed `relay_route`.** An unwrapped
   `encrypted` message is refused.
3. **Rotate keys deliberately.** Sign with `current`; set `key_id` explicitly so
   the relay's rotation window is usable during a re-key. Never omit `key_id`.
4. **Build 53-byte v2 binary frames**, with a strictly increasing per-connection
   sequence number and a tag computed over
   `hex(from_device_id || 0x1F || frame[0..21] || payload)`, where
   `from_device_id` is the authenticated identity.
5. **Keep routed frames under 1 MiB** (§2.5) or move to a binary path.
6. **Pair before anything except `pairing`, `ping` and `pong`** — `discovery`
   is now gated, and `pairing/local_auth` is mandatory for the desktop's own
   loopback webview.

### 9.3 Known gaps

Documented, not fixed here:

* **The desktop's own relay client does not yet sign conformantly.**
  `apps/desktop/src-tauri/src/server/mod.rs` builds a `relay_route` in
  `send_to` with five fields, omitting `from_device_id` and `key_id`, and MACs
  it with `RELAY_TOKEN`. Such a frame is rejected by a current relay with
  `incomplete_relay_route`. Until that call site is moved to
  `RelayRoute::signed_with`, desktop-originated relay routing is broken and
  `send_to`'s fallback path is dead against a compliant relay.
* **`protocol_version` is not enforced by the relay** (§1.1), so the §9.1
  changes are not detectable through version negotiation.
* **`sms/new` and `sms/sent` still have two incompatible shapes** (§4.11).
* **Mobile-side drift could not be verified** from this crate. The Dart client
  was read only for the port constants that `types.rs` asserts against; whether
  it has adopted the signed `relay_route`, the `remote_input/key` message, the
  `fps` field, or `actionType: "move"` is **[unverified]**.
