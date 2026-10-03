# Changelog

All notable changes to Conduit are recorded here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Conduit is pre-release. The version below is the first public release of the
source tree, and the entries describe what the code in this tree does. Read
[Known limitations](#known-limitations) before relying on any of it.

## [Unreleased]

### Fixed

#### A broadcast now reaches a phone that is only behind the relay

- Every frame the desktop fans out — clipboard, notifications, SMS, calls,
  discovery and removal, and every file-transfer control frame — is now also sent
  to each paired device with **no socket on this machine**, as one signed and
  sealed `relay_route` per recipient. The fan-out previously walked local
  connections only, and the relay connection is not one of those, so a
  relay-only phone received nothing from any of those handlers and failed
  silently: iterating an empty set looks exactly like success.
- The recipient set is the pairing registry, not the set of connected devices. A
  phone that paired on the LAN and then walked out of range is still paired while
  being absent from the latter, which is the case this feature exists for.
- Each recipient's copy is sealed for that device with its own shared secret. The
  relay egress previously read that secret from the connected-device map, which
  does not contain off-LAN devices — so a clipboard body or SMS routed to one
  would have gone out in the clear.
- A device that is on the LAN *and* joined to the relay — what a network
  transition looks like — is served once, not twice. The desktop is never its own
  relay recipient.
- A relayed refusal, a relayed `pong`, and a relayed clipboard body all arrive at
  the phone as `relay_delivery` → decrypt → dispatch, which the mobile client
  now handles; it previously handled only a bare `error` on the socket and
  silently dropped the same frame when it arrived wrapped.

#### A device behind the relay is told when it is refused

- Rate limits, invalid messages, failed authentication, refused settings and
  `pong` are now answered to a device that has no local socket, by routing the
  `error` back through the relay. Previously every refusal aimed at such a device
  was looked up in the local connection table, missed, and was discarded — so the
  peer was refused and never told why, which from the far end is
  indistinguishable from a dropped connection.
- This is **not** a protocol change. A routed payload is an arbitrary JSON
  message and the relay forwards it without inspecting it, so an `error` was
  always legal to route; the route simply was not being taken.

#### Relayed traffic is rate-limited, and relayed file chunks arrive

- A relayed frame is charged to the **sending device's** budget, once. The relay
  connection never passed through the limiter at all, so relayed traffic was
  charged neither a count nor a byte, by any device. Frames that name no trusted
  sender are charged to the relay connection instead, so an unauthenticated peer
  cannot obtain unlimited free parse attempts.
- Relayed binary frames are charged to the relay connection's budget. A v2 frame
  names its recipient rather than its sender, so the transport is the only budget
  available to it, and it previously had none.
- A relayed binary file chunk is no longer discarded. Two separate defects sat on
  that path: the receiver looked the sender's device id up in a map keyed by
  connection id and missed, and the unwrapper that has to name the sender first
  drew its candidates from that same connection-liveness map, so a relay-only
  sender was refused one layer above the delivery — after the relay had already
  verified the tag.
- The desktop now answers "what secret do I encrypt for this device" in one
  place, instead of five call sites reading a connection-liveness map as though
  it were a trust registry.

#### Defects the fan-out introduced, and one it exposed

- **Unpairing takes effect immediately.** The route-key registry refreshes on a
  5 s timer, and making it the fallback the authentication gate reads turned that
  into a trust window rather than a routing detail: a revoked device stayed both
  routable and trusted for up to five seconds, so `pairing/accept`'s copy kept
  delivering clipboard and notification bodies to a phone the user had just
  unpaired. `disconnect_client` now clears the registry synchronously. The relay
  keeps its own copy of the key set and refreshes that on its own schedule.
- **The registry publishes its two maps in a safe order.** A reader landing
  between the two writes could previously see a device in the recipient set whose
  pairing secret was not yet published, and the egress would have put that frame
  on the wire in plaintext. Secrets are now published before route keys, so the
  only reachable intermediate state is a device that is simply never named.
- **An oversized broadcast no longer takes the relay down.** Sealing hex-encodes
  the payload, and the relay's read ceiling is 1 MiB — and crossing that ceiling
  closes the connection rather than dropping one frame. A paired peer could send
  a multi-megabyte `sms`, `call`, `clipboard` or `file` frame (those types close
  no field set in validation) and take every relay-only peer offline for a
  reconnect backoff. The fan-out now refuses anything over 256 KiB. The
  validation gap itself is still open.
- **A declined file request now reaches a relay-only sender.** With
  `auto_accept_files` off, the `file_accept_disabled` refusal was looked up in
  the local connection table using a *device* id, never matched, and dropped — so
  a phone behind the relay pushing a file was declined and told nothing, while
  waiting for an ack that now arrives.
- **A relay route that cannot be built is refused rather than sent empty.** The
  route builder substituted a null payload on a parse failure, which the relay
  would have forwarded and counted as delivered, and the recipient would have
  dropped. It now declines to build the route.

### Changed

- Test counts re-measured rather than carried over: `conduit` 799 → 817,
  `conduit-relay` 247 (1 ignored) plus 2 doctests, `conduit-protocol` 281 plus 1
  doctest, desktop frontend 249 across 22 files, mobile 190 → 194, and
  `flutter analyze` 370 infos with no errors and no warnings.

## [0.1.0] - 2026-09-28

First public release of the source tree.

### Added

#### Local device pairing

- QR-code and manual pairing between a desktop and a phone, with a per-launch
  capability token that is derived once and not persisted.
- Device registry with a configurable device limit, stored in the encrypted
  desktop database.
- mDNS advertisement from the desktop and mDNS browsing from the phone, so a
  phone on the same network can find the desktop without typing an address.
- Two local listeners on the desktop: `ws://` on port 9527 for the local hub
  and diagnostics, and `wss://` on port 9531, which is the port a LAN peer is
  expected to dial. Port 9531 has a second, internal role: it is also the
  relay's own loopback-only plaintext listener, which the desktop uses to join
  the relay it hosts itself (see *Relay* below).
- Protocol-version enforcement at the desktop hub. A peer announcing a version
  above `PROTOCOL_VERSION` is answered with `unsupported_protocol_version`.

#### Clipboard and messages

- Clipboard history sync, with the history stored locally in the encrypted
  database and configurable on or off.
- SMS threads and individual messages relayed to the desktop, with per-thread
  read state.
- Notification relay: posted notifications are stored on the desktop, can be
  marked read, dismissed, and replied to.
- Search across the relayed content from either device.

#### Files

- File transfer in both directions, chunked, with request, accept, progress,
  resume, cancel, and completion messages.
- Chunked transfer is bounded and rejects path-traversal attempts and invalid
  download paths. An optional auto-accept setting is available for trusted
  peers.

#### Calls, audio, screen and input

- Call signalling relayed between the two devices: the phone reports incoming
  and forwarded calls, the desktop can answer or reject.
- Audio stream and playback control between the two devices.
- Screen mirroring from the phone to the desktop.
- Remote input from the phone to the desktop as a trackpad (move, click,
  scroll) and as a keyboard, including modifier keys.
- Remote input is gated on authentication. An unauthenticated client is
  refused at the dispatcher, before the handler runs.

#### Automation

- Trigger and action rules held on the desktop, with rule create, delete, sync
  and triggered-event messages.
- Shell commands in automation rules are denied by default. A command must
  appear in the operator-configured allowlist, which is parsed fail-closed: a
  missing, malformed, or non-array setting produces an empty allowlist rather
  than an open one.
- The allowlist is enforced on the write path, on the triggered path, and
  again at execution time, so it cannot be bypassed by a field alias or by a
  batched sync payload.

#### Encryption and storage

- Per-peer payload encryption using X25519 key agreement with XChaCha20-Poly1305
  for the payload envelope.
- Desktop storage in SQLCipher-encrypted SQLite, with the database key held in
  the OS keyring and a permission-restricted key file as a fallback when the
  keyring is unavailable.
- The keyring fallback is written so that a keyring outage does not orphan an
  existing database, and there is a test that pins this behaviour.
- Mobile secure storage via the platform keystore.
- The desktop reports that this encryption is **not** end-to-end. The envelope
  is hop encryption terminated by the desktop process. A relay routes it without
  ever decrypting it — it forwards opaque envelopes — but the relay runs inside
  the desktop process, so it terminates nowhere and isolates nothing. See
  [ADR-0007](docs/decisions/0007-refuse-to-ship-end-to-end-encryption-claims.md)
  and [`SECURITY.md`](SECURITY.md).

#### Relay

- The relay, crate `conduit-relay`, is a **library**, not a service. It has no
  `[[bin]]` and no `src/main.rs`, and the desktop app hosts it in-process as a
  background task (`apps/desktop/src-tauri/src/relay.rs`, `RelayHost`). There is
  nothing to deploy, install or configure, and no container image: the
  `Dockerfile`, the compose file and the container build script that used to ship
  it are gone. `relay_enabled` defaults to true, and a relay restart is a desktop
  restart.
- Because it is a library hosted by the desktop, the relay operator *is* the
  desktop user. There is no third party to trust and no service to harden, and
  equally no process boundary between the relay and the hub it runs inside.
- Relay authentication handshake, `relay_route` HMAC verification under a
  **per-device** key, and a replay-protection nonce cache.
  `relay_route` is HMAC-SHA256 over
  `"conduit-protocol/v1/derive:conduit-relay/v1/route-key:" + device_id`, keyed by
  the pairing secret, with `key_id` required to equal `from_device_id`. There is
  no shared signing key: the former `RELAY_SIGNING_KEY`, `RELAY_SIGNING_KEY_ID`,
  `RELAY_SIGNING_KEY_PREVIOUS` and `RELAY_SIGNING_KEY_PREVIOUS_ID` settings are
  gone, and with them the current/previous key ring and the rotation window they
  implied. A re-pair rotates a device's key automatically, and the relay resolves
  keys by asking its host through a `RouteKeys` trait, so a removed device loses
  its key within five seconds.
- Binary frame v2 with a versioned prefix. The relay rejects the superseded v1
  prefix rather than guessing.
- Health, metrics and certificate-pin HTTP endpoints on port 9530, bound to
  loopback only. `/health` and `/` are behind a bearer token. `/metrics` is
  behind `RELAY_METRICS_TOKEN` **when one is configured** and open when it is
  not. `/healthz` and `/pin` are **not** behind bearer tokens.
- Self-signed certificate generation with SAN handling, key-format detection,
  key and certificate mismatch refusal, and permission tightening on the key
  file.
- Startup bootstraps rather than fails closed: with no master HMAC secret
  configured, the relay generates one and persists it. Failing to start would
  have meant a key that changed on every launch.
- Configuration precedence is desktop **Settings** (passed in as
  `conduit_relay::Overrides`) → environment variables → library defaults. The
  environment is a fallback for headless and test use, not how the app is
  configured. `health_bind` and `ws_bind` are pinned to loopback by the desktop
  and are deliberately not environment-overridable.

#### Shared protocol and tooling

- A shared wire-format crate, `conduit-protocol`, holding the message types,
  the port and frame constants, HMAC signing, key derivation, and the replay
  nonce cache.
- A hand-maintained JSON Schema, `packages/protocol/schema.json`, with a
  normative prose specification in [`packages/protocol/PROTOCOL.md`](packages/protocol/PROTOCOL.md).
- Five tests hold the Rust types and the schema together, including one that
  serialises every Rust message type and asserts the schema accepts the
  result.
- Code generation from the schema into TypeScript and Dart client models.
- PowerShell build and lint wrappers, and a single aggregate lint script.

#### Tests and CI

- 187 tests in `conduit-relay` plus 2 doctests, 275 in `conduit-protocol`, and
  a 724-test suite in the desktop backend, covering per-message-type handlers,
  the pairing and authentication boundary, automation rule evaluation,
  encryption key resolution, storage migrations, file transfer chunking, rate
  limiters, and discovery.
- The authentication boundary has exploit-shaped tests that walk the actual
  attack path rather than asserting on a helper.
- 222 frontend unit tests across 20 files, plus a Playwright suite that
  exercises the React shell in a plain browser.
- 87 mobile unit tests, including one for the phone's relay route-key derivation
  and binary frame codec.
- A CI workflow with six jobs: Rust, Node, codegen drift, brand asset drift,
  Flutter, and repository hygiene.

### Known limitations

This release is a pre-release. The following are known and are not fixed in
this tree.

- **The project is pre-release.** Version numbers across all three crates and
  both frontends are `0.1.0`. There is no release artefact, no installer, and
  no published package for any component. Building from source is the only
  supported path.
- **The desktop app cannot be built from a fresh clone.** The desktop backend
  uses SQLCipher, which links against a vendored OpenSSL distribution under
  `.tools/openssl-win64/`. That directory is roughly 80 MB, is gitignored, and
  has no download script, no version pin, and no checksum. It must be supplied
  by hand on Windows, and on Linux there is no working path at all because
  `.cargo/config.toml` is unconditional. The other two crates build without it.
  This is the single largest onboarding obstacle.
- **The cloud relay client on the desktop is not a client.** The `relay_url`
  setting is legacy: it is still persisted, and it is not dialled. What the
  desktop does dial is its own relay, over the loopback-only plaintext listener
  on 9531, because the relay runs in the same process and a relay can only
  reach a device that is itself connected to it. Pointing `relay_url` at a
  third-party relay is not a configuration change that works today; it needs a
  code change.
- **The mobile client and the relay agree on the wire format.** The superseded
  v1 frame limitation from the previous pass is resolved: the phone now emits
  v2 frames through `lib/services/relay_route.dart` and signs its routes with a
  key derived from its own pairing secret. There is no separate integration
  test proving a real phone completes a relayed connection end to end, though —
  see [`docs/TESTING.md`](docs/TESTING.md) §7.
- **Call signalling verbs differ between the two clients.** The phone emits
  `incoming` and `forward`; the desktop emits `answer` and `reject`. This is
  deliberate, because the two sides do not share a verb set, and the protocol
  models `action` as an open string for this reason.
- **Mobile certificate pinning does not work as specified.** The mobile client
  hashes the whole DER certificate while the relay publishes an SPKI pin, so
  the two can never match. The desktop has no pin configuration target. See
  [`docs/relay-tls.md`](docs/relay-tls.md).
- **`dart analyze --fatal-infos` reports 362 lint infos** across the Dart tree
  (no errors and no warnings — the count is style infos, mostly
  `prefer_single_quotes` and naming rules inside the generated
  `lib/models/protocol.dart`). The `--fatal-infos` flag promotes those to a
  non-zero exit, which is why that step is non-blocking in CI. `flutter test`
  passes 87 of 87.
- **`dart format` is not enforced.** The check runs in CI and in the lint
  script but is non-blocking in both, and most of the Dart tree is unformatted.
  Dart diffs carry unrelated churn until that is fixed.
- **Test coverage has three structural gaps**, documented in detail in
  [`docs/TESTING.md`](docs/TESTING.md). There are no Rust integration tests
  anywhere, because no `tests/` directory exists; every Rust test is an
  in-crate unit test. The Tauri IPC mock returns `undefined` for every
  command, so no test exercises a real command's return value. And there is no
  cross-language conformance test verifying that the Rust types, the generated
  TypeScript, and the generated Dart agree.

No comparison link is included. This is the first entry, so there is no earlier
version to compare against. Add the usual `keepachangelog.com` comparison links
when the second release is cut.
