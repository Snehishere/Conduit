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

- [ ] **W1.2 (CRITICAL)** Both relay clients sign `relay_route` in a way the
  relay rejects on three axes.
  > **⚠ AFFECTED BY THE RELAY REFACTOR.** The three axes below were derived
  > against the deleted `services/relay/src/main.rs` and a shared
  > `SigningKeyring` keyed from `HMAC_SECRET`. There is no shared signing key
  > and no `SigningKeyring` now: verification is per-device, keyed by the
  > pairing secret, with `key_id` required to equal `from_device_id`
  > ([ADR-0011](decisions/0011-per-device-relay-route-keys.md)). The *caller* of
  > this item — both clients signing with the relay token instead of a derived
  > per-device route key — is the substance, and that substance has not been
  > re-verified against the new relay. Re-audit before acting.
  Desktop `src-tauri/src/server/mod.rs:991-1012`, mobile
  `websocket_service.dart:600-612, 672-684`.
  1. Key: both HMAC with `RELAY_TOKEN`; the relay verifies against
     `SigningKeyring`, derived from `HMAC_SECRET`
     (`services/relay/src/main.rs:1532-1535`).
  2. Field set: both send 5 fields; `SIGNED_FIELDS`
     (`packages/protocol/src/lib.rs:177-185`) requires 7, including
     `from_device_id` and a mandatory `key_id`.
  3. Byte order: both serialize in insertion order. The verifier
     (`lib.rs:193-201`) rebuilds through a `BTreeMap`, so it is alphabetical.
     Even with the right key the digests differ.
  The correct signer already exists at `packages/protocol/src/types.rs:1475`
  (`RelayRoute::signed_with`) and the relay's own tests use it
  (`services/relay/src/main.rs:3457, 4266, 5416`). Neither client calls it.
  `PROTOCOL.md:1670-1676` names this fix.
  *Left to do:* call `RelayRoute::signed_with` from both clients; delete the
  hand-rolled map construction in each.

- [ ] **W1.3 (CRITICAL)** The phone emits the superseded binary frame version.
  `websocket_service.dart:592` writes `0x01`;
  `packages/protocol/src/types.rs:1280` requires `0x02`. The relay rejects on
  the literal at `services/relay/src/main.rs:1968`.
  The Dart frame is also 17 bytes (`version || target_id || payload`) where the
  relay requires at least 53 — `BINARY_HEADER_LEN` at `types.rs:1291` adds a
  4-byte sequence number and a 32-byte HMAC tag, both mandatory.
  *Note:* the LAN-direct chunk frame in `apps/mobile/lib/services/file_service.dart:288-298`
  **is** correct and matches `types.rs:1305-1307`. Only the relay-routed binary
  path is broken.

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

---

## W2 — Mobile client

- [ ] **W2.1 (CRITICAL)** Screen mirroring never delivers a frame.
  `apps/mobile/lib/main.dart:308-323` registers branches for `start`, `stop`,
  `touch`, `key`, `scroll` — **there is no `frame` branch**. Native frames
  arrive on EventChannel arg `'screen_mirror'`
  (`android/.../MainActivity.kt:111`, `ScreenMirrorService.kt:152-165`) and
  nothing subscribes with that argument; the only two subscribers are
  `'notifications'` and `'calls'`. `lib/services/screen_mirror_service.dart`,
  which *does* read frames, is never instantiated in `main.dart`.
  The screen is additionally gated on `hasDesktop`, which is always false —
  see W2.2.
  *Left to do:* subscribe to the `screen_mirror` EventChannel; add the `frame`
  branch; instantiate the service; move capture into
  `ConduitForegroundService` so an Android rotation does not end mirroring
  (`MainActivity.kt:634, 643-649`).

- [ ] **W2.2 (CRITICAL)** Discovery results are discarded. The desktop announces
  `device_id`, `device_name`, `device_type`, `battery`, `ws_port`, `wss_port`
  (`src-tauri/src/server/mod.rs:1554-1566`); the home-screen handler reads
  `map['id']`, `map['name']`, `map['type']` (`apps/mobile/lib/screens/home_screen.dart:54-79`).
  `_deviceMap` can never populate, so `hasDesktop` is always false and several
  screens are unreachable. `StatusUpdate` has no `device_id` either
  (`types.rs:921-938`).
  Separately, `setAdvertisedWssPort` (`websocket_service.dart:171`) is declared
  and never called, so the mDNS-derived TLS port is dropped and every LAN dial
  falls back to the literal 9531.

- [ ] **W2.3 (CRITICAL)** SMS on Android is real but its data is mangled.
  `MainActivity.kt:371-375` returns `address, body, timestamp, is_outgoing, read`;
  `apps/mobile/lib/services/sms_service.dart:315-320` reads `_id`, `date`, `type`.
  Every message therefore gets a synthetic id, a "now" timestamp, and
  `isOutgoing = (null ?? 1) == 2 = false`. **All messages render as incoming.**
  On iOS `getSmsThreads` returns `result([])` (`ios/Runner/AppDelegate.swift:139`)
  where Dart does `invokeMethod<String>` + `jsonDecode` (`sms_service.dart:300, 307`),
  raising a `TypeError` caught at `sms_service.dart:342`.

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

- [ ] **W2.9 (HIGH)** iOS frame payloads do not match the Dart reader.
  `AppDelegate.swift:305-311` emits `{"type":"screen_mirror_frame","frame":…}`
  with no `action` field; `screen_mirror_service.dart:23-29` requires
  `action == 'frame'` and reads `msg['data']`. `AppDelegate.swift:294` also
  hardcodes `maxWidth = 640` and ignores the `quality`/`fps` arguments.

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

- [ ] **W2.14 (MEDIUM)** Notification read state is not modelled.
  `notification_service.dart:175` — `void markReadNotification(String id) {}`,
  an empty body, while the UI calls the wire-level
  `sendNotificationMarkRead` (`screens/notifications_screen.dart:63`). The
  desktop is told "mark read"; the phone stores nothing.

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

---

## W3 — Desktop backend

- [ ] **W3.1 (CRITICAL)** Five of seven automation actions report success while
  doing nothing. `src-tauri/src/automation.rs:742-804`:
  `SendNotification` (`:742`), `RouteAudio` (`:765`), `ToggleWiFi` (`:775`),
  `ToggleBluetooth` (`:785`) and `OpenUrl` (`:795`) each log and return
  `success: true`. No notification is emitted, `AudioStream` is never touched,
  no WiFi or Bluetooth code exists in the crate, and `tauri-plugin-opener` is
  not a dependency. A rule fires, writes `success=1` to `automation_logs`, and
  the UI shows green.
  *Left to do:* implement each action, or narrow
  `is_desktop_executable` (`automation.rs:493-504`) and make the unimplemented
  arms return `success: false` so the log stops lying.
  The test at `main.rs:603-639` asserts a message exists — which every stub
  satisfies — and never asserts `success`. Add that assertion.

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

- [ ] **W3.3 (HIGH)** `handlers/files.rs:66-75` has the same bypass plus a
  device-id/connection-id confusion. The `to.is_empty()` branch is unfiltered,
  so unpaired sockets receive `file/request` (filenames, sizes, MIME types);
  and `clients_lock.get(to)` treats `to` — a stable *device* id
  (`types.rs:289-291`) — as a per-connection UUID (`server/mod.rs:241`), so a
  targeted file request delivers nothing. The same class of bug is documented as
  fixed in `WsServer::send_to` (`server/mod.rs:948-977`).
  *Left to do:* use `broadcast_to_others` for the empty case; resolve `to`
  through `ws_to_device_id` as `send_to` already does.

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

- [ ] **W4.3 (HIGH)** The Files surface shows a stale clipboard list.
  Live `clipboard/sync` frames land in `useClipboard.history`
  (`hooks/useClipboard.ts:82`), which `App.tsx:154-158` never destructures;
  `refreshClipboard` (`hooks/useClipboardState.ts:59`) is never called by
  `App.tsx:152` and is not routed from the message handler. Neither the new item
  nor the peer device name ever appears.
  *Left to do:* on `clipboard/sync`, call `refreshClipboard()` and resolve
  `source_device` to a device name.

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

- [ ] **W4.16 (MEDIUM)** Discovery is write-only. `hooks/useDiscovery.ts:283-290`
  returns `startDiscovery`, `stopDiscovery`, `clearDiscovered`,
  `discoveredDevices` and `isDiscovering`; `App.tsx:133` destructures only
  `handleDiscoveryMessage`. The `start_discovery`/`stop_discovery` invokes are
  unreachable, and `components/network/DeviceHub.tsx:28, 43` declares
  `onShiftClickDevice` and `onNavigate` that `App.tsx:283-289` never passes.

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

- [ ] **W4.22 (MEDIUM)** `hooks/useSms.ts` exposes `markRead` (`:127-133, 147`)
  and `hooks/useCalls.ts` exposes `dismissCall` (`:168-170, 231`); neither is
  consumed by `App.tsx:141-144`, so opening a thread never clears its unread
  count. `components/messages/MessageInput.tsx:7` declares a `disabled` prop that
  `MessageThread.tsx:60` never passes, so the composer is never disabled while
  offline.

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

- [ ] **W4.26 (MEDIUM)** `components/automation/automationMeta.ts:25-44`
  hand-maintains 7 trigger and 8 action labels over generated unions that
  declare 8 and 9. `audio_device_disconnect` and `set_window_state` have no UI
  entry, and the file's own comment claims omissions are a compile error — they
  are not, because the arrays are explicitly typed as a subset.

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

- [ ] **W5.1 (CRITICAL)** A fresh clone cannot build the desktop app.
  `apps/desktop/src-tauri` depends on `rusqlite` with `bundled-sqlcipher`,
  which links against OpenSSL. `.cargo/config.toml:8-10` supplies it
  unconditionally:
  ```toml
  [env]
  OPENSSL_LIB_DIR     = { value = ".tools/openssl-win64/lib/static", relative = true }
  OPENSSL_INCLUDE_DIR = { value = ".tools/openssl-win64/include",     relative = true }
  ```
  Four problems: (1) **no fetch script** — `scripts/` contains no
  `fetch-openssl.ps1`; (2) **no version pin** in any tracked file (only the
  untracked `.tools/openssl-win64/version.txt`); (3) **no checksum**; (4)
  `.tools/` is gitignored (`.gitignore:51`), so a clone never receives it. The
  failure surfaces as a link error from `libsqlite3-sys`, not as a message
  naming the missing directory. There is **no `[target.'cfg(...)']` guard**, so
  Linux and macOS get the same Windows `.lib` path and there is no working build
  path at all.
  *Left to do — three options, in order:*
  - **B (immediate):** move the `[env]` table to
    `[target.x86_64-pc-windows-msvc.env]`. One-line change; unblocks three CI
    steps and makes the desktop clippy/test/build steps eligible to become
    blocking.
  - **A:** delete `.cargo/config.toml` and have a fetch script export the
    variables into its own process before exec'ing cargo.
  - **C (the real fix):** replace the vendored copy with `vcpkg install
    openssl:x64-windows-static` on Windows and `libssl-dev` / `brew install
    openssl@3` elsewhere.

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

- [ ] **W5.3 (CRITICAL)** The desktop crate's 699 tests never run in CI.
  `.github/workflows/ci.yml:230` `cargo test -p conduit --locked` is
  `continue-on-error: true`, as are the clippy (`:224`) and release build
  (`:236`) steps. **699 of 1154 Rust tests — 61% — execute only on a
  maintainer's Windows machine.** Direct consequence of W5.1.

- [ ] **W5.4 (CRITICAL)** The CI workflow has never been executed.
  `ci.yml:11` says so in its own banner: *"THIS WORKFLOW HAS NEVER BEEN
  EXECUTED."* `git log` shows 5 commits and `git remote -v` is empty. The
  `hygiene` job's entitlements step is documented as currently red (`:706-707`).
  Every "Verified" claim in the file (`:33-37`) is a local Windows measurement
  extrapolated to a Linux runner.

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

- [ ] **W5.14 (MEDIUM)** CI has no `timeout-minutes` and no `permissions:` block
  anywhere. The `rust` job runs two full source builds of Rust tools
  (`cargo install cargo-audit`, `cargo install cargo-deny`) plus three release
  builds. Every job's `GITHUB_TOKEN` gets repository defaults.

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
  the latter is wrong: 359 of the 362 infos are in the generated
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

- [ ] **W6.1 (CRITICAL)** No size cap on binary frames.
  `services/relay/src/connection.rs:465` handles `Message::Binary` with no
  `MAX_TEXT_SIZE` check, unlike the two Text arms at `:109` and `:324`.
  tungstenite 0.24's default `max_message_size` is 64 MiB, and the payload is
  `to_vec()`'d into a 1024-deep queue with no byte budget — roughly **64 GiB per
  target connection** from one authenticated client. `PROTOCOL.md:184` claims
  the 1 MiB check bounds memory; for binary frames it does not exist.
  *Left to do:* enforce a cap before `handle_binary_frame`; pass an explicit
  `WebSocketConfig`; add a per-queue byte budget.

- [ ] **W6.6 (HIGH)** A read lock is held across `await`.
  `services/relay/src/route.rs:331` and `:387` take `state.clients.read().await`
  and then hold the guard for the full `FORWARD_TIMEOUT_SECS` while
  `target_tx` is borrowed from it. Every writer blocks meanwhile — registration,
  deregistration (`connection.rs:560`) and the 30 s sweep
  (`service.rs:spawn_housekeeping`). tokio's `RwLock` is write-preferring, so one
  slow target stalls the whole routing table.
  *Left to do:* clone the `Sender` out and drop the guard before awaiting.

- [ ] **W6.4 (HIGH)** A stale disconnect deregisters the live device.
  `connection.rs:560` removes by key with no check that the stored sender is
  still *this* connection's. A device that reconnected keeps its connection but
  is permanently unroutable, and `reconcile_clients` cannot repair it.
  *Left to do:* store `(tx, connection_id)` and remove only on a match.

- [ ] **W6.5 (HIGH)** Binary frames have no cross-connection replay protection.
  `connection.rs:264` holds `last_binary_seq` as a per-connection in-memory
  `Option<u32>`, reset on every reconnect. A reconnect or a restart resets it,
  after which any captured frame replays forever. The `relay_route` path has the
  persisted nonce cache; this path has nothing. The delivery re-framing added for
  `relay_delivery` does **not** change this: it re-stamps the sequence per
  connection, which is correct for the receiver's own guard and is still not a
  durable replay bound.
  *Left to do:* add a timestamp and nonce (a frame-version bump), or persist a
  per-device high-water sequence number alongside the nonce cache.

- [ ] **W6.2 (HIGH)** The `/health` bearer token still defaults to the master
  HMAC secret (`config.rs:54`, `with_health_token_fallback`). The master secret
  is no longer a signing-key input — W6.2's original consequence was **removed**
  by per-device keys — but it remains a credential shared by two roles, and
  anyone holding the health token reads the secret the nonce cache is keyed from.
  *Left to do:* require `RELAY_HEALTH_TOKEN` explicitly, or derive a separate
  purpose-bound token.

- [ ] **W6.7 (HIGH)** Handshakes are untimed and uncounted.
  `service.rs::admit` runs **after** `acceptor.accept(stream).await`, which has
  no timeout, and the relay's own source says so: a client that opens TCP and
  sends no ClientHello holds a task and an fd forever and is never counted. The
  WebSocket upgrade is likewise untimed and holds an `active_connections` slot,
  and the per-IP rate limit is applied *after* the upgrade, so it protects
  neither path.

- [ ] **W6.8 (HIGH)** The auth deadline is per-message, not total.
  `connection.rs:97` re-arms `auth_timeout` on every frame. A client sending one
  junk frame every 9 s holds an unauthenticated connection and a slot
  indefinitely.

- [ ] **W6.15 (MEDIUM)** Self-signed certificates are valid for ~2000 years.
  `tls.rs` never sets a validity window, so rcgen applies its defaults. No
  `key_usages` or `extended_key_usages` are asserted, so stricter clients may
  reject the server certificate, and `load_tls_context` never validates a
  supplied certificate's validity window at all.

- [ ] **W6.12 (MEDIUM)** Cross-restart replay protection depends on a 30 s flush
  against a 35 s freshness window, so a crash loses up to 30 s of accepted
  nonces. `packages/protocol/src/lib.rs` also returns an empty nonce map on
  **any** read error, including `PermissionDenied`, with no log — a relay whose
  nonce file becomes unreadable silently loses replay protection.

- [ ] **W6.13 (MEDIUM)** The global `MAX_NONCES` ceiling can evict another
  device's in-window nonces. Per-device quota is 4096 and the process-wide cap is
  10 000 (`lib.rs:55`, `:62`); three devices at quota exceeds the global cap, at
  which point the global loop evicts the globally oldest entry, which may belong
  to a different device.
  *Left to do:* raise the ceiling, or make the global loop skip devices still
  under quota; correct the docs.

- [ ] **W6.19 (MEDIUM)** `bearer_token_authorized` (`config.rs:451`) still has no
  empty-expected-token guard: `bearer_token_authorized("Bearer ", "")` returns
  `true`, because both sides are zero-length and the length check passes. Not
  reachable today only because `effective_health_token()` falls back to a
  never-empty secret.

- [ ] **W6.21 (MEDIUM)** No minimum entropy on secrets. `RELAY_TOKEN="   "` and a
  one-character token are both accepted, and a 1-byte `HMAC_SECRET` yields a
  1-byte-strength secret. Port parse failures silently fall back to the default
  (`config::env_port`), so `RELAY_WSS_PORT=95x9` silently binds 9529.

- [ ] **W6.22 (MEDIUM)** `Config` still derives `Debug` while holding
  `hmac_secret` and `relay_token` (`config.rs:26`). Any `{:?}` — including
  inside a `panic!`/`expect` — prints both.
  *Left to do:* implement `Debug` manually with redaction.

- [ ] **W6.23 (MEDIUM)** Operational constants are hardcoded in `limits.rs` and
  are not configurable: `MAX_TEXT_SIZE`, `MAX_CONNECTIONS`, message rate and
  burst, forward timeout, queue depth, ping and idle-read intervals. Only
  `RELAY_AUTH_TIMEOUT_SECS` is tunable. The 10/60 s connect rate breaks any
  NAT'd deployment with more than 10 devices.

- [ ] **W6.27 (LOW)** `packages/protocol/src/lib.rs:346` — the unscoped free
  function `check_replay` is superseded by `NonceCache::check_replay` and used
  only in its own tests. It is a footgun: unscoped, a single cap, no per-device
  isolation.

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

- [ ] **W6.11 (MEDIUM)** Over-size text frames close the connection with only a
  `warn!` (`connection.rs:108-112` during auth, `:324` afterwards), and the
  documented `message_too_large` code is **never emitted anywhere in the crate** —
  the only occurrence of the string is a comment in `limits.rs:15` and a
  `PROTOCOL.md` reference. The sender learns nothing, so "my messages vanish" and
  "the peer is gone" are indistinguishable.
  *Left to do:* send `error_frame("message_too_large", …)` before closing, on both
  the auth and post-auth paths.

- [ ] **W6.10 (MEDIUM)** The relay never checks an inbound `protocol_version`, so
  a newer client is silently accepted. `connection.rs:582` only *emits*
  `PROTOCOL_VERSION` in its error frames; nothing reads one off the wire.
  `PROTOCOL.md:50-51, 1671` documents the gap and there is still no code for it.
  *Left to do:* reject a `protocol_version` above what the relay speaks, with
  `unsupported_protocol_version`.

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

- [ ] **W7.3 (MEDIUM)` The relay forwards a v2 payload that the desktop parses as
  a 28-byte LAN chunk frame. **The first half is fixed** — the relay now re-frames
  a binary delivery instead of stripping the v2 header, and the desktop unwraps a
  v2 frame at `handle_binary_message` before the LAN chunk parser sees it, so the
  two formats are no longer confused.
  **The rest stands:** encrypted binary chunks have no AAD on either side, so
  the 28-byte header is unauthenticated — an on-path peer can rewrite `index` or
  `id` and still pass Poly1305. The desktop defends structurally
  (`file_transfer.rs`) but the metadata itself is unsigned.
  *Left to do:* set AAD to the metadata block on both sides.

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

- [ ] **W7.3 (MEDIUM)` Encrypted binary chunks have no AAD on either side, so
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

- [ ] **W7.8 (LOW)` `discovery.rs:101` byte-slices a `String` that came from a
  file: `&device_id[..8.min(device_id.len())]`. A non-ASCII value panics the
  discovery task. Use `.chars().take(8)`.

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

---

## W9 — Documentation accuracy

- [ ] **W9.1 (HIGH)` The desktop Rust test count is wrong in five places: the
  actual figure is **699**, the docs say 696. `README.md:458`,
  `docs/DEVELOPMENT.md:241, 548`, `docs/TESTING.md:29, 36, 157, 160`,
  `CHANGELOG.md:125`, `.github/workflows/ci.yml:228`.
  Cause: the per-module table at `docs/TESTING.md:162-175` omits
  `src/audio.rs` (3 tests). Every other module count is correct.

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
| Relay | repo root | `cargo test -p conduit-relay` | **187** passed + 2 doctests |
| Protocol | repo root | `cargo test -p conduit-protocol` | **275** passed |
| Desktop Rust | repo root | `cargo test -p conduit` | **724** passed |
| Clippy | repo root | `cargo clippy --workspace --all-targets` | exit 0, zero warnings |
| Rust format | repo root | `cargo fmt --all -- --check` | exit 0 |
| Desktop typecheck | `apps/desktop` | `npx tsc --noEmit` | exit 0 |
| Desktop unit | `apps/desktop` | `npm test` | **222** passed, 20 files |
| Desktop e2e | `apps/desktop` | `npm run test:e2e` | **31/31 fail** — no `playwright install` bootstrap (W5.13) |
| Desktop codegen | `apps/desktop` | `npm run generate` | exit 0, clean tree |
| npm audit | `apps/desktop` | `npm audit` | 0 vulnerabilities |
| Mobile unit | `apps/mobile` | `flutter test` | **87** passed |
| Mobile analyze | `apps/mobile` | `flutter analyze lib test` | exit 0, no errors or warnings |
| Lockfile | repo root | — | see W5.6 |

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
