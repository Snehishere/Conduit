# Conduit

Conduit links a phone to a desktop over your local network. It syncs the
clipboard between them, transfers files in both directions, relays SMS threads
and notifications, forwards calls, mirrors the phone's screen onto the
desktop, acts as a remote trackpad and keyboard, and runs trigger-to-action
automation. Transport is a local WebSocket hub on the desktop with mDNS
discovery, so a phone on the same network finds the desktop on its own. An
optional relay is included that you can host yourself, for when the two devices
are not on the same network. There is no account system, no hosted service and
no telemetry.

## Status

**Conduit is pre-release software.** The feature set is incomplete, two
subsystems are known not to work, and the desktop app cannot be built from a
fresh clone. Do not depend on it for anything you cannot lose. Every component
is at version `0.1.0`; there is no release artefact, installer or published
package for any of them, so building from source is the only supported path.

### What works

- Pairing a phone with a desktop over the local network, by QR code or by hand.
- Clipboard sharing and history, held in the desktop's encrypted database.
- File transfer in both directions, chunked, with progress, resume and cancel.
- Notification relay to the desktop, with read, dismiss and reply.
- SMS thread and message relay to the desktop, with per-thread read state.
- Call signalling: the phone reports incoming and forwarded calls and the
  desktop can answer or reject.
- Remote input from the phone to the desktop, as a trackpad and as a keyboard
  including modifier keys.
- Screen mirroring, where the phone is the capture device and the desktop is
  the viewer.
- Trigger-to-action automation rules, held on the desktop, with a
  deny-by-default shell command allowlist.
- The relay server itself, which builds, runs and passes its tests. It is
  reachable and its routing logic is implemented; see the two items below for
  why a client cannot use it yet.

### What is not finished

- **The optional relay client is compiled but not enabled.** The desktop's
  relay client exists in
  `apps/desktop/src-tauri/src/server/mod.rs` (`WsServer::spawn_relay_client`),
  but its only call site is commented out in
  `apps/desktop/src-tauri/src/main.rs`, with the note "Disabled cloud relay
  client to keep the app strictly local". The `relay_url` setting is persisted
  but never dialled, so enabling relay routing from the desktop is a code
  change rather than a configuration change.
- **The mobile client is not yet updated for the current relay wire format.**
  The relay accepts only binary frame version `0x02`
  (`BINARY_FRAME_VERSION` in `packages/protocol/src/types.rs`), but
  `apps/mobile/lib/services/websocket_service.dart` still emits the superseded
  `0x01` prefix. A phone therefore cannot complete a relayed connection, and
  relay routing does not work from the phone either.
- **Mobile certificate pinning does not work as specified.** The mobile client
  hashes the whole DER certificate while the relay publishes an SPKI pin, so
  the two can never match. The desktop has no pin configuration target. See
  [`docs/relay-tls.md`](docs/relay-tls.md).
- **A fresh clone cannot build the desktop app.** See
  [The vendored OpenSSL requirement](#the-vendored-openssl-requirement).

The full list of known limitations is in
[`CHANGELOG.md`](CHANGELOG.md), under "Known limitations".

## Screenshots

Screenshots are not committed yet. To add them:

<!--
  1. Put image files in docs/images/ (create the directory; it is not in the
     repository today). PNG or WebP, under about 400 KB each.
  2. Replace the placeholder list below with markdown image links, and keep the
     exact relative paths so they resolve from the repository root:

       ![Desktop home screen](docs/images/desktop-home.png)
       ![Desktop paired devices](docs/images/desktop-devices.png)
       ![Desktop clipboard history](docs/images/desktop-clipboard.png)
       ![Desktop viewing the phone's screen](docs/images/desktop-screen-mirror.png)
       ![Mobile home screen](docs/images/mobile-home.png)
       ![Mobile pairing screen](docs/images/mobile-pairing.png)

  3. Only reference a path that exists. A broken image link is worse than no
     screenshot.
  4. Remove this comment and the placeholder list once the files are committed.

  The desktop app is the easiest to capture: `cd apps/desktop` then
  `npm run tauri -- dev`. The mobile app needs a device or a running
  emulator, so capture it second if time is short.
-->

The desktop application is the easiest of the two to screenshot, because it
runs on the machine you are already using. The mobile app needs a connected
device or a running emulator.

## Features

### Pairing and discovery

- A phone pairs with the desktop by QR code or by entering the details
  manually, using a single-use pairing token that expires.
- The desktop's own webview reaches the local hub with a per-launch capability
  token that is held in memory and never written to disk.
- The desktop advertises itself over mDNS and the phone browses for it, so a
  phone on the same network finds the desktop without an address being typed.
- The desktop hub answers a peer that announces a protocol version above the
  one it implements with `unsupported_protocol_version`.
- Paired devices are listed in the desktop interface and can be found by name
  or device type from the desktop search field.

### Clipboard

- Clipboard contents are shared between the two devices, with the history kept
  in the desktop's encrypted database.
- Clipboard sync can be turned off in settings.

### Files

- Files are transferred in both directions in chunks, with request, accept,
  progress, resume, cancel and completion states.
- Chunked transfer is bounded, and download paths are validated and rejected
  when they fall outside the downloads root or attempt path traversal.
- An auto-accept setting is available for trusted peers.

### Notifications

- Notifications posted on the phone are stored on the desktop, where they can
  be marked read, dismissed or replied to.
- The seed list of notification apps is configurable; the desktop cannot
  enumerate the apps installed on a phone, so it is told which ones to expect.
- Relayed notifications can be searched by title or body from the desktop
  search field.

### SMS

- SMS threads and individual messages are relayed to the desktop with
  per-thread read state.

### Calls and audio

- The phone reports incoming and forwarded calls, and the desktop can answer
  or reject them.
- An audio stream between the two devices, with playback control.
- The two clients use different call verbs on purpose: the phone emits
  `incoming` and `forward`, the desktop emits `answer` and `reject`.

### Screen mirroring

- The phone captures its own screen and the desktop displays it, with touch,
  key and scroll input sent back to the phone.

### Remote input

- The phone acts as a trackpad (move, click, scroll) and as a keyboard,
  including modifier keys.
- Remote input is refused at the dispatcher for any client that has not
  authenticated, before the handler runs.

### Automation

- Trigger and action rules are held on the desktop, with create, delete, sync
  and triggered-event messages.
- Shell commands in rules are denied by default. A command has to appear in
  the operator-configured allowlist, and that allowlist is parsed fail-closed:
  a missing, malformed or non-array setting produces an empty allowlist rather
  than an open one.
- The allowlist is enforced on the write path, on the triggered path and again
  at execution time.

### Encryption and storage

- Messages are wrapped in an encrypted envelope using X25519 key agreement
  and XChaCha20-Poly1305, with HMAC-SHA256 for message authentication.
- **This is not end-to-end encryption.** The desktop hub decrypts each message
  and dispatches the inner payload, and a relay verifies signatures on messages
  and routes them. Traffic is protected from a passive observer on your local
  network and from tampering in transit. It is not protected from the desktop
  itself, nor from an operator who controls a relay. The status bar reports
  this as "Not E2E". See [`SECURITY.md`](SECURITY.md) and
  [ADR-0007](docs/decisions/0007-refuse-to-ship-end-to-end-encryption-claims.md).
- Desktop data is held in a SQLCipher-encrypted SQLite database. The database
  key is stored in the OS keyring under service `conduit_app` and account
  `sqlite_key`, with a permission-restricted key file as a fallback when the
  keyring cannot be written.
- The mobile app stores its secrets in the platform keystore.
- Conduit is not a sandbox. A process running as your own user can read the
  database and the configuration; the threat model says so in
  [`SECURITY.md`](SECURITY.md).

### Optional relay

- A self-hostable relay, crate `relay`, shipped with a `Dockerfile` and a
  compose file. Conduit works on a local network without it.
- An authentication handshake, `relay_route` HMAC verification with a
  domain-separated key and a key rotation window, and a replay-protection
  nonce cache.
- Binary frame version 2 with a versioned prefix. The relay rejects the
  superseded version 1 prefix rather than guessing.
- Health, metrics and certificate-pin HTTP endpoints, behind bearer tokens.
- *Not yet working from a client.* Both the desktop and the mobile relay
  clients are incomplete for the reasons given under
  [Status](#status). Nothing in this list should be read as a working
  end-to-end path today.

## Getting started

### Prerequisites

| Tool | Version | Needed for | Notes |
|---|---|---|---|
| Rust | 1.98, pinned by `rust-toolchain.toml` | `conduit`, `relay` | Both are edition 2024. `conduit-protocol` is edition 2021. |
| Node.js | 24, pinned by `.nvmrc` | desktop frontend, code generation | npm 11 or newer. `apps/desktop/.npmrc` sets `legacy-peer-deps=true`, so use `npm ci`. |
| Flutter | 3.47 | `apps/mobile` | With Dart 3.13. `pubspec.yaml` declares `sdk: '>=3.12.0 <4.0.0'`. |
| Docker | any recent release | the relay container only | Not needed for the protocol crate, the relay binary, the desktop app or the mobile app. |
| **Vendored OpenSSL** | **3.5.8, win64** | **the desktop crate only** | **Blocking. See below.** |

### The vendored OpenSSL requirement

**The desktop app cannot be built from a fresh clone.** This is the largest
onboarding obstacle in the repository, and it is not fixed.

`apps/desktop/src-tauri` depends on `rusqlite` with the `bundled-sqlcipher`
feature. That compiles SQLCipher from source, but SQLCipher still has to link
against a crypto library, so the build needs OpenSSL headers and static
libraries. `.cargo/config.toml` supplies them by pointing two environment
variables at a vendored copy:

```toml
# .cargo/config.toml
[env]
OPENSSL_LIB_DIR   = { value = ".tools/openssl-win64/lib/static", relative = true }
OPENSSL_INCLUDE_DIR = { value = ".tools/openssl-win64/include", relative = true }
```

`relative = true` resolves those paths against the workspace root, so the
directory has to be `<repo>/.tools/openssl-win64/`, with `lib/static/` and
`include/` directly under it. The source is named in the comment in that file:
the `OpenSSL-Distribution` releases published under `TaurusTLS-Developers`.
The copy this release was developed against is 3.5.8 win64, about 80 MB.

Four things make this a blocker rather than a footnote:

1. **There is no download script.** `.gitignore` labels `.tools/` as fetched
   by `scripts/`, but no `scripts/fetch-openssl.ps1` exists.
2. **There is no version pin** recorded in any tracked file, so the 3.5.8 in
   use is unrecorded knowledge.
3. **There is no checksum.** An arbitrary archive from a release page would be
   accepted without verification.
4. **`.tools/` is gitignored**, so a clone never receives it. The failure
   surfaces as a link error from `libsqlite3-sys`'s build script rather than
   as a message that names the missing directory.

To build the desktop app on Windows, download the win64 package from that
release page, extract it to `<repo>/.tools/openssl-win64/`, and confirm that
`lib/static/*.lib` and `include/openssl/*.h` sit directly under it.

`.cargo/config.toml` is unconditional, so on Linux the two variables still
point at a Windows-only layout and the desktop crate has no working build
path. For that reason the CI workflow runs the desktop crate's clippy, test
and build steps as non-blocking and emits a warning instead.

`conduit-protocol` and `relay` do not need any of this. Neither depends on
`rusqlite` or SQLCipher, and both build with no vendored dependencies.

### Build and run

Run every `cargo` command from the **repository root**. Run `npm` commands
from `apps/desktop` and `flutter` commands from `apps/mobile`. There is no
root `package.json` and no root Flutter project; the root `Cargo.toml` is a
virtual workspace that owns all three Rust crates.

The crate name and the directory name do not always match, and the mismatch is
the second most common source of build friction:

| Directory | `-p` name |
|---|---|
| `packages/protocol` | `conduit-protocol` |
| `apps/desktop/src-tauri` | `conduit` |
| `services/relay` | `relay` |

Always pass `-p`. `cargo test -p src-tauri` will not resolve, and neither will
`cargo test -p desktop`.

**Protocol crate** (library only, no binary and no I/O):

```bash
# repository root
cargo build -p conduit-protocol
cargo test  -p conduit-protocol
```

**Relay** (optional):

```bash
# repository root
cargo build -p relay
cargo test  -p relay
cargo run   -p relay
```

The relay reads its configuration from process environment variables only. A
bare `cargo run -p relay` does not read `.env`; that is `docker compose`'s
job. For a local run, export the variables yourself, or use compose.
[`.env.example`](.env.example) is the annotated list of all of them.

**Desktop app** (requires the vendored OpenSSL above):

```powershell
cd apps\desktop
npm ci
npm run tauri -- dev        # Vite on 5173 plus the Tauri window
npm run tauri -- build      # release bundle
```

`scripts\build-desktop.ps1` wraps the same sequence and accepts `-Release` and
`-Clean`. Frontend-only work needs neither Rust nor OpenSSL: `npm run dev`
serves the React app on port 5173, where Tauri IPC is unavailable and every
`invoke()` call fails, so the app renders in a degraded state. That is the same
environment the Playwright suite runs in.

**Mobile app**:

```powershell
cd apps\mobile
flutter pub get
flutter run
```

If `flutter` is not on `PATH`, add your Flutter SDK's `bin` directory to `PATH`
for the session before running it. Build with `flutter build apk --debug` for
Android, or `flutter build ios --debug --no-codesign` for iOS from a macOS
host. `scripts\build-mobile.ps1 -Target android` wraps this.

**Code generation** (regenerates the client models from
`packages/protocol/schema.json`):

```powershell
cd apps\desktop
npm run generate
```

That writes `apps/desktop/src/types/websocket.ts` and
`apps/mobile/lib/models/protocol.dart`. Both are generated files and must
never be hand-edited. See [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md).

## Project layout

| Path | Crate or package | What it is |
|---|---|---|
| `packages/protocol` | `conduit-protocol` (Rust, edition 2021) | The shared wire-format source of truth. `src/types.rs` holds the message types, the port constants and the binary frame constants; `src/lib.rs` holds HMAC signing, key derivation and the replay nonce cache; `schema.json` is hand-maintained and held in sync with the types by five tests; `PROTOCOL.md` is the normative prose specification. Library only. |
| `apps/desktop` | `conduit-desktop` (npm) and `conduit` (Rust, in `src-tauri/`) | The Tauri v2 desktop app. `src/` is React 19, TypeScript, Vite 8 and Tailwind 4. `src-tauri/` is the Rust backend and owns the local WebSocket hub, SQLCipher storage, mDNS advertisement, automation, file transfer, screen mirroring, remote input and audio. |
| `apps/mobile` | `conduit` (Flutter) | The Flutter app for Android and iOS. Owns mDNS browsing, the WebSocket client, secure storage, on-device QR scanning, screen capture and the SMS, notification and call bridges. |
| `services/relay` | `relay` (Rust, edition 2024) | The optional self-hostable relay: the router, the authentication handshake, `relay_route` verification, binary frame v2, TLS certificate generation, and the health, metrics and pin endpoints. Ships with a `Dockerfile`. |
| `scripts` | — | PowerShell build and lint wrappers, the JavaScript code generators, the certificate pin helper, and `scripts/icons/`, the Python brand-asset generator. |
| `assets/brand` | — | The brand source of truth. `conduit.mark.json` is rendered into every icon, the logo component and the Dart logo widget. |
| `docs` | — | `ARCHITECTURE.md`, `DEVELOPMENT.md`, `TESTING.md`, `relay-tls.md` and `decisions/`. |
| `docker-compose.yml`, `.env.example` | — | Relay deployment and its annotated configuration. |
| `Cargo.lock` | — | The only lockfile. Nested ones are forbidden by `.gitignore` (`**/Cargo.lock` with `!/Cargo.lock`). |

Both Rust clients bind to the wire format by path dependency, and both
frontends bind to it by code generation from `schema.json`. `schema.json` is
maintained by hand rather than generated from `types.rs`, and five tests in
`conduit-protocol` hold the two together, the load-bearing one serialising
every Rust message type and asserting that the schema accepts the result.

## Ports

The port constants live in `packages/protocol/src/types.rs` (`LAN_WS_PORT`,
`LAN_WSS_PORT`).

| Port | Protocol | Component | Purpose |
|---|---|---|---|
| **9527** | `ws://`, plaintext | desktop | The local hub, and the desktop's own diagnostic path. |
| **9531** | `wss://`, TLS | desktop | **The only port a LAN client dials.** |
| 9528 | `ws://`, plaintext | relay | Off by default (`RELAY_ENABLE_PLAIN_WS=false`). Local testing. |
| 9529 | `wss://`, TLS | relay | The relay's WebSocket listener. |
| 9530 | HTTP | relay | `/healthz`, `/health`, `/metrics`, `/pin`. Bound to loopback in the compose file. |

**Rule: never dial `wss://` on 9527, and never silently downgrade a LAN peer to
`ws://`.**

The mobile app once inlined `wss://$ip:9527` at three call sites, which made
local pairing non-functional. It now carries `kLanWsPort = 9527` and
`kLanWssPort = 9531`, mirrored from `types.rs`, and a test in
`conduit-protocol` reads the Dart source to assert that no call site
hard-codes either number. The desktop binds `0.0.0.0:9527` and serves TLS
separately on 9531, with a test pinning the bind address to the plaintext
port.

## Self-hosting the relay

The relay is optional. Conduit works on a local network without it.

```bash
cp .env.example .env          # then set RELAY_TOKEN
mkdir -p secrets
openssl rand -hex 32 > secrets/hmac_secret
chmod 600 secrets/hmac_secret
docker compose up -d --build
```

`RELAY_TOKEN` is required, and the relay refuses to start without it. The
relay fails closed on a missing master secret rather than generating a
throwaway one, and it refuses to start if the certificate and key do not
belong together.

**Persist the volumes.** `docker-compose.yml` mounts three, because every path
the relay writes to is state that would otherwise be destroyed on the next
`up`, `pull` or image rebuild.

| Mount | What it holds | What is lost without it |
|---|---|---|
| `./secrets:/data/secrets` | the master HMAC secret | A fresh secret invalidates every deployed key and certificate pin. |
| `relay-nonces:/data` | the replay-protection nonce cache at `/data/nonces.json`, flushed every 30 seconds | The cross-restart replay guarantee does not hold, so a message captured inside the 30-second freshness window can be replayed once after every restart. |
| `relay-certs:/data/certs` | `cert.pem` and `key.pem` | The relay regenerates a self-signed certificate, which changes the SPKI pin every client has to verify. |

Self-signed certificates default to SANs of `localhost`, `127.0.0.1` and
`::1`. That is the most common cause of a relay that works locally and fails
everywhere else: set `RELAY_TLS_HOSTNAME` to the name clients actually use
(and `RELAY_TLS_EXTRA_SANS` for alternates) and restart the relay to
regenerate the certificate.

[`docs/relay-tls.md`](docs/relay-tls.md) is the full runbook: the pin
algorithm, bringing your own certificate, ACME, renewal, and distributing the
pin to clients.

## Cost and hosting

The stack is free and open source. There is no paid API, no managed database,
no telemetry SDK and no license-key-gated dependency in any of the three
ecosystems: 70 direct Rust dependency declarations across the three
`Cargo.toml` files and 750 packages in the single root `Cargo.lock`, 37 npm
dependencies in `apps/desktop/package.json`, and 20 Dart dependencies in
`apps/mobile/pubspec.yaml`.

Several choices exist specifically to keep a cost off a hosted service:
secrets go to the OS keyring on the desktop and the platform keystore on the
phone, discovery uses `mdns-sd` and `multicast_dns` rather than a rendezvous
service, QR scanning runs on the device, and at-rest storage is SQLCipher
compiled from source rather than a managed database.

Those choices move cost onto your hardware rather than removing it. The QR
scanner needs a one-time model download on Android before it works offline,
the keyring is only as durable as the operating system's credential store
(a desktop with no secret service falls back to a permission-restricted key
file), and SQLCipher compiled from source is exactly what makes the desktop
Rust build depend on a vendored OpenSSL.

## Testing

Run every `cargo` command from the repository root with `-p`, and run `npm`
and `flutter` commands from their own component directory.

| Suite | Working directory | Command | Current result |
|---|---|---|---|
| Relay | repository root | `cargo test -p relay` | 189 passed, 0 failed |
| Protocol | repository root | `cargo test -p conduit-protocol` | 266 passed, 0 failed |
| Desktop Rust | repository root | `cargo test -p conduit` | 696 passed, 0 failed. Needs the vendored OpenSSL. |
| Desktop typecheck | `apps/desktop` | `npx tsc --noEmit` | exit 0 |
| Desktop unit | `apps/desktop` | `npm test` | 216 passed, 20 files |
| Desktop e2e | `apps/desktop` | `npm run test:e2e` | Playwright against a plain browser. Tauri IPC is unavailable, so it covers the shell and nothing behind it. |
| Mobile unit | `apps/mobile` | `flutter test` | 43 passed |
| Mobile analyze | `apps/mobile` | `dart analyze --fatal-infos` | 362 lint infos, 0 errors, 0 warnings. Exits non-zero because `--fatal-infos` treats infos as fatal, which is why the CI step is non-blocking. Plain `dart analyze` exits 0. |
| Rust format | repository root | `cargo fmt --all -- --check` | clean |
| Mobile integration | `apps/mobile` | `flutter test integration_test` | needs a connected device or a running emulator |

The coverage is not uniform, and the gaps are worth stating plainly rather
than reading a green run as more than it is:

- **There are no Rust integration tests.** No `tests/` directory exists
  anywhere in the repository. Every Rust test is an in-crate unit test, so
  none of them can exercise the crate the way a consumer would.
- **No test exercises real Tauri IPC.** The mock at
  `apps/desktop/__mocks__/tauri.ts` is three lines and its `invoke` returns
  `undefined` for every command.
- **There is no cross-language conformance test.** Nothing verifies that
  `types.rs`, the generated TypeScript and the generated Dart agree. The
  schema-sync tests link the Rust types to `schema.json` and check a few
  textual invariants in the Dart source; beyond that, the guarantee is two
  regeneration steps and discipline.
- **The Playwright suite cannot reach the backend at all.** It proves the
  React shell renders and degrades. It proves nothing about a Rust command.
- **`dart format` is checked and is not blocking.** Most of the Dart tree is
  unformatted, so do not assume a Dart diff is format-clean.

[`docs/TESTING.md`](docs/TESTING.md) covers all of this in detail, including
what each suite does cover and a prioritised list of the gaps.

## Documentation

| Document | What it covers |
|---|---|
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | Component design, ownership, data flow and the module boundaries. |
| [`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) | The three working directories, prerequisites, build, lint, code generation, and the checklist for a cross-layer protocol change. |
| [`docs/TESTING.md`](docs/TESTING.md) | Every test command, what each suite covers, and what is not covered. |
| [`docs/relay-tls.md`](docs/relay-tls.md) | Relay certificates, SANs, the pin algorithm, ACME, renewal and troubleshooting. |
| [`docs/decisions/`](docs/decisions/README.md) | Architecture decision records, with an index and a one-line summary per decision. |
| [`packages/protocol/PROTOCOL.md`](packages/protocol/PROTOCOL.md) | The normative wire protocol specification, message by message. |
| [`packages/protocol/README.md`](packages/protocol/README.md) | The `conduit-protocol` crate specifically. |
| [`SECURITY.md`](SECURITY.md) | Threat model, what is and is not defended against, and how to report a vulnerability. |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | How to get the project running, which checks to run, and the conventions a change is expected to follow. |
| [`CHANGELOG.md`](CHANGELOG.md) | Release history and the current list of known limitations. |
| [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md) | The Contributor Covenant, and how to report a violation. |
| [`LICENSE`](LICENSE) | The MIT licence text. |

## Contributing

Conduit is maintained by one person outside working hours, so a first response
to an issue or a pull request can take a while, and review tends to be detailed
and slow. Read [`CONTRIBUTING.md`](CONTRIBUTING.md) before your first change:
it covers the setup, the checks, and the two invariants worth defending in
review, the single root `Cargo.lock` and the rule that generated files are
never hand-edited. Small, well-scoped pull requests are easier to land than
large ones.

## License

Conduit is released under the MIT licence. The full text is in
[`LICENSE`](LICENSE).

Only `conduit-protocol` declares `license = "MIT"` in its crate manifest so
far; the `conduit` and `relay` manifests do not yet carry a `license` field.
