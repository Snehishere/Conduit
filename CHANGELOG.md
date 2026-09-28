# Changelog

All notable changes to Conduit are recorded here.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project uses [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

Conduit is pre-release. The version below is the first public release of the
source tree, and the entries describe what the code in this tree does. Read
[Known limitations](#known-limitations) before relying on any of it.

## [Unreleased]

Nothing yet.

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
  and diagnostics, and `wss://` on port 9531, which is the only port a LAN peer
  is expected to dial.
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
  is hop encryption terminated by the desktop process, so a LAN peer and any
  relay can read the traffic. See [ADR-0007](docs/decisions/0007-refuse-to-ship-end-to-end-encryption-claims.md)
  and [`SECURITY.md`](SECURITY.md).

#### Optional relay

- A self-hostable relay, crate `relay`, shipped with a `Dockerfile` and a
  compose file. It is optional; Conduit works on a LAN without it.
- Relay authentication handshake, `relay_route` HMAC verification with a
  domain-separated key and a key-rotation window, and a replay-protection nonce
  cache.
- Binary frame v2 with a versioned prefix. The relay rejects the superseded
  v1 prefix rather than guessing.
- Health, metrics, and certificate-pin HTTP endpoints, all behind bearer
  tokens.
- Self-signed certificate generation with SAN handling, key-format detection,
  key and certificate mismatch refusal, and permission tightening on the key
  file.
- Fail-closed startup: the relay refuses to start without its master HMAC
  secret rather than generating a temporary one, and reads its configuration
  from process environment variables only.

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

- 189 tests in `relay`, 266 in `conduit-protocol`, and a 696-test suite in the
  desktop backend, covering per-message-type handlers, the pairing and
  authentication boundary, automation rule evaluation, encryption key
  resolution, storage migrations, file transfer chunking, rate limiters, and
  discovery.
- The authentication boundary has exploit-shaped tests that walk the actual
  attack path rather than asserting on a helper.
- 216 frontend unit tests across 20 files, plus a Playwright suite that
  exercises the React shell in a plain browser.
- 43 mobile unit tests for screens and widgets.
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
- **The cloud relay client on the desktop is compiled but not enabled.** The
  `relay_url` setting is persisted but not dialled. Enabling it is a code
  change, not a configuration change.
- **The mobile client is not yet updated for the current relay wire format.**
  The mobile client emits the superseded v1 frame prefix while the relay
  accepts only v2, so a phone cannot complete a relayed connection. Relay
  routing from the phone is therefore not functional even where the desktop
  side could support it.
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
  passes 43 of 43.
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
