# Remaining work

The single tracked backlog for Conduit. Every item the repository's own
documentation points at for outstanding work lives here: this is the file
`README.md`, `docs/DEVELOPMENT.md`, `docs/TESTING.md`, `docs/decisions/README.md`
and `.gitignore` mean when they say "see `REMAINING_WORK.md`".

It was generated from a full file-by-file audit of the tree at commit `48724e0`
on `main`. Every item carries a `file:line` reference so it can be verified
without re-reading the audit.

> **The relay section no longer satisfies that claim.** Since the audit was
> taken, the relay has been refactored from a standalone service into a
> **library** that the desktop app hosts in-process. `services/relay/src/main.rs`,
> the `Dockerfile`, `docker-compose.yml`, `.env.example`, `.dockerignore` and
> `scripts/build-relay.ps1` have all been deleted, and the code they described
> now lives in `services/relay/src/{lib,config,connection,route,state,health,
> metrics,limits,service,suite}.rs` plus
> `apps/desktop/src-tauri/src/relay.rs`. Every `W6` item therefore cites
> `file:line` references into files that **no longer exist**, and its line
> numbers mean nothing against the current tree.
>
> **W6 is STALE.** Do not action any W6 item as written. Re-audit W6 from the
> tree first. The items have deliberately been left in place and un-renumbered
> so the history of what was believed at `48724e0` is not silently rewritten; the
> top of the section lists what is now definitively settled.

## How to read this file

**Severity**

| Label | Meaning |
|---|---|
| `CRITICAL` | Data loss, security exposure, or a headline feature that silently does nothing. |
| `HIGH` | A user-visible feature is broken, or a security control is bypassable. |
| `MEDIUM` | Works, but is wrong, misleading, or fragile in a way that will bite. |
| `LOW` | Dead code, cosmetic drift, or a documentation nit. |

**Status**

- `[ ]` open, not started
- `[~]` partially done
- `[x]` done — and the completion note says what changed

**Workstreams**

| ID | Area |
|---|---|
| `W1` | Protocol and cross-layer contract |
| `W2` | Mobile client (`apps/mobile`) |
| `W3` | Desktop backend (`apps/desktop/src-tauri`) |
| `W4` | Desktop frontend (`apps/desktop/src`) |
| `W5` | Build, CI and dependencies |
| `W6` | Relay library (`services/relay`, hosted by the desktop app) |
| `W7` | Security and hardening |
| `W8` | Test coverage and verification |
| `W9` | Documentation accuracy |

## Critical path

If only five things get done, do these. Everything else is downstream of them.

1. ~~**W1.1** — hand the phone a stable `device_id`. Until this lands, every
   encrypted frame the phone sends is dropped by the desktop and the whole
   mobile encryption surface is inert.~~ **Done.**
2. **W5.1** — `scripts/fetch-openssl.ps1` with a pinned version and checksum.
   Until this lands, a fresh clone cannot build the desktop app and 61% of the
   Rust test suite cannot run in CI.
3. **W1.2** — move both relay clients onto `RelayRoute::signed_with`. The
   helper already exists and the relay's own tests already use it; this closes
   the key, field-set and byte-order divergence in one edit.
4. **W3.1** — stop the five automation actions from reporting success.
5. **W2.1** — give screen mirroring a `frame` handler and instantiate
   `ScreenMirrorService`.

---

## W1 — Protocol and cross-layer contract

`schema.json` is hand-maintained by design ([ADR-0010](decisions/0010-schema-json-is-hand-maintained-and-test-enforced.md)).
The five guard tests in `conduit-protocol` check schema-internal consistency;
only one test links the Rust types to the schema, and it skips the three types
where the drift actually is. The items below are that drift.

- [x] **W1.1 (CRITICAL)** The phone never learns its own device id, so every
  encrypted frame it sends is dropped. **Fixed.**
  `apps/mobile/lib/services/websocket_service.dart:656` sends
  `'source_device': _deviceId ?? 'mobile'`, and `apps/mobile/lib/main.dart`
  never calls `setDeviceId(...)` — the setter at `websocket_service.dart:106`
  is exercised only by `integration_test/websocket_service_test.dart:218`.
  The desktop resolves the shared secret from that field at
  `apps/desktop/src-tauri/src/server/mod.rs:605-615`, so it never resolves, logs
  a warning and drops. The phone cannot learn the id because
  `PairingAccept` (`src-tauri/src/server/handlers/pairing.rs:155-167`) does not
  carry one and `pairing/accept` has no such field in the schema.
  *Left to do:* ~~add `device_id` to `PairingAccept`~~ **done**, but the fallback
  in `mod.rs` is not exercised end to end against a real phone — see W8.1/W8.3.

  **What landed (2026-09-29, commit uncommitted on `main`):**

  - `PairingAccept` gained `device_id: Option<String>` (`types.rs`), documented
    as the only channel by which a peer learns the id the hub filed it under.
    Optional on the wire, so a pre-existing peer keeps parsing.
  - `schema.json` "Pairing Accept" gained the matching property; both generated
    clients regenerated from it (`websocket.ts`, `protocol.dart`). The
    schema-sync tests in `conduit-protocol` pass, which is the check that the
    hand-maintained schema still agrees with the Rust type.
  - `handlers/pairing.rs` populates it with the same `stable_id` it persisted
    and mapped, so the value cannot disagree with the `devices` row.
    While there, the fabricated `battery: Some(100)` on the desktop's own
    `DeviceInfo` became `None` (part of W3.16).
  - The decrypt path is now `WsServer::resolve_sender_secret`, a named,
    testable unit that resolves in order of trust: the connection's own
    `ws_to_device_id` entry first, the peer's claimed `source_device` only when
    that did not resolve. A claim can no longer override a live connection
    identity, so one socket has exactly one identity.
  - Mobile captures `device_id` on `pairing/accept`, persists it under
    `paired_device_id` in the platform keystore, restores it on startup before
    `autoConnect()`, and clears it on `pairing/revoke` and via a new
    `clearDeviceId()` for an explicit unpair.
  - 8 new tests: 2 on the accept frame (that it names the persisted
    `devices.id`, that the field is omitted rather than null when absent) and 6
    on `resolve_sender_secret` (a placeholder claim does not cost a mapped
    socket its own secret; a claim cannot borrow another device; a stale
    mapping does not strand a peer; an unmapped socket with no usable claim
    resolves to nothing; and a reconnecting peer resolves by claim without
    re-pairing — the last is deliberate and documented, because refusing it
    would force a QR re-pair after every hub restart).

  **Verified:** `cargo fmt --all --check` clean · `cargo clippy --workspace
  --all-targets` 0 warnings · `conduit-protocol` 266 · `relay` 189 ·
  `conduit` 707 (was 699) · `npx tsc --noEmit` 0 · `npm test` 216/216 ·
  `npm run generate` clean · `flutter test` 43/43 · `dart analyze` 362 infos,
  0 errors, 0 warnings (unchanged; the one hit in a file touched here is
  pre-existing at `websocket_service.dart:791`).

  **Not covered by this change:** the phone's `source_device` is now correct,
  but nothing yet asserts the *round trip* — that an envelope sealed on a
  device and opened on its peer decrypts. W8.2/W8.3.

- [x] **W1.2 (RESOLVED — was stale)** Both relay clients signed `relay_route`
  in a way the relay rejected on three axes. **Already fixed** by the relay
  refactor; re-audited axis by axis and none of the three defects is present.

  The entry carried its own warning that its axes were derived against the
  deleted `services/relay/src/main.rs` and a `SigningKeyring` that no longer
  exists. Re-verified against the current tree:

  1. **Key** — the relay has no shared keyring. It resolves per device
     (`services/relay/src/route.rs:125`), after requiring `from_device_id` to
     equal the authenticated connection identity (`route.rs:110`). The desktop
     calls the existing signer `RelayRoute::signed_with` with that device's own
     key (`server/mod.rs:1174`). The phone derives its own from the pairing
     secret (`websocket_service.dart:940` → `relay_route.dart:74`). The relay
     token appears in exactly one place in mobile — `relay_auth`.
  2. **Field set** — both sides send all seven signed fields including `key_id`.
  3. **Byte order** — `canonicalSigningString` (`relay_route.dart:97`) sorts
     recursively to match the verifier's `BTreeMap` re-serialisation.

  One description correction: the relay does **not** look the key up by
  `key_id`. `key_id` is signed and must equal `from_device_id`, but lookup is
  under the authenticated connection id — so a signed `key_id` is checked, not
  trusted as a selector.

  What the audit got right and the earlier fix had no test for: the Dart and
  Rust signing paths had never been pinned to each other. They are now —
  `apps/mobile/test/services/relay_route_test.dart` asserts the exact canonical
  string, HMAC and route key, and a new
  `websocket_service_relay_test.dart` drives the real `WebSocketService` over a
  real socket to check what actually leaves the phone. Reverting each of the
  three axes in turn makes those tests fail.

- [x] **W1.3 (RESOLVED — was stale)** The phone emitted the superseded binary
  frame version. **Already fixed**; the cited line is now the *receive* path.

  The send path builds a v2 frame (`websocket_service.dart:997` →
  `relay_route.dart:257`) with the version literal at `relay_route.dart:218`, a
  strictly-increasing per-connection sequence and a 32-byte tag. The relay's
  checks are at `services/relay/src/route.rs:185` and `:195`, not the deleted
  `main.rs:1968`. The tag is keyed by the sender's per-device route key, which
  the phone legitimately holds, and the MAC input is
  `from_device_id || 0x1F || frame[0..21] || payload`.

  The Dart v2 tests were previously self-consistent only — build and parse both
  in Dart — so Dart↔Rust drift was undetectable. They are now pinned to whole
  frames produced by `conduit_protocol::build_binary_frame`, which is the same
  producer the relay re-frames with.

  The LAN-direct chunk frame in `file_service.dart` was already correct and is
  untouched.

  The `// Right side of the pin` comment in `types.rs` claims the two are pinned
  to a shared vector. Until now only the Dart half was; the Rust half is worth
  adding so the pin is two-way.

- [x] **W1.4 (RESOLVED)** Mobile certificate pinning can never succeed. **Fixed.**
  The phone hashed the whole DER certificate while the relay publishes an SPKI
  pin at `GET /pin`, so the two could never match and every pinned connection
  was rejected. `apps/mobile/lib/services/relay_route.dart` now carries a
  minimal DER walker (`extractSpkiDer`) that pulls the SubjectPublicKeyInfo out
  of `cert.der`, and `computeSpkiPin` produces the `sha256/<base64>` form the
  relay publishes. The desktop now has the pin target this item asked for: the
  `relay_cert_pin` setting, enforced by the relay at startup
  (`service.rs` refuses to start on a mismatch).

  Both halves are pinned against each other, which is what stops them drifting
  again: `services/relay/src/tls.rs::spki_vector_for_the_dart_client` and
  `apps/mobile/test/services/relay_route_test.dart` assert the same recorded
  certificate and the same SPKI pin, and the Rust side cross-checks the
  extraction against `openssl`. The vector is a fixture
  (`services/relay/tests/fixtures/`) rather than a generated certificate,
  because two sides handed different certificates would prove nothing.

  Residual, tracked as W7.2: the mobile client still re-pins on mismatch during
  pairing rather than failing.

- [ ] **W1.5 (HIGH)** `automation/rule` is flat in the schema and nested on the
  wire. `schema.json` "Automation Rule" (and the generated
  `apps/desktop/src/types/websocket.ts:327-335`) has `id`, `name`, `trigger`,
  `rule_action` at the top level. The real wire format is a nested `rule`
  object, accepted by `apps/desktop/src-tauri/src/automation.rs:576-607` and
  sent by `apps/desktop/src/hooks/useAutomation.ts:143`. The generated type
  describes a frame nobody sends.
  *Left to do:* correct `schema.json` and regenerate; also replace the
  `rules: Array<any>` at `websocket.ts:360` with a real definition.

- [ ] **W1.6 (HIGH)** `actionType` `move` exists in Rust, in the UI and in the
  injector, but not in the schema. `types.rs` validates 5 values; `schema.json`
  lists 4. The generated TS therefore rejects a value the desktop sends
  (`apps/desktop/src/components/screen-mirror/ScreenMirror.tsx:13, 193`), which
  is why that file widens the union by hand.
  *Left to do:* add `"move"` to `schema.json`; regenerate.

- [ ] **W1.7 (HIGH)** `file/progress` carries `percent` in the schema but the
  phone reads `chunks_received` (`apps/mobile/lib/main.dart:232`), which is
  unmodelled. `chunks_received` is the more useful of the two; decide which is
  canonical and model both or one.

- [ ] **W1.8 (HIGH)** Scroll input is dead on both screen-mirror and
  remote-input return paths. The schema defines `dx`/`dy`; the Dart handlers
  read `delta` (`apps/mobile/lib/main.dart:321, 339`), and
  `MainActivity.kt:96` reads `delta` too — so injected scroll is always `0.0`.

- [ ] **W1.9 (MEDIUM)** Types present in Rust and `PROTOCOL.md` but absent from
  the schema, therefore absent from both generated clients:
  `RemoteInputKey`, `AudioPlaybackStopped` (which the desktop emits in
  production), and `fps` on `screen_mirror/start`.

- [ ] **W1.10 (MEDIUM)** The phone sends `automation/rule_remove`
  (`apps/mobile/lib/services/automation_service.dart:196`); the desktop only
  handles `automation/delete` (`server/mod.rs:823`) and answers `other` with
  `unknown_action` (`mod.rs:855`).

- [ ] **W1.11 (MEDIUM)** The desktop sends `file/chunk` (base64,
  `handlers/files.rs:588, 622`) and `file/complete` (`handlers/files.rs:688`);
  the phone discards both (`apps/mobile/lib/main.dart:211-216`, and there is no
  `case 'complete'` at `main.dart:206-243`).

- [ ] **W1.12 (MEDIUM)** `clipboard/request` is defined at
  `packages/protocol/src/types.rs:199-210` and passes validation
  (`src-tauri/src/security.rs:384, 490`), but the hub only handles
  `("clipboard", "sync")` (`server/mod.rs:762`). A request gets
  `unsupported_message`.

- [ ] **W1.13 (MEDIUM)** Four relay message types sit in the generated union
  (`websocket.ts:413-433`) for a client that is compiled out at
  `src-tauri/src/main.rs:304-305`. `relay_auth_ok` and `relay_auth_rejected`
  are handled by neither client; the desktop answers the relay with
  `unsupported_message`.
  > **⚠ AFFECTED BY THE RELAY REFACTOR.** The premise "a client that is compiled
  > out" is false as written. The desktop now runs a live relay client, because
  > it hosts the relay itself (`src/relay.rs`, `RelayHost`) and joins it over
  > the loopback-only plaintext listener on 9531. What was an inert code path is
  > now an in-process network client on every launch when `relay_enabled` is
  > true, which is the default. Re-audit whether the four types are still
  > unhandled.
  *Left to do:* decide whether to finish the relay client or remove the
  message types from the schema.

- [ ] **W1.14 (MEDIUM)** `tv` and `watch` are accepted by the validator
  (`src-tauri/src/security.rs:394-395`) but have no handler, so they burn
  rate-limit budget and land in the catch-all at `server/mod.rs:899-914`.

- [x] **W1.15 (RESOLVED)** The receiver of a relayed message gets no authenticated
  attribution. **Fixed — and the investigation found this was worse than a
  missing attribution: the feature did not work at all.**

  What the audit recorded as an attribution gap was in fact a dead path. The
  relay forwarded the bare inner payload and the desktop dispatched it under the
  literal `"relay_server"`, which is in no pairing registry, so
  `is_trusted_peer` returned false and **every** inbound relayed message was
  refused with `not_authenticated`. A phone on another network could not deliver
  a notification at all. `resolve_sender_secret` also fell through to the
  unauthenticated `source_device` self-claim. Every existing test drove
  `handle_message` with a `ws_*` id the test had itself registered, so the relay
  id was never exercised; `relay_delivery` existed only as a branch in the
  mobile client with no producer anywhere in the tree.

  The fix, and why it is one change rather than three:

  - `RelayDelivery` in `packages/protocol/src/types.rs` — the envelope carrying
    the relay's verified attribution. Produced only by the relay, after
    `handle_relay_route` has checked `from_device_id` against the identity the
    connection authenticated as. It is deliberately *not* signed: the trust is
    the relay's, and a client cannot mint one that says anything the relay did
    not verify.
  - `services/relay/src/connection.rs` wraps the verified payload in it instead
    of forwarding `route.payload` alone, and the binary path now re-frames rather
    than stripping the v2 header (which made relayed file chunks undecodable).
    `conduit_protocol::build_binary_frame` is the single producer of a v2 frame.
  - The desktop unwraps at `handle_message`, re-enters as the *sender*, and
    resolves that device against the pairing registry — so `is_trusted_peer` and
    `resolve_sender_secret` now have a real identity to work with. An unpaired
    sender is refused before dispatch.
  - The mobile side already had the `relay_delivery` branch and needed no change;
    it had been written against an envelope nothing produced.

  Regression tests that would have caught this: the relay suite asserts the
  envelope and its attribution, and that a forwarded binary frame is re-framed
  and re-tagged; the desktop suite drives the real `"relay_server"` id end to
  end, asserting a paired phone's relayed notification reaches storage, that an
  unpaired sender's does not, and that the relay socket is never itself a
  trusted peer.

  Not done, and deliberately: there is no `relay_delivery` version negotiation.
  The envelope is only ever produced by a relay and consumed by a client of the
  same build generation, and inventing a version byte for it would be ceremony
  around a case that does not arise.

- [ ] **W1.16 (LOW)** `screen_mirror/hologram` (`handlers/screen_mirror.rs:1249`)
  has no Dart branch. `automation/triggered` is never emitted outbound by the
  desktop, so `apps/mobile/lib/services/automation_service.dart:178` and
  `main.dart:290-295` are unreachable.

- [x] **W1.17 (RESOLVED)** A routed `error` had no client requirement, so the
  phone received one and dropped it. **Fixed on both sides**, and the desktop
  half had to land first or the client half would have had nothing to receive.

  The gap was invisible because a LAN refusal and a relayed refusal are the same
  `error` frame on the wire and arrive by different routes. On the LAN the phone's
  socket-level listener sees a bare `type: "error"` and returns. Through the
  relay the *outer* type is `relay_delivery`, so that listener never sees it: the
  frame reaches `_handleMessage`, which had no `error` arm and fell through to
  `_messageHandlers['error']` — which no caller registers. So a phone behind the
  relay was throttled, refused, and told nothing, while still reporting itself
  connected.

  `relay_delivery` has never been documented in `PROTOCOL.md` — it is a
  relay-produced envelope (`packages/protocol/src/types.rs`, produced only after
  `handle_relay_route` returns `Ok`) and the spec has no section for it at all.
  §9.2 of `PROTOCOL.md` now carries the requirement in words: **a client must
  handle `error` inside `relay_delivery`, after unwrapping, whether the payload
  arrived sealed or in the clear.** `pairing` is exempt from sealing because it is
  what establishes the secret, so a refusal about pairing arrives unwrapped and a
  refusal about anything else arrives sealed; both shapes are live and both are
  pinned.

  Implementation is W2.27. The desktop egress that made a routed refusal
  possible at all is W3.27.

---

## W2 — Mobile client

- [x] **W2.1 (RESOLVED)** Screen mirroring never delivered a frame. **Fixed** —
  and only in one of the two directions the audit did not distinguish.

  The audit's claims were largely wrong. `screen_mirror_service.dart` *is*
  instantiated (`screen_mirror_screen.dart:32`), the `frame` branch *does*
  exist (`screen_mirror_service.dart:24`), and `registerHandler` appends to a
  list rather than replacing, so the two handlers coexist. The `hasDesktop`
  gate it blamed is now derived from `connectedDevices`
  (`actions_menu_sheet.dart:22`), not hardcoded false.

  **Direction A (phone views the desktop) was never broken.**

  **Direction B (desktop views the phone) was broken in two independent
  places**, both now fixed:

  - **Nothing subscribed to the native frames.** Android and iOS push frames on
    the shared `com.conduit.mobile/events` channel with argument
    `screen_mirror`; `receiveBroadcastStream` existed only for `calls`, `sms`
    and `notifications`. The frames were produced and discarded.
    `apps/mobile/lib/services/native_screen_capture.dart` is the missing
    subscriber, attached for the app's lifetime from `main.dart` — the relay must
    outlive any one screen, since the desktop may request a stream while the
    user is elsewhere.
  - **Every relayed frame was dropped.** `relay_frame` resolved the sender
    through `ws_to_device_id`, but `unwrap_relay_delivery` dispatches a relayed
    frame under the *sender's device id*, and a relay-only device has no LAN
    socket, so the lookup returned `None` for every viewer. Attribution now
    falls back to accepting the client id as a device id **only** when the
    pairing registry holds a row for it with a derived secret — a strictly
    higher bar than the string match it replaces. `is_trusted_peer` already
    handled this case; `relay_frame` did not.

  **Still open, and it blocks the UI path:** a relay-only target cannot be
  *asked* to start. `resolve_target_client` resolves through `ws_to_device_id`,
  i.e. it needs a socket, so the desktop refuses with `capture_stopped` and
  never touches the relay socket. Closing it means sending `start` over
  `relay_route` — the auth boundary, and a design decision rather than a fix.
  Pinned as a known gap by
  `known_gap_a_relay_only_target_cannot_be_asked_to_start`.
  > **Narrowed by W3.25, still open.** `fan_out_to_relay` now reaches every
  > paired device with no socket here, so anything the desktop *originates* —
  > including the `screen_mirror/frame` the viewer would then be receiving —
  > arrives. What is still missing is the **directed request**: `start` is
  > addressed to a device, and `resolve_target_client` still resolves it against
  > a connection map. The gap test above is untouched by that change and still
  > describes the code.
  Android capture still lives in an activity rather than
  `ConduitForegroundService`, so an Android rotation still ends mirroring
  (`MainActivity.kt:634, 643-649`).

- [x] **W2.2 (RESOLVED)** Discovery results were discarded. **Fixed** — and the
  two halves were separate defects in separate files.

  **Mobile (`hasDesktop` always false).** The desktop's announce carries
  `device_id`/`device_name`/`device_type`
  (`server/mod.rs:1725-1738`), `WebSocketService` passes the frame through
  verbatim (`websocket_service.dart:920-929`), and the home-screen handler read
  `map['id']`/`map['name']`/`map['type']`. No announce can contain those keys,
  so `_deviceMap` was never written. It now reads the wire names — the same ones
  the pairing handler beside it already used. `hasDesktop` is computed at
  `actions_menu_sheet.dart:22`.

  **The dropped TLS port.** `setAdvertisedWssPort` was declared and never
  called, so every LAN dial fell back to the literal 9531. The chain was longer
  than it looked: the service layer already threaded it correctly
  (`pairFromCode(wssPort:)` → `lanUrl`), `DiscoveredDevice` carried it, and
  `PairingScreen` simply had no field to receive it, so
  `discovery_screen.dart` could not pass it. Added the field and threaded it
  through. `port` is still passed and still means the plaintext SRV port — it is
  shown in the UI so the entry reads as it appeared on the network.

  **Not fixed, and it is a real remaining hole:** `start_discovery`
  (`commands/pairing.rs:110-113`) builds a `DiscoveryService` and calls
  `.start()` without `set_app_handle`, unlike `main.rs:405`. After a
  `stop_discovery`, a later `start_discovery` installs a browse loop that
  resolves peers and has nowhere to emit — results are discarded again. There is
  now a `warn!` at the discard point so it is at least visible.

  Also found while auditing, unlisted here before: `DiscoveredDevice` carries no
  `service_name`, but `mdns-device-removed` emits an mDNS fullname, so the exact
  -match removal branch can never fire and removal depends on an instance-label
  heuristic. Mobile's mDNS service uses the SRV hostname as device identity
  rather than the advertised `device_id` TXT property, which means
  `deviceLost()` cannot match a `discovery/remove` id and dedup collides when
  two peers share a hostname. Both are behavioural changes to files this pass
  did not own.

- [x] **W2.3 (RESOLVED)** SMS on Android was real but its data was
  mangled. **Fixed, and worse than documented.**
  The audit says messages get a synthetic id, a "now" timestamp and
  `isOutgoing = false`. In fact the whole load **aborted on the first record**,
  leaving an empty inbox and an error toast: `read` came back as a bool and Dart
  cast it `as num?`.

  Four separate mismatches, field by field:

  | Dart read | Android emitted | Effect |
  |---|---|---|
  | `_id` | *(absent)* | synthetic `'${address}_${DateTime.now()}'` — **changed on every reload** |
  | `date` (ms) | `timestamp` (already `date/1000`, i.e. **seconds**) | key missing → fell back to "now"; units disagreed by 1000× even had the key matched |
  | `type` (int, `== 2`) | `is_outgoing` (**bool**) | key missing → `1 == 2` → **everything incoming** |
  | `as num?` | `read` (**bool**) | **throws** → whole load caught → empty inbox |

  Android now emits a stable `sms_<_id>`, and the Dart parser accepts both
  spellings and both time units rather than assuming one.

  The iOS `TypeError` (`result([])` where Dart does `invokeMethod<String>` +
  `jsonDecode`) is fixed too — the parser now accepts an already-decoded list —
  but it was **unreachable**: `permission_handler` denies SMS on iOS, so
  `loadThreads` is never called there.

  Also fixed alongside: `_addIncomingToLocal` wrote the wrong thread to SQLite,
  because it indexed the list *after* re-sorting it, so a message that moved its
  thread upward persisted a different thread's rows. **This one has no test** —
  `DatabaseService` is a concrete class with a private handle and no injectable
  seam, and `sqflite_common_ffi` is not a dev dependency. Recorded here as an
  untested correctness fix rather than presented as covered.

- [ ] **W2.4 (HIGH)** The mDNS multicast permission is missing.
  `android/app/src/main/AndroidManifest.xml` declares `ACCESS_WIFI_STATE`,
  `ACCESS_NETWORK_STATE` and `NEARBY_WIFI_DEVICES` but **not**
  `CHANGE_WIFI_MULTICAST_STATE`, which `multicast_dns` needs to receive
  multicast at all. Discovery will not work on many devices.

- [ ] **W2.5 (HIGH)** Remote input is one-directional in practice. The desktop
  emits `move`, `click`, `scroll`, `key`, `event`, `start`, `stop`
  (`handlers/remote_input.rs:381-742`); `main.dart:327-342` handles only
  `touch`, `key` and `scroll`. `click` and `move` have no branch, and
  `MainActivity.kt:62-98` has no `injectMouseMove`/`injectMouseClick` handler
  even though `ConduitAccessibilityService.kt:121` defines one.

- [ ] **W2.6 (HIGH)** Calls are a stub in both directions.
  `CallService.answerCall` (`call_service.dart:132-149`) and `rejectCall`
  (`:152-160`) mutate local state and send nothing, so a call answered on the
  phone is invisible to the hub. `forwardCall('desktop')` targets a device id
  that does not exist (`calls_screen.dart:294`,
  `widgets/incoming_call_overlay.dart:97`). `setAudioRoute` (`:116-129`) sends
  `audio/route`, which `handlers/audio.rs:11-166` has no arm for.

- [ ] **W2.7 (HIGH)** The file-accept wait loop never waits.
  `file_service.dart:246-258` polls for a `transferring` or `complete` state,
  but `sendFile` already set `transferring` at `:216`, so it breaks on the first
  iteration and blasts every chunk without waiting for `file/accept`.

- [ ] **W2.8 (HIGH)** Touch injection targets the wrong device.
  `screens/screen_mirror_screen.dart:157-172` sends taps as 0..1 fractions of the
  widget, then `screen_mirror_service.dart:78-92` invokes the **phone's own**
  `injectTouch`. The frames are the **desktop's** screen, so tapping it injects
  a tap into the phone.

- [x] **W2.9 (RESOLVED)** iOS frame payloads did not match the Dart reader.
  **Fixed**, and it was a *separate* defect from W2.1 on the same hop — same
  native EventChannel, different cause. W2.1 was "nothing subscribes at all"
  (breaking Android and iOS); this was "the iOS producer's payload disagrees
  with the reader" (breaking iOS only, and only once a subscriber exists).
  Fixing one would not have fixed the other.

  `AppDelegate.swift` emitted `{"type":"screen_mirror_frame","frame":…,"width",
  "height","timestamp"}`. The reader requires `action == 'frame'` and reads
  `msg['data']`, and Android's shape matched neither. iOS now emits the
  canonical shape; the reader also still accepts the legacy one, so a phone
  that has not been updated keeps working.

  It also ignored the `quality`/`fps` arguments entirely, hardcoding
  `maxWidth = 640` and `compressionQuality: 0.5`, and had **no frame-rate
  throttle at all** while building a `CIContext` per sample buffer. It now
  honours both arguments using Android's ladder and hoists the context to one
  per session.

  The Swift has no test target and `swiftc` cannot run on this host, so the
  producer/reader contract is pinned by a source assertion in
  `ios_screen_capture_contract_test.dart` rather than a behavioural one. That
  is weaker than it looks and is called out here on purpose: the Swift is
  unbuilt and unrun.

- [ ] **W2.10 (HIGH)** Two native methods have no Android implementation:
  `getApnsToken` (`main.dart:88`, no handler at `MainActivity.kt:62-99`) and
  `setPhoneProfile` (`automation_service.dart:280`), which makes the entire
  `set_phone_profile` automation action a no-op on Android.

- [ ] **W2.11 (HIGH)** The private key rotates silently.
  `encryption_service.dart:37-41` mints a new X25519 key when restore fails, and
  `pairing_service.dart:292-301` swallows the error, leaving the app reporting
  `PairingState.connected` with a stale shared secret.

- [ ] **W2.12 (MEDIUM)** `automation/rule_remove` and `audio/route` are sent to
  a desktop that does not handle them (see W1.10, W2.6).

- [ ] **W2.13 (MEDIUM)** Four settings toggles are wired to nothing;
  `screens/settings_screen.dart:216-320`. The subtitles admit it:
  *"Always on for now — this switch has no effect"* (`:224, :243`),
  *"Not connected yet"* (`:290`), *"No diagnostic data is collected or sent, so
  this switch has no effect"* (`:309-310`). `sync_notifications` and
  `sync_clipboard` are never consulted by the listener at `main.dart:261-288`.

- [x] **W2.14 (RESOLVED)** Notification read state is not modelled. **Fixed.**
  `markReadNotification` had an empty body, `ConduitNotification` had no read
  field, and the table had no read column — so the UI "marked read" by calling
  `dismissNotification`, which means **the phone deleted the notification while
  telling the desktop it was read.**

  There is now a real read field through the whole stack: the model, all four
  (de)serialisers, a schema migration (`_dbVersion` 1 → 2, with the `ALTER
  TABLE`), and `markReadNotification` returning rows-changed so the UI can tell
  the difference between a mark that landed and one that did not. The screen
  marks read instead of deleting, and the swipe is handled in `confirmDismiss`
  returning `false` — otherwise the card left the list while the model kept it.

  Worth recording: the desktop's `handle_notification_mark_read`
  (`handlers/notifications.rs:65-71`) is a *deliberate* pure relay with a test
  asserting that mark-read must never be conflated with dismiss. So the phone
  is the only place this state can live, and the desktop half needed no change.

- [ ] **W2.15 (MEDIUM)** `context.sms` and `remote_input` are declared in the
  manifest and requested at `MainActivity.kt:149-176` but no Dart code calls
  `getBluetoothDevices`, so the whole Bluetooth surface
  (`MainActivity.kt:77, 482`, `AppDelegate.swift:121, 353-375`) is dead.

- [ ] **W2.16 (MEDIUM)** `UIBackgroundModes` (`Info.plist:54-59`) lists `fetch`,
  `processing` and `remote-notification` but not `audio`, so continuous mic
  capture is suspended in the background. `processing` also needs a
  `BGTaskSchedulerPermittedIdentifiers` entry, which is absent, making the mode
  inert.

- [ ] **W2.17 (MEDIUM)** `isNotificationListenerEnabled` returns `false` on iOS
  (`AppDelegate.swift:141-142`), so `notification_service.dart:92` pops a
  "not available" alert on every launch.

- [ ] **W2.18 (MEDIUM)** `android:usesCleartextTraffic="true"`
  (`AndroidManifest.xml:30`) is app-wide with no network-security-config, while
  pairing is TLS-only by design (`pairing_service.dart:56-60`).

- [ ] **W2.19 (MEDIUM)** `hexToBytes` (`encryption_service.dart:144-150`) throws
  on odd-length or non-hex input. It is called on attacker-controlled wire data
  at `websocket_service.dart:378, 407, 408, 410, 417, 646, 599, 671`; the
  surrounding `catch` (`websocket_service.dart:356, 427-429`) turns a malformed
  frame into a silent drop.

- [ ] **W2.20 (MEDIUM)** `playAudioData` re-creates an `AudioSource` and calls
  `setAudioSource`+`play()` on **every chunk** (`audio_stream_service.dart:173-177`).
  `startDuplexMode`/`stopDuplexMode` (`:123-158`) have no caller.

- [ ] **W2.21 (MEDIUM)** Dead code: `_isSyncing` is a `final` field that can never
  change (`clipboard_service.dart:45`); `_frameTimer` is cancelled but never
  assigned (`screen_mirror_service.dart:13, 111-112`); `MainActivity.kt:36-37`
  declares two channel constants that are never used;
  `AppDelegate.swift:339` invalidates a timer that is never assigned;
  `getNotifications`/`removeDeliveredNotification` exist natively
  (`MainActivity.kt:63`, `AppDelegate.swift:104-114`) with no Dart caller;
  `sendFileRequest`/`sendFileAccept`/`sendFileCancel`/`sendFileChunk`
  (`websocket_service.dart:764-793`) are entirely dead;
  the base64 `file/chunk` arm is disabled with a debug log
  (`websocket_service.dart:210-216`).

- [ ] **W2.22 (LOW)** Hardcoded that bypasses the theme:
  `lib/theme/theme_provider.dart:11` defaults to emerald
  (`0xFF34D399`) while `app_theme.dart:122, 136, 169` all use `0xFF00F0FF`, so
  the first launch renders a different accent than the first tap. Also
  `screens/home_screen.dart:350-484`, `widgets/conduit_logo.dart:81`,
  `screens/notifications_screen.dart:132-138`,
  `screens/discovery_screen.dart:129, 133`.

- [ ] **W2.23 (LOW)** Hardcoded `'os': 'android'` is sent from iOS too
  (`pairing_service.dart:186-189, 260-263`). Use `Platform.operatingSystem`.

- [ ] **W2.24 (LOW)** `flutter_animate ^4.5.0` is the only unused pubspec
  dependency — `widgets/entrance.dart:5-6` says so explicitly. Remove it.

- [ ] **W2.25 (LOW)** `AppDelegate.swift:411, 416` `print()` a push token and a
  full notification payload to the device log. `Build.SERIAL`
  (`MainActivity.kt:442`) is deprecated and returns `"unknown"` on Android 10+.

- [ ] **W2.26 (LOW)** ~25 undocumented magic numbers, e.g. the 30 s / 200 ms
  poll (`file_service.dart:247-248`), 64 KiB chunk size (`:13`), 2 s clipboard
  poll (`clipboard_service.dart:52`), 30 s automation timer
  (`automation_service.dart:84`), `for (var i = 0; i < 10000; i++)`
  (`file_service.dart:481`). Hoist to named constants.

- [x] **W2.27 (RESOLVED)** The phone had no `error` arm on the relayed path, so
  a refusal sent through the relay was received and silently dropped. **Fixed.**
  Desktop side: W3.27. Client requirement: W1.17.

  `websocket_service.dart` already handled a bare `error` (C-F9, `da66802`) — but
  only the socket-level listener does, and that listener never sees a relayed
  frame. The relay wraps **every** payload it forwards in `relay_delivery`, so a
  refusal the desktop routes back arrives as `relay_delivery` → unwrap →
  `_handleMessage`, which then looked the type up in `_messageHandlers` and found
  nothing registered for it. A phone behind the relay was therefore throttled,
  refused, and told nothing: no screen, no log, still `connected: true` — which
  is indistinguishable from a dropped socket.

  The new `type == 'error'` arm sits in `_handleMessage`
  (`websocket_service.dart:1253`) rather than in either unwrap helper, because
  that is the single funnel every *inner* message passes through —
  `_handleAttributed` for an unwrapped payload and `_handleEncrypted` for a
  sealed one. An arm in either would cover one path and miss the other, and the
  two are not redundant: the desktop seals its ordinary outbound traffic
  (everything except `pairing`), so which shape carries a given refusal depends
  on what the refusal is about.

  4 tests, all driving the real `WebSocketService` over a real socket: unwrap +
  decrypt + dispatch of a relayed `clipboard/sync` (the desktop's fan-out seen
  from the far end), a **sealed** `error` with `code: "rate_limited"` surfacing
  in `lastError`, an **unsealed** `error` with `code: "not_authenticated"` doing
  the same, and the attribution of the frame to the sender the relay
  authenticated rather than the self-reported `source_device` inside the payload.

---

## W3 — Desktop backend

- [x] **W3.1 (RESOLVED)** Five of seven automation actions reported success
  while doing nothing. **Fixed** — and there were seven, not five.

  `SendNotification`, `RouteAudio`, `ToggleWiFi`, `ToggleBluetooth`,
  `OpenUrl` and `SetPhoneProfile` each logged and returned `success: true`.
  Confirmed there is no implementation to find: no notification emitter, no
  `AudioStream` access (the executor is a pure function with no `AppState`), no
  Wi-Fi or Bluetooth code anywhere in the crate, and no URL opener
  (`tauri-plugin-opener` is not a dependency). The remaining two were already
  honest — `RunShellCommand` reports the process's real exit status, and
  `SetWindowState` already returned `false`.

  **The audit's enumeration was incomplete.** `handlers/auto_rules.rs:172-188`
  was a sixth and seventh site of the same shape: for a non-desktop action it
  logged `success = true` with the message "action executes on the target
  phone/tablet; desktop logged only" — and `handle_automation_triggered` sends
  nothing to any device. Nothing was ever forwarded, so the claim was pure
  fiction one layer up. Now logged as a failure that says why.

  **The audit's suggested fix was harmful and was not taken.** It proposed
  narrowing `is_desktop_executable` so the five actions fall into the `else`
  branch — which is exactly the branch that writes `success = 1` and claims a
  phone did it. That trades five false successes for seven.
  `is_desktop_executable` is a routing predicate ("which side runs this"), not
  a capability predicate; the truthful report belongs in the executor. This is
  written down at the function so it is not "tidied" back later.

  No trust gate was touched. The shell allowlist, metacharacter rejection,
  `shell_rule_gate` and `triggered_rule_gate` are byte-identical, and the diff
  touches no gate. Six of the seven actions still cannot be *implemented* here —
  that remains new capability, deliberately not built. What changed is that they
  no longer lie about having run.

  **Still open, and separate:** `main.rs:603-639` asserts a message exists —
  which every stub satisfies — and never asserts `success`.
  `every_action_without_an_implementation_reports_failure` covers it in
  `automation.rs` instead, but the integration-level assertion is worth adding
  where the path is actually exercised.

- [x] **W3.24 (RESOLVED — HIGH, found by the cross-platform audit)** The per-type
  rate limit capped screen mirroring at **1 fps** and shared its bucket with
  `stop`. `security.rs` set `"screen_mirror" => TypeLimitConfig::new(60, 60)`, i.e.
  60 messages per **60 seconds**, under a comment reading
  `// Screen mirror frames: ~30 fps × 2 msgs each` — three orders of magnitude
  away from the budget the comment describes. `audio` had the same numbers.

  The defect was the **window**, not the count, which is why "raise the number"
  would not have worked: a 60-second window admits a one-second burst and then
  refuses for 59 seconds. Streaming types now have per-second budgets sized to
  their documented rate (`screen_mirror` 120/1 s, `audio` 100/1 s), and `file`
  was raised to 3000/60 s against the 256 MiB/10 s byte budget it shares.

  **Control actions are separated into their own bounded bucket** — start, stop,
  cancel, complete, capture_stopped and the pairing handshake steps. They were
  sharing the frame budget, so the message that *stops* a stream was starved by
  the stream itself, and capture could continue with no way to halt it from the
  desktop. `pairing/request` is deliberately **not** exempt: it is the
  brute-force vector, while `accept`/`local_auth` are authenticated by something
  the sender must already hold.

  A second, separate ceiling sat on top and was also fixed: the **global**
  limiter was charged twice per inbound text frame — by the reader and again by
  the dispatcher, against the same bucket — giving a real ceiling of ~5 msg/s on
  all types combined. The reader's charge is the one that survives, because it
  happens before the parse: every frame costs one count whatever becomes of it,
  so an unauthenticated peer cannot get free parse attempts. The transport
  budget is now sized to exceed the sum of the per-type ceilings over the same
  window, or the per-type budgets would be dead code for exactly the types they
  were sized for.

  Tests assert the window, the sustained throughput, and that a stream cannot
  starve the action that stops it. `test_type_limit_config_values` previously
  pinned `max_messages` and never asserted `.window`, which is how the drift
  survived.

- [ ] **W3.2 (HIGH)** `handlers/audio.rs` leaks live system-audio PCM to
  unpaired LAN sockets. Three sites iterate `ctx.clients` directly —
  `:45-50` (the continuous stream loop), `:69-74` (`stream_stop`) and `:153-160`
  (the catch-all) — bypassing `broadcast_to_others`
  (`handlers/mod.rs:130-138`), which exists specifically to stop plaintext
  reaching unpaired sockets. Because `seal_for_peer` (`server/mod.rs:325-337`)
  returns the plaintext unchanged for a peer with no shared secret, any socket
  that merely connects receives audio. `handlers/mod.rs:329` calls that case
  *"should be unreachable"* — this is the assumption that is violated.
  *Left to do:* replace all three loops with `broadcast_to_others`.

- [ ] **W3.3 (HIGH)** `handlers/files.rs:197-206` has the same bypass plus a
  device-id/connection-id confusion. The `to.is_empty()` branch is unfiltered,
  so unpaired sockets receive `file/request` (filenames, sizes, MIME types);
  and `clients_lock.get(to)` treats `to` — a stable *device* id
  (`types.rs:289-291`) — as a per-connection UUID (`server/mod.rs:241`), so a
  targeted file request delivers nothing. The same class of bug is documented as
  fixed in `WsServer::send_to` (`server/mod.rs:1368-1432`).
  *Left to do:* use `broadcast_to_others` for the empty case; resolve `to`
  through `ws_to_device_id` as `send_to` already does.
  > **⚠ CITATIONS HAVE MOVED, AND ONE WAS ON THE WRONG FUNCTION.** The entry said
  > `handlers/files.rs:66-75`, which by `e6ca781` was the candidate-sender
  > enumeration inside `unwrap_relay_binary_frame` — the function W3.28 is about
  > — and `server/mod.rs:948-977` for `send_to`, which is now at `:1368`. Both
  > had drifted before this round; the lines above are re-read.

- [ ] **W3.4 (HIGH)** Migrations are executed from the filesystem at startup.
  `storage.rs:299-339` scans `manifest_dir/src/migrations`,
  `manifest_dir/migrations` and `<exe dir>/migrations`, then runs whatever
  `.sql` it finds against the application database. Migrations are already
  embedded via `include_str!` (`storage.rs:94-96`), so the scan buys nothing.
  Deduplication is by filename only (`:295, 322`), so an attacker-chosen name
  wins. Anyone who can write to the executable's directory gets arbitrary SQL
  on next launch.
  *Left to do:* delete the directory scan; rely on `EMBEDDED_MIGRATIONS`.

- [ ] **W3.5 (HIGH)** A `Mutex` poisoning cascade bricks the app.
  `storage.rs:5` uses `std::sync::Mutex`, and ~20 call sites do
  `self.conn.lock().unwrap()` (`:693, 717, 748, 779, 787, 798, 813, 826, 854,
  871, 896, 922, 933, 941, 977, 1034, 1050`). Any panic while the lock is held
  poisons it, and every later read *and write* panics.
  *Left to do:* `lock().unwrap_or_else(|e| e.into_inner())`.

- [ ] **W3.6 (HIGH)** Undecodable database rows vanish silently. Three read paths
  end in `.filter_map(|r| r.ok())`: `storage.rs:770` (`get_all_devices`),
  `:845` (`get_notifications`), `:913` (`get_clipboard_history`). A corrupt
  device row makes a paired device disappear from the UI with no error and no
  log. This is the exact failure mode `storage.rs:1055-1068` documents as fixed
  for settings reads — the fix was applied to one accessor, not the others.
  *Left to do:* propagate the row error.

- [ ] **W3.7 (HIGH)** Duplex audio playback is dead from launch. `audio.rs:43`
  initialises `is_playing` to `false`; `:49-51` spawns one thread;
  `playback_loop` gates its entire body on that flag (`:285`), so it skips the
  loop, drops the receiver at `:306` and exits. `start_playback`
  (`:130-136`) flips an atomic nobody polls; `send_playback_data` (`:145-149`)
  sends into a channel whose receiver is gone, and `handlers/audio.rs:150`
  still acks the peer.
  *Left to do:* loop independently of `is_playing`, gating per buffer instead
  of terminating the thread.

- [ ] **W3.8 (HIGH)** The TLS identity changes on every launch. `tls.rs:1` is
  `#![allow(dead_code)]`; `get_tls_acceptor` (`:12`) — the only path that
  persists the certificate in the OS keyring — is never called.
  `server/mod.rs:192` calls only `get_wss_acceptor`, which generates a fresh
  key pair and self-signed cert each start (`tls.rs:88-97`). Anything that
  pins or remembers the certificate breaks on restart.
  *Left to do:* route the WSS listener through `get_tls_acceptor`, or delete it
  and the blanket allow.

- [ ] **W3.9 (MEDIUM)** `save_settings_field` writes any key with no allowlist
  (`commands/settings.rs:154-175`). A compromised or XSS'd webview can write
  `allowed_commands` as `["*"]`, bypassing the sanitization in
  `set_allowed_commands` (`:228`) and `security.rs:1075` — the only thing
  between a peer-supplied `RunShellCommand` rule and `Command::new`
  (`main.rs:255-259`).
  *Left to do:* reject unknown keys; route `allowed_commands` through
  `set_allowed_commands` only.

- [ ] **W3.10 (MEDIUM)** `send_encrypted_message` returns `Ok(())` when the peer
  is unreachable (`commands/pairing.rs:236-245`), so the UI reports success for
  a dropped message. `reply_notification` (`commands/notifications.rs:174`)
  deliberately does the opposite.

- [ ] **W3.11 (MEDIUM)** The outbound file chunk loop silently no-ops when the
  server is absent (`commands/file.rs:213-222`): the `if let` is skipped, the
  loop still reads every remaining chunk from disk, and `remove_outgoing` runs
  at `:228`. The accept poll at `:192-200` also falls through to
  `remove_outgoing` with no state change, leaving both sides on a permanently
  `pending` transfer.

- [ ] **W3.12 (MEDIUM)** The JPEG quality setting is accepted, plumbed and
  discarded. `handlers/screen_mirror.rs:577` — the parameter is
  `_jpeg_quality` and unused; encoding at `:598-601` always applies the
  `image` crate default. The user's low/medium/high choice has no effect.
  *Left to do:* construct `JpegEncoder::new_with_quality(&mut buf, q)`.

- [ ] **W3.13 (MEDIUM)** `handle_pairing_request` (`handlers/pairing.rs:60-179`)
  and `handle_pairing_accept` (`:181-278`) are ~100-line near-duplicates that
  have already drifted: different OS normalisation (`:112-122` vs `:204-207`),
  different `mark_paired` ordering relative to `add_client` (`:106`/`:133` vs
  `:266`/`:244`), and only one replies.
  *Left to do:* collapse into one `pair_pairing(action)` helper.

- [ ] **W3.14 (MEDIUM)** `automation::trigger_tag`/`action_tag`
  (`automation.rs:481, 488`) return `""` on serialisation failure, so
  `re_evaluate_rule` (`:553`) can never match and `automation_logs.trigger_type`
  is written empty.

- [ ] **W3.15 (MEDIUM)** Three schema columns are never written:
  `devices.signal` (`migrations/001_initial.sql:12`),
  `notifications.actions` (`:23` — the protocol *does* send them at
  `types.rs:231-232` and they are dropped), and
  `automation_rules.last_triggered` (`:64`).

- [ ] **W3.16 (MEDIUM)** Hardcoded: `battery: Some(100)` for every desktop
  (`server/mod.rs:1562`, `handlers/pairing.rs:162`,
  `commands/pairing.rs:211`); `discovery.rs:221` returns `127.0.0.1` — the
  exact lie `commands/system.rs:16-26` documents as removed — so an
  IPv6-only host advertises an unreachable address; three different app-data
  roots (`encryption.rs:65`, `security.rs:963`, `main.rs:57-60`,
  `file_transfer.rs:1102`) despite a comment demanding "exactly one
  definition"; `screen_mirror.rs:702` fabricates a 1920×1080 screen;
  `commands/pairing.rs:227` hardcodes `"protocol_version": 1`.

- [ ] **W3.17 (MEDIUM)** `fix_firewall` (`commands/system.rs:81-103`) has no
  `#[cfg(windows)]` (so Linux gets a confusing "Failed to elevate"), returns
  `Ok(true)` as soon as `spawn()` succeeds without waiting for the elevated
  process, and hides the window and output.

- [ ] **W3.18 (MEDIUM)** `remote_input.rs:340-357` discards every `enigo`
  result while `inject_key_chord` returns `KeyOutcome::Injected`; a failed
  modifier `Press` with a successful `Release` leaves a modifier stuck down.
  `remote_input.rs:33` panics on a headless host and is reachable from the
  network.

- [ ] **W3.19 (LOW)** Silently swallowed errors: `main.rs:71` mints a new device
  UUID on every launch if the id file is unwritable;
  `storage.rs:538, 545, 559, 632` delete failures leave a **plaintext** database
  on disk; `file_transfer.rs:515, 648, 884, 1064`; `server/mod.rs:1486, 544, 1507`.

- [ ] **W3.20 (LOW)** Dead code: three `SyncEngine` methods with no production
  caller (`sync.rs:56, 60, 64`) while `auto_rules.rs:139` reaches into the
  `pub` field directly; stale `#[allow(dead_code)]` at `automation.rs:118`;
  `handlers/auto_rules.rs:28-29` does redundant work (`load_rules` replaces the
  set); `auto_rules.rs` is missing from the flow at
  `commands/file.rs`.

- [ ] **W3.21 (LOW)** Unused frontend-facing commands: `save_window_state`
  (`commands/system.rs:216`) and `get_window_state` (`:232`) are registered and
  never invoked — `main.rs:423-441` writes the setting directly.

- [ ] **W3.22 (LOW)** `tauri-plugin-shell = "2"` (`Cargo.toml:9`) is a
  deliberately unregistered dependency; `main.rs:197-203` explains why. Remove
  the dependency and keep the rationale in `capabilities/default.json`.

- [ ] **W3.23 (LOW)** macOS: no signing identity or notarization
  (`tauri.conf.json:48-49`), and no `NSMicrophoneUsageDescription` /
  `NSScreenCaptureUsageDescription` even though `entitlements.plist:22-32`
  requests those capabilities. On macOS 14+ both audio capture and screen
  capture fail silently behind a `warn!`. `tauri.conf.json:50` also names
  `./Entitlements.plist` while the file is `entitlements.plist`.

- [x] **W3.25 (RESOLVED — HIGH, found by the cross-platform audit)** No fan-out
  reached a relay-only peer. **Fixed**, and the recipient set had to come from the
  wrong map to make it work at all.

  Both fan-out functions walked `ctx.clients`, and the relay is **not** a member
  of it: `relay_tx` is an `mpsc::Sender<String>`, because the relay socket is
  outbound-only and the desktop *hosts* the relay. So a phone reachable only
  through the relay received nothing from any of the seventeen handlers that
  fan out — clipboard, notifications, SMS, calls, discovery/remove, and every
  file-transfer control frame — and it failed **silently**, because iterating an
  empty set is indistinguishable from success. Directed sends already worked
  (`send_to` has had a relay arm since the relay moved in-process); that
  asymmetry is what made the feature look intermittent rather than broken.

  **The recipient set cannot be `SyncEngine`, and that is the whole difficulty.**
  `SyncEngine` is a *liveness* map: `handle_client` removes a device from it the
  moment its LAN socket closes. The flagship scenario — pair on the LAN, then
  walk out of range — is precisely the case where the device is absent from it, so
  a fan-out built on `SyncEngine` produces an empty recipient set for the one
  device the feature exists for, and passes every test that models the device as
  connected. `DeviceRouteKeys::routable_device_ids()` (`relay.rs:197`) enumerates
  the **pairing registry** instead, which is refreshed from the device table and
  refreshed again every `KEY_REFRESH_INTERVAL` — and which is the same set the
  relay keys its own verification against, so enumeration cannot name a device the
  relay would then drop.

  Three exclusions, each load-bearing and each with its own test:

  - **This desktop.** `DeviceRouteKeys::refresh` registers the desktop under its
    own id, so an unfiltered enumeration has it route a frame to itself — which
    the relay delivers straight back, `unwrap_relay_delivery` accepts, `dispatch`
    re-enters, and *that* fans out again. Multiplicative, until the 1024-slot
    queue fills and then hub-wide.
  - **Devices with a live socket here**, which the LAN loop that called us
    already serves. Without this a phone that is on the LAN *and* joined to the
    relay — exactly what a network transition looks like — gets every
    notification twice, and duplicated file chunks corrupt a transfer.
  - **The sender**, compared as a **device id**, because that is what the relay
    addresses. `sender_id` is overloaded in this hub: a per-connection UUID from
    the accept loop, or a relay-authenticated device id once `handle_message` has
    unwrapped the delivery. Comparing the raw string excluded nothing for a
    relayed sender and handed the phone its own notification back.

  **The egress half was a plaintext leak, and had to land with it.** The relay
  egress resolved the recipient's secret from `SyncEngine`, so every frame routed
  to a socket-less device fell through to cleartext — already true of directed
  `file/chunk` frames, base64 payload and all, and about to become true of
  clipboard bodies and SMS. `seal_for_device` now reads `handlers::peer_secret`
  (`handlers/mod.rs:108`), which consults `SyncEngine` first so every device with
  a live socket keeps byte-identical behaviour, and falls back to the pairing
  registry. `build_sealed_route` is now the single builder both egresses call, so
  the LAN writer and the relay route cannot drift apart again.

  Also fixed in the same place, because it was the same bug wearing a different
  hat: `relay_route_to` uses `try_send`, never `send().await`. The queue is
  bounded at 1024 and the writer behind it can stall, and this is reached from
  broadcast paths that hold `ctx.clients` and `ws_to_device_id` readers — parking
  on an await there converts one slow relay into a hub-wide stall of registration,
  pairing and every LAN broadcast, because tokio's `RwLock` is write-preferring.
  And `send_to`'s relay arm no longer holds the `relay_tx` read guard across an
  await.

  `build_sealed_route` **fails closed** (`server/mod.rs:1486-1532`). It returns
  `Option<String>`, so a payload that will not parse refuses the route instead of
  substituting `Value::Null` — and a null payload is not a harmless default. The
  relay does not inspect payloads, so it would have forwarded the literal string
  `null`, counted it as `messages_routed`, and the recipient, whose envelope
  parser requires a map, would have dropped it with no error on either side. A
  signed, counted-as-delivered frame that vanishes is the same silent-failure
  class this egress exists to remove. The branch is unreachable in practice —
  `seal_for_device` returns either the caller's own JSON or an
  `EncryptedEnvelope` serialisation, both of which parse — which is the argument
  for handling it rather than asserting it cannot happen: `unwrap_or` is what
  makes "cannot happen" true.

  7 tests. The one that would catch a regression to the liveness map is
  `a_relay_only_peer_receives_a_broadcast`: the device has a registry row and
  **no socket**, and the route's payload must open with the phone's own secret to
  exactly the bytes a LAN peer would receive.
  `the_desktops_own_broadcast_reaches_a_relay_only_peer` is the other half of the
  same defect and arrived with the fix round: `WsServer::broadcast` is the path
  every desktop-originated frame takes
  (`commands::notifications::forward_to_peers_checked` reaches it for a
  dismissal, a reply and a clipboard sync), and a fix that taught only
  `broadcast_to_others` about the relay would have left the desktop's own
  notifications and clipboard on the LAN and nowhere else.

- [x] **W3.26 (RESOLVED)** Relayed frames were charged nothing. **Fixed — and
  the audit's description of the defect was wrong in a way that made it look
  smaller than it was.**

  [CROSS-PLATFORM-GAPS.md](CROSS-PLATFORM-GAPS.md) G-R11 recorded this as *"All
  phones behind the relay share **one** rate-limit bucket keyed on the literal
  `"relay_server"`"*, and the previous commit's message repeated it. There was no
  such bucket. `admit_frame` has exactly one caller — `handle_client`, the LAN
  reader — and the relay read loop calls `handle_message` directly, so the
  `"relay_server"` bucket was **never created**: relayed frames were charged no
  count *and no bytes* by the transport limiter, by any phone, at all.
  `max_bytes`, the field `security::RateLimitConfig` documents as "the budget that
  bounds CPU", did not exist on this path.

  The per-type limiter was already per-device, because the same re-entry dispatches
  under the sender (`server/mod.rs:878` → `:1069`). So one phone could not
  throttle another — and neither could be throttled. The fix is therefore
  **strictly more accounting than before, not a repartition of an existing
  charge**, which is the opposite of what the audit described and is recorded as
  such on the code.

  One relayed frame, one charge:

  - a delivery whose sender is named and trusted is charged to **that sender's**
    device id, once, before dispatch (`server/mod.rs:864`);
  - a frame no device can be named for — malformed, or claiming an id this
    desktop has not paired — is charged to the **transport** instead
    (`server/mod.rs:894`), so "refusal is free" does not become true on this path
    either. These are precisely the frames that never reach a handler at all, so
    without it an unauthenticated peer has an unlimited supply of parse attempts.
  - a relayed **binary** frame is charged to the transport (`admit_relay_binary`,
    `server/mod.rs:424`), because a v2 frame names its **recipient**, not its
    sender — the format gives no sender to charge. It is charged at all only
    because it previously was not: the text arm was metered and the binary arm,
    the cheaper flood, was not.

  5 tests. Four are regression tests, each verified to fail with its fix removed:
  exactly one count on the sender's budget and **zero** on the transport's; a peer
  saturating its own budget not throttling a second peer (the fairness property
  the audit was reaching for, and the one that would fail if the charge were still
  keyed on the transport); a refused relayed frame still costing its sender; and
  three forged `relay_delivery` frames from unknown senders costing the transport.

  The fifth,
  `a_relayed_binary_frame_is_charged_to_the_transport_budget`, is a **guard, not a
  regression test**, and is labelled as one on the code
  (`server/mod.rs:5321-5343`). It cannot be one: a relayed binary frame has to be
  charged through the relay read loop, that loop needs a live WebSocket, so the
  accounting was extracted into `admit_relay_binary` (`server/mod.rs:424`) to be
  reachable at all — and a test on the extracted function passes whether or not
  the loop calls it. Deleting the call site would leave it green. That gap is
  real, and it is the argument for the one test this round cannot have: a
  single-process phone-encoder → real relay socket → real dispatcher round trip
  (W8.11, and `docs/TESTING.md` §7 item 5). What the test buys is that the
  accounting is written down and correct at the seam rather than a line inside a
  `select!` nobody can reach.

- [x] **W3.27 (RESOLVED)** A relay-only peer was refused in silence. **Fixed,
  and it was never a protocol change.**

  The previous commit's message, and the test that went with it, both recorded
  this as unfixable without one: *"a device with no socket here has nowhere to
  receive an `error` frame … routing refusals back is a protocol change."* It is
  not. `relay_route.payload` is an arbitrary JSON message and the relay forwards
  it without inspecting it, so an `error` was always legal to route; the relay
  wraps it in `relay_delivery` exactly as it does any other payload. The route
  simply was not being taken. Nothing about the wire format changed, and no
  `PROTOCOL.md` compatibility entry is needed — which is why §9.1 does not list
  it.

  `answer_channel` answers **sockets only**, and that is now the deliberate half
  of the answer rather than the whole of it: the question it answers is "which
  socket is this id on?", and a caller that wants the relay has to say so. The
  new `answer()` (`server/mod.rs:2179`) tries the socket first and otherwise
  routes a signed, sealed route, and replaced `answer_channel`'s use at every
  refusal site: `rate_limited`, `invalid_message`, `not_authenticated`,
  `unsupported_protocol_version`, the settings gates, `send_error`, and the
  `ping`/`pong` reply — so a relay-only device is now probeable for reachability
  as well as refusable.

  **The one thing this must not become is a pre-auth signing oracle.**
  `send_error` is reachable from `not_authenticated`, so routing refusals
  unconditionally would let a stranger make the desktop mint a signed, encrypted
  route per refused frame just by sending junk. `relay_route_to` therefore gates
  on `is_trusted_peer` — the same predicate the auth gate uses, so the two cannot
  disagree about who a device is — and a refusal aimed at an unpaired id gets no
  route at all. There is a test for that specific shape, labelled as the guard it
  is rather than dressed up as a regression.

  **One refusal site was missed, and it was the one that mattered most.**
  `handlers::handle_file_request` did not go through `answer_channel` or
  `send_error` at all — it resolved the `file_accept_disabled` refusal with a bare
  `clients.get(client_id)` (`handlers/files.rs:172`). `client_id` is a **device**
  id for a relayed sender, and `clients` is keyed by connection id, so the lookup
  never matched: a phone behind the relay pushing a file with `auto_accept_files`
  off was declined, buffered nothing, and was told nothing — on the one path where
  the peer is actively waiting for an ack that will now arrive. It calls
  `send_error_to` now. It is the same defect as every other refusal here, and it
  survived the first pass because it is a refusal that does not look like one.

  4 tests: a relay-only sender is told `rate_limited` (driven through six
  `pairing/request` frames, because `pairing` is exempt from the auth gate and
  has the cheapest budget to exhaust); a relay-only device gets its `pong`; a
  refusal for an unknown id gets no signed route and the transport id never
  acquires peer authority; and `answer_channel_still_answers_sockets_only`, split
  out from the test that used to pin the absence of this capability so the two
  claims cannot be confused — *"this map cannot reach a relay-only device"* was
  true and stayed true; *"so nothing can"* was the bug.

  **The `file_accept_disabled` fix is the one part of this item with no test.**
  `file_request_invalid_declined_sends_error_frame_to_sender`
  (`handlers/files.rs:522`) covers it, but through `add_test_client`, which
  registers a `ws_to_device_id` entry — so it exercises the socket arm and would
  still pass with the bare `clients.get` back in place. What is missing is the
  relay-only shape, and `add_test_relay` now makes it a two-line test.

  The client half is W2.27 and would have made this invisible without it.

- [x] **W3.28 (RESOLVED)** Every relayed binary file chunk was dropped, after the
  relay had verified its tag. **Fixed**, and it was found while fixing the
  fan-out rather than looked for.

  `handle_lan_chunk` receives a **device** id when the frame was relayed and a
  **connection** id on the LAN, and it looked that id up in `ws_to_device_id`,
  which is keyed the other way. `unwrap_or_default()` produced `""`, the secret
  lookup missed, and the chunk was discarded — after the relay had already
  verified its tag, so the drop looked like corruption rather than a lookup bug.
  It resolved through the registry first and fell back to the id itself, which
  handles both transports and is the same one-line resolution
  `persist_inbound_clipboard` and `handle_status_update` already used. The text
  path had this right; the binary path threw the id away and re-resolved it.

  The same diff introduced **`handlers::peer_secret`** (`handlers/mod.rs:108`),
  one resolver replacing five separate reads of `SyncEngine` as a trust registry
  (`is_trusted_peer`, `seal_for_device`, `unwrap_relay_delivery`,
  `resolve_sender_secret`, `handle_lan_chunk`). They read two maps that mean
  different things and disagreed: `SyncEngine` is liveness, the pairing registry
  is identity. Order is deliberate and not arbitrary — `SyncEngine` first, so
  every device with a live socket keeps byte-identical behaviour including the
  case where a re-pair has written a fresh secret there and the relay's copy is
  up to 5 s behind. That disagreement was not theoretical: a phone that paired on
  the LAN and moved networks was refused **on arrival** by
  `unwrap_relay_delivery`, so the egress fix would have delivered into a socket
  whose inbound refused everything.

  Two lock-order inversions were removed while in there. `is_trusted_peer` and
  `resolve_sender_secret` held the `sync_engine` guard across the
  `ws_to_device_id` read, which is the inverse of the order `broadcast_to_others`
  uses — harmless while every holder is a reader, a deadlock the moment a writer
  queues. Both now release one guard before taking the other.

  **There were two defects on this path, and the first fix only closed one.**
  `unwrap_relay_binary_frame` enumerated its candidate senders from `SyncEngine`
  (`handlers/files.rs:93-116`), the same liveness map — so a relay-only sender's
  frame was refused with *"no paired device to attribute this frame to"* one
  layer **above** the id resolution above, and the fix described in this item
  never ran for exactly the sender it was written for. It enumerates
  `route_keys.routable_device_ids()` now, which is the registry, and which is the
  right set on its own terms: these are the devices whose route keys the relay
  will verify, and the loop skips any id with no key, so it cannot widen what is
  accepted.

  `a_relay_only_senders_binary_frame_is_attributed_to_it` (`server/mod.rs:5416`) covers
  both halves and separates them on purpose: it asserts attribution first, on its
  own, so a failure says which of the two defects it belongs to; then it asserts
  delivery on the file engine's byte count, because "did it arrive" was the whole
  defect and a log line would not distinguish a drop from a decrypt failure. The
  transfer is deliberately partial (`total: 2`) — a single chunk would finalize on
  arrival and remove the entry, after which `received_bytes` reads 0 and the
  assertion could not tell a delivered chunk from a finalized one.

- [x] **W3.29 (RESOLVED — HIGH, found reviewing W3.28)** A revoked device stayed
  routable **and** trusted for up to 5 s. **Fixed**, and it is a window W3.28
  itself opened.

  `handlers::peer_secret` falling back to `DeviceRouteKeys` is what turned the
  route-key registry from a routing detail into an authority for the auth gate —
  and that registry refreshes on a timer (`spawn_key_refresh`, `relay.rs:499-516`,
  every `KEY_REFRESH_INTERVAL`, 5 s at `relay.rs:79`). `SyncEngine`, which
  `peer_secret` consults first, drops a device synchronously, so **unpairing used
  to take effect at once**; after W3.28 it would have taken up to five seconds,
  through the fallback. Both halves of that are exposure, not inconvenience: the
  device is still a fan-out recipient, and it is still `is_trusted_peer`, so
  `pairing/accept`'s copy keeps delivering clipboard and notification bodies to a
  phone the user has just unpaired.

  `DeviceRouteKeys::forget` (`relay.rs:244-251`) clears **both** maps in one call,
  so there is no window in which the device is unroutable but still trusted, and
  `WsServer::disconnect_client` calls it at `server/mod.rs:1843` beside the
  `clients` and `sync_engine` removals rather than waiting for a tick. It is
  idempotent, and a no-op for an id that was never registered.

  **What it deliberately does not do** is withdraw the device from the *relay*,
  which keeps its own copy of the registry and answers `unknown_device` from
  that. That copy refreshes on the relay's schedule, not this process's.

  **No test.** `relay.rs`'s 27 tests pin the `refresh()` filter
  (`an_unpaired_or_revoked_device_cannot_route`,
  `a_device_that_is_not_in_the_registry_cannot_route`) and neither exercises
  `forget`. The missing test is short: seed both maps through `register_for_tests`,
  call `forget` once, and assert that `routable_device_ids()` and `secret_for()`
  both stop naming the id.

- [x] **W3.30 (RESOLVED — HIGH, found reviewing W3.28)** The registry could
  publish a device as routable before its secret was published. **Fixed**, and the
  fix is an ordering.

  `DeviceRouteKeys` holds two maps and the egress reads both:
  `routable_device_ids()` reads `keys`, `secret_for` reads `secrets`, and the
  fan-out does "enumerate a target, then seal for it"
  (`server/mod.rs:1677-1692`). `refresh` published `keys` first, so a reader
  landing between the two writes would see a device in the recipient set whose
  secret was not there yet — and `seal_for_device` takes its no-secret path and
  puts the frame on the wire **in plaintext**. Not a leak that needs an attacker:
  it needs a thread scheduled in the wrong place.

  It writes `secrets` first, then `keys` (`relay.rs:299-311`). That makes the
  transient state the safe one: a target whose secret is missing is simply never
  named, because `refresh` builds both maps from the same rows in one pass, so
  `keys ⊆ secrets` holds at every observable moment. The original order made a
  plaintext egress reachable by scheduling alone.

  No test, and honestly none is available: the property is a claim about the order
  of two writes, and observing it from outside needs a hook between them. It is
  documented at the writes instead.

- [x] **W3.31 (RESOLVED — HIGH, found reviewing the fan-out)** One padded
  broadcast from a paired peer could close the relay connection for every peer.
  **Fixed at the fan-out; the validation gap that made it reachable is not.**

  `fan_out_to_relay` had no size ceiling, and sealing hex-encodes the payload, so
  a frame of *n* plaintext bytes leaves as roughly 2*n* plus the envelope. The
  relay's read ceiling is `MAX_TEXT_SIZE = 1 MiB`
  (`services/relay/src/limits.rs:20`, installed as `max_message_size` at
  `connection.rs:99-103`), and crossing it does not drop one frame: the handler
  answers `message_too_large`, lingers, and returns
  (`connection.rs:163-175`), which drops the socket. So anything over about half
  a megabyte took **every relay-only peer offline** for the length of that
  sender's reconnect backoff, and one peer chose when.

  It was reachable because `validate_message` closes no field set for the types
  that fan out. `sms`, `call`, `screen_mirror` and `remote_input` have no
  validation arm at all (`security.rs:748-750`), and `clipboard` (`:963-969`),
  `notification` (`:901-915`) and every `file` action (`:917-961`) bound the
  fields they know but never call `reject_unknown_fields`. A paired peer sends
  `{"type":"sms","action":"new","body":"x","pad":"<9 MB>"}` and it is accepted.
  Before the fan-out existed that frame cost only memory on a local
  `broadcast::Sender`; routing it turned a validation gap into an availability
  bug on a resource every peer shares.

  The guard is `MAX_RELAY_BROADCAST_BYTES = 256 * 1024` (`server/mod.rs:1459`),
  checked once per broadcast rather than per recipient so the cost is not
  multiplied by the peer count (`server/mod.rs:1663-1671`). Generous rather than
  tight on purpose: the largest legitimate broadcast is a `file/progress` at about
  100 bytes, and the alternative to being under this number is not "a smaller
  frame" but "no relay at all, for any peer".

  `an_oversized_broadcast_is_refused_before_it_can_kill_the_relay_leg`
  (`server/mod.rs:4979`) asserts the refusal **and** that an ordinary frame still
  routes, so the guard cannot quietly become a blanket ban.

  *Left to do:* close the field sets. That is the better fix and it is not this
  one — the guard bounds the shared relay leg, while an open field set is still a
  validation gap on the LAN path, where the same padded frame reaches every
  paired socket. Add `reject_unknown_fields` to
  `validate_notification_message`, `validate_file_message` and
  `validate_clipboard_message`, and give `sms` and `call` arms that bound what
  their handlers read.

---

## W4 — Desktop frontend

- [ ] **W4.1 (CRITICAL)** The pairing dialog can never complete.
  `components/pairing/PairingFlow.tsx:39-49` waits for an inbound `pairing`
  frame, but `handle_pairing_accept` (`handlers/pairing.rs:181-278`) broadcasts
  nothing and `handle_pairing_request` replies only to the phone (`:167`).
  **A successful pairing times out with a failure message.**
  Three compounding defects: it waits 120 s (`:56-59`) for a token whose TTL is
  60 s (`main.rs:190-192`); the effect at `:28-34` regenerates the token and
  resets the dialog whenever the device count changes; `data.device_info.name`
  at `:44` throws on a frame without `device_info` (optional at
  `websocket.ts:88`).
  *Left to do:* broadcast the accept, or watch the device list; align the
  timeout with the backend; do not regenerate on device-count change.

- [ ] **W4.2 (CRITICAL)** The automation panel can never list a rule.
  `hooks/useAutomation.ts:46` starts `rules` empty and the only writer is an
  inbound `automation/sync` frame (`:118-119`) — which nothing ever broadcasts.
  `handlers/auto_rules.rs:121-189` only writes `automation_logs`, and **there is
  no Tauri command for automation rules or logs at all**
  (`main.rs:219-262`). "No automation rules" is shown forever.
  *Left to do:* add `get_automation_rules` and `get_automation_logs`, hydrate on
  mount, and broadcast `triggered` to clients.

- [x] **W4.3 (RESOLVED)** The Files surface showed a stale clipboard list — in
  fact an empty one, permanently. **The stated cause was a symptom.** The real
  one is upstream: `clipboard_history` was **never written at all**.
  `save_clipboard` had exactly one production caller, `sync_clipboard`, and that
  command had no caller. The `("clipboard","sync")` arm only broadcast; nothing
  persisted. So `get_clipboard_history` always returned `[]`, the clear button
  could never render (it requires a non-empty list), and the dock badge was
  permanently 0.

  An inbound sync now persists an entry for the sending device before fanning
  out. Two details that are load-bearing:

  - `source_device` records the **connection's registered identity**, not the
    frame's own `source_device` field. A payload field is forgeable; a socket
    identity was written by the hub at pairing time from a one-time token. Same
    rule as `resolve_sender_secret`.
  - `pinned` is left at the column default, so a remote peer cannot pin a row
    that then survives `clear_clipboard_history`.

  A wildly-future timestamp falls back to arrival time, because `save_clipboard`
  prunes by `ORDER BY timestamp DESC` and an unbounded future stamp is a
  retention attack. Persist happens before broadcast and outside the client-map
  locks: the local row is the non-recoverable effect, so if we crash after
  broadcasting, nothing re-sends that copy.

  The UI half was a second, independent bug: `App.tsx:157` wired `onSynced` to
  `refreshDevices()` — the *device* refresh. It now calls `refreshClipboard`.

  `useClipboard.ts`'s in-memory `history` array remains dead state with zero
  readers. Deliberately not wired up: `clipboardItems` now comes from the
  database, which is pinned, ordered, prunable and has ids. A second
  50-entry array with no ids and no persistence would be a competing source of
  truth for the same data.

  See also [CROSS-PLATFORM-GAPS.md](CROSS-PLATFORM-GAPS.md) D-F4, and note that
  inbound clipboard content is still capped at 10 KB by `MAX_STRING_FIELD_LEN`
  while the outbound command allows 5 MB — pre-existing and untouched.

- [ ] **W4.4 (HIGH)** Notification read state is lost and never persisted.
  `mark_read` writes only to localStorage; the database has **no `read` column**
  (`migrations/001_initial.sql`, confirmed at `handlers/notifications.rs:65-71`).
  `get_notifications` returns 50 rows (`useWebSocket.tsx:64`), and
  `App.tsx:99-117` prunes read ids against that list — so with more than 50
  notifications, every read id outside the window is wiped on every frame.
  *Left to do:* add the column; store a read timestamp rather than a per-id set.

- [ ] **W4.5 (HIGH)** `useDevices()` is instantiated twice
  (`hooks/useSearch.ts:98` and `hooks/useDevices.ts`), each with its own 5 s
  `get_devices` poll. `NavigationContext.tsx:17-21` claims the context instance
  is "the one and only", so the search result at `useSearch.ts:26` calls
  `setSelectedDevice` on the **orphaned** instance — selecting a device from
  global search has no visible effect.
  *Left to do:* consume `useNavigation()` in `useSearch`.

- [ ] **W4.6 (HIGH)** `Ctrl+F` does not focus the search field. `App.tsx:215`
  sets `searchFocused`, but `SearchBar` takes only `results`/`onSearch`
  (`App.tsx:427`), and the ref the effect uses (`hooks/useSearch.ts:10`) is
  never attached to any element. The shortcut is advertised at
  `ShortcutsOverlay.tsx:17` and the placeholder at `App.tsx:427`.

- [ ] **W4.7 (HIGH)** UI that does nothing:
  `components/network/DeviceDetailPanel.tsx:188` "Rename" (with a fake `F2`
  shortcut), `:191` "Send file" (fake `Ctrl+S`), and `:353-360` a **prominent
  primary button** "Send File" gated on an unrelated condition;
  `components/files/FileCard.tsx:75` "Show in folder" calls the same handler as
  "Open" (`:74`), so it launches the file instead of revealing it.

- [ ] **W4.8 (HIGH)** `auto_accept_files` is persisted but never enforced.
  `components/settings/FilesSection.tsx:17` exposes the toggle; no gate reads
  it. The `settings_gate` in `server/mod.rs` covers `notification/post`,
  `clipboard/sync` and `file/*` but not this.
  *Left to do:* implement the gate in `handlers/files.rs::handle_file_request`,
  or hide the toggle.

- [ ] **W4.9 (HIGH)** Settings written but never read back. `theme` and
  `accent_color` hydrate from localStorage only (`lib/theme.tsx:26, 214`);
  `device_name` is used for the mDNS instance name but `get_devices` labels the
  hub with `gethostname()` (`commands/pairing.rs:77`) and `get_device_info`
  (`:199`) does too, so the name the user types never appears in the app or the
  QR. `relay_url` is persisted for a disabled feature.
  *Left to do:* hydrate the theme from `get_settings` on boot; have
  `get_devices`/`get_device_info` read the setting.

- [ ] **W4.10 (HIGH)** `useBubblePhysics.ts:252-255` re-renders every bubble
  **60×/s even when nothing moves** — `frame % 1 === 0` is always true.
  `useSearch.ts:11` also duplicates the device poll.

- [ ] **W4.11 (HIGH)** The capability list in
  `components/network/DeviceDetailPanel.tsx:316-320` is a hardcoded array
  derived from `device_type`, never from anything the peer reports.
  `DeviceBubble.tsx:200` renders a hardcoded clock face "10:42" plus two fake
  complication dots (`:204-205`).
  `components/settings/AdvancedSection.tsx:184` renders a hardcoded
  "~2.1 MB" storage figure with nothing behind it.

- [ ] **W4.12 (HIGH)** `websocket.ts:360` is in sync with `schema.json`
  (verified byte-identical on regeneration) but the *consumers* read fields that
  do not exist: `useAutomation.ts:122` reads `rule_id` and `timestamp` (the
  wire has `id` and `trigger_type`); `useCalls.ts:118, 148` reads
  `target_device_id` (the wire has `from`), so `streamingTarget` is always
  `null`; `useCalls.ts:121-134, 151-164` forces `stream_stopped`,
  `surround_started` and `surround_stopped` through `as any` because the schema
  has no surround-sound protocol at all.

- [ ] **W4.13 (MEDIUM)** The device cap is hardcoded and off by one.
  `PairingFlow.tsx:8` `const MAX_DEVICES = 5` ignores the user-configurable
  `settings.max_devices` (exposed at `GeneralSection.tsx:24-37`, enforced at
  `commands/pairing.rs:155-163`), and the gate at `:29` counts all rows from
  `get_devices` — including the desktop hub itself, which
  `commands/pairing.rs:71-85` always inserts — whereas the Rust counts only
  `status == "paired"`.

- [ ] **W4.14 (MEDIUM)** Four independent poll timers run for the session
  (`useDevices.ts:51` 5 s, `useFiles.ts:322-326` 10 s, `StatusBar.tsx:54` 5 s,
  `useClipboard.ts:68` **1 s**), none paused when the window is hidden. The
  10 s `useFiles` poll overwrites live WS state, so a completed transfer can
  visibly revert (`useFiles.ts:322-326`).

- [ ] **W4.15 (MEDIUM)** `useSms.ts:120-124` and `useCalls.ts:72-74` discard the
  boolean that `useWebSocket.sendMessage` returns specifically so they can
  check it. A message typed while offline is silently discarded, and
  `answerCall` (`useCalls.ts:183-186`) flips local state to `active` for a
  frame that was dropped.

- [ ] **W4.16 (PARTLY RESOLVED)** Discovery was write-only. **Half of this
  entry was stale.** `useDiscovery.ts` is *not* write-only: it subscribes to both
  mDNS events, dispatches into its reducer, handles the WebSocket announce, and
  now has a hook-level test (12 tests) that mounts it, registers a fake `listen`
  and fires the exact payload `serde_json::to_value(DiscoveredDevice)` produces.
  Every value it exposes works.

  **Still real:** `App.tsx:133` destructures only `handleDiscoveryMessage`, so
  `discoveredDevices` is populated in memory and never read — the *observable*
  claim holds even though the stated mechanism was wrong. There is no UI surface
  for discovered devices anywhere in the app, so closing this means designing
  one, which is new capability rather than a bug fix; it is left open for that
  reason and not because the plumbing is missing.

  `components/network/DeviceHub.tsx:28, 43` still declares `onShiftClickDevice`
  and `onNavigate` that `App.tsx:283-289` never passes.

- [ ] **W4.17 (MEDIUM)** The search categories do not all exist.
  `components/ui/SearchBar.tsx:9, 22-23` offers `file` and `message`, but
  `useSearch.ts:19-41` only ever produces `device` and `notification`. The
  `{ key: 'date' }` sort option (`:93`) does nothing because the comparator
  (`:98-102`) handles only `name` and `type`. Click-selected searches never
  call `saveHistory` (`:303-308` vs `:150-154`).

- [ ] **W4.18 (MEDIUM)** `useClipboard.ts:63` — `if (content && ...)` means
  clearing the clipboard never syncs, and the 10 MB cap silently drops large
  pastes with no log.

- [ ] **W4.19 (MEDIUM)** `hooks/useClipboardState.ts:38, 46, 53` await three
  mutations with **no `.catch`**, so a rejection is an unhandled promise
  rejection and the list silently keeps stale state. `App.tsx:169` has an empty
  `catch(() => {})` on `get_device_info`, after which every `clipboard/sync`
  frame is sent with `source_device: ''`.

- [ ] **W4.20 (MEDIUM)** The local capability token is cached for the process
  lifetime and never re-fetched on reconnect (`hooks/useWebSocket.tsx:131-143`).
  One early failure leaves the app permanently unpaired for the session, with
  only a `console.warn` and a UI that still says "Connected".
  *Left to do:* clear the ref on `onclose` and on `not_authenticated`.

- [ ] **W4.21 (MEDIUM)** The remote-input ack loops are broken. Two paths —
  `useSearch`'s duplicate `useDevices` and `useReadNotifications.ts:31-45`,
  which calls a side-effecting `onMarkRead` **inside a `setState` updater** —
  double-fire or target dead instances under `React.StrictMode`
  (`main.tsx:13`).

- [ ] **W4.22 (PARTLY RESOLVED)** `hooks/useSms.ts` exposes `markRead`
  (`:127-133, 147`) and `hooks/useCalls.ts` exposes `dismissCall` (`:168-170,
  231`); neither is consumed by `App.tsx:141-144`, so opening a thread never
  clears its unread count. `components/messages/MessageInput.tsx:7` declares a
  `disabled` prop that `MessageThread.tsx:60` never passes, so the composer is
  never disabled while offline.
  **The `useSms` half is fixed:** selecting a thread now clears its unread count
  on both ends — locally and by telling the phone — and `markRead` sets
  per-message read state and sends `sms/mark_read`, no-op when already read.
  Unlike W2.14 this was *not* a missing model: `msg_read`/`msg_outgoing`,
  `unread_count` on the wire and `SmsMessage.read` all existed. The only fault
  was that the desktop never invoked the writer.
  **Still open:** `dismissCall`, and the `MessageInput.disabled` prop.

- [ ] **W4.23 (MEDIUM)** `components/settings/AdvancedSection.tsx:112-126` —
  `handleClearCache`'s catch reports *"Cache cleared"*, the same message as the
  success path, so a failure is reported as success. The `keysToKeep` list
  (`:115`) is hardcoded and omits `conduit-read-notifications`, so "Clear Cache"
  wipes the user's read state.

- [ ] **W4.24 (MEDIUM)** Pairing error surfaces give no feedback.
  `PairingFlow.tsx:223-226` — "Troubleshoot connection" offers no success or
  failure state and is a guaranteed failure off Windows.
  `components/pairing/QRCode.tsx:16-23` — a failure leaves `src` empty, so the
  user sees "Loading QR…" forever.

- [ ] **W4.25 (MEDIUM)** Two dead screens. `components/layout/TitleBar.tsx` —
  all 144 lines, never imported anywhere. `components/ui/InteractiveTutorial.tsx`
  — 181 lines, unreachable because `useAppShell.ts:18` initialises
  `showTutorial: false` and `App.tsx:204, 222` only ever sets it false.
  Also `components/layout/FloatingDock.tsx:230-267` — 38 lines behind a prop
  `App.tsx:450-456` never passes.

- [x] **W4.26 (RESOLVED, one deliberate exclusion)** `automationMeta.ts:25-44`
  hand-maintained 7 trigger and 8 action labels over generated unions declaring
  8 and 9. `audio_device_disconnect` and `set_window_state` had no UI entry, and
  the file's own comment claimed omissions were a compile error — they were not,
  because the arrays were explicitly typed as a subset.
  **That was the more interesting half of the finding.** The arrays were
  annotated `Record<TriggerType, …>` / `Record<ActionType, …>`, which widens the
  literal back to the full union and makes the annotation a no-op. The first
  attempt at a compile-time coverage assertion was *vacuous for exactly this
  reason*. Switched to `as const satisfies`, which both adds the missing
  `audio_device_disconnect` entry and makes a future omission a real
  `TS2344`.
  **Still excluded on purpose:** `set_window_state` has no UI entry and is
  recorded as an explicit `Exclude<>` with the reasoning — `ActionEditor` has no
  control for `state`, so the rule would carry `{type: 'set_window_state'}`, which
  the Rust parser rejects outright. Offering an option that is guaranteed to fail
  would be worse than not offering it.
  `@ts-nocheck` is also gone from `automationMeta.ts`, `ActionEditor.tsx`,
  `AutomationPanel.tsx` and `useAutomation.ts`.

- [ ] **W4.27 (MEDIUM)** Design-system leakage. ~40 hardcoded `rgba()`/`hex`
  literals across `ContextMenu.tsx:107`, `SearchBar.tsx:62`,
  `PairingFlow.tsx:211-280`, `EmptyState.tsx:27`, `NotificationCard.tsx:104-115`,
  `SwipeableCard.tsx:27-28`, `GeneralSection.tsx:8`, `NotificationsSection.tsx:84`,
  `FilesSection.tsx:26`, `AppearanceSection.tsx:29`, `ActionEditor.tsx:39, 86`,
  `TriggerEditor.tsx:38`, `MessageThread.tsx:83`, plus ~30 raw-palette Tailwind
  classes in `FileCard.tsx:38-48` and `ClipboardCard.tsx:56-91`. None respond to
  the light theme. `DeviceBubble.tsx:30-317` paints seven 3-D models with
  hardcoded hex that ignore the accent.

- [ ] **W4.28 (MEDIUM)** Google Fonts are requested **twice** — an HTML `<link>`
  in `index.html:9-11` and a render-blocking CSS `@import` in
  `styles/globals.css:1`, with a different Inter weight range. The app also
  makes a third-party request on every launch, which contradicts
  `Onboarding.tsx:45` ("nothing is sent to a server") and `SECURITY.md:6-7`.
  `tauri.conf.json:28` lists the font CDNs under `connect-src` as well as
  `style-src`/`font-src`, which is almost certainly a copy-paste error.
  *Left to do:* self-host the two files; drop the font origins from
  `connect-src`.

- [ ] **W4.29 (MEDIUM)** `config.ts:20` — `WS_URL` is a hardcoded
  `ws://127.0.0.1:9527` with no `import.meta.env` override and no Vite proxy,
  so a dev build cannot target another backend. The same port is duplicated in
  `tauri.conf.json`'s CSP; a change to one silently breaks the other with a CSP
  violation rather than a clear error. `config.ts:41`
  `FILE_CHUNK_SIZE_BYTES` is never imported; the real size is hardcoded again at
  `useFiles.ts:249`. `config.ts:25` `APP_ID = 'com.conduit.app'` contradicts
  `tauri.conf.json:5` (`"app.conduit.desktop"`) and is never used.

- [ ] **W4.30 (LOW)** Dead props and functions: `getFileIcon` chain
  (`useFiles.ts:32-40` → `MergedMasonry.tsx:92` → never read by `FileCard.tsx`);
  `invokeWithTimeout` (`lib/tauri.ts:7-12`); `DEVICE_ICON_MAP` (`lib/utils.ts:17-29`);
  `ENVELOPE_TYPES` (`useEncryption.ts:113-115`); `SkeletonPage`
  (`components/ui/Skeleton.tsx:54-60`); `battery`/`os` on `DeviceBubble`;
  `deviceCount` on `FloatingDock`; `timeAgo` on `CallHistory`/`MessageBubble`.
  Two different `formatTime` functions with the same name and different output
  are both live (`lib/utils.ts:31-44` relative, `lib/time.ts:19-22` absolute) —
  a notification and a message from the same minute render different timestamps.
  `components/ui/SwipeableCard.tsx:35` writes `dragging` and never reads it.

- [ ] **W4.31 (LOW)** Dead CSS: `.glass-ball--noblur`, `.gradient-text`,
  `.list-item-contain`, the `.starfield` selector (no element ever receives the
  class), 6 animation classes with 6 keyframe blocks, 10 unused custom
  properties, and a local `@keyframes spin` that shadows Tailwind v4's.

- [ ] **W4.32 (LOW)** `10` files carry `// @ts-nocheck`
  (`useWebSocket.tsx:3`, `useFiles.ts:1`, `useCalls.ts:1`, `useAutomation.ts:1`,
  `useMessageHandlers.ts:1`, `PairingFlow.tsx:1`, `AutomationPanel.tsx:1`,
  `ActionEditor.tsx:4`, `automationMeta.ts:1`, `__tests__/mocks.ts:1`). This is
  what lets `AutomationAction` be imported from a module that does not export it
  with no error. It is also 9 of the 12 eslint errors.

- [ ] **W4.33 (LOW)** `__mocks__/tauri.ts` is three lines: only `invoke`, no
  `getCurrentWindow`, `listen`, `transformCallback` or `metadata`.
  `vitest.config.ts:19` aliases `@tauri-apps/api/window` to it, so
  `useAppShell.ts:56` destructures `undefined` and throws — swallowed by the
  `catch` at `:59-61`. The clipboard, dialog and fs plugins are not aliased at
  all (`:16-21`), so those tests load the real plugin in jsdom.

- [ ] **W4.34 (LOW)** `e2e/fixtures.ts` has drifted from the backend: six
  `defaultSettings` keys do not exist in `ConduitSettings` (`:27-34`), two are
  missing, `get_system_info` has two fields the Rust does not return (`:45`),
  every collection mock is empty (`:61-64`), and `check_for_update` /
  `install_update` mock commands that do not exist (`:72-73`).

- [ ] **W4.35 (LOW)** `components/calls/IncomingCall.tsx` is still on the
  pre-`ink-*` token set (`:132-148, 228, 236, 273, 282, 293, 315, 359, 390`)
  and is styled `.surf-glass` — a dark obsidian surface — even in `embedded`
  mode, where `CallsNotificationsSplit.tsx:65` places it inside a white
  `.surf-frost` panel.

---

## W5 — Build, CI and dependencies

- [ ] **W5.1 (PARTLY RESOLVED)** A fresh clone cannot build the desktop app.
  `apps/desktop/src-tauri` depends on `rusqlite` with `bundled-sqlcipher`,
  which links against OpenSSL. `.cargo/config.toml:8-10` supplies it
  unconditionally via `[env]` with `relative = true`, and `.tools/` is
  gitignored (`.gitignore:51`), so a clone never receives it.

  **Closed: the bootstrap.** `scripts/fetch-openssl.ps1` now exists. It pins
  OpenSSL **3.5.8**, verifies size → SHA-256 *before* extracting, then asserts
  the four files the build actually needs (`version.txt`, `opensslv.h`,
  `libcrypto.lib`, `libssl.lib`). The expected digest is GitHub's own published
  asset digest, so the pin is independently checkable rather than a hash
  somebody typed. `-Check` verifies without downloading; `-Force` replaces.
  Running it produced `libcrypto.lib`, `libssl.lib` and `opensslv.h`
  **byte-identical (SHA-256)** to the copy already in use, which is the
  strongest evidence available that the pin is right.

  **Option B in the old entry does not work, and this was measured rather than
  assumed.** The proposed "one-line change" — moving `[env]` to
  `[target.x86_64-pc-windows-msvc.env]` — is **not implementable** on cargo
  1.98.1:

  | Config | Result |
  |---|---|
  | `[env]` + `{value, relative}` (current) | **works** |
  | `[target.x86_64-pc-windows-msvc.env]` + `{value, relative}` | **hard parse error**: `expected a string, but found a table` |
  | `[target.x86_64-pc-windows-msvc.env]` + plain string | parses, **never exports the variable** |
  | `[target.'cfg(windows)'.env]` + `{value, relative}` | parses, **never exports the variable** |

  Plain strings are not a fallback either: without `relative` the value resolves
  against the process CWD, and build scripts do not run in cargo's invocation
  directory. The change was reverted rather than forced through.

  **Still open:** Linux and macOS still get the same Windows `.lib` path with no
  guard, so there is no working non-Windows desktop build path. Options A (fetch
  script exports into its own process) and C (`vcpkg` / `libssl-dev` /
  `brew install openssl@3`) remain. **We still do not know whether this even
  manifests on Linux** — the CI build died on `alsa-sys` before reaching
  `libsqlite3-sys`, so it is unknown whether that build script prefers
  `OPENSSL_LIB_DIR` over pkg-config.

- [ ] **W5.2 (CRITICAL)** The vendored OpenSSL static libraries may not be
  redistributable, and `tauri.conf.json:38` bundles them.
  `.tools/openssl-win64/README.txt` marks `include/` and `lib/static/`
  `[non-redistributable]` — exactly the two directories `.cargo/config.toml`
  points SQLCipher at, and they are **statically** linked. The project has no
  CLA, no `NOTICE`, and no `SECURITY.md` entry on third-party redistribution
  terms. This is the one item that could block a release outright.
  *Left to do:* read the actual licence terms (the packaging labels describe the
  archive, not the licence); if restricted, switch to the dynamic
  `lib/import/*.lib` plus the shipped DLLs, or adopt W5.1 option C; add a
  third-party-licences file.
  `fetch-openssl.ps1` now prints this warning at the end of a successful run, so
  whoever fetches the binaries is told, rather than only whoever reads the
  archive's own README. No binaries were committed and the licence question is
  deliberately still yours to decide.

- [ ] **W5.3 (CRITICAL, diagnosis corrected — still open)** The desktop crate's
  tests do not run in CI. **Every stated reason was wrong; the substance is
  real.** `cargo test -p conduit --locked` *does* exist (`ci.yml:225`) and *does*
  execute — but it dies in a build script before running a single test:

  ```
  error: failed to run custom build command for `alsa-sys v0.4.0`
  pkg-config exited with status code 1
  Package alsa was not found in the pkg-config search path.
  ```

  All three desktop steps exit **101**. Three corrections: it is **not**
  `continue-on-error` — the step runs and fails; the cause is **not** W5.1 or
  OpenSSL, it is a **missing apt package**; and "699 of 1154" is stale — the
  baseline is 748.
  `libasound2-dev` is now installed in the workflow's apt line, which is the
  observed blocker and is a real package in Ubuntu noble *and* resolute (so it
  survives the announced `ubuntu-latest` migration). **This is unverified end to
  end:** there is no Docker or WSL on the machine this was fixed from, and the
  next native crate to fail is unpredictable. `keyring` needs a Secret Service
  at *run* time and a headless runner has none, so the crate may compile and
  still fail tests.

  Also newly found: `cargo audit` and `cargo deny check advisories` both exit
  non-zero on real advisories (RUSTSEC-2024-0429 `glib` unsoundness, a yanked
  `yoke-derive`, ~10 unmaintained), hidden behind `continue-on-error`.

- [x] **W5.4 (RESOLVED — was stale)** The CI workflow has never been executed.
  **False, and decisively so.** Verified against the GitHub API rather than
  assumed: there are 15 runs of `ci.yml`, and **four consecutive green runs on
  `main`, the last being `e21d715`** — `36737997124`, `36834961099`,
  `36840097378`, `36850507922`, 6/6 jobs each. `origin` is
  `Snehishere/Conduit.git` (not empty) and there are 9 commits (not 5).
  The "NEVER BEEN EXECUTED" banner is gone from `ci.yml`, replaced with the
  actual history and the list of steps currently red.

  **One part was true and is kept:** the `hygiene` job's entitlements step is
  genuinely red (exit 1 — `tauri.conf.json` references an `./Entitlements.plist`
  that does not exist on disk). Non-blocking, as before. And the per-step
  "Verified" comments *were* written before the first run, which is worth
  remembering rather than treating as a defect.

- [ ] **W5.5 (HIGH)** `dtolnay/rust-toolchain@master` (`ci.yml:111`) pins a
  **mutable branch**, not a version tag — and it is the step that installs the
  toolchain that compiles every Rust step. Twelve other `uses:` are not
  SHA-pinned (`:98, 130, 290, 292, 377, 413, 415, 479, 481, 542, 598, 668`).
  Tracked as a decision at `ci.yml:57-61`, but a branch ref is not even
  covered by that reasoning.

- [ ] **W5.6 (HIGH)** 74 duplicated crate names in `Cargo.lock`, not 5. The root
  `Cargo.toml:33-44` names tungstenite, tokio-tungstenite, rand, rcgen, sha2 and
  thiserror. Also duplicated, and more consequential because they are the
  transitive crypto stack: **`hmac` 0.12 + 0.13** (two HMAC implementations in
  one binary), `digest`, `block-buffer`, `crypto-common`, `sha1`, `rand_core`,
  `getrandom` (three copies), `base64` (three copies), `dirs` 6 + 7, `pem`,
  `nom`, `bitflags`, `syn`, and **both `ring` and `aws-lc-rs`**.
  `hmac`, `sha2` and `rand` are exactly the crates that produce Conduit's wire
  signatures, so "the most consequential is tungstenite" (`Cargo.toml:36`) is
  arguable at best.
  *Left to do:* extend the comment to the full list; add the crypto crates to
  the Dependabot ignore block; converge crate by crate. `rand 0.8` → `0.10` is
  a real code change in the relay (`main.rs:220-221` uses `thread_rng().fill`),
  and `tungstenite` 0.24 → 0.30 changes `Message::Text(String)` to
  `Utf8Bytes` at `relay/src/main.rs:1385, 1597, 1669, 1710`.

- [ ] **W5.7 (HIGH)** Two TypeScript compilers are installed simultaneously.
  `apps/desktop/package.json:49` `typescript: "npm:@typescript/typescript6@^6.0.2"`
  and `:58` `@typescript/native: "npm:typescript@^7.0.2"`. `npx tsc` resolves
  through `@typescript/native`, so the build typechecks with TS 7 while
  `eslint.config.js:16` runs `strictTypeChecked` — written against the TS 6 API
  — under a TS 7 typechecker. The alias is also a non-standard package name, so
  most tooling resolves the real `typescript` instead.
  *Left to do:* pick one, delete the other, re-baseline typecheck + test + lint
  + build in a single commit.

- [ ] **W5.8 (HIGH)** Seven `@tauri-apps/*` packages are a full minor behind and
  Dependabot structurally cannot fix them. Every range is `^2.x.y`, so 2.11.1 →
  2.12.0 already satisfies the declared range; no PR is ever opened, and the
  committed `package-lock.json` pins the stale version in CI. The
  `npm-patch` group at `dependabot.yml:144-148` can never fire for them.
  *Left to do:* `npm update` in `apps/desktop`, commit the lockfile delta,
  re-run the full check set.

- [ ] **W5.9 (HIGH)** No supply-chain or licence enforcement.
  `cargo audit` (`ci.yml:248`) and `cargo deny check advisories` (`:259`) are the
  only checks and both are `continue-on-error`. There is **no `npm audit` step**
  (it is currently clean, 0 vulnerabilities) and **no `deny.toml`** — so
  `dependabot.yml:270-274` explicitly disabled `cargo deny check licenses bans`.
  A `deny.toml` with an `allow` list is the control that would have caught W5.2.
  *Left to do:* write `deny.toml`; promote both deny steps to blocking; add
  `npm audit --audit-level=high`; add `flutter pub outdated`.

- [ ] **W5.10 (MEDIUM)** `quicktype@26.0.0` pulls `vm2@3.12.2` into the dev
  tree via `typescript-json-schema` → `quicktype-typescript-input`.
  `vm2` is a long-archived sandbox with a history of unpatched RCE advisories.
  `npm audit` reports 0 because those live in GitHub advisories, not the npm
  DB — false assurance. Conduit only uses quicktype's Dart output
  (`scripts/generate_dart.js:52-53`).
  *Left to do:* add an `overrides` entry or drop the unused input plugin;
  replacing `generate_dart.js` with a hand-written projection (as
  `generate_types.js` already is) removes the whole chain.

- [ ] **W5.11 (MEDIUM)** `build_icons.py --check` rewrites the working tree.
  `scripts/icons/build_icons.py:306` calls `build()` unconditionally *before* the
  `--check` branch at `:308`, and `build()` writes all 36 assets. The script's
  own docstring (`:27`) and argparse help (`:299`) both claim it verifies
  without writing. `package.json:8` wires it to `prebuild`, so
  `npm run tauri -- build` dirties 36 tracked files on every release build.
  *Left to do:* render into a `TemporaryDirectory` and compare in memory, or
  read the committed bytes via `git show HEAD:<path>` as
  `compare_generated.py:61-68` already does.

- [ ] **W5.12 (MEDIUM)** The three `build-*.ps1` scripts print "Build complete!"
  and exit 0 on failure. `$ErrorActionPreference = "Stop"` does not apply to
  native executables in Windows PowerShell 5.1, and none of
  `build-desktop.ps1`, `build-mobile.ps1` or `build-relay.ps1` checks
  `$LASTEXITCODE`. `build-icons.ps1:27, 33` gets it right, so the pattern is
  known and simply not applied. None of the four has a `-WhatIf`/dry-run.

- [ ] **W5.13 (MEDIUM)** The e2e suite is unrunnable out of the box. There is
  no `playwright install` bootstrap; running `npm run test:e2e` produced
  **31/31 failures** with "Executable doesn't exist". Four assertions are also
  guaranteed to fail against the current copy, because Playwright `toHaveText`
  is case-sensitive: `automation-rule-crud.spec.ts:31, 64` ("Create Rule" vs
  "Create rule"), `pairing-flow.spec.ts:56` ("Troubleshoot Connection" vs
  "Troubleshoot connection"), `settings-page.spec.ts:55` ("Saved!" vs "Saved").
  Two specs also assert contradictory nav-button counts
  (`dock-navigation.spec.ts:34` and `revision3-surfaces.spec.ts:57`).
  *Left to do:* add a `pretest:e2e`; fix the four strings; pick one count.

- [x] **W5.14 (RESOLVED)** CI had no `timeout-minutes` and no `permissions:`
  block. Both are now in `ci.yml`: `permissions: contents: read` (every step was
  reviewed for a write requirement — there are none; no deploy, no publish, and
  all caches use the runtime token) and per-job `timeout-minutes` of
  60/45/20/20/45/10, each set against that job's observed duration rather than
  picked round.
  `cargo install cargo-audit` and `cargo install cargo-deny` in the `rust` job
  are still the long pole; the 60 is sized for them.

- [ ] **W5.15 (MEDIUM)** `scripts/generate_dart.js:59-76` silently falls back to
  `npx --yes quicktype` when the local install is missing, downloading whatever
  is latest. A contributor with a partial `node_modules` gets different output
  and an unexplainable `codegen` failure on their PR.
  *Left to do:* `process.exit(1)` with a "run `npm ci`" message.

- [ ] **W5.16 (MEDIUM)** The two code generators use CommonJS `require()` in a
  repo whose only `package.json` declares `"type": "module"`. They work only
  because no `package.json` is a parent of `scripts/`. Adding a root
  `package.json` (a workspaces migration is the obvious trigger) breaks both at
  load with `ERR_REQUIRE_ESM`.
  *Left to do:* rename both to `.cjs`.

- [ ] **W5.17 (MEDIUM)** `apps/desktop/.npmrc` sets `legacy-peer-deps=true`,
  which suppresses exactly the conflicts that would flag W5.7 and the
  `@typescript-eslint` duplication. It also makes `npm install` in CI resolve
  differently from `npm ci`.

- [ ] **W5.18 (MEDIUM)** Unused and redundant dependencies:
  `@tauri-apps/plugin-shell` (zero imports, Rust plugin deliberately
  unregistered), `@tauri-apps/plugin-process` (zero imports),
  `esbuild` (Vite bundles its own), the two scoped `@typescript-eslint` packages
  (the umbrella re-exports both), `@tauri-apps/plugin-fs` needing a companion
  permission it does not have (W3 capability), and `flutter_animate` on mobile.

- [ ] **W5.19 (MEDIUM)** `scripts/lint-all.ps1:170-171` hardcodes
  `C:\flutter\bin` as a fallback path, and `docs/DEVELOPMENT.md:88-93, 601`
  instruct every contributor to use that exact directory.

- [ ] **W5.20 (LOW)** `scripts/build-icons.ps1` is referenced by nothing — no
  CI step, no `package.json` entry, no doc. It is the only Windows-friendly
  icon entry point.

- [ ] **W5.21 (LOW)** `scripts/generate-cert-pin.sh` is bash-only. It is the only
  documented way to obtain a relay pin and the primary audience is Windows.

- [ ] **W5.22 (LOW)** `scripts/icons/verify_icons.py` never reads
  `assets/brand/icon-manifest.json`, and `build_icons.py:312-323` (the
  manifest-writing path) is skipped in `--check` mode — so the committed 36-entry
  hash table can be stale with no check noticing.

- [ ] **W5.23 (LOW)** The mobile `integration_test/` suite (5 files, ~2000 lines)
  is wired into nothing — no CI step, no script.

- [ ] **W5.24 (LOW)** `dart format` (63 of 74 files unformatted) and
  `flutter analyze --fatal-infos` are both non-blocking. The stated reason for
  the latter is wrong: 365 of the 367 infos are in the generated
  `lib/models/protocol.dart`, which `analysis_options.yaml` does not exclude.
  Excluding one file would let this become a gate.

- [ ] **W5.25 (LOW)** `e2e/`, `vitest.config.ts`, `playwright.config.ts` and
  `eslint.config.js` are never typechecked — `tsconfig.json:20` references
  `./tsconfig.node.json` but `npm run build` runs bare `tsc`, not `tsc -b`, so
  the reference is never built.

---

## W6 — Relay library

### W6 status after re-audit at `5b1b694` + the relay_delivery work

**Re-audited against the tree, not carried over.** Every item below was
re-derived from the current modules. Where an item was already settled, it says
so and says how that was checked; the ones that survive are the ones that were
reproduced in the current code, with the current line reference.

Items are renumbered into three groups: **OPEN** (still reproducible),
**SETTLED** (no longer true, with the evidence), and **OBSOLETE** (the artefact
they describe is gone and there is nothing to action).

#### Settled by the service → library refactor

- **The health listener is loopback-pinned and tested.** W6.20 as written is
  resolved: the desktop pins `health_bind` and `ws_bind` to loopback in
  `build_config` (`apps/desktop/src-tauri/src/relay.rs`), deliberately not
  overridable by the environment, and `conduit-relay` covers it
  (`the_health_surface_is_always_loopback`,
  `the_plaintext_listener_defaults_to_loopback`,
  `graceful_shutdown_stops_health_listener`). No compose file is involved.
- **The shared signing key and its rotation window are gone entirely.** There is
  no shared message-signing key in the tree. `RELAY_SIGNING_KEY`,
  `RELAY_SIGNING_KEY_ID`, `RELAY_SIGNING_KEY_PREVIOUS` and
  `RELAY_SIGNING_KEY_PREVIOUS_ID` have all been removed. Signing is per-device:
  HMAC-SHA256 keyed by the pairing secret over
  `"conduit-protocol/v1/derive:conduit-relay/v1/route-key:" + device_id`, with
  `key_id == from_device_id`. See
  [ADR-0011](decisions/0011-per-device-relay-route-keys.md), which supersedes
  [ADR-0004](decisions/0004-domain-separated-relay-signing-key.md).
- **`-p relay` no longer resolves.** The crate is `conduit-relay`. Every command
  in this file that says `cargo test -p relay` is wrong; use `-p conduit-relay`.
- **The relay is not optional and there is nothing to deploy.** `relay_enabled`
  defaults to true, and a relay restart is a desktop restart.

#### OPEN — reproduced in the current tree

- [x] **W6.1 (RESOLVED)** No size cap on binary frames. **Fixed, all three
  halves.** Binary frames were accepted with no application-level check while
  both Text arms enforced `MAX_TEXT_SIZE`, and the relay used `accept_async`
  with no `WebSocketConfig` — so tungstenite's 64 MiB default was the only bound
  in the entire path. The `to_vec()`'d payload then went into a 1024-deep queue
  with no byte budget, giving roughly 64 GiB per target connection from one
  authenticated client.

  The fix is layered, and the layers are not redundant:

  - **Read-time** — `accept_async_with_config` with
    `max_message_size`/`max_frame_size` at `MAX_TEXT_SIZE`. This is the layer
    that matters most: it refuses the frame *while it is being read*, before the
    bytes are allocated. The application check arrives too late to save that.
  - **Application** — `MAX_BINARY_SIZE` (`= MAX_TEXT_SIZE`) checked in both the
    auth loop and the main loop, mirroring the Text arms, so the ceiling holds
    even if the `WebSocketConfig` is ever changed. `SIZE_BUCKETS` already topped
    out at exactly 1 MiB, so the metrics were built assuming this ceiling.
  - **Queue** — `state::Queue` pairs each connection's `mpsc::Sender` with a
    `Semaphore` of `QUEUE_BYTE_BUDGET` (16 MiB). Every message holds
    `message.len()` permits while queued and the writer returns exactly those on
    dequeue, so queued *bytes* are bounded however deep the channel runs. The
    raw sender is deliberately not exposed: this accounting only holds if every
    producer reserves, and a future `tx.send` that bypassed it would quietly
    inflate the budget back towards unbounded.

  Worst case per target connection went from ~64 GiB to 16 MiB.

  Regression tests, each verified to fail with its fix removed: an e2e test
  drives a `MAX_BINARY_SIZE + 1` frame through a real socket and asserts the
  connection is dropped and the device deregistered (fails with both the
  read-time and application caps reverted); two `Queue` tests assert the budget
  refuses an over-ceiling frame while the channel still has free slots, and that
  a release returns exactly one frame's worth and no more (both fail with the
  reservation removed).

  `packages/protocol/PROTOCOL.md` §2.5 documented the old behaviour — including
  the claim that the relay passes no `WebSocketConfig` — and is updated.

- [x] **W6.6 (RESOLVED)** A read lock is held across `await`. **Fixed.**
  `forward_text_with_timeout` and `forward_binary` took
  `state.clients.read().await`, borrowed the target out of it and then held the
  guard for the full `FORWARD_TIMEOUT_SECS` while the send was in flight. Every
  writer blocked meanwhile — registration, deregistration and the 30 s sweep —
  and tokio's `RwLock` is write-preferring, so one slow target stalled the whole
  routing table.

  Both now clone the `Queue` out of a short-lived guard (`route.rs:346`,
  `route.rs:412`) and await the send after it is dropped. The clone shares the
  `Arc<Semaphore>` underneath, so W6.1's byte budget is unaffected: reservation
  and release still meet on the same semaphore however long the guard lived.
  `a_slow_forward_does_not_hold_the_routing_table_lock` proves writers proceed
  while a forward is parked on a stalled target.

- [x] **W6.4 (RESOLVED)** A stale disconnect deregisters the live device.
  **Fixed.** Registration stored only the `Queue` and teardown removed by key,
  so a connection that lost the race with its own replacement deleted the *new*
  connection's entry: the device stayed connected and became permanently
  unroutable, and `reconcile_clients` could not repair it because the entry it
  would prune looked perfectly healthy.

  Every `Queue` now carries a `connection_id` minted from a process-wide atomic
  (`state.rs:139`), and teardown goes through `deregister_if_current`
  (`state.rs:280`), which removes only when the stored id still matches. A
  superseded connection's late teardown is a no-op rather than a hijacking.
  Two tests drive the exact interleaving (old disconnect lands after new
  registration).

- [ ] **W6.5 (HIGH)** Binary frames have no cross-connection replay protection.
  `connection.rs:264` holds `last_binary_seq` as a per-connection in-memory
  `Option<u32>`, reset on every reconnect. A reconnect or a restart resets it,
  after which any captured frame replays forever. The `relay_route` path has the
  persisted nonce cache; this path has nothing. The delivery re-framing added for
  `relay_delivery` does **not** change this: it re-stamps the sequence per
  connection, which is correct for the receiver's own guard and is still not a
  durable replay bound.
  *Left to do:* add a timestamp and nonce (a frame-version bump), or persist a
  per-device high-water sequence number alongside the nonce cache. This needs a
  wire-format decision and is deliberately untouched.

- [x] **W6.2 (RESOLVED)** The `/health` bearer token defaulted to the master
  HMAC secret. **Fixed.** `with_health_token_fallback` copied `hmac_secret` into
  `health_token`, so the credential gating the health endpoint *was* the secret
  the nonce cache is keyed from — one credential, two roles.

  Resolution no longer copies anything. An explicit `RELAY_HEALTH_TOKEN` wins;
  otherwise `derive_health_token` (`config.rs:589`) produces a token as
  `hex(derive_key(secret, "conduit-relay/v1/health-token"))` — stable across
  restarts, non-invertible, and domain-separated from every other key derived
  from the same secret. `with_derived_health_token` (`config.rs:601`) fills the
  default in one place and `effective_health_token` still yields a never-empty
  value for hand-built `Config`s. `docs/PROTOCOL.md`'s env table and both ADRs
  that described the old default are updated.

- [x] **W6.7 (RESOLVED)** Handshakes were untimed and uncounted. **Fixed.**
  Admission ran *after* the TLS handshake, so a client that completed TCP and
  never sent a `ClientHello` held a task and an fd indefinitely; the WebSocket
  upgrade was untimed; and the per-IP rate limit sat after the upgrade and so
  bounded neither path.

  - `accept_tls` (`service.rs:917`) wraps the handshake in a
    `TLS_HANDSHAKE_TIMEOUT_SECS` (10 s) budget.
  - `UpgradeDeadline<S>` (`service.rs:691`) gives the upgrade its own
    `WS_UPGRADE_TIMEOUT_SECS` (10 s) budget, disarmed on the first completed
    write so an established connection is never judged by it.
  - `admit` (`service.rs:953`) now runs **before** either, in the order
    rate-limit → guard → cap, and returns `Option<ConnectionGuard>` to the
    caller. Because the guard is the caller's to hold, every early return drops
    the slot by construction.

  Two corrections to the audit's description while doing this: the old `admit`
  created its guard locally and dropped it at function end, so `active_connections`
  netted zero and `MAX_CONNECTIONS` never actually engaged — the upgrade was
  *not* holding a slot, there was no slot to hold. And the per-IP limit was
  applied twice (pre- and post-upgrade), halving the effective rate for any
  client that finished the handshake.

- [x] **W6.8 (RESOLVED)** The auth deadline was per-message, not total.
  **Fixed.** `auth_timeout` was re-armed on every frame, so one junk frame every
  9 s held an unauthenticated connection and a slot indefinitely. There is now a
  single `auth_deadline` (`connection.rs:149`) set once when the socket is
  admitted, driven through `timeout_at` rather than `timeout`. The explicit
  top-of-loop check exists because `tokio::time::Timeout` polls the read first
  and would let a frame arriving exactly on the deadline be processed anyway.

- [x] **W6.15 (RESOLVED)** Self-signed certificates were valid for ~2000 years.
  **Fixed, all three halves.**
  - rcgen's default window is 1975-01-01 → 4096-01-01. Generated certs now get
    an explicit window: 7 days of backdate (client clock skew) and
    `GENERATED_CERT_LIFETIME_DAYS` = 3653 days, i.e. a finite, checkable ten
    years rather than a span in which nothing can ever be expired. Regeneration
    is deliberately operator-triggered — silently rotating the cert would
    silently rotate the SPKI pin clients hold at `GET /pin`.
  - `key_usages`/`extended_key_usages` are now asserted (`DigitalSignature` +
    `ServerAuth`). The acceptor is TLS 1.3-only, so `keyEncipherment` is
    deliberately *not* claimed — it would advertise key transport TLS 1.3
    removed.
  - `load_tls_context` validates a supplied certificate's window through
    `check_validity_window` (`tls.rs:774`) and refuses rather than warns: rustls
    never checks the window of the cert it serves, so without this the relay
    starts "healthy" and fails every client. That matches how a mismatched
    cert/key pair is already refused.

  Hand-rolled UTCTime/GeneralizedTime parsing reuses the file's existing DER
  walk, so no date dependency was added.

- [ ] **W6.12 (MEDIUM)** Cross-restart replay protection depends on a 30 s flush
  against a 35 s freshness window, so a crash loses up to 30 s of accepted
  nonces. **Half fixed:** `load_nonces` no longer returns an empty map on *any*
  read error. A missing file (first start) is still empty; every other read
  failure and every parse failure now logs with the path and cause and refuses
  to start, rather than silently handing the relay an empty cache.
  *Left to do:* the flush-versus-freshness window itself, which is a policy
  decision between write amplification and crash exposure. Untouched.

- [x] **W6.13 (RESOLVED)** The global `MAX_NONCES` ceiling could evict another
  device's in-window nonces. **Fixed.** Per-device quota is 4096 and the
  process-wide cap was 10 000, so three devices at quota exceeded it and the
  eviction loop then removed the globally oldest entry — possibly belonging to a
  device still under its own quota, silently destroying *its* replay protection.

  The global loop now picks its victim through `oldest_device_at_quota`
  (`lib.rs:548`), which considers only devices already at quota, and stops when
  none qualifies: the isolation invariant wins and the ceiling yields. The hard
  bound becomes one quota per registered device rather than 10 000 entries.
  `load_nonces` had the mirror-image bug — it truncated to the globally most
  recent 10 000 on load, which would have stripped an under-quota device's
  in-window nonces on *every* restart — and now truncates per device.

- [x] **W6.19 (RESOLVED)** `bearer_token_authorized` had no empty-expected-token
  guard. **Fixed:** an empty expected token is refused before any comparison
  (`config.rs:630`). This was never reachable only because
  `effective_health_token()` fell back to a never-empty value — which is exactly
  the accidental backstop W6.2 removed, so the guard is what actually closes it.

- [x] **W6.21 (RESOLVED)** No minimum entropy on secrets, and silent port
  fallback. **Fixed.** `MIN_SECRET_LEN = 8` bytes after trimming
  (`config.rs:323`) now applies to the relay token, the HMAC secret and explicit
  health/metrics tokens; a whitespace-only value counts as unset rather than
  as a one-glyph credential. Error messages state the required length and never
  echo the value. `env_port` (`config.rs:543`) returns `Result` and a
  set-but-unparseable port is now a hard fail-closed error naming the variable
  and the bad value — `RELAY_WSS_PORT=95x9` used to bind 9529 without a word.

- [x] **W6.22 (RESOLVED)** `Config` derived `Debug` while holding
  `hmac_secret` and `relay_token`. **Fixed:** `#[derive(Debug)]` is gone and a
  manual impl (`config.rs:147`) prints both as `[REDACTED n bytes]`, keeping the
  shape of the output useful. A repo-wide grep found no `{:?}` of a `Config`
  today, so the exposure was latent rather than live — but `panic!`/`expect`
  formatting is not where you want to discover it.

- [ ] **W6.23 (MEDIUM)** Operational constants are hardcoded in `limits.rs` and
  are not configurable: `MAX_TEXT_SIZE`, `MAX_CONNECTIONS`, message rate and
  burst, forward timeout, queue depth, ping and idle-read intervals. Only
  `RELAY_AUTH_TIMEOUT_SECS` is tunable. The 10/60 s connect rate breaks any
  NAT'd deployment with more than 10 devices.
  *Left to do:* deliberately untouched this pass — making these configurable is
  a wide surface (env, `Overrides`, host settings, docs) rather than a bug fix.

- [x] **W6.27 (RESOLVED)** The unscoped free function `check_replay` is gone.
  It was superseded by `NonceCache::check_replay`, used only by its own tests,
  and was a footgun: unscoped, a single cap, no per-device isolation. A
  repo-wide grep confirmed the only callers were its own tests. A
  `compile_fail` doctest on the `hmac` module now pins the old path as a
  compile error, so the deletion is itself regression-tested.

#### SETTLED — re-checked against the current tree, no longer true

- [x] **W6.3** A mismatched cert/key pair panics instead of failing closed.
  **Resolved.** `load_tls_context` now validates before building: `tls.rs:224`
  and `:226` carry `expect`s whose premises are now guaranteed, and the suite
  covers the bad-pair path (`cert_key_mismatch_is_refused`, plus the
  "no private key found" and key-format cases). The claim in
  `docs/relay-tls.md` that this was unhandled is stale.

- [x] **W6.16** A partial cert directory is destructive.
  **Resolved.** `tls.rs:159` loads only when `cert_path.exists() &&
  key_path.exists()`, so a half-present directory can no longer overwrite an
  operator's `cert.pem` and then fail on the key. The misleading
  "Failed to create key file with restricted permissions" path is gone.

- [x] **W6.26** Unused direct dependencies.
  **Partly resolved.** `services/relay/Cargo.toml` now has a `license` field, and
  the dev-dependency is `serial_test` (used), not `tokio-test`. `serde` is
  still declared and still has only 2 `serde::` paths in the crate — it is used
  by the `Deserialize` derives, so it is not unused in the strict sense, but it
  could be dropped if the derives were spelled through `serde_json`'s re-export.
  `docs/TESTING.md:249` no longer claims `tokio-test`.

- [x] **W6.28** README inaccuracy about the master secret.
  **Resolved.** The claim "fails closed on a missing master secret rather than
  generating a throwaway one" no longer appears in `README.md`; the behaviour
  (generate + persist) is documented correctly.

- [x] **W6.11 (RESOLVED)** Over-size frames now answer `message_too_large`.
  **Fixed.** Both the auth-loop and post-auth checks logged a `warn!` and closed
  without saying anything, and the documented `message_too_large` code appeared
  nowhere in the crate but a comment and a `PROTOCOL.md` reference — so "my
  messages vanish" and "the peer is gone" were indistinguishable.

  Every refusal site now sends `error_frame("message_too_large", …)` first, on
  the text *and* the binary path (they are one ceiling now), and stays open for
  `REFUSAL_LINGER` (500 ms) before closing. The linger is load-bearing rather
  than defensive: tungstenite refuses an over-ceiling frame from the header
  alone, so the payload is never read, and closing a socket with unread data
  sends RST — which destroys the answer on the wire before the peer can read it.
  That was observed as `10054` before the linger existed, and the negative test
  (answer without linger) fails on 5 of 5 runs.

- [x] **W6.10 (RESOLVED)** The relay now checks an inbound `protocol_version`.
  **Fixed.** It previously only *emitted* the constant in its own error frames;
  nothing read one off the wire, so a newer client was silently accepted.

  The check reads `protocol_version` off the raw `Value` before the typed parse
  (`connection.rs:867`) — necessary because neither `RelayAuth` nor `RelayRoute`
  declares the field, so serde would drop it. A declared value above the relay's
  own is answered `unsupported_protocol_version`; during auth that is followed
  by closing the connection and counting `auth_attempts_failure`, while after
  auth the frame alone is refused and the session continues, counted under
  `messages_dropped_unknown_type`. A missing field and a value at or below the
  relay's own are both accepted, per `PROTOCOL.md` §1.1.

#### OBSOLETE — the artefact is gone

These describe a standalone service. There is nothing to action; they are listed
only so nobody goes looking for a container.

- **W6.9** the documented `docker compose` quickstart, and its uid-1001 secret
  ownership problem.
- **W6.24** the `alpine:3.20` base image and floating tags.
- **W6.25** compose hardening (`read_only`, `cap_drop`, `pids_limit`).
- **W6.29** the `Dockerfile`, its `render.yaml` reference, the double
  `conduit-protocol` build and the duplicated healthcheck.

`Dockerfile`, `docker-compose.yml`, `.env.example` and
`scripts/build-relay.ps1` were all deleted. The concerns behind some of them
survive as library concerns and are restated above where they do: bounded memory
is W6.1, and bounded logging is part of W6.23.

---

## W7 — Security and hardening

- [ ] **W7.1 (HIGH)` Pairing rate limiting is per-connection only and cleared on
  disconnect (`security.rs:152, 213, 233-239`). Reconnecting resets it. The
  token is 6 alphanumeric characters (`commands/pairing.rs:168-173`) — 62⁶ ≈
  5.7 × 10¹⁰ — expiring in 60 s, and `pairing` is the only route to
  `automation/rule`, `screen_mirror` and `remote_input`. There is no
  IP-keyed or global limit anywhere.
  *Left to do:* add an IP-keyed bucket; lengthen the token to 8+ characters.

- [ ] **W7.2 (HIGH)` Certificate pinning is one-sided. **Partly resolved.** The
  desktop now has a pin configuration target: the `relay_cert_pin` setting,
  enforced by the relay at startup, and a mismatch refuses to start rather than
  warning. The "hashes the wrong thing" half was W1.4 and is fixed — the phone
  pins the SPKI, verified against a cross-language vector.

  Still open, and this is the part that matters:

  - **The mobile client re-pins on mismatch during pairing** instead of failing.
    Pairing is the trust bootstrap, so a peer that can answer the handshake
    replaces the pin. A pin that can be replaced by whoever asks is not a pin.
  - The desktop's relay client joins over **loopback plaintext**
    (`relay.rs::local_relay_url`) and so performs no TLS check at all. That is a
    defensible choice — the traffic never leaves the machine — but it means the
    `relay_cert_pin` setting protects the *served* certificate, not the
    connection the desktop itself makes. An operator standing behind a remote
    relay needs the client side too, and `connect_async` still takes no
    connector.
  - The mobile pin is a single global field shared between the LAN hub and the
    relay, so a pin captured from one is compared against the other.

- [ ] **W7.3 (RESOLVED — was unreachable)** The relay forwards a v2 payload that
  the desktop parses as a 28-byte LAN chunk frame. **Fixed, but it took three
  passes to become actually reachable**, and the audit entry recorded only the
  first:
  - The relay re-frames a binary delivery instead of stripping the v2 header,
    and the desktop unwraps the v2 frame before the LAN chunk parser sees it, so
    the two formats are no longer confused.
  - But the **16-byte target field is smaller than every device id the protocol
    produces** (all 36-char UUIDs), and both the relay's exact lookup and the
    desktop's string comparison refused every frame. The path was dead in both
    directions, and the existing tests missed it because they all used short ids
    like `device-1`. The field is now *defined* as the id's first 16 bytes in one
    place, the relay resolves a prefix and **fails closed on zero matches and on
    ambiguity** — a misdelivery would hand one device another's file.
  - And the phone never built a frame at all, because the target device id was
    never set. See [CROSS-PLATFORM-GAPS.md](CROSS-PLATFORM-GAPS.md) B-F3/G-R3.

  **The residual is W7.11**, not this entry: the AAD gap.

- [ ] **W7.4 (MEDIUM)` Unbounded audio queue fed by a remote peer
  (`audio.rs:38` `unbounded_channel`, pushed at `handlers/audio.rs:150` with
  only a per-message 50 MB cap). Currently moot because the consumer thread is
  dead (W3.7), but it must be bounded before that is fixed.

- [ ] **W7.5 (MEDIUM)` The relay has no proof of possession of a `device_id`
  (`main.rs:1427-1431`): any client holding the shared `RELAY_TOKEN` may
  authenticate *as* any device id and evict the incumbent from the routing
  table. Inherent to the shared-token design, but it compounds W6.4.
  > **⚠ AFFECTED BY THE RELAY REFACTOR.** The `main.rs:1427-1431` reference is
  > dead. Note what is *not* dead: the shared bearer token is still shared. Any
  > client that presents `relay_token` still authenticates as whatever
  > `device_id` it claims, and nothing stops it evicting the incumbent. Signing
  > is now per-device, so a frame cannot be *forged*, but the routing table can
  > still be *hijacked* by a token holder. Re-audit against `connection.rs`.

- [x] **W7.6 (RESOLVED)** The relay forwards no authenticated attribution.
  **Fixed by the same change as W1.15**, and the risk this item described is
  closed: the relay now emits `relay_delivery` carrying the sender it
  authenticated, the desktop unwraps it and re-enters as that device, and an
  unpaired sender is refused before any handler runs. A LAN peer cannot inject a
  relayed frame as a paired device, because the desktop resolves the claimed
  sender against its own registry and the `relay_server` id is never itself a
  trusted peer. Both facts are asserted by tests.

- [ ] **W7.11 (MEDIUM)` Encrypted binary chunks have no AAD on either side, so
  the 28-byte LAN chunk header is unauthenticated — an on-path peer can rewrite
  `index` or `id` and still pass Poly1305. The desktop defends structurally
  (`file_transfer.rs`) but the metadata itself is unsigned.

- [ ] **W7.7 (MEDIUM)` `verify_binary_tag` (`relay/src/main.rs:2064-2074`)
  hex-encodes the entire MAC input (up to 128 MiB of `String`) and MACs the hex
  string rather than the raw bytes; `.any()` short-circuits across the keyring,
  giving a timing oracle for which key matched.
  *Left to do:* MAC the raw bytes; use a constant-time fold.
  > **⚠ AFFECTED BY THE RELAY REFACTOR.** The function moved out of
  > `main.rs`. Note that the last clause no longer applies as described: the
  > relay no longer scans a keyring of candidate keys — it resolves the one key
  > for `from_device_id` through the `RouteKeys` trait, so there is no `.any()`
  > across keys to short-circuit. The "MAC the raw bytes" half may still stand.
  > Re-audit `route.rs`.

- [x] **W7.8 (RESOLVED)** `discovery.rs` byte-sliced a `String` that came from a
  file: `&device_id[..8.min(device_id.len())]`. A non-ASCII value panicked the
  discovery task. Replaced with `short_id()`, which cuts on a character boundary
  via `char_indices().nth(8)`.
  Two tests prove it, including one that panics with the old expression
  restored (`end byte index 8 … is inside 'é'`). Note the id comes from
  `device_id.txt` on disk, not a fresh UUID, so it is attacker-adjacent input in
  the sense that matters here: it is whatever an operator or a previous install
  left behind.

- [ ] **W7.9 (LOW)` A stale comment inverts the current design.
  `handlers/screen_mirror.rs:128-131` says loopback peers map to the
  `local_desktop` sentinel; `server/mod.rs:242-247` says the opposite and
  documents it as a fixed vulnerability. A future reader could reintroduce the
  hole.

- [ ] **W7.10 (LOW)` `handlers/screen_mirror.rs:137` hardcodes the
  `"local_desktop"` literal instead of using `LOCAL_DESKTOP_ID`
  (`handlers/mod.rs:29`).

---

## W8 — Test coverage

- [ ] **W8.1 (HIGH)` No Rust integration tests exist anywhere. There is no
  `tests/` directory in any of the three crates, so every Rust test is in-crate
  and none can exercise a crate the way a consumer would. The
  `conduit-protocol` → `conduit`/`relay` boundary is untested.

- [ ] **W8.2 (HIGH)` No cross-language conformance test. Nothing verifies that
  `types.rs`, the generated TypeScript and the generated Dart agree. The
  schema-sync tests link Rust to `schema.json` and check a few textual
  invariants in the Dart source; beyond that the guarantee is two regeneration
  steps and discipline. `docs/DEVELOPMENT.md:555` and `docs/TESTING.md:444` both
  point here.
  *Left to do:* assert that every variant in the Rust enums has a schema entry,
  and that every schema entry round-trips through both generated clients.

- [ ] **W8.3 (HIGH)` The Playwright suite cannot reach the backend. It runs in
  plain Chromium with a hand-written `__TAURI_INTERNALS__`, so it cannot reach
  any Tauri IPC, the WebSocket server (every test runs with
  `connected === false`, which is why every list is in its skeleton branch),
  `plugin-dialog`/`plugin-fs`/`clipboard-manager`, mDNS `listen` events, or the
  Rust settings gate. **Every finding in W4 is invisible to it by construction.**
  *Left to do:* add a Tauri-driver project, or a Rust-side integration test that
  boots `WsServer` and drives the real protocol.

- [ ] **W8.4 (HIGH)` No test exercises a real Tauri command's return value. The
  mock at `apps/desktop/__mocks__/tauri.ts` returns `undefined` for every
  command. 35 commands are registered (`main.rs:220-262`); none is covered
  end to end.

- [ ] **W8.5 (HIGH)` No error-frame handling is tested, and neither frontend
  implements it. The backend emits `not_authenticated`, `rate_limited`,
  `command_not_allowed`, `notification_app_not_allowed`,
  `clipboard_sync_disabled`, `file_sync_disabled`, `unsupported_message`,
  `unknown_action`, `invalid_message` and more; a rule rejected for
  `command_not_allowed` currently looks like nothing happened.
  *Left to do:* register a central `error` handler in both frontends, surfacing
  `code` + `message`.
  > **⚠ PARTLY STALE.** "Neither frontend implements it" was already false when
  > this entry was written: the mobile client has had a central `error` handler
  > since C-F9 (`da66802`), and W2.27 closed the one route it did not cover — an
  > `error` that arrived inside `relay_delivery`, which the socket-level listener
  > never sees. Both routes now have tests. The desktop frontend still has no
  > `error` handler at all, and the codes above are still silently dropped there,
  > so the item's substance stands on the desktop half alone.

- [ ] **W8.6 (MEDIUM)` No generated-file drift guard of its own. The `codegen`
  CI job does run `generate && git diff --exit-code` (`ci.yml:435-447`), but
  that proves the artifact is a *fresh render*, not that the render is
  semantically correct. The 55 message types are never checked for
  exhaustiveness against the Rust enums.

- [ ] **W8.7 (MEDIUM)` Untested relay surfaces: the WSS path end to end
  (`tls.rs:1086-1175` handshakes against a bare rustls server; nothing drives
  `handle_connection` over TLS); `load_tls_context` failure paths; expired
  certificates; `MAX_CONNECTIONS` enforcement; the 30 s nonce-flush and
  housekeeping tasks; cross-restart replay; binary replay after reconnect; the
  stale-disconnect bug; routing-table lock contention; the auth slow-loris.

- [ ] **W8.8 (MEDIUM)` Untested desktop surfaces: `useFiles` (341 lines, 6
  invokes, a 6-arm WS switch), `useSms`, `useCalls`, `useAppShell`,
  `useClipboardState`, `useMessageHandlers`, `useBubblePhysics`, `App.tsx`
  (490 lines), `ScreenMirror`, `RemoteInput`, `DeviceDetailPanel`,
  `PairingFlow`, and the whole `automation/` tree. The largest untested
  surfaces are exactly the ones with the W4 findings.

- [ ] **W8.9 (MEDIUM)` The audio loops are explicitly untested
  (`audio.rs:730-733`: "need a real audio device"), which is why W3.7 survived.
  At minimum, the thread-lifecycle logic should be testable without a device.

- [ ] **W8.10 (LOW)` No fuzz or property tests for the hand-rolled DER walk
  (`relay/src/tls.rs:271-298`), the hand-rolled base64 encoder
  (`tls.rs:884-897`), or the v2 binary-frame parser.

- [ ] **W8.11 (HIGH)` No single-process relay round trip against a real client.
  Every relay test builds its frames by hand. The relay's end-to-end lifecycle
  tests assert the wire format a real client depends on, and the desktop crate
  drives the real `"relay_server"` connection id through the real dispatcher, but
  no single test puts a real encoder on a real socket and runs
  phone → relay → desktop in one process. This is the gap a non-functional relay
  shipped through, and it is why
  `a_relayed_binary_frame_is_charged_to_the_transport_budget` is a guard rather
  than a regression test (W3.26): the accounting had to be extracted out of the
  relay read loop to be reachable at all, and a test on the extracted function
  cannot tell whether the loop calls it. `docs/TESTING.md` §7 item 5 tracks the
  same gap from the coverage side.
  *Left to do:* stand up an in-process relay listener, feed it the output of the
  mobile `relay_route.dart` encoder, and assert on what the desktop dispatcher
  receives.

---

## W9 — Documentation accuracy

- [ ] **W9.1 (HIGH)` The desktop Rust test count is wrong in five places: the
  actual figure is **699**, the docs say 696. `README.md:458`,
  `docs/DEVELOPMENT.md:241, 548`, `docs/TESTING.md:29, 36, 157, 160`,
  `CHANGELOG.md:125`, `.github/workflows/ci.yml:228`.
  Cause: the per-module table at `docs/TESTING.md:162-175` omits
  `src/audio.rs` (3 tests). Every other module count is correct.
  > **⚠ BOTH HALVES NOW STALE, AND THE CITATIONS HAVE MOVED.** The figure is
  > **817**, and it was already wrong in a second way: the per-module *totals* in
  > `docs/TESTING.md` were right but the split beneath them was not — the
  > handler tree has held **158** tests, not 145, and `server/mod.rs` held 69, not
  > 82, both measured on the unmodified tree. The `file:line` list above points
  > at lines that no longer carry those counts, and the `CHANGELOG.md` one names a
  > line in the `0.1.0` entry's relay bullet. `README.md` was worse: every figure
  > in its testing table was the 0.1.0 figure (187/275/724/222/87/362). All of
  > them are re-measured now; `CHANGELOG.md`'s `0.1.0` entry and
  > `.github/workflows/ci.yml` are not, so what remains here is the CI banner and
  > the historical release entry.

- [ ] **W9.2 (HIGH)` Three documents cite commit `eed58bd`, which **does not
  exist** in this repository. `docs/DEVELOPMENT.md:174`,
  `docs/decisions/0010-…md:64`, `docs/ARCHITECTURE.md:712`. The real history is
  `48724e0, 8bfc9ac, 0650695, 75b44d8, c48a832`.
  `docs/decisions/README.md:43` states "Every claim is checkable" — this
  violates the repo's own standard, and it is consistent with the CI banner
  admitting the workflow was written by reading the tree rather than running it.

- [ ] **W9.3 (HIGH)` `CONTRIBUTING.md:265-292` makes three "does not pass"
  claims that are now false: that `cargo test -p conduit` could not be run
  (it passes, 699/699, 0.88 s); that the Dart tree does not compile (43/43 tests
  pass, 0 analyzer errors — the `emit_sources.py` header fix was already
  applied in `0650695`/`8bfc9ac`); and that clippy is "clean for
  `conduit-protocol` and `relay`" (the desktop is clean too — the whole
  workspace exits 0 with zero warnings).

- [ ] **W9.4 (HIGH)` ADR-0001 asserts a CI check "does not exist yet"
  (`docs/decisions/0001-…md:51-54`); it does, at `ci.yml:670-676`, and
  ADR-0010 admits the contradiction at `:67-69` without fixing it. Per
  `docs/decisions/README.md:48`, this needs a superseding ADR rather than an
  edit.

- [ ] **W9.5 (HIGH)` ADR-0010 contradicts itself 30 lines apart. Decision §5
  (`:61-69`) says the codegen drift check "now exists"; Consequences
  (`:90-92`) say it "is specified but absent". `ci.yml:435-447` implements what
  §5 describes — the Consequences paragraph is the stale half.

- [ ] **W9.6 (MEDIUM)` Six dead document references, all now resolved by this
  file: `docs/DEVELOPMENT.md:191, 555`, `docs/TESTING.md:444`,
  `docs/decisions/README.md:55-56, 64`, `.gitignore:50`. `docs/HANDOFF.md` is
  separately cited at `docs/decisions/README.md:55` and is still missing, and
  `.dockerignore:20` references a `docs/archive/` that does not exist.

- [ ] **W9.7 (MEDIUM)` README claims about the relay are inaccurate: that
  "Health, metrics and certificate-pin HTTP endpoints" are all "behind bearer
  tokens" (`README.md:199` — `/healthz` and `/pin` are not, and `/metrics` is
  not by default); and that it "fails closed on a missing master secret rather
  than generating a throwaway one" (`README.md:402-403` — it generates one; see
  W6.28).

- [ ] **W9.8 (MEDIUM)` `docs/DEVELOPMENT.md:398` says "no `rustfmt.toml` in the
  tree". It exists, is 614 bytes, and sets `edition = "2021"`. The reasoning
  behind that setting is good and should be documented instead.

- [ ] **W9.9 (MEDIUM)` `docs/DEVELOPMENT.md:303-315` lists 16 lint steps and
  omits the icon-drift step entirely, misnumbering everything after it.
  `docs/DEVELOPMENT.md:321-323` claims `lint-all.ps1` runs plain
  `flutter analyze` for Dart; it runs `--fatal-infos`, which means
  `lint-all.ps1` **cannot pass today**.

- [ ] **W9.10 (MEDIUM)` `docs/TESTING.md:249` states the relay's tests use
  `tokio-test`; the dependency is declared but never referenced (W6.26). The
  relay test total is also stale (docs say 189, tree has 190).

- [ ] **W9.11 (MEDIUM)` The Dart SDK floor is stated as 3.13 in three documents
  while `pubspec.yaml:10` declares `>=3.12.0`. `README.md:213` states both in a
  single table cell. CI resolves a floating `flutter-version: '3.47'`
  (`ci.yml:554`), so the declared floor is never exercised.
  *Left to do:* pick a floor, make the documents agree, add an
  `environment: flutter:` constraint so it is machine-checkable, and pin the
  Flutter patch version the way `rust-toolchain.toml` pins Rust.

- [ ] **W9.12 (MEDIUM)` `rust-toolchain.toml:3-5` gives a rationale that is not
  the real one: there is no musl toolchain in this repo, and the relay — the
  only Alpine build — does not depend on SQLCipher. The pin is still justified;
  the stated reason will mislead the next reader.

- [ ] **W9.13 (MEDIUM)` The CI banner's own "Verified" claims (`ci.yml:33-37`)
  are local Windows measurements extrapolated to a Linux runner, in a workflow
  that has never executed (W5.4). They should be labelled as such until the
  first real run.

- [ ] **W9.14 (LOW)` `README.md:522` notes that only `conduit-protocol` declares
  `license = "MIT"`; `conduit` and `relay` do not (W6.26).

- [ ] **W9.15 (LOW)` `apps/mobile/pubspec.yaml:2-5` describes the app as
  relaying "SMS, notifications and calls" — all three are broken or partial
  (W2.3, W2.14, W2.6). `version: 0.1.0+1` is duplicated as a hardcoded string
  at `apps/mobile/lib/screens/settings_screen.dart:346`.

---

## Verification baseline

Measured on the current tree, on Windows, after the relay was refactored from a
standalone service into a library hosted by the desktop app.

| Suite | Working directory | Command | Measured |
|---|---|---|---|
| Relay | repo root | `cargo test -p conduit-relay` | **247** passed, 1 ignored + 2 doctests |
| Protocol | repo root | `cargo test -p conduit-protocol` | **281** passed + 1 doctest |
| Desktop Rust | repo root | `cargo test -p conduit` | **817** passed |
| Clippy | repo root | `cargo clippy --workspace --all-targets` | exit 0, zero warnings |
| Rust format | repo root | `cargo fmt --all -- --check` | exit 0 |
| Desktop typecheck | `apps/desktop` | `npx tsc --noEmit` | exit 0 |
| Desktop unit | `apps/desktop` | `npm test` | **249** passed, 22 files |
| Desktop e2e | `apps/desktop` | `npm run test:e2e` | **31/31 fail** — no `playwright install` bootstrap (W5.13) |
| Desktop codegen | `apps/desktop` | `npm run generate` | exit 0, clean tree |
| npm audit | `apps/desktop` | `npm audit` | 0 vulnerabilities |
| Mobile unit | `apps/mobile` | `flutter test` | **194** passed |
| Mobile analyze | `apps/mobile` | `flutter analyze` | **370** infos, 0 errors, 0 warnings — 365 of them in the *generated* `lib/models/protocol.dart` |
| Lockfile | repo root | — | see W5.6 |

Three of those rows moved, and one of them was **already wrong before** this
change rather than moved by it. Desktop Rust 799 → 817 is this change (17 new
relay tests, all in `server/mod.rs`, three of them from the fix round rather than
from the fan-out). Mobile unit 190 → 194 is this change (4 new relayed-frame
tests). Mobile analyze was recorded as 366 and measured 370 on the **unmodified**
tree, so that one was four behind and this change added no new infos; `dart
analyze` reports the same 370 as `flutter analyze`.

## Notes

- Severity labels here are the auditor's judgement, not a project policy. If the
  project wants a different bar, change the labels — the references do not
  depend on them.
- `docs/decisions/README.md:55-56` says `HANDOFF.md` and this file are "owned
  elsewhere". This file is now the single tracked backlog; anything that was
  meant to live in `HANDOFF.md` belongs in a workstream above.
- When an item is closed, tick the box **and** update the reference if the line
  moved. A stale `file:line` is worse than no reference, because it sends the
  next reader to the wrong place.
