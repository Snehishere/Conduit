# Testing

How to run every suite in Conduit, what each one actually covers, and — stated
plainly — what is not covered at all. For build setup see
[DEVELOPMENT.md](DEVELOPMENT.md).

- [1. Commands](#1-commands)
- [2. What each suite covers](#2-what-each-suite-covers)
- [3. Coverage reality](#3-coverage-reality)
- [4. What is well covered, and why it matters](#4-what-is-well-covered-and-why-it-matters)
- [5. Test-helper conventions](#5-test-helper-conventions)
- [6. Mobile integration tests](#6-mobile-integration-tests)
- [7. Prioritised coverage gaps](#7-prioritised-coverage-gaps)

---

## 1. Commands

Run every Rust command **from the repository root** with `-p <crate>`. Run npm
and Flutter commands from their own component directory. This is the most
common source of "command not found" / "no such package" in this project.

### Rust

```bash
# repository root
cargo test -p conduit-protocol     # 281 passed, 0 failed, plus 1 doctest
cargo test -p conduit-relay        # 247 passed, 0 failed, 1 ignored, plus 2 doctests
cargo test -p conduit              # 799 passed, 0 failed
```

| Crate | `-p` name | Working directory | Result |
|---|---|---|---|
| `packages/protocol` | `conduit-protocol` | repository root | 281 passed, 0 failed, plus 1 doctest |
| `services/relay` | `conduit-relay` | repository root | 247 passed, 0 failed, 1 ignored, plus 2 doctests |
| `apps/desktop/src-tauri` | `conduit` | repository root | 799 passed, 0 failed |

`cargo test -p conduit` requires the vendored OpenSSL — see
[DEVELOPMENT.md §3](DEVELOPMENT.md#3-the-vendored-openssl-problem).
`conduit-relay` and `conduit-protocol` do not.

Single test, by name:

```bash
cargo test -p conduit remote_input_unauthenticated_rejected_by_dispatcher
cargo test -p conduit server::handlers::remote_input::tests
cargo test -p conduit-protocol schema_    # all schema_ prefixed tests
cargo test -p conduit-protocol schema_oneof_branch_titles_are_unique
cargo test -p conduit-protocol schema_accepts_every_serialized_rust_type
```

Note the crate directory and the `-p` name do not match (`apps/desktop/src-tauri`
→ `conduit`), so `-p` is required, not optional.

### Desktop frontend

```bash
# apps/desktop
npx tsc --noEmit      # typecheck, no emit        -> exit 0
npm test              # vitest run                -> 249 passed (22 files)
npm run test:watch    # vitest, interactive
npm run test:e2e      # playwright test           -> browser shell only
npm run lint          # eslint src/
```

`npm test` and `npm run build` both invoke the `tsc` binary in
`node_modules/.bin`, which currently resolves to **TypeScript 7.0.2**, not the
`typescript@6.0.2` entry in `package.json`. See
[DEVELOPMENT.md §7](DEVELOPMENT.md#7-code-style). This matters when a test run
and a typecheck disagree.

### Mobile

```bash
# apps/mobile
flutter test                        # 190 passed
dart analyze                       # 0 errors, 0 warnings (366 infos)
dart format --output=none --set-exit-if-changed lib test integration_test
flutter test integration_test       # needs a device or running emulator
```

`dart format` is run by `scripts/lint-all.ps1` and by the CI `flutter` job, but
non-blocking in both (`-Optional` / `continue-on-error: true`). 63 of 74 Dart
files are currently unformatted.

### Lint

```powershell
# repository root
.\scripts\lint-all.ps1
```

Clippy for all three crates (`conduit`, `conduit-relay`, `conduit-protocol`),
the three Rust test suites, `flutter analyze`, `npx eslint src/` and `vitest`.
To run one crate's lint on its own:

```bash
cargo clippy -p conduit-relay     -- -D warnings
cargo clippy -p conduit-protocol  -- -D warnings
```

### Everything at once

`scripts/lint-all.ps1` is the aggregate. It accumulates per-step results rather
than short-circuiting, so one run reports everything that is broken, and it
exits non-zero if any blocking step failed. CI
(`.github/workflows/ci.yml`) is the merge gate; the script is its local
equivalent plus Playwright.


`lint-all.ps1` will report that step as failing. That is the expected state
until the failure in §5 is fixed.

---

## 2. What each suite covers

### `conduit-protocol` — 281 tests, plus 1 doctest

| File | Tests | Covers |
|---|---|---|
| `src/types.rs` | 226 | Serde round-trips for every message type; optional `protocol_version`; `const` tag serialisation; snake_case vs camelCase field naming; the port constants; the binary frame layout constants; **the 5 schema-sync invariants**; `PROTOCOL.md` content assertions; the mobile port mirror |
| `src/lib.rs` (`mod hmac`) | 55 | HMAC signing and verification, key derivation with a domain-separated KDF, per-device route-key derivation and the `RouteKeyring` (including a key id that disagrees with the sender, a key belonging to another device, and forgetting an unpaired device), and the replay-protection nonce cache (including cross-restart behaviour, per-device quota isolation under the global ceiling, and a store that fails closed when it cannot be read) |

There is also 1 doctest: a `compile_fail` case pinning the deleted unscoped
`hmac::check_replay` free function as a compile error (W6.27), so the deletion
itself is regression-tested.

The schema-sync tests are the load-bearing ones:

| Test | Enforces |
|---|---|
| `schema_oneof_branches_are_unambiguous` | no two `oneOf` branches accept the same JSON document |
| `schema_oneof_branch_titles_are_unique` | no two branches share a `title` — the invariant behind the duplicate-`ClipboardRequestMessage` bug |
| `schema_sample_set_covers_every_oneof_branch` | every branch has a sample in the sample set |
| `schema_accepts_every_sample_in_sample_set` | every sample validates against `schema.json` |
| `schema_accepts_every_serialized_rust_type` | **every message serialised by `types.rs` is accepted by `schema.json`** — the only automated link between the two |

Plus roughly 20 negative tests (`schema_rejects_*`) pinning individual schema
constraints: missing required fields per message, invalid enums, negative file
size, `file_progress` over 100, empty pairing token, `protocol_version` of 0,
unknown type tag, wrong `audio_format` const, `additionalProperties` on
`device_info`, and the allowance of unknown extra top-level fields.

There are also non-`schema_*` cross-language tests in `types.rs` that read the
Dart and Markdown sources as text and assert on their contents — e.g. that
`PROTOCOL.md` documents 9527 as plaintext and 9531 as TLS, that the discovery
example advertises `"wss_port": 9531`, and that
`apps/mobile/lib/services/websocket_service.dart` contains
`kLanWssPort = 9531` / `kLanWsPort = 9527` and hard-codes neither number in any
call site.

### `conduit-relay` — 247 tests, plus 2 doctests

The crate is a library with no `src/main.rs` and no `[[bin]]`. Its tests live in
four places: `src/suite.rs` is the in-crate suite written against the relay as a
whole, `src/tls.rs` keeps its own `mod tests` beside the code it exercises, and
`config.rs` and `service.rs` now carry narrow in-file test modules of their own
so a fix can be regression-tested without reaching through the suite. The
remaining modules (`connection.rs`, `route.rs`, `state.rs`, `health.rs`,
`metrics.rs`, `limits.rs`) are covered through the suite.

| File | Tests | Covers |
|---|---|---|
| `src/suite.rs` | 181 | Auth and the `relay_auth` handshake; the auth timeout **and the total auth deadline**; `relay_route` HMAC verification under a per-device key resolved through the `RouteKeys` trait; binary frame v2 parsing and the rejection of version `0x01`; replay protection and nonce persistence; health / metrics / `/pin` endpoints and the bearer-token behaviour that actually applies to each of them (`/health` and `/` always; `/metrics` only when a metrics token is configured); the `Overrides` → environment → default precedence and its fail-closed paths; rate and connection limits; the text and binary frame size ceilings at both the read and the application layer, each answered `message_too_large`; the outbound queue's per-connection byte budget; the release of the routing read lock before a slow forward; a superseded connection's inability to deregister the live one; inbound `protocol_version` enforcement; and an end-to-end WebSocket lifecycle against a real listener |
| `src/tls.rs` | 43 | Self-signed certificate generation and SAN handling, including the bounded validity window and the asserted key usages; `load_tls_context` refusing an expired or not-yet-valid supplied certificate, with the parser cross-checked against the `openssl` fixture; PKCS#8 / PKCS#1 / SEC1 key detection; the "no private key found in key PEM" path; key/cert mismatch refusal; `0600` tightening; SPKI extraction cross-checked against `openssl` |
| `src/config.rs` | 14 | The `MIN_SECRET_LEN` floor on every secret, the derived rather than copied health token, the empty-expected-token guard in `bearer_token_authorized`, `env_port` failing closed on an unparseable value, and the redacting `Debug` |
| `src/service.rs` | 10 | The TLS handshake budget, the WebSocket upgrade budget and its disarm on first write, admission ordering (rate limit → guard → cap) and the fact that every early return drops its connection slot |

`cargo test -p conduit-relay -- --list` reports 43 for `tls.rs` on Windows; the
`#[cfg(unix)]` tests do not compile here, and one of those listed is the
`#[ignore]`d fixture generator. There are 2 doctests in `lib.rs`'s module docs.

The suite lives in one file because it is written against the relay as a whole
rather than module by module; the doc comment at the top of `suite.rs` says so.
`tls.rs`, `config.rs` and `service.rs` are the exceptions — each hosts a narrow
module beside the code it exercises, for the cases where reaching through the
suite would hide what is actually being tested. The desktop crate is
`conduit-relay`'s only consumer,
and it hosts the relay in-process — `src/relay.rs` has its own 16 tests for the
`Overrides` it builds and the `RouteKeys` it implements — but those live in the
`conduit` crate, not here, so nothing in `conduit-relay`'s own suite exercises
the crate the way that consumer does. See §3.

### `conduit` (desktop Rust) — 799 tests, all passing

Test count per module, as reported by `cargo test -p conduit -- --list`
(799 total):

| Module | Tests | What it protects |
|---|---|---|
| `src/server/` | 227 | 145 in `server/handlers/*` (per-message-type handler behaviour, see §4) + 82 in `server/mod.rs` (connection lifecycle, pairing gates, broadcast filtering, protocol-version enforcement, **the authentication boundary**) |
| `src/commands/` | 123 | Every Tauri command, via `tauri::test::mock_builder` |
| `src/automation.rs` | 104 | Rule evaluation, the shell allowlist gate, trigger dispatch, and **the honest reporting of actions with no implementation** — five of seven actions used to return `success: true` while doing nothing |
| `src/encryption.rs` | 86 | Key resolution across keyring and 0600 key file, secret reuse, migration back into a recovered keyring |
| `src/storage.rs` | 72 | SQLCipher persistence, migrations, settings round-trips, the keyring-loss data path |
| `src/file_transfer.rs` | 53 | Chunking, path-traversal rejection, download-path validation |
| `src/security.rs` | 60 | Rate limiters, file-request validation, `allowed_commands` parsing (fail-closed) |
| `src/discovery.rs` | 20 | mDNS advertisement contents, the TXT-record consumption decision, and character-boundary-safe device-id truncation |
| `src/relay.rs` | 27 | The `RelayHost`: building `conduit_relay::Overrides` from the desktop settings, resolving the keyring-backed relay token and the desktop's own route key, and the `RouteKeys` implementation |
| `src/main.rs` → `mod integration_tests` | 13 | Bind address / port invariants |
| `src/sync.rs` | 6 | Sync engine state |
| `src/audio.rs` | 3 | Audio stream and playback state |
| `src/error.rs` | 3 | Error mapping |
| `src/tray.rs` | 2 | Tray setup |

> The module in `src/main.rs` is **named** `integration_tests`. It is not a
> Cargo integration test — it is an in-crate `#[cfg(test)]` module with
> `use super::*;`, exactly like every other test in the crate. It does not get
> the crate's public API surface and runs in the same binary. Nothing in this
> repository is a real Cargo integration test.

### Desktop frontend — vitest, 249 tests in 22 files

| Location | Files | Covers |
|---|---|---|
| `src/components/__tests__/` | 10 | `ContextMenu`, `EmptyState`, `FileCard`, `Onboarding`, `SearchBar`, `ShortcutsOverlay`, `Skeleton`, `StatusBadge`, `UnifiedSurfaces`, `WhatsNewDialog` |
| `src/components/settings/__tests__/` | 1 | `Settings` |
| `src/lib/__tests__/` | 2 | `toast`, `utils` |
| `src/__tests__/hooks/` | 7 | `useAutomation`, `useClipboard`, `useDevices`, `useDiscovery`, `useEncryption`, `useSearch`, `useWebSocket` |

The per-file split of the 249 is not recorded here: it is a `vitest` reporter
figure and it moves every time a `it()` block is added, so a hand-maintained
table of it would be wrong within a week. The file counts above are the useful
part and they are the ones that describe where coverage lives.

Config: `apps/desktop/vitest.config.ts` — `jsdom`, `globals: true`,
`setupFiles: ./src/__tests__/setup.ts` (one line: imports
`@testing-library/jest-dom/vitest`). `include` is `src/**/*.test.{ts,tsx}`, so
Playwright specs under `e2e/` are excluded.

`@tauri-apps/api/core` and `@tauri-apps/api/window` are aliased to
`apps/desktop/__mocks__/tauri.ts`, which is three lines:

```ts
import { vi } from 'vitest';

export const invoke = vi.fn();
```

### Desktop e2e — Playwright, browser only

`npm run test:e2e` from `apps/desktop`. 9 spec files in `e2e/`, plus
`fixtures.ts` as a shared helper:

`app-loads.spec.ts`, `automation-rule-crud.spec.ts`, `bubble-physics.spec.ts`,
`dock-navigation.spec.ts`, `error-empty-states.spec.ts`,
`keyboard-navigation.spec.ts`, `pairing-flow.spec.ts`,
`revision3-surfaces.spec.ts`, `settings-page.spec.ts`.

`playwright.config.ts` starts the Vite dev server on port 5173 and runs one
project, `chromium`. Its own header comment states the constraint:

> Tauri IPC calls are not available in the browser environment, so the app
> must gracefully degrade when `invoke()` fails.

### Mobile — `flutter test`, 190 tests in 26 files

All under `apps/mobile/test/`:

| Location | Files |
|---|---|
| `test/screens/` | 13 — one per screen (`automation_rules`, `calls`, `clipboard`, `discovery`, `files`, `home`, `messages`, `notifications`, `pairing`, `remote_input`, `screen_mirror`, `search`, `settings`) |
| `test/widgets/` | 5 — `ContextMenu`, `EmptyState`, `SearchBar`, `Skeleton`, `StatusBadge` |
| `test/services/` | 6 — `relay_route`, `websocket_service_relay`, `native_screen_capture`, `ios_screen_capture_contract`, `discovery_service`, `sms_service`, `notification_read_state` |
| `test/theme/` | 1 — `theme_provider` |

The service layer is no longer bare. The largest gap — that nothing tested the
WebSocket path — is now partly closed by
`test/services/websocket_service_relay_test.dart`, which drives the **real**
`WebSocketService` over a real loopback socket standing in for the relay and
inspects what actually leaves the phone. That is not the same as the real
in-process relay, and it is not claimed to be.

Two other gaps closed this round:

- **Dart↔Rust interop on the binary frame.** The v2 frame tests were previously
  self-consistent only (build and parse both in Dart), so a drift between
  `relay_route.dart` and `conduit_protocol::build_binary_frame` was undetectable.
  Whole-frame vectors from the Rust producer are now pinned on the Dart side.
  The reverse pin — a Rust test asserting the Dart-built frame — does not exist;
  `types.rs` claims it does.
- **Native screen capture.** `test/services/native_screen_capture_test.dart`
  covers the EventChannel subscriber (argument binding, frame normalisation, the
  legacy iOS shape, the size cap, and refusing another stream on the same
  channel). The Swift producer is pinned by a **source** assertion
  (`ios_screen_capture_contract_test.dart`) rather than a behavioural one —
  there is no Swift test target and `swiftc` cannot run on this host, so the
  iOS side is unbuilt and unrun.

Still untested at the unit level: the keyring, the camera, and the OS
integration behind `integration_test/`, which needs a device. The mobile
database has no injectable seam, so a persistence fix made this round is
recorded as **untested** rather than covered.

---

## 3. Coverage reality

Stated without softening:

| Claim | Reality |
|---|---|
| "Integration tested" | **There are zero Rust integration tests.** No `tests/` directory exists anywhere in the repository. Every Rust test is an in-crate `#[cfg(test)] mod tests`, so each one can only reach `pub(crate)` and private items — it cannot exercise the crate as a consumer would. (One in-crate module in `src/main.rs` is *named* `integration_tests`; it is still a unit test and is covered in §2.) |
| "The frontend is tested against the backend" | **No test exercises real Tauri IPC.** `__mocks__/tauri.ts` is a 3-line stub whose `invoke` is a bare `vi.fn()` returning `undefined`. Any test that depends on a command's real return value is either asserting on `undefined` or mocking the return itself. |
| "End-to-end tested" | **The Playwright suite runs in a plain browser and cannot reach Tauri at all.** It exercises layout, navigation, keyboard handling and the app's behaviour when `invoke()` rejects. It proves the shell renders and degrades. It does not prove a single Rust command works. |
| "The protocol is consistent across languages" | **There is no cross-language conformance test.** Nothing verifies that `types.rs`, `websocket.ts` and `protocol.dart` agree. The `conduit-protocol` tests link `types.rs` to `schema.json` and check a handful of *textual* invariants in the Dart source (two port constants, no hard-coded port literals). Beyond that, the only link between the three languages is two codegen invocations and a developer's discipline. |
| "The relay is tested against a real client" | **Partly.** The relay's unit tests still build their frames by hand, but the end-to-end lifecycle tests now assert the wire format a real client depends on: that a routed message arrives as a `relay_delivery` carrying the authenticated sender, and that a forwarded binary frame is re-framed as a well-formed v2 frame whose tag verifies under the sender's route key. The *receiver* half is tested in the desktop crate, which drives the real `"relay_server"` connection id through the real dispatcher. What is still missing is one test that puts a real client on a real socket and runs the whole path in a single process. |
| "Certificate pinning is tested" | **Yes, on the SPKI computation.** `services/relay/src/tls.rs::spki_vector_for_the_dart_client` and `apps/mobile/test/services/relay_route_test.dart` assert the same recorded certificate and the same pin, and the Rust side cross-checks the DER walk against `openssl`. That is what would have caught the client hashing `cert.der`. The pin *enforcement* path — the relay refusing to start on a configured-pin mismatch — is also covered. What is not covered is a live TLS handshake through the mobile client, which needs a device. |
| "`dart format` is clean" | **No.** 63 of 74 Dart files are unformatted. `scripts/lint-all.ps1` and the CI `flutter` job both run the check, but non-blocking (`-Optional` / `continue-on-error: true`), so nothing fails on it. |

What that means in practice: the Rust backend and the protocol crate are
genuinely well tested. The three seams where languages meet — Tauri IPC,
WebSocket dispatch on the client, and the shared message schema — are where the
tests stop.

---

## 4. What is well covered, and why it matters

It is worth being precise about this, because the coverage above is not uniform.

### Every `server/handlers/*.rs` has a populated test module — 145 tests

All eight files under `apps/desktop/src-tauri/src/server/handlers/` carry a
`mod tests` with real tests:

| File | Tests |
|---|---|
| `screen_mirror.rs` | 31 |
| `remote_input.rs` | 32 |
| `auto_rules.rs` | 18 |
| `files.rs` | 18 |
| `pairing.rs` | 19 |
| `mod.rs` | 11 |
| `notifications.rs` | 9 |
| `audio.rs` | 7 |

Every one of those modules was **empty** at one point. That is precisely how a
set of critical bugs survived: the handler was the only place the bug lived, and
the handler had no tests. Populating them is the single most valuable testing
change in this codebase's history, and the counts above are the reason it stuck.

They share `crate::server::handlers::test_helpers` in `mod.rs:158`, whose
`create_test_ctx()` builds a real in-memory SQLite database, runs the real
migration, and constructs the real `TokenStore` (with the production 60 s TTL so
expiry behaviour is exercised) and the real rate limiters. These are not
shallow mocks.

The `add_test_client` helper has an explicitly documented three-way contract
(`mod.rs:205-222`) distinguishing *paired*, *has-identity-but-untrusted*, and
*unpaired* — and that contract is what the tests in this section rely on. Choose the helper deliberately: `add_test_client` models a real connection's transient state (identity registered, secret not yet derived), which is not the same as "unpaired".

### The authentication boundary has exploit-shaped tests

`src/server/mod.rs` contains nine `rce_chain_*` tests that walk the actual
attack path rather than asserting on a helper. They are named for the threat
they model — an *unauthenticated* client must not be able to reach privileged
operations — and that property is the point:

| Test | Asserts |
|---|---|
| `rce_chain_unpaired_loopback_client_cannot_reach_automation_rule` | an unauthenticated loopback client cannot create a rule |
| `rce_chain_paired_client_may_still_create_a_non_shell_rule` | a paired client can create a benign rule |
| `rce_chain_paired_client_cannot_persist_a_shell_rule_by_default` | a paired client cannot persist a shell rule |
| `rce_chain_shell_gate_covers_every_action_field_alias` | the gate is not bypassable via a field alias |
| `rce_chain_shell_gate_allows_an_explicitly_allowlisted_command` | the allowlist actually permits |
| `rce_chain_triggered_path_refuses_a_legacy_blocked_rule` | the *triggered* path is gated too, not only the create path |
| `rce_chain_triggered_gate_allows_non_shell_and_unknown_rules` | the gate is not over-broad |
| `rce_chain_shell_gate_covers_automation_sync_batches` | the gate survives a batched sync payload |
| `rce_chain_execution_path_enforces_the_allowlist` | the allowlist is enforced at execution, not only at write time |

That last one matters most: a write-time-only gate is bypassable by any other
path that creates or mutates a rule.

### The keyring data-loss bug has a byte-for-byte integrity test

`src/encryption.rs:764` —
`healthy_keyring_gets_the_secret_and_no_key_file_is_created`:

```rust
let secret = resolve_secret(&keyring, &key_file, ACCOUNT).expect("must succeed");

assert_eq!(secret.len(), 64, "a 32-byte key as 64 hex characters");
assert_eq!(
    keyring.contents().as_deref(),
    Some(secret.as_str()),
    "the keyring must hold exactly what was returned"
);
assert_eq!(
    key_file.contents(),
    None,
    "a working keyring must not leave a plaintext key on disk"
);
```

This is the shape of the bug: the function returned a secret that was *not* the
one it had stored, so a subsequent read produced a different key and the
SQLCipher database became undecryptable. Comparing the returned bytes to the
stored bytes is the only assertion that catches it.

Around it, nine more tests in the same module pin the whole fallback matrix:
reuse-not-regenerate, write-failure fallback, a keyring that *silently ignores
writes*, key stability across launches while the keyring is broken, survival
across the keyring coming back, migration of the key file back into a
recovered keyring, retention when migration cannot be confirmed, a divergent
keyring entry being left alone, and a clear error when both stores fail.

### Schema drift is caught by five invariant tests

See §2. The critical one is `schema_accepts_every_serialized_rust_type`, which
serialises every message type from `types.rs` and validates the result against
`schema.json`. Because the schema is hand-maintained, this is the only thing
stopping the two from drifting.

### The mobile port bug is pinned from both sides

`conduit-protocol` reads the Dart source as text and asserts
`kLanWssPort = 9531` and `kLanWsPort = 9527` are present, and that **no call
site** in that file hard-codes either number. That is what caught the
`wss://$ip:9527` bug that made LAN pairing non-functional. The desktop side has
the mirror: `ws_bind_addr_matches_ws_port`,
`ws_bind_addr_is_not_the_tls_port`, and
`ws_bind_addr_parses_as_a_socket_address` in `src/main.rs`.

---

## 5. Test-helper conventions

`src/server/handlers/mod.rs` exposes three constructors for adding a client in
tests, and they are not interchangeable:

| Helper | Models | Use when |
|---|---|---|
| `add_test_unpaired_client` | connected, **no identity** in `ws_to_device_id` | asserting the auth gate rejects something |
| `add_test_client` | identity registered, shared secret **not yet** derived | the transient state a real connection passes through before `local_auth` |
| `add_test_paired_client` | fully paired | asserting delivery / broadcast |

This exists because a single original helper had contradictory callers, and a
test that picks the wrong one fails in a way that looks like a product
regression when it is not. `remote_input_unauthenticated_rejected_by_dispatcher_never_reaches_handler`
is the canonical example: it dispatches a real `remote_input` / `move` /
`dx: 500` / `dy: 500` frame and asserts the client is told `not_authenticated`
*and* that `ws_to_device_id` is still empty. The first assertion passing and the
second failing means the dispatcher is correct and the helper is wrong.

Reproduce:

```bash
cargo test -p conduit remote_input_unauthenticated_rejected_by_dispatcher
```

---

## 6. Mobile integration tests

Five files in `apps/mobile/integration_test/`:

| File | Lines | Exercises |
|---|---|---|
| `websocket_service_test.dart` | 656 | Handler registration and dispatch, encoding/decoding |
| `notification_service_test.dart` | 454 | In-memory notification flows against a real `DatabaseService` |
| `database_persistence_test.dart` | 429 | `sqflite_sqlcipher` open, migrate, write, read back, close |
| `encryption_service_test.dart` | 322 | X25519 key generation and shape, real crypto |
| `settings_connection_test.dart` | 172 | Settings screen + connection status with `shared_preferences` |

These use `IntegrationTestWidgetsFlutterBinding` and real platform channels, so
they need a device or a running emulator. **They do not run on `flutter test`.**

```bash
# apps/mobile
flutter devices                 # confirm a target is attached
flutter test integration_test

# Android emulator
flutter emulators
flutter test integration_test -d <emulator-id>

# physical Android over USB
adb devices
flutter test integration_test -d <device-id>

# iOS simulator (macOS host only)
open -a Simulator
flutter test integration_test -d <simulator-id>
```

`settings_connection_test.dart` is the closest thing to a full-stack test on
this side: it mounts `HomeScreen` and `SettingsScreen` with the real services
(`WebSocketService`, `CallService`, `ClipboardService`, `FileService`,
`NotificationService`) wired through `provider`.

None of these five is wired into any script or CI job — the CI `flutter` job
runs `flutter test`, which does not include `integration_test/`. They have to be
run manually against a device, which is why `lib/services/` and `lib/models/`
have no coverage in a normal `flutter test` run.

---

## 7. Prioritised coverage gaps

Ordered by the risk each one leaves uncovered. All are also tracked in
`docs/REMAINING_WORK.md`.

| # | Gap | Why it ranks here |
|---|---|---|
| 1 | **`src/hooks/useMessageHandlers.ts` has no tests** | This is the desktop's client protocol dispatcher — the direct counterpart to the Rust `server/handlers/*` modules that now have 145 tests. `useWebSocket.tsx` is covered by `src/__tests__/hooks/useWebSocket.test.tsx`, but the handler module next to it is not, so a malformed or mistyped message handler can still fail silently in the UI. |
| 2 | **No cross-language conformance test** (`types.rs` ↔ `websocket.ts` ↔ `protocol.dart`) | The only guarantee is codegen discipline. The `codegen` CI job regenerates and runs `git diff --exit-code`, which catches a stale artifact; a real conformance suite that validates the runtime behaviour of both generated clients is still missing. |
| 3 | **No test exercises real Tauri IPC** | `__mocks__/tauri.ts` returns `undefined` for every command. A richer mock — or a Tauri-driver-based test against a built app — would cover the 123 command tests' actual return shapes. |
| 4 | **Almost no tests for mobile `lib/services/` (14 files) or `lib/models/` (5 files)** | `test/services/relay_route_test.dart` covers one service; the other 13 and all 5 models have no unit tests. The bulk of `flutter test` is still 13 screens, 5 widget suites and the theme — presentation only. |
| 5 | **No single-process relay round-trip against a real client** | The relay's own frame tests build input by hand. The end-to-end lifecycle tests assert the wire format a real client depends on, and the desktop crate drives the real `"relay_server"` path, but no single test puts a real encoder on a real socket and runs phone → relay → desktop in one process. That is the gap that let a non-functional relay ship. |
| 6 | ~~**No certificate-pinning tests**~~ — **closed** | The SPKI pin is now computed and asserted on both sides against a shared fixture, cross-checked against `openssl` on the Rust side. This is the test whose absence let the mobile client hash `cert.der` for a year. A live TLS handshake through the mobile client still needs a device. |
| 7 | **No Rust integration tests** (no `tests/` directory) | Every Rust test is an in-crate unit test. Crate-level behaviour — the public API as a consumer sees it, and the `conduit-protocol` → `conduit` / `conduit-relay` boundary — has no coverage. |
| 8 | **Playwright cannot reach Tauri** | Structural, not fixable without a Tauri-aware harness. Documented so nobody reads a green Playwright run as backend coverage. |
| 9 | **`dart format` is checked but non-blocking; 63 of 74 files unformatted** | Both `scripts/lint-all.ps1` and the CI `flutter` job run the check and neither fails on it. Mechanical to fix, but until one does, every Dart diff carries unrelated churn. |
