# Conduit — Architecture

This document describes how Conduit is put together: which component owns what,
how the pieces talk, what the two LAN transports are for, how a connection
becomes trusted, and where each feature's data actually lives.

It is a description of the code as it is today, including the places where a
feature is only partly wired. Where a claim could not be verified from source it
is marked **[unverified]** rather than asserted.

Related documents, which this one does not duplicate:

| Document | Owns |
|---|---|
| [`README.md`](../README.md) | What Conduit is, install/run, feature tour |
| [`DEVELOPMENT.md`](DEVELOPMENT.md) | Build, run, lint, codegen commands |
| [`TESTING.md`](TESTING.md) | Test strategy and suites |
| [`CHANGELOG.md`](../CHANGELOG.md) | Release history and known limitations |
| [`CONTRIBUTING.md`](../CONTRIBUTING.md) | How to contribute |
| [`decisions/`](decisions/) | Architecture decision records |
| [`relay-tls.md`](relay-tls.md) | Relay TLS bring-up, renewal, pinning runbook |
| [`../SECURITY.md`](../SECURITY.md) | Security model and how to report a vulnerability |
| [`../packages/protocol/PROTOCOL.md`](../packages/protocol/PROTOCOL.md) | The wire format, message by message |
| [`../apps/desktop/DESIGN.md`](../apps/desktop/DESIGN.md) | Desktop UI/UX design system |

---

## 1. Component map

Conduit is a Cargo virtual workspace with three members
(`Cargo.toml:1-7`) plus two frontends that are not workspace members.

```
                    ┌──────────────────────────────────────┐
                    │  packages/protocol  (conduit-protocol)│
                    │  types.rs, lib.rs (hmac module)      │
                    │  schema.json, PROTOCOL.md            │
                    └───────────────┬──────────────────────┘
                                    │ path dependency
              ┌─────────────────────┼─────────────────────┐
              │                     │                     │
   ┌──────────▼──────────┐ ┌────────▼─────────┐ ┌─────────▼──────────┐
   │ apps/desktop/       │ │ services/relay   │ │ apps/mobile        │
   │   src-tauri         │ │   (relay)        │ │   (Flutter)        │
   │   (conduit)         │ │                 │ │                    │
   │  + src/ React+TS    │ │  message router  │ │  Dart services     │
   └─────────────────────┘ └──────────────────┘ └────────────────────┘
```

| Component | Crate / package | Owns | Depends on |
|---|---|---|---|
| `packages/protocol` | `conduit-protocol` | Every wire message struct, the port constants, the `hmac` module (HMAC-SHA256, domain separation, replay cache, nonce persistence, binary frame constants), the JSON Schema, `PROTOCOL.md` | `serde`, `serde_json`, `hmac`, `sha2`, `hex`, `subtle`, `log` |
| `apps/desktop/src-tauri` | `conduit` | The hub: WebSocket server (two listeners), mDNS advertisement, pairing, the pairing registry, SQLCipher storage, file-transfer engine, automation engine, audio capture, screen capture, input injection, all Tauri commands | `conduit-protocol` |
| `services/relay` | `relay` | Optional self-hosted router: bearer auth, signed `relay_route` verification, v2 binary frame verification, replay cache persistence, TLS, health/metrics/pin HTTP endpoints | `conduit-protocol` |
| `apps/desktop/src` | not a workspace member | React/TypeScript UI | the desktop backend over Tauri IPC **and** a loopback WebSocket |
| `apps/mobile` | not a workspace member | Flutter app: platform services (notification listeners, SMS, calls, MediaProjection capture, accessibility input injection), secure storage, sqflite database | the wire format, transcribed into `lib/models/protocol.dart` by codegen |

### Dependency direction

`conduit-protocol` depends on nothing in the workspace. Both `conduit` and
`relay` depend on it, and they do not depend on each other. There is no
shared mutable state between the desktop hub and the relay; the only coupling
is the wire format, which is what the codegen pipeline in §8 exists to keep
honest.

The mobile app is not a Rust crate and therefore does not import
`conduit-protocol`; it gets an equivalent view of the schema through
`apps/mobile/lib/models/protocol.dart`, produced by `scripts/generate_dart.js`
(§8).

### The unusual part: two channels inside the desktop

The desktop frontend talks to its own backend over **both** Tauri IPC and a
loopback WebSocket, and it is worth being precise about why.

- **Tauri IPC** (`invoke(...)`, `@tauri-apps/api/core`) is the *command* path.
  It is synchronous, request/response, authenticated by process identity (only
  the app's own webview can call a registered command), and it is the path that
  can touch the filesystem, the database and the OS. The complete command list
  is the `tauri::generate_handler!` array at
  `apps/desktop/src-tauri/src/main.rs:219-263`.
- **The loopback WebSocket** (`ws://127.0.0.1:9527`, set in
  `apps/desktop/src/config.ts:20`) is the *event* path. It is the same socket
  the LAN peers use, so the desktop's own UI is served by exactly the same hub
  code — the same broadcast filter, the same per-peer encryption decision, the
  same dispatch table — that serves a phone. Anything the UI can do over this
  socket, a paired phone can do, and vice versa.

The consequence is the whole reason the identity model in §5 exists: because the
webview shares the socket with untrusted peers, being on loopback grants it
nothing. It has to authenticate like everyone else.

`apps/desktop/src/hooks/useWebSocket.tsx` uses the socket for inbound events
and, for a handful of locally-initiated actions, the Tauri command instead —
deliberately, so a state change is persisted before it is forwarded and the
same act is never applied twice. `useWebSocket.tsx:218-265` documents the rule
for `dismiss_notification`, `reply_notification` and `sync_clipboard`.

---

## 2. The two transports

The desktop binds **two** WebSocket listeners in the same accept path
(`apps/desktop/src-tauri/src/server/mod.rs:109-223`).

| | Plaintext | TLS |
|---|---|---|
| Port | `9527` (`LAN_WS_PORT`) | `9531` (`LAN_WSS_PORT`) |
| Constant | `packages/protocol/src/types.rs:47` | `packages/protocol/src/types.rs:53` |
| Desktop alias | `main.rs:23` (`WS_PORT`) | `main.rs:23` (`WSS_PORT`) |
| Dart mirror | `kLanWsPort` | `kLanWssPort` |
| Bind address | `0.0.0.0:9527` (`main.rs:30`) | `0.0.0.0:9531` (`server/mod.rs:177`) |
| Certificate | none | self-signed, TLS 1.3 only, SANs include every non-loopback IPv4 at generation time (`tls.rs:68-128`) |
| Accept loop | `server/mod.rs:229-261` | `server/mod.rs:263-300` |
| Intended client | the desktop's own webview, diagnostics | the mobile app, for every LAN peer |

Both numbers are defined exactly once in the Rust workspace
(`packages/protocol/src/types.rs`), re-exported by the desktop, and mirrored in
Dart. Four tests pin the three representations plus the table in `PROTOCOL.md`
against each other, so a drift in any of them fails the build:
`types.rs:1608-1653`, `types.rs:1664-1676`, and
`apps/desktop/src-tauri/src/main.rs:461-507`.

### Why both exist

The plaintext listener is what the desktop's own webview connects to, and it is
where plaintext traffic is acceptable because loopback traffic never leaves the
machine. The TLS listener is what a phone dials, because the mobile client's
certificate-pin trust bootstrap only runs on the `wss://` code path
(`apps/mobile/lib/services/websocket_service.dart:291-316`).

### The hard rules

1. **A LAN client must never dial `wss://` on 9527.** A `wss://` handshake
   against a plaintext socket throws. Because the pin bootstrap lives on the
   `wss://` path only, the pin is never captured and every later connection
   fails closed. This was the mobile pairing bug; `types.rs:19-42` records it
   and `PROTOCOL.md` §2.2 states the rule.
2. **A LAN client must never silently downgrade to `ws://`.** The
   `preferLanWss` upgrade in `websocket_service.dart:281-288` replaces the
   scheme and port, and `pairing_service.dart:53-70` builds `wss://` URLs with
   no plaintext fallback at all. The mDNS reader
   (`discovery_service.dart:112-127`) explicitly falls back to the TLS
   constant and never to the plaintext port.
3. **A peer needing TLS must read `wss_port`, not the mDNS SRV port.** The SRV
   record carries 9527; the TLS port is in the TXT record. See §4.

---

## 3. Connection lifecycle

```
  mDNS browse / QR scan                TCP 9527 or wss 9531
            │                                     │
            ▼                                     ▼
   device_id, device_type,              accept_async_with_config
   version, ws_port, wss_port                    │
   (TXT: device_id, device_type,                 │ unpaired from this instant
    version, ws_port, wss_port)                  │ (server/mod.rs:377-389)
            │                                     ▼
            └────────────► pairing/request  or  pairing/accept
                              (one-time token, X25519 public key)
                                          │
                                          ▼
                          authorize_pairing (handlers/pairing.rs:26-58)
                          token valid? → consume → derive shared secret
                                          │
                                          ▼
                    mark_paired → SyncEngine::add_client → devices row
                          (+ DeviceConnect automation triggers)
                                          │
                                          ▼
                            is_trusted_peer == true
                            authenticated session

  Desktop webview, separately:
    invoke('get_local_ws_token')  →  43-char per-launch capability
        (security.rs:871-1000, commands/settings.rs:250-252)
                                          │
                                          ▼
                        pairing / action: "local_auth"
                                          │
                                          ▼
                          mark_local_desktop → identity `local_desktop`
```

### The `local_auth` handshake is mandatory, and loopback grants nothing

An accepted connection is **unpaired from the instant the TCP handshake
completes**. Nothing about the connection grants an identity: not loopback, not
having a socket (`server/mod.rs:377-389`). Until a peer authenticates it may
only `ping`, `pong`, pair, and receive unicast replies to its own
`pairing/request` and `discovery/announce` frames
(`handlers/mod.rs:57-61`).

The desktop's own webview authenticates with a **per-launch capability**:

- Generated on first use by `security::local_capability()` — 43 characters from
  `rand::distr::Alphanumeric` (a 62-symbol alphabet, so ≈256 bits of entropy)
  drawn from `rand::rng()` (ChaCha12 CSPRNG) (`security.rs:871-899`). It lives
  in a `OnceLock`, so it exists exactly once per process and is regenerated
  every launch. This resists guessing only; a same-user process can read the
  token file directly, which ADR-0005 records as out of scope.
- Retrieved over Tauri IPC via the `get_local_ws_token` command
  (`commands/settings.rs:250-252`), which a remote peer cannot reach.
- Presented as `{"type": "pairing", "action": "local_auth", "token": "..."}`,
  verified in constant time (`security.rs:905-914`), and on success the
  connection is registered as `local_desktop`
  (`handlers/pairing.rs:290-309`, `handlers/mod.rs:64-70`).
- A copy is written to `<app data>/conduit/local_ws_token` with mode `0600` on
  POSIX (`security.rs:965-1000`). That file is a diagnostic convenience; the
  in-process copy is authoritative.

> **Gap — the frontend does not perform the handshake yet.** The Rust side is
> complete and tested (`server/mod.rs:2777-2820`, `handlers/pairing.rs:706-758`),
> but `apps/desktop/src/hooks/useWebSocket.tsx` never calls
> `get_local_ws_token` and never sends a `local_auth` frame. As the tree
> stands, the desktop's own webview therefore holds **no** identity, receives no
> broadcasts, and every message it sends is rejected with
> `not_authenticated`. This is a functional break, not a security hole — the
> failure mode is closed — but it is the single most important wiring gap in the
> project. `commands/settings.rs:244-248` refers to it as "the two-line
> `useWebSocket.tsx` change".

### Protocol version

`PROTOCOL_VERSION = 1` (`types.rs:17`). The desktop hub rejects an inbound
frame whose `protocol_version` is greater and answers
`unsupported_protocol_version` (`server/mod.rs:564-586`). The relay and the
mobile app do not enforce it; `PROTOCOL.md` §1.1 records this asymmetry.

---

## 4. Identity model

### The pairing registry

`WsContext` holds two maps (`handlers/mod.rs:21-23`):

| Map | Key | Value | Meaning |
|---|---|---|---|
| `clients` | per-connection UUID | `broadcast::Sender<String>` | every accepted socket, paired or not |
| `ws_to_device_id` | per-connection UUID | stable device id, or `local_desktop` | **the** identity store |

`SyncEngine` (`sync.rs`) is a third map keyed by **stable device id**, holding
the `ConnectedClient` including the derived `shared_secret`. It is not a trust
store: it is the shared-secret cache and the connection-status display.

`ws_to_device_id` is the single source of truth for "is this connection
trusted?" (`handlers/mod.rs:47-61`). It is written in exactly three places:
`mark_local_desktop`, `mark_paired`, and the disconnect/revocation cleanup. A
second "paired" set would be a second source of truth that could drift from the
one the auth gate reads, which is exactly how the plaintext-leak defect
arose; see `SECURITY.md` §3.2.

### Two predicates, both needed

| Predicate | Definition | Used by | Why it is weaker/stronger |
|---|---|---|---|
| `has_identity` | `ws_to_device_id` contains `client_id` (`handlers/mod.rs:88-90`) | broadcast eligibility (`server/mod.rs:920-934`, `handlers/mod.rs:130-138`) | Deliberately the weak one. `local_desktop` has no key pair and therefore no shared secret, but it must still receive broadcasts — that is what renders the notification list. |
| `is_trusted_peer` | identity entry **and** (stable id is `local_desktop` **or** `SyncEngine` holds a non-empty `shared_secret` for it) (`handlers/mod.rs:101-114`) | the authentication gate for every non-handshake message type (`server/mod.rs:672-704`) | The extra requirement is redundant for the two writers of the registry, but it means an entry on its own is never sufficient: a half-finished pairing, or a device whose `ConnectedClient` has been reaped, is treated as untrusted. |

Both are tested in both directions in
`handlers/mod.rs:436-484`, and `server/mod.rs:2777-2820` covers the
end-to-end `local_auth` path through the dispatcher.

### The auth gate

`server/mod.rs:684-704`. Every message whose `(type, action)` is not one of
`pairing/*`, `ping`, `pong` requires `is_trusted_peer`; otherwise the peer gets
an `error` frame with code `not_authenticated` and the message is dropped.
`discovery` is **not** exempt, because it answers with the hub's own identity,
hostname, OS, version and both ports, and because its `remove` action mutates
peer state (`server/mod.rs:672-687`).

### mDNS advertisement

`discovery.rs:15-23` builds the TXT record, `discovery.rs:77-86` registers the
service:

| Record | Value |
|---|---|
| Service type | `_conduit._tcp.local.` (`main.rs:32`) |
| SRV port | `WS_PORT` (9527) — the plaintext listener |
| TXT `device_id` | the desktop's `device_id.txt` UUID |
| TXT `device_type` | `"desktop"` |
| TXT `version` | `CARGO_PKG_VERSION` |
| TXT `ws_port` | `9527` |
| TXT `wss_port` | `9531` |

The Dart reader parses `wss_port` out of the TXT payload and never falls back
to the SRV port (`discovery_service.dart:106-153`). Five Rust tests pin the TXT
contents (`discovery.rs:288-331`).

### `DiscoveryAnnounce` fields

`types.rs:83-105` and `schema.json` (`"Discovery Announce"`). Note what is
**not** on it: there is no `name` field (it is `device_name`), no `address`
field, and no `port` field — ports are `ws_port` and `wss_port`, both optional
and independently `Option`. A client that expects `address` or `port` on an
announce will silently get nothing; the transport address comes from mDNS, not
from the announce. The desktop's own reply is built at
`server/mod.rs:1549-1562` and always populates both ports.

### Outbound encryption

`seal_for_peer` (`server/mod.rs:321-361`) is the **only** place an outbound
frame is encrypted, and it applies exactly one layer:

- a frame already of type `encrypted` passes through untouched (double-wrapping
  produced frames the mobile client could not dispatch);
- a `pairing` frame is never wrapped (it is what establishes the secret);
- a peer with no resolvable secret gets plaintext. For a paired device that is
  never reached, because the frame is then replaced with a
  `delivery_failed` error — encryption failure is fail-closed, not fail-open
  (`server/mod.rs:351-360`, `drop_message` at `server/mod.rs:64-72`);
- `local_desktop` gets plaintext **by necessity**: it has no key pair. This is
  sound only because reaching that state requires the per-launch capability and
  broadcasts are identity-filtered.

---

## 5. Trust boundaries

| # | Boundary | From → To | What crosses | What authenticates it | Where enforced |
|---|---|---|---|---|---|
| 1 | Webview → Rust commands | React app → in-process Rust | `invoke` calls, file dialogs, clipboard, window ops | Tauri capability set: only the 7 permissions in `capabilities/default.json` are granted, and IPC itself is reachable only from the app's own webview origin under the CSP in `tauri.conf.json` | `main.rs:219-263`; drift tests `commands/file.rs:1179-1255` |
| 2 | Webview → local hub | React app → `ws://127.0.0.1:9527` | protocol frames, both directions | **Nothing yet** — the webview does not send `local_auth` (§3). Until it does, it is treated as an unpaired peer. | `server/mod.rs:377-389`; `handlers/pairing.rs:290-309` |
| 3 | LAN peer → desktop | phone → `0.0.0.0:9527` or `:9531` | any protocol frame | one-time pairing token (60 s TTL) → X25519 shared secret; thereafter `is_trusted_peer` per connection | `handlers/pairing.rs:26-58`; `server/mod.rs:672-704` |
| 4 | Desktop → LAN peer | hub → paired phone | encrypted envelope per peer, or plaintext for `local_desktop` | per-peer X25519 shared secret; TLS 1.3 on 9531 | `server/mod.rs:321-361`; `tls.rs:120-128` |
| 5 | Desktop → relay | hub → `services/relay` | signed `relay_route`; v2 binary frames | relay bearer token for the connection; HMAC over the canonical field set with the relay's *signing* key; `from_device_id` bound to the authenticated connection | `services/relay/src/main.rs:1870-1932` |
| 6 | Mobile → desktop | phone → hub over `wss://` | protocol frames, wrapped in the `encrypted` envelope where the type is enveloped (§6.4 of `PROTOCOL.md`); most types are not | TLS 1.3 + SPKI pin captured during the pairing handshake; shared secret from X25519 | `websocket_service.dart:188-222`, `291-316` |
| 7 | Mobile ↔ desktop key exchange | `pairing/request` → `pairing/accept` | X25519 public keys both ways | the one-time token, out-of-band via the QR code | `handlers/pairing.rs:84-94`, `213-223` |
| 8 | Client → relay (mobile) | phone → relay | `relay_auth` then `relay_route` | same as boundary 5 | `websocket_service.dart:453-480` |
| 9 | Operator → relay config | `.env` / mounted file | `RELAY_TOKEN`, `HMAC_SECRET`, optional signing keys | filesystem permissions (`0600`) plus container env | `docker-compose.yml`, `.env.example` |
| 10 | Database at rest | SQLite file → disk | devices, notifications, clipboard history, settings, transfers, automation | SQLCipher; the key itself comes from the OS keyring, or a `0600` key file | `storage.rs:464-472`; `encryption.rs:264-406` |

Boundary 7 deserves a note: the token is the *consent* event, and the public
keys are exchanged over the same TLS-protected socket that the QR code
identified. The mobile app pins the certificate during exactly that handshake
and refuses any later connection whose certificate does not match
(`websocket_service.dart:188-222`).

Boundary 2 is the one to re-read after §3: the webview currently sits outside
every trust boundary, and the fix is a client-side change, not a server one.

---

## 6. Data flow per feature

Common pattern: the **phone** is the capture device for anything with a
platform API (notifications, SMS, calls, phone screen, phone audio); the
**desktop** is the capture device for anything with a desktop API (clipboard,
desktop screen, desktop audio, desktop window state, shell). The hub relays and
persists; it does not own the primary copy of most things.

| Feature | Captured by | Relayed by | Persisted on desktop | Persisted on phone |
|---|---|---|---|---|
| Clipboard | desktop: Tauri clipboard plugin; phone: platform clipboard service | hub, `clipboard/sync` | `clipboard_history` (capped at 500 unpinned) | `clipboard_history` |
| Files | whichever device the user picks on | hub, `file/*` frames; binary chunks on the socket | `file_transfers` row + the file in the download root | `file_transfers` |
| Notifications | phone OS notification listener | hub, `notification/post` | `notifications` (no reply/read columns) | `notification_history` |
| Screen mirror | the device that owns the screen | hub, `screen_mirror/*` | nothing (frames are transient) | nothing |
| Remote input | the device acting as trackpad/keyboard | hub, `remote_input/*` | nothing | nothing |
| Audio | the device being listened to | hub, `audio/*` | nothing | nothing |
| SMS | phone native SMS store | hub, `sms/*` | **nothing** | `sms_threads` |
| Calls | phone telephony | hub, `call/*` | **nothing** | call state (in memory) |
| Automation | triggers evaluated on the hub; the action runs on whichever side can execute it | `automation/*` | `automation_rules` + `automation_logs` | automation preferences in `SharedPreferences` |

### Clipboard

- **Desktop → peers.** `useClipboard` reads through the Tauri clipboard
  capability and calls `invoke('sync_clipboard', {content, mime, sourceDevice})`.
  `commands/notifications.rs:186-235` validates length, persists to
  `clipboard_history`, then broadcasts one `clipboard/sync` through
  `WsServer::broadcast` (which is identity-filtered).
- **Phone → peers.** `SmsService`-style listeners aside, the phone sends
  `clipboard/sync`; the hub's dispatch arm
  (`server/mod.rs:758-760`) is a plain identity-filtered relay, gated on the
  `sync_clipboard` setting (`server/mod.rs:1268-1277`).
- **Persistence.** Desktop: `clipboard_history`, auto-pruned to the newest 500
  unpinned entries in `save_clipboard` (`storage.rs:863-891`).
- **Encryption.** Everything the hub sends to a paired phone is wrapped by
  `seal_for_peer`; nothing the hub sends to the webview is encrypted.

### File transfer

- **Send.** `send_file` (`commands/file.rs:110-238`) takes a transfer target and
  a path. Authorisation is in the engine, not the command:
  `FileTransferEngine::start_outgoing` → `validate_send_path`
  (`file_transfer.rs:437-487`) rejects a `..` component, rejects a symlink,
  requires a regular file under `MAX_SEND_SIZE` (10 GB), and requires the
  canonical path to be either in the download root or in the per-session send
  allowlist the file dialog populates (`approve_send_path`,
  `file_transfer.rs:405-434`).
- **Chunking.** 64 KiB (`file_transfer.rs:14`), base64 inside a JSON
  `file/chunk` frame, or the binary chunk format
  (`types.rs:1293-1307`: 24-byte XChaCha nonce, 4-byte LE metadata length,
  JSON `BinaryFileMetadata`, then AEAD ciphertext) for the direct LAN path.
- **Receive.** `receive_chunk_binary` (`file_transfer.rs:574-629`) enforces
  three independent bounds: chunk index inside the declared range, a non-zero
  declared size, and a running byte total no larger than
  `min(declared size, MAX_TRANSFER_BYTES)`. A breach aborts the transfer and
  deletes its chunk directory. Chunks are staged on disk under
  `<temp>/conduit_chunks/<sanitised id>/chunk_%08d`, never in memory.
- **Finalize.** `finalize_incoming` (`file_transfer.rs:737-873`) sanitises and
  truncates the sender-chosen name, picks a non-colliding name, reassembles,
  verifies the assembled size equals the declared size and, if a checksum was
  declared, SHA-256 over the whole file. On mismatch the file is deleted and
  the transfer fails.
- **Routing.** Outbound `file/*` frames from the desktop are **unicast**, not
  broadcast: `resolve_peer_device` (`commands/file.rs:34-58`) picks one peer
  from the durable `file_transfers` row, and refuses (`Validation` error) when
  the peer is unknown rather than falling back to a fan-out. This is what
  removed the self-echo.

### Notifications

- **Post.** `handle_notification_post` (`handlers/notifications.rs:6-35`)
  persists a `notifications` row, then relays.
- **Gates.** Three settings, evaluated once at the dispatch boundary
  (`server/mod.rs:1240-1338`): `sync_notifications`, `notifications_enabled`,
  and a per-app allowlist `notification_apps` matched case-insensitively
  against either the reported app name or its last dot-segment
  (`server/mod.rs:1212-1222`).
- **Dismiss / reply / mark_read.** The desktop-originated path is the Tauri
  command (persist first, then forward one frame). The remote-originated path
  is the WS handler (apply, then fan out). `reply` and `mark_read` are **pure
  relays** — the schema has no `read` column and no reply table
  (`migrations/001_initial.sql:16-25`), so there is nothing local to store.
  This is honest asymmetry, not an omission.

### Screen mirroring — partly wired, and the direction is inverted from the obvious reading

`handlers/screen_mirror.rs:1-28` is explicit about it, and the implementation
matches: whoever sends `screen_mirror/start` is the **viewer**; the capture
side is the device the request names.

- **Desktop as viewer, phone as capture** is the primary flow. The desktop
  forwards the request to the phone
  (`remote_mirror_target`, `screen_mirror.rs:146-155`, gated on
  `is_local_desktop`); the phone captures with `MediaProjection` and emits
  `screen_mirror/frame`; the hub relays the frame **unicast to the requesting
  viewer** (`send_to_client` at `screen_mirror.rs:489`), and drops a frame from
  a device nobody is watching.
- **Phone as viewer, desktop as capture** is the reverse: the phone's request
  has no other target, so the hub starts a local capture with `xcap` and
  streams JPEG. `remote_mirror_target` returns `None` for a non-local
  requester, which is what selects this path.
- **Input.** `touch`/`key`/`scroll` are injected locally when the target is this
  machine, and forwarded when it is another device
  (`screen_mirror.rs:600-650`). Relative coordinates are clamped at the
  injection side, not rejected on the wire.
- **Not persisted.** Frames are transient. Session state is per-client
  (`VIEWERS` / `LOCAL_CAPTURES`, `screen_mirror.rs:94-98`) rather than global,
  so a second viewer gets its own stream.
- **Honest gaps.** The desktop frontend's `ScreenMirror.tsx` renders frames and
  sends input, but the phone's capture pipeline and the accessibility input
  injector live under `apps/mobile/android/**`, which is outside the four
  components in §1 and outside anything verifiable from the Rust/Dart sources
  read for this document. The claim that Android clamps `fps` to `1..=30`
  (`types.rs:534-535`) is asserted by the protocol type and enforced again on
  the desktop at `screen_mirror.rs:71-77`; the Android side is **[unverified]**
  from here.

### Remote input

`handlers/remote_input.rs:1-12` states the direction: the sender is the input
source, the receiver injects. Frames are deserialised into the
`RemoteInput*` protocol structs rather than being read out of
`serde_json::Value`, so a client cannot invent a field name the injector
silently ignores. Bounds: `MAX_KEY_LEN = 32` for `key`, `MAX_DELTA = 4096.0`
for `move`/`scroll`, and an unknown mouse button is rejected rather than
defaulting to left-click (`remote_input.rs:41-68`). Injection goes through one
process-wide `Enigo` handle shared with screen mirroring, so modifier state
cannot interleave across two injectors (`remote_input.rs:28-33`). Nothing is
persisted.

### Audio

Two roles, both through the hub:

- `audio/stream_start` asks the **desktop** to capture system audio; the capture
  task emits `audio/stream_data` (PCM-16 LE, 16 kHz, mono, base64) to the
  other clients (`handlers/audio.rs:12-64`).
- `audio/stream_data` inbound asks the **desktop** to play; odd-length payloads
  are dropped with a warning (`handlers/audio.rs:76-100`).
- `audio/playback_start` enables a persistent output stream for duplex
  (`handlers/audio.rs:101-120`).

Nothing is persisted. Note that the audio fan-out is one of the four sites that
iterate `ctx.clients` directly rather than calling
`handlers::broadcast_to_others`, so it does not apply the pairing filter — see
`SECURITY.md` §4.1.

### SMS relay

- **Send.** Desktop → phone: `sms/send` with `{to, body}`. The hub relays
  (`server/mod.rs:782-784`) and does not persist.
- **Receive.** The phone's native SMS listener emits `sms/new` and the send
  success path emits `sms/sent`; the hub relays both.
- **Snapshot.** `sms/sync` carries the full `SmsThread` list.
- **Shape divergence, unresolved.** The phone *sends* `{from, body, timestamp}`
  (`apps/mobile/lib/services/sms_service.dart:235-237`) and `{to, body,
  timestamp}` (`sms_service.dart:368-370`); the desktop *reads*
  `{thread_id, message}` (`apps/desktop/src/hooks/useSms.ts:49-82`). Because the
  hub blindly relays `("sms", _)`, both variants have to stay valid, and
  `types.rs:844-911` therefore models `SmsNew`/`SmsSent` with every field
  optional so both shapes deserialise. **The two ends do not currently agree**;
  the desktop's thread list will not be populated from a `sms/new` frame. This
  is a real, open defect, documented in `PROTOCOL.md` and in
  the project Limitations section.
- **Nothing is stored on the desktop.** `migrations/001_initial.sql` has no
  SMS table. The desktop is a transport for SMS, not a second archive of it.

### Automation

Two halves, split by whether the desktop can execute the action
(`automation.rs:493-504`):

- **Desktop-executable** actions: `send_notification`, `route_audio`,
  `run_shell_command`, `toggle_wifi`, `toggle_bluetooth`, `open_url`,
  `set_window_state`. These run on the hub and are logged to
  `automation_logs`.
- **Phone-targeted** actions: `set_phone_profile`, `open_app`, and anything the
  phone executes. The hub logs "action executes on the target phone" and
  forwards; the phone performs it.

Triggers are evaluated in three places: the per-minute timer
(`main.rs:330-382`), `handle_status_update` for battery (`server/mod.rs:1486-1515`),
and connect/disconnect (`server/mod.rs:524-553`). Rules are persisted to
`automation_rules` and reloaded into the in-memory engine on every write
(`handlers/auto_rules.rs:22-30`, `110-118`), so the engine and the table cannot
disagree. The shell allowlist that guards `run_shell_command` is a **security**
control, documented in `SECURITY.md` §3.1 and §5, not here.

---

## 7. Storage

### Two databases, both SQLCipher

| | Desktop | Mobile |
|---|---|---|
| Engine | `rusqlite` with `bundled-sqlcipher` (`apps/desktop/src-tauri/Cargo.toml`) | `sqflite_sqlcipher` |
| Path | `<data_local_dir>/conduit/conduit.db` (`storage.rs:685-689`) | `getDatabasesPath()/conduit_mobile.db` |
| Key location | OS keyring, else a `0600` key file | `flutter_secure_storage` key `database_encryption_key`, generated from `Random.secure()` |
| Tables | `devices`, `notifications`, `clipboard_history`, `settings`, `file_transfers`, `automation_rules`, `automation_logs` | `notification_history`, `clipboard_history`, `file_transfers`, `sms_threads` |
| Pruning | clipboard: 500 unpinned; file rows: pruned by the UI at 50 | notification_history 200, clipboard_history 500, file_transfers 100; SMS threads are never pruned because the native store is the source of truth |

Both directories come from one definition per platform so the database and the
key that decrypts it cannot end up under two different roots
(`encryption.rs:60-68`).

### Migrations

`apps/desktop/src-tauri/src/migrations/001_initial.sql` is the only migration
today, embedded at compile time via `include_str!`
(`storage.rs:94-97`) and applied by `rusqlite_migration` for a fresh database
(`storage.rs:424-446`). Two runtime discovery directories are also scanned —
`<crate>/src/migrations`, `<crate>/migrations`, and `<exe dir>/migrations` —
so an operator can drop a `NNN_name.sql` next to the binary and upgrade an
existing database without a rebuild (`storage.rs:289-347`). Versions come from
the filename prefix, sorted by `(version, filename)`, each applied in its own
transaction, with `schema_version` as the durable record and
`PRAGMA user_version` mirrored for external inspection.

The mobile database uses plain `sqflite` on-version callbacks
(`database_service.dart:29-52`), not the SQL migration file.

### Key management, desktop

Two long-lived secrets: the X25519 identity key (`x25519_private_key`) and the
SQLCipher key (`sqlite_key`), both under service name `conduit_app`
(`encryption.rs:54-58`). `load_or_create_secret` → `resolve_secret`
(`encryption.rs:264-406`) is the whole policy, and it exists because the
previous inline version discarded the result of `set_password` and minted a new
key on every launch on a host without a running Secret Service — see
`SECURITY.md` §3.3.

The rule the implementation enforces:

> A secret is never returned to the caller until it has been read back from a
> durable store.

In order:

1. **The key file wins if it exists.** `<app data>/conduit/keys/<account>.key`,
   mode `0600` on POSIX with `0700` on its directory, `sync_all()` before the
   secret is considered durable. Reading it first makes the fallback
   idempotent — if the keyring later comes back, the same key is used, so
   `PRAGMA key` keeps matching.
2. **Otherwise the keyring.** An existing entry is used as-is; an empty entry is
   a hard error, because an empty key would silently make the database
   unreadable.
3. **Otherwise mint, then prove durability.** `set_password` is followed by a
   read-back comparison (`encryption.rs:343-365`). A write that cannot be
   confirmed is treated as a write that did not happen.
4. **Only a failure of both stores is fatal**, and the error says so.

`migrate_key_file_into_keyring` (`encryption.rs:413+`) moves the file back into
the keyring when the keyring becomes reachable, and deletes the file only after
the keyring write has been confirmed. An extra plaintext key on disk is a
lesser evil than a rotated key.

The fallback is **reduced protection** and the code says so at every use: the
file is readable by anything running as the user, is not covered by keychain
access prompts or auditing, and on Windows is protected only by the per-user
`%LOCALAPPDATA%` ACL rather than a per-file DACL (`encryption.rs:40-51`,
`70-77`).

### Opening the database without destroying it

`open_or_recover` (`storage.rs:604-646`) exists because "cannot open" and "is
corrupt" are different events. A SQLCipher database opened with the wrong key
fails to decrypt exactly the way a corrupt file does, so the old code renamed
the user's database to `.bak` and started an empty one — on every launch.
`classify_db_file` (`storage.rs:160-189`) reads the first 16 bytes and the file
length, and the classification is deliberately biased towards "intact":

| `DbFileKind` | Decision |
|---|---|
| `Empty` | nothing to lose — recreate |
| `PlainSqlite` / `SqlCipher` | structurally intact, so it is a **key** problem: leave the file untouched, surface an error naming the key file and the keyring account, and refuse to start |
| `Unrecognised` | genuinely unreadable — quarantine to `conduit.corrupt_<ts>.bak`, capped at 3 (`storage.rs:191-269`), then recreate |

There is no plaintext marker to grep for in an encrypted file: SQLCipher v4
leaves only the 16-byte salt readable. Two structural signals remain — the file
length is a whole number of pages, and the first 16 bytes are not all zero.

A plaintext legacy database is upgraded in place
(`export_plaintext_to_sqlcipher`, `storage.rs:491-527`) using
`ATTACH DATABASE ?1` with a **bound** path parameter and a `PRAGMA` for the key,
which is safe only because `validate_db_key` proves the key is exactly 64 hex
characters (`storage.rs:448-462`).

### Query hygiene

Every `LIMIT` that reaches SQLite is clamped to `1..=1000` by
`clamp_limit` (`storage.rs:18-23`), because SQLite reads a negative `LIMIT` as
"no limit at all". `get_notifications` additionally clamps at the command
boundary to `1..=500` (`commands/notifications.rs:41-58`). Every statement in
`storage.rs` uses `params!` bound parameters; the only interpolations are the
two `PRAGMA key` statements, which cannot take one, and which the hex
validation makes inert.

---

## 8. Codegen pipeline

```
packages/protocol/src/types.rs        ← the Rust structs, hand-written, canonical
        │  (a developer edits this)
        ▼
packages/protocol/schema.json         ← the JSON Schema
        │
        ├── scripts/generate_types.js ──► apps/desktop/src/types/websocket.ts
        │      (no dependency; hand-rolled projection)
        │
        └── scripts/generate_dart.js  ──► apps/mobile/lib/models/protocol.dart
               (quicktype 26.0.0, pinned in apps/desktop devDependencies)
```

Run with `npm run generate` from `apps/desktop`
(`apps/desktop/package.json:14-17`).

`generate_types.js` is a pure projection of `schema.json`. It handles `$ref`,
`const`, `enum`, `type` arrays, `allOf` (intersection) and `anyOf`/`oneOf`
(union), and it fails loudly in three places rather than guessing:

- a `oneOf` branch with no `title` throws, because the title becomes the
  TypeScript interface name and the union tag;
- a **duplicate** `oneOf` title throws, because the failure mode is otherwise a
  silently duplicated union member;
- a dropped constraint becomes a compile-time hole, so the script comments each
  such case with the reason it exists.

`generate_dart.js` resolves a pinned `quicktype` from
`apps/desktop/node_modules` and only falls back to unpinned `npx` with a
warning, so the Dart output is reproducible as long as the lockfile is
honoured.

### The generated files are frozen artifacts

`websocket.ts` and `protocol.dart` both carry a "GENERATED FILE — DO NOT EDIT"
header. They are **outputs**, not sources. Any change to a message shape is
made in `types.rs` (and `schema.json`), then regenerated.

### Why a `git diff --exit-code` gate is needed

A hand-edit to `websocket.ts` that disagrees with `schema.json` is a silent
failure. TypeScript's structural typing will accept a **duplicate interface
declaration** with the same name and merge the two declarations rather than
erroring: the result still type-checks, and the type is wrong. The generator
now refuses to emit that shape
(`scripts/generate_types.js:213-222`), which prevents the *generator* from
producing it, but nothing prevents a human from adding it by hand afterwards.

There is no CI configuration in the repository today (no `.github/`,
`.gitlab-ci.yml`, or equivalent), so the gate does not exist. It should be:

```
npm run generate
git diff --exit-code -- apps/desktop/src/types/websocket.ts apps/mobile/lib/models/protocol.dart
```

run alongside the existing `scripts/lint-all.ps1` (Clippy, `flutter analyze`,
ESLint). The alternative — a header check or an ESLint `no-restricted-syntax`
rule forbidding edits to those two paths — is weaker, because it detects an
edit and not an omission.

**This gate now exists.** The `codegen` job in `.github/workflows/ci.yml`
(added in `eed58bd`) performs exactly the two commands above, with a third
path — `packages/protocol/schema.json` — added to the diff. It has not executed
against a remote, because the repository has none.

---

## 9. Extension points and known structural weaknesses

### Extension points

| To add… | Touch | Watch out for |
|---|---|---|
| a new message type | `types.rs` struct + `schema.json` + regenerate + `security::validate_msg_type` + the dispatch `match` in `server/mod.rs:736` | the auth gate exempts only `pairing`/`ping`/`pong`; the per-type rate limiter falls back to 100/60s for unknown types; `settings_gate` matches on `msg_type`, so a new `file` action is automatically covered by `sync_files` |
| a new binary frame version | `types.rs` constants + `services/relay/src/main.rs:handle_binary_frame` | v2 rejects v1 outright; the MAC input is `from_device_id ‖ 0x1F ‖ header ‖ payload`, so changing the header changes the MAC |
| a new SQLCipher migration | `migrations/NNN_name.sql` | embedded list wins on filename collision; ordering is by `(version, filename)` |
| a new setting | `ConduitSettings` (`commands/settings.rs:29`) **and** `Storage::get_settings`/`save_settings` **and** `settingsTypes.ts` | three places, and the two Rust ones have a test (`test_save_settings_writes_every_settings_key`) that will fail if you miss one. `allowed_commands` is deliberately *not* a `ConduitSettings` field; see the comment at `commands/settings.rs:181-189` |
| a new inbound command | `#[tauri::command]` + `main.rs:219-263` | anything the frontend can call needs a capability only if it is a plugin API, not a command |
| a new desktop-executable automation action | `automation.rs:493-504` + `action_tag` | anything added there inherits the shell allowlist path; see `SECURITY.md` §3.1 |

### Structural weaknesses

**Single global vs per-client state.** Several subsystems hold process-wide
state in a `LazyLock` or a `OnceLock` while the rest is per-client, and the
split is not always the right one:

- `VIEWERS` / `LOCAL_CAPTURES` in `screen_mirror.rs:94-98` — correctly
  per-client, so a second viewer gets its own stream. A global map here was the
  bug that made two viewers fight.
- `ENIGO` in `remote_input.rs:32-33` — correctly global, because two injectors
  interleave modifier state and produce chords that never release.
- `COMMAND_ALLOWLIST` in `security.rs:1019-1029` — global by design, so the
  allowlist that exists and the allowlist that is consulted cannot be different
  values. It is a `std::sync::RwLock`, not tokio's, because the guard is only
  held long enough to clone a `Vec<String>`.
- `LOCAL_CAPABILITY` in `security.rs:941-955` — global, because a per-launch
  token with two construction sites is a token with an ordering dependency.

The tension is real: a global is correct where "there must be exactly one"
matters for safety, and per-client is correct where "each viewer needs its own
stream" matters for correctness. There is no rule in the tree that says which
is which, so the next subsystem can pick wrong either way.

**The shared helpers in `handlers/mod.rs`.** `has_identity`,
`is_trusted_peer`, `paired_device_id`, `broadcast_to_others`, `send_to_client`
are the only sanctioned way to reach another connection. That centralisation is
deliberate (`handlers/mod.rs:47-61`) and is what makes the broadcast filter
auditable in one place — but it is a convention, not a type. A handler that
wants "everyone including the unpaired" has to reach into `ctx.clients` itself,
and there are currently four such sites (three in `handlers/audio.rs`, one in
`handlers/files.rs:68`). That is exactly how the plaintext-leak defect
survived a fix. **The structural fix is to make `Clients` a private field with
accessors, not documentation.**

**The test-helper convention.** `handlers/mod.rs:157-287` provides three
constructors, and their existence is a direct response to the helpers having
contradictory callers:

| Helper | Registry entry | Shared secret | `has_identity` | `is_trusted_peer` |
|---|---|---|---|---|
| `add_test_unpaired_client` | no | no | false | false |
| `add_test_client` / `add_test_client_mapped` | yes | no | true | **false** |
| `add_test_paired_client` | yes | yes | true | true |

The middle row is the subtle one: a client with an identity but no secret **is**
broadcast-eligible and **is not** authorised to send. That is a real
intermediate state in production — it is exactly what exists between
`mark_paired` and `SyncEngine::add_client` — and the old helpers collapsed it,
so tests passed against a state the production code never reaches. A test that
needs a client past the auth gate must use `add_test_paired_client`; one that
needs a broadcast recipient can use `add_test_client`; one that needs to prove
the filter works needs `add_test_unpaired_client`.

**The `DEFAULT`-vs-`#[serde(default)]` hazard in `ConduitSettings`.**
`ConduitSettings` (`commands/settings.rs:29-59`) defines each field's default
**twice**: once as a `#[serde(default = "...")]` attribute, once in the
hand-written `impl Default`. `#[derive(Default)]` is deliberately not used,
because it would produce `theme: ""` and `minimize_to_tray: false`, which
contradict every attribute. Today the two definitions agree because both call
the same functions and `settings_default_matches_serde_defaults`
(`commands/settings.rs:293+`) pins them. That is a mitigation, not a
structural fix: the hazard recurs the moment someone hand-writes a literal in
one place and not the other, and the failure is silent (a field that defaults
one way for an absent key and another way for an absent struct field).

The related risk on the storage side is a third definition:
`Storage::get_settings` supplies its own per-key defaults as string literals
(`storage.rs:990-1028`). `default_notification_apps` is already shared between
the struct and the storage layer for exactly this reason; the scalar defaults
(`"5"`, `"true"`, `"dark"`, `"#00f0ff"`, the relay URL) are still duplicated as
literals in three places.

---

## 10. Where the code and this document disagree

Recorded so a reader does not have to discover these:

1. **The webview does not send `local_auth`.** §3. The Rust side is complete;
   the frontend is not wired.
2. **`sms/new` and `sms/sent` shapes disagree** between the phone and the
   desktop. §6. Modelled permissively so both parse; the desktop's thread list
   will not populate from a phone-originated frame.
3. **Audio and one file-transfer fan-out bypass the pairing filter.** §6,
   `SECURITY.md` §4.1.
4. **Screen mirroring depends on native Android code** that is outside the four
   components documented here. The direction split is implemented and tested in
   Rust; the capture and injection pipelines are **[unverified]** from the
   sources read for this document.
5. **Call signalling verbs disagree**: the phone emits `incoming`/`forward`
   (`call_service.dart`) and the desktop emits `answer`/`reject`
   (`useCalls.ts`). `CallMessage.action` is an open `String` rather than an
   enum for exactly this reason (`types.rs:779-809`).
6. **`tv` and `watch` are accepted message types with no handler anywhere in the
   workspace.** The dispatcher answers them with
   `unsupported_message` (`server/mod.rs:895-910`) instead of dropping them
   silently, but they remain dead protocol surface.
