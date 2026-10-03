# Cross-platform gaps

An audit of the seams between Conduit's four code surfaces and the four places
it declares what it may do.

**Commit audited:** `3051caf`. **Method:** ten read-only agents, one per
boundary, followed by two adversarial agents whose only job was to *refute* the
headline findings. Every agent worked from the tree, not from
[`REMAINING_WORK.md`](REMAINING_WORK.md), and every finding below was checked
against code on both sides.

> **This document supersedes nothing.** `REMAINING_WORK.md` remains the tracker
> of *known* work; this is the record of what a fresh read of the tree found,
> including entries in that document that are wrong. Where the two disagree, the
> evidence is here.

## Status

Findings are marked as they are closed. **Every fix below was verified by an
adversarial pass first** — a separate agent whose only job was to try to refute
the finding — and **two headline claims were refuted**, which is why the
severity ordering below is not the same as the order the findings were first
reported in.

### Fixed

| Finding | Closed in |
|---|---|
| **G-R2** — the phone's entire relay egress was dead (`_targetDeviceId` always null) | `da66802` + the follow-up commit |
| **G-R4** — the phone handled none of the relay's answers (19 error codes, both auth results) | `da66802` |
| **G-R7** — inbound relayed frames all rejected as replays after any reconnect | `da66802` |
| **G-R16** — no way to point a phone at a relay | `da66802` (mobile) + follow-up (desktop delivers it at pairing) |
| **G-R17** — the phone could not produce a real `ping`/`pong` on a relay socket | `da66802` |
| **B-F3 / G-R3** — the relay could not route a binary frame to anybody | `da66802` (relay + desktop); phone receiver in the follow-up |
| **B-F4** — `handle_lan_chunk` looked a device id up in a client-id-keyed map | `da66802` — **that closure claim was wrong**; see the second B-F4 row below |
| **C-F9** — no `error` handler on the phone | `da66802` |
| **D-F1** — `relay_enabled` defaulted to off; a fresh install never started the relay | `da66802` |
| **D-F4** — the clipboard history table was never written | `da66802` |
| **B-F2** — the per-type rate limit was 60 per *60 seconds*, and `stop` shared the frame budget | `da66802` |
| **G-R8** — directed frames were not sealed on the relay egress | `da66802` |
| **E-F1** — three EventChannel subscribers shared one channel name, so two were dead | `da66802` |
| **Global limiter double-charge** — ~5 msg/s ceiling on all types | follow-up commit |
| **G-R9** — no fan-out handler reached a relay-only peer; the relay socket is an `mpsc::Sender`, not a `ctx.clients` entry | uncommitted |
| **G-R9's egress half** — the relay egress read the recipient's secret from a *liveness* map, so every frame routed to a socket-less device went out in the clear | uncommitted |
| **G-R11** — corrected: relayed frames were charged **no count and no bytes** by anyone, not a shared `"relay_server"` bucket | uncommitted |
| **B-F1, second half** — `send_error` wrote nothing for a peer with no socket, so every refusal aimed at a relay-only device was looked up, missed and dropped | uncommitted |
| **B-F4** — `handle_lan_chunk` looked a device id up in a connection-keyed map and dropped every relayed binary chunk, *after* the relay had verified its tag | uncommitted |
| **The phone discarded a routed `error`** — `_handleMessage` had no `error` arm on the `relay_delivery` path, so a refusal sent through the relay was received and silently dropped | uncommitted |
| **A revoked device stayed routable *and* trusted for up to 5 s** — the route-key registry refreshes on a timer, and the `peer_secret` fallback had just made it an authority for the auth gate | uncommitted |
| **The registry could publish a device as routable before its secret** — `keys` was written before `secrets`, so a reader in between took `seal_for_device`'s plaintext path | uncommitted |
| **One padded broadcast could close the relay leg** — the fan-out had no size ceiling, sealing doubles the size, and crossing the relay's 1 MiB read ceiling closes the connection rather than dropping a frame | uncommitted |
| **A declined file request told a relay-only sender nothing** — `handle_file_request` resolved the `file_accept_disabled` refusal with a bare `clients.get`, which never matches a device id | uncommitted |
| **A relayed binary chunk was refused one layer above the chunk path** — `unwrap_relay_binary_frame` enumerated candidate senders from `SyncEngine`, a liveness map, so a relay-only sender's frame never reached the id resolution | uncommitted |
| **`build_sealed_route` substituted `Value::Null`** on an unparseable payload; the relay would have forwarded it and counted it as `messages_routed` | uncommitted |

### Refuted or corrected before any code was written

- **G-R1 (port 9531 collision) — REFUTED on Windows.** Measured, not inferred:
  `0.0.0.0:P` and `127.0.0.1:P` bind successfully together, and each listener
  receives its own traffic. It survives as a **portability risk on Linux/macOS**
  (the POSIX rule differs and `SO_REUSEADDR` is not set), not a live bug. The
  "both outcomes are fatal" framing was also wrong: the hub's arm is
  warn-and-continue.
- **G-R8's framing was wrong**, though the mechanism held. "Every desktop→phone
  relayed frame is plaintext" was true but nearly vacuous, because `send_to` is
  the only egress and most types are plaintext *by design on every transport*.
  The real defect was narrower: directed frames were unsealed on the relay path
  only. The cited `ARCHITECTURE.md:60` does not contain the claim at all.
- **"No inbound frame is deserialised into a protocol struct" — REFUTED.** There
  are 11 such parses; it is a deliberate two-tier split. See the structural
  finding above for the corrected version.
- **iOS: `NEHotspotNetwork` is not an independent blocker.** It is downstream of
  the missing Xcode target and dissolves itself if the project is restored from
  the current Flutter template (deployment target 15.0). So §F's "three
  independent reasons" is the accurate count, not four.

### Corrected during the fix round — the audit's mechanism was wrong

One finding in this document was **half wrong in a way that made it look less bad,
not more**: it named a mechanism the code never had, and the real defect was the
opposite shape. The finding row in §G is left as the historical record.

- **G-R11 — the shared `"relay_server"` bucket never existed.** The row says
  *"All phones behind the relay share **one** rate-limit bucket keyed on the
  literal `"relay_server"`"*, and infers a cross-phone throttle from it.
  `admit_frame` has exactly one caller — `handle_client`, the LAN reader
  (`server/mod.rs:527`) — and the relay read loop calls `handle_message`
  directly (`server/mod.rs:1760` → `:790`). So no `"relay_server"` bucket was
  ever created: **relayed frames were charged no count and no bytes** by the
  transport limiter, by any phone, at all. The per-type limiter was already
  per-device, because the same re-entry dispatches under the sender
  (`server/mod.rs:878` → `:1069`), so one phone could not throttle another —
  and neither could be throttled. `max_bytes`, the field
  `security::RateLimitConfig` documents as "the budget that bounds CPU", did not
  exist on this path. The consequence for the fix is recorded on the code:
  charging per sender is **strictly more** accounting than before, not a
  repartition of an existing charge (`server/mod.rs:838-864`).

### Still open

Everything else in this document, plus the items listed at the end. The largest
remaining are the ones no single boundary could see: **iOS cannot build or
launch** (three independent blockers), **`remote_input` into the phone is
unreachable on both platforms**, and **screen capture and remote input into the
desktop are broken on Linux**.

The relay is in better shape than this section used to say, and the residual is
narrower than "phones behind it are unaccounted for". What is left there:

- **A _directed_ request to a relay-only device still needs a socket to resolve
  against.** `fan_out_to_relay` covers frames the desktop originates; a
  `screen_mirror/start` addressed to an off-LAN phone is still refused with
  `capture_stopped`, because `resolve_target_client` reads `ws_to_device_id`
  (`handlers/screen_mirror.rs:106`, pinned by
  `known_gap_a_relay_only_target_cannot_be_asked_to_start`).
- **Two listeners in one process still want TCP 9531** (G-R1) — a portability
  risk on Linux/macOS, not a live bug on Windows.
- **Four unfiltered fan-out loops on the LAN path are untouched** (B-F5):
  `handlers/audio.rs:45-50`, `:69-74` and `:153-160` push live system-audio PCM,
  and `handlers/files.rs:197-203` broadcasts a `file/request`, to every entry in
  `ctx.clients` without the pairing filter. The relay arm does not touch these,
  so the plaintext-leak class they represent is untouched on both transports.
- **`validate_message` still closes no field set for the types that fan out.**
  `sms`, `call`, `screen_mirror` and `remote_input` have no validation arm at all
  (`security.rs:748-750`), and `clipboard`, `notification` and every `file`
  action bound the fields they know without ever calling `reject_unknown_fields`.
  A paired peer can therefore pad any of them arbitrarily. The relay fan-out now
  refuses anything over 256 KiB, which bounds the shared leg — but the guard is
  downstream of the gap, not a replacement for it, and the same padded frame
  still reaches every paired socket on the LAN. The fix is tracked as
  `REMAINING_WORK.md` W3.31's *Left to do*.

## How to read this

Each finding carries a **verdict**, because "these two sides differ" is not
itself a defect:

| Verdict | Meaning |
|---|---|
| **REAL GAP** | The two sides disagree and something is wrong. |
| **INTENTIONAL** | They differ, and the difference is defensible. The reason is given, so it is not re-raised. |
| **STALE** | A known concern that is in fact already handled. |
| **NEEDS DECISION** | Genuinely ambiguous. A human has to choose. |

**INTENTIONAL findings are load-bearing.** Roughly a fifth of this document is
the same class of difference that is *correct*, and recording why is what stops
the next reader from "fixing" it.

## Coverage

| Boundary | Scope | Findings |
|---|---|---|
| [A](#a-the-four-representations-of-one-protocol) | `schema.json` ↔ `types.rs` ↔ `websocket.ts` ↔ `protocol.dart` | 21 |
| [B](#b-protocol--the-desktop-rust-backend) | 56 protocol types ↔ desktop dispatch | 24 |
| [C](#c-protocol--the-mobile-dart-app) | 56 protocol types ↔ mobile Dart | 24 |
| [D](#d-the-tauri-bridge) | desktop Rust ↔ React | 15 |
| [E](#e-the-android-bridge) | mobile Dart ↔ Kotlin | 29 |
| [F](#f-the-ios-bridge) | mobile Dart ↔ Swift | 30 |
| [G](#g-the-relay-and-its-clients) | relay ↔ desktop ↔ mobile | 18 |
| [H](#h-declared-capability-versus-used-capability) | 4 manifests ↔ usage in 3 languages | 36 |

**Cross-cutting:** four agents independently converged on the same structural
root cause from different directions. That is the single most important result
here, and it is [below](#the-structural-finding).

---

## The structural finding

Four agents, working eight unrelated boundaries, described the same mechanism.
This is why the field-level findings below are mostly *latent* rather than
firing: nothing enforces agreement, so drift accumulates silently and the
guard tests are shaped to miss it.

**S1 — The generated Dart model is imported by nothing.**
`apps/mobile/lib/models/protocol.dart` (806 lines, generated from `schema.json`)
has **zero importers**. Verified by enumerating all 445 import/export/part
directives across all 91 Dart files; `PurpleType` and `Protocol.fromJson` are
referenced only inside the file itself. No `part`, no `build.yaml`, no
`build_runner`. The phone models the entire protocol as raw
`Map<String, dynamic>` with `msg['x'] as String?` at every hop.

To be fair to the codebase: the mobile *does* have protocol modelling.
`lib/services/relay_route.dart` is a hand-written mirror of the binary frame,
route signing and KDF, and it **is** pinned against the Rust producer by real
cross-language tests. The problem is specific and worse than "no coupling" — the
*generated* model is orphaned, and the coupling that does exist is hand-maintained
somewhere else. The file also carries 365 of `flutter analyze`'s 367 infos: lint
noise on a file nothing compiles against, which is why that count is high and
uninformative.

Two documentation defects fall out: `docs/ARCHITECTURE.md:725` claims
`protocol.dart` carries a "GENERATED FILE — DO NOT EDIT" banner (it carries
quicktype's stock header instead — so the one generated file nobody imports is
also the one a developer could hand-edit unchallenged), and `docs/TESTING.md:307`
claims the protocol crate pins "textual invariants in the Dart source" (it pins
them against `websocket_service.dart`, `pairing_service.dart` and
`discovery_service.dart` — never against `protocol.dart`).

**S2 — The generated TypeScript union is inert outbound.**
`sendMessage: (msg: Record<string, unknown>) => boolean` at
`useWebSocket.tsx:34,361`. All 15 outbound call sites pass bare literals or local
types; **not one** is typed against `WebSocketMessage`. The repo documents this
itself at `useAutomation.ts:102-104`.

*The inbound half needs correcting, because it is better than "untyped".*
`WebSocketMessage` *is* used as a parameter type in six places, and in the three
files without `@ts-nocheck` the union is genuinely load-bearing: removing the
discriminant narrowing in `ScreenMirror.tsx` produces `TS2322`, and
`npx tsc --noEmit` exits 0 *because* `data.width`, `data.height` and
`data.action === 'frame'` are checked against it. So the accurate statement is:
**load-bearing inbound in three typechecked files, inert outbound everywhere —
and five of the six inbound handlers live in `@ts-nocheck` files.**

**S3 — The schema is consulted for tag equality, never for field shape.**
This is the claim that needed the most correction, and the correction matters.

The strong version was wrong. **There are 11 typed inbound parses** into
`conduit_protocol` types: nine in the desktop (`screen_mirror.rs` ×4,
`remote_input.rs` ×4, `files.rs:222` for the binary envelope) and two in the
relay (`connection.rs:230` `RelayAuth`, `route.rs:95` `RelayRoute`). The imports
are explicit and deliberate, with doc comments stating the intent —
`remote_input.rs:3-6`: *"deserialised into those types instead of being read
field-by-field out of `serde_json::Value`, so a client cannot invent a field name
the injector silently ignores."* And `security.rs:508-510` explicitly hands those
types off: *"relay types whose boundary validation is owned by their handlers."*

**It is a deliberate two-tier split, not a free-for-all.** `security.rs` owns
shape and size for `pairing`/`notification`/`file`/`clipboard`/`automation`/
`audio`/`status`/`discovery`; typed structs own `screen_mirror`, `remote_input`
and the binary envelope.

What survives is narrower and more useful: **`pairing`, `audio` and `automation`
inbound shapes are hand-maintained.** `automation.rs:611-624` parses an inbound
rule into *local* `TriggerType`/`ActionType` enums — a genuine fourth authority,
and it has already drifted (see A-F6).

**S4 — `generate_types.js` drops every numeric bound.**
`minimum`, `maximum`, `minLength`, `additionalProperties` and `description` are
all discarded. Every constraint in `websocket.ts` is unenforced at compile time.
The generator's own comment says it *"deliberately fails loudly rather than
guessing"* — true for tags and titles, not for bounds.

**S5 — The schema↔Rust guard test skips exactly the types that drifted.**
`schema_accepts_every_serialized_rust_type` (`types.rs:5218`) is the one test
linking Rust to the schema. It deliberately omits `AudioPlaybackStopped` and
`RemoteInputKey`, and serialises `ScreenMirrorStart` with `fps: None`. It passes
while three of the findings below are present.

### What follows from this

**Fixing `schema.json` alone cannot close any schema-drift item.** That is why
W1.5 in particular has stayed HIGH across several commits: the schema is not the
authority anyone reads at runtime.

---

## A. The four representations of one protocol

56 `oneOf` branches, 21 distinct `type` tags, 5 `definitions`. Field-level diff:
**schema↔Rust = 5 discrepancies; schema↔TS = 0.** The generator is a faithful
projection — which means every schema gap propagates into the TypeScript.

### Fields present in code, absent from the schema

| # | Finding | Verdict |
|---|---|---|
| A-F1 | **`sms/mark_read` — a whole message type that no representation knows.** The desktop emits it (`useSms.ts:154`), the phone consumes it (`main.dart:167`). No validator, no generated type, no `PROTOCOL.md` entry. | REAL GAP |
| A-F2 | `screen_mirror/start.fps` exists in `types.rs:576`, is read by the hub, is depended on by the phone (`main.dart:326` reads `msg['fps'] ?? 15`) — and is absent from the schema. A schema-conformant reader cannot discover the field. | REAL GAP |
| A-F3 | `screen_mirror/touch.actionType`: Rust accepts 5 values including `move`; the schema has 4. The desktop's own UI emits `'move'` (`ScreenMirror.tsx:193`) and widens the union by hand (`:13`). Any schema validation of a real capture frame fails. | REAL GAP |
| A-F4 | **`relay_route`: 5 of 8 fields absent from the schema** (`from_device_id`, `timestamp`, `nonce`, `key_id`, `hmac`). From `websocket.ts` a `relay_route` looks like an unsigned 3-field envelope. And `protocol.dart` has **no `key_id` field at all** — the Dart code *signs* a route carrying `key_id` but the model cannot read one back. | REAL GAP |
| A-F5 | `notification/mark_read`: `types.rs:285` has `id: String` (**required**); the schema branch has no `id` property. A schema-valid `mark_read` cannot be deserialised by `types.rs`. | REAL GAP |
| A-F6 | `automation/rule`: schema, `types.rs` and both generated clients all agree with each other and are all **wrong about the wire**. The desktop sends nested (`useAutomation.ts:112-120`); the hub parses nested-first (`automation.rs:595-598`). Note the correction to `REMAINING_WORK`: the *mobile* producer is flat (`automation_rule.dart:333-338`), matching the schema — only the desktop's webview is nested. The hub accepts three spellings, so nothing breaks in-tree. | REAL GAP |
| A-F7 | `discovery/announce.os`/`.version`: required in `types.rs` and `PROTOCOL.md`, omitted from the schema's `required` list ⇒ generated TS makes them optional and can build an announce the Rust struct cannot represent. | REAL GAP |
| A-F8 | `file/progress`: the phone reads `chunks_received`, a **local DB column**, not a wire field — so incoming-transfer progress never advances on Android. | REAL GAP |
| A-F9 | `remote_input/scroll` carries `dx`/`dy`; the phone reads `delta`. Always `0f`. **Remote scroll is a silent no-op.** | REAL GAP |
| A-F10 | The `remote_input` handler reads `msg['type']` — the message-type tag, always the literal `"remote_input"` — as the gesture type. Latent (no sender emits `remote_input/touch`), but the field name is wrong. | REAL GAP (latent) |
| A-F11 | `automation/rule_remove` — emitted by the phone, in no representation; the hub answers `unknown_action`. | REAL GAP |
| A-F12 | `file/chunk_binary` — synthesised in-process by `websocket_service.dart`, never crosses a socket. Defensible, but undocumented. | INTENTIONAL |
| A-F13 | `main.dart:265-266` reads `address` and `port` off a `discovery/announce`; **neither is a protocol field** (always `''`), and the fallback is a hardcoded `9527` — the *plaintext* port — where `kLanWssPort = 9531` exists. The one un-exempted port literal in the mobile tree. | REAL GAP |
| A-F14 | `status/update`: the hub accepts `device_id`, `app_package`, `timestamp`; **no representation models them**, and `StatusUpdate` has no `deny_unknown_fields`, so `serde` succeeds and silently drops all three. | REAL GAP |
| A-F15 | The schema declares **11** numeric bounds; `types.rs` enforces **one**. Rust can serialise `percent: 101`, `unread_count: -1`, `protocol_version: 0`. | REAL GAP (latent) |

### Confirmed consistent — do not re-litigate

- **Binary frame v2, Rust ↔ Dart, byte for byte.** version `0x02`; 16-byte
  zero-padded target; sequence u32 **big-endian** at offset 17; 32-byte tag at
  21..53; `BINARY_HEADER_LEN = 53`. MAC input identical, hex-encoded before HMAC.
  Both truncate an over-long target to 16 bytes and say why. v1 rejected by both.
- **LAN chunk envelope, Rust ↔ Dart.** 24-byte nonce, u32 **little-endian** length,
  JSON metadata, ciphertext. The two formats are dispatched on `_isRelayConnection`
  with an explicit comment that they must not be confused.
- **All 12 shared constants agree across four languages**, pinned by
  `relay_route_test.dart` and the relay's `interop_vector_for_the_dart_client`.
- **8 enum sites agree four ways** (touch types, quality, trigger type, action
  type, profile, window state, device type, key modifiers).
- **`generate_types.js` is faithful** — 0 field-level discrepancies, including
  correct `anyOf` handling.
- Top-level branches omit `additionalProperties:false` while all 5 `definitions`
  set it — deliberate, pinned by a test, and load-bearing.

### Two documentation defects that will cause a future outage

- `PROTOCOL.md:1243` and `:1415` both show `"key_id": "v1"` in a `relay_route`
  example, while `:1256,1327` say it **must equal `from_device_id`** and
  `lib.rs:348-354` rejects otherwise. The doc's example contradicts its own rule
  in the most-copied part of the spec.
- `PROTOCOL.md:1259` says "the **four** optional fields are `#[serde(default)]`".
  There are five.

---

## B. Protocol ↔ the desktop Rust backend

Full 56-row inventory performed. **45 handled · 4 admitted-but-unhandled ·
3 handled-but-unreachable · 2 send-only and correctly refused inbound · 6 types
absent from the schema entirely.**

| # | Finding | Verdict |
|---|---|---|
| B-F1 | **The relay control-plane arms are dead, but not silent.** `("relay_auth_ok",_)`, `("relay_auth_rejected",_)` and `("error",_)` (`mod.rs:1047-1070`) sit behind the auth gate (`:835-855`), and `"relay_server"` is never written into `ws_to_device_id` or `SyncEngine`, so `is_trusted_peer` returns false. Probed: `["Rejected unauthenticated relay_auth_ok message from relay_server"]`. **Corrected:** the gate fires a `warn!` naming the type first, so this is not silence — the specific diagnosis at `:1055-1058` ("the stored relay token is wrong or was rotated") is replaced by a worse generic one. `send_error` writes nothing because the relay socket has **no `ctx.clients` entry at all**. | REAL GAP |
| B-F2 | **Per-type rate limit is 60 per 60 s, not per second.** `security.rs:155-158` passes `TypeLimitConfig::new(60, 60)`; the constructor's second parameter is `window_secs`. The comment states throughput expectations (*"~30 fps × 2 msgs each"*) three orders of magnitude above the implemented budget. Probed: 60 accepted, 61st refused in under 1 s. **Worst part:** `screen_mirror/stop` shares the frame bucket, so after 60 frames **0** further `screen_mirror` messages are accepted — **the message that stops the stream is starved by the stream itself.** Inbound-only, which is why desktop→phone is unaffected and this went unnoticed. `test_type_limit_config_values` pins `max_messages` and never asserts `.window`. | REAL GAP |
| B-F3 | **No device can be the target of a v2 binary frame.** `BINARY_DEVICE_ID_LEN = 16`; every device id is a 36-char UUID. The relay truncates and looks up exactly ⇒ never found. Both receivers also compare 16 bytes against 36 and refuse. **Three independent breaks**, per verification: the truncation; the phone never builds a frame (`_targetDeviceId` is always null); and **nobody produces a v2 frame anywhere** — `build_binary_frame` has zero callers under `src-tauri/`. | REAL GAP |
| B-F4 | `handle_lan_chunk` receives a device id and looks it up in `ws_to_device_id`, which is keyed the **reverse** way — gets `None`, defaults `""`, drops the chunk after successful authentication. Latent behind B-F3; **no test** exists for `unwrap_relay_binary_frame`. | REAL GAP |
| B-F5 | **Four unfiltered fan-out loops in two handlers** (`files.rs:145-155`, `audio.rs:45-50,69-74,153-159`) iterate `ctx.clients` directly, bypassing the pairing filter every other fan-out uses. **Corrected by verification:** on *files* this exposes metadata (filename, size), not content, and the auth gate **does** block unpaired *senders* — so it is a receive-side leak triggered by a paired peer, not an auth bypass. **On audio it is worse than stated**: the capture is WASAPI loopback, i.e. *all system audio*, not the mic. `add_test_unpaired_client` appears in zero audio/files tests — the suite is green because nothing covers it. | REAL GAP |
| B-F6 | `notification/post.actions`: column exists, reader returns it, generated TS carries it — the handler hardcodes `actions: None`. Inline action buttons from a phone are discarded. | REAL GAP |
| B-F7 | `file/progress.percent` unbounded inbound; outbound, `total: 0` is schema-legal and `(received/total*100) as u32` saturates to **4 294 967 295**. | REAL GAP |
| B-F14 | `file/request.size` / `chunk.index` guards use `as_u64()`, which is `None` for a negative integer, so the schema bound is skipped; `-1` becomes `u64::MAX` / `4 294 967 295`, and the transfer never finalises. | REAL GAP (low) |
| B-F15 | **A directed `file/request` reaches nobody.** `files.rs:153` treats `to` as a connection id; both senders set a stable device id. The desktop's own `send_file` uses the correct resolver two files away. Phone→desktop file send dies at step one. | REAL GAP |
| B-F16 | `PROTOCOL.md:1488` says "Mobile and desktop both send" the v2 frame. The desktop **cannot** — the relay egress channel is `mpsc::Sender<String>`, text only. | REAL GAP (doc) |
| B-F17 | `trusted_source_only` — declared, documented as a security gate, **read by nothing**. Latent only because the auth gate already blocks unpaired senders. ADR-0002 has been corrected. | NEEDS DECISION |
| B-F19 | `("automation","")` admits a schema-forbidden frame and fails **silently** (no `error` frame). Three unknown-action paths are silent; one is not. | NEEDS DECISION |
| B-F23 | **The desktop announces one device id and is reachable under another.** `handle_discovery_announce` sets `device_id` to a 16-hex prefix of the X25519 public key; the UUID in `device_id.txt` is what goes into `relay_auth`, `hub_device_id` and every route-key derivation. A phone caching the announced id gets `unknown_device`. Compounded by `discovery` now requiring auth, which makes the wrong id *more* likely to be cached. | REAL GAP |
| B-F24 | `notification/post.device_id` is required by schema and the handler trusts the sender's claim. `status/update` uses the connection identity correctly — the two disagree. | REAL GAP (low) |
| B-F10/11/12/13 | `automation/rule_remove` → `unknown_action`; `clipboard/request` admitted but unhandled; `sms/mark_read` invented by the desktop webview with an invented field; `audio/*.from` set to a per-connection UUID that changes on every reconnect. | REAL GAP |

### Confirmed consistent

Auth-gate **ordering** is right (after `validate_message`, before `settings_gate`,
so an unauthenticated peer cannot probe the user's settings — pinned by a test).
One socket, one identity: `resolve_sender_secret` never lets a claimed
`source_device` override a live connection identity, and `unwrap_relay_delivery`
checks the relay-stamped id against the pairing registry — both with negative
tests. **Exactly one encryption layer**, and `seal_for_peer` **fails closed** to
`delivery_failed` rather than falling back to plaintext. `reject_unknown_fields`
on `status` and `discovery` is genuine deny-by-default for the two types whose
handlers read a closed field set. All 8 trigger and 9 action enum values agree
across schema, `types.rs` and `automation.rs`. `pairing/revoke` →
`unsupported_message` is correct and documented — accepting a revoke from any
paired peer would be a remote DoS on the user's own pairing list.

---

## C. Protocol ↔ the mobile Dart app

**24 REAL GAPs.** 56 types: modelled 12 · sent 30 · acted-on-receive 31.

**19 distinct (type, action) pairs arrive on a registered channel and are
silently dropped** — and because there is no `error` handler, none of the drops
is ever reported back.

| # | Finding | Verdict |
|---|---|---|
| C-F1 | **Desktop→phone file transfer is 100% dead.** `main.dart:223-229` logs *"Received legacy base64 chunk, ignored"*; the desktop's only phone-bound chunk path sends base64 (`commands/file.rs:205-221`). `request` arrives, the phone accepts, every chunk is discarded; the transfer sits at 0% forever. **Understated in the audit** — recorded there as "the phone discards an extra frame". Verified with no surviving alternate path: base64 is used on **both** LAN and relay. | REAL GAP |
| C-F2 | `main.dart:334` calls `invokeMethod(action!)` with `touch`/`key`/`scroll`; both natives register `injectTouch`/`injectKey`/`injectScroll`. **Tapping, typing and scrolling the mirrored desktop screen does nothing.** The future is discarded with no `catchError` ⇒ one `debugPrint` the user never sees. The sibling `remote_input` handler uses the correct names — the two handlers disagree. | REAL GAP |
| C-F3 | `remote_input`: reads `delta` (wire has `dx`/`dy`) and reads `msg['type']` as the gesture type. | REAL GAP |
| C-F4 | `remote_input` handles only `touch`/`key`/`scroll`; the desktop emits `move`, `click`, `scroll`, `key`. **Remote mouse move and click are dropped.** | REAL GAP |
| C-F9 | **No `error` handler anywhere.** Every rejection the hub returns — `unsupported_message`, `unknown_action`, `command_not_allowed`, `invalid_message`, `reject_due_to_setting` — is discarded. This is the mechanism that hides several other findings, converting "the hub refused you" into "nothing happened". | REAL GAP |
| C-F11 | **`stopStreaming()` never sends `stream_stop`.** The hub starts system-audio capture and is never told to stop — after the user stops the mic the desktop keeps capturing and pushing encrypted PCM. **Privacy-visible, and the phone cannot stop it.** | REAL GAP |
| C-F13 | The phone **handles** `call/answered`/`ended` but never sends them; `sendCallAction` — the exact sender — has **no caller anywhere**. Answering on the phone leaves the desktop ringing forever. Accidental asymmetry: the sender exists and is unused. | REAL GAP |
| C-F14 | The phone handles `sms/mark_read`, which **is not a protocol type at all**. `markThreadRead` is reachable only from that branch ⇒ reading a thread on the phone never propagates anywhere. Dead in both directions. | REAL GAP |
| C-F15 | `sms/new` accepted only in the `{thread_id, message}` shape, but the phone **emits** `{from, body, timestamp}`. With two phones paired, phone A's SMS reaches phone B and both fields are null ⇒ ignored. | REAL GAP |
| C-F16 | Tapping the mirrored **desktop's** screen injects a tap into the **phone**. Misrouted, not merely unimplemented. | REAL GAP |
| C-F19 | No `relay_auth_ok`/`relay_auth_rejected` handler, and `connect()` sets `_isConnected = true` **before** `_sendRelayAuth()`. A rejected token leaves the phone showing "connected" forever. | REAL GAP |
| C-F23 | `remote_input_service.dart` and `screen_mirror_service.dart` are imported by **no test**. Both are wire-building classes, and `flutter test` is green, so none of C-F2/C-F3/C-F4/C-F14 is caught. | REAL GAP |
| C-F6 | `_lastInboundSequence` is reset only in `clearDeviceId()`, never in `connect()`, while the relay re-bases its outbound sequence per connection. After any reconnect every inbound relayed binary frame is dropped as "replayed", permanently. | REAL GAP |
| C-F20 | The `discovery` handler builds a dud `DiscoveredDevice(address: '', port: 9527)` — neither field is a protocol field — and never calls `setAdvertisedWssPort`, whose doc comment says it exists for exactly this. | REAL GAP (latent) |
| C-F10, C-F12 | `notification/reply` and both audio playback acks are dropped. The phone's duplex UI sets its own state optimistically, so nothing visibly breaks today. | REAL GAP (latent) |

**Dead senders with zero callers, verified by grep:** `sendCallAction`,
`sendSmsMessage`, `sendFileRequest`, `sendFileAccept`, `sendFileCancel`,
`sendFileChunk`, `startDuplexMode`, `stopDuplexMode`, `setAdvertisedWssPort`,
`setPreferLanWss`, `FileTransfer.fromJson`, `DeviceInfo.fromMap`.

---

## D. The Tauri bridge

The sharpest boundary in the repo: a mismatch here is invisible to both
compilers.

**The starting numbers were both wrong, and are corrected here.** The brief
guessed ~40 registered commands with ~18 uncalled. Actual: **36 registered, and
only 2 genuinely uncalled.** The frontend makes 41 production `invoke()` call
sites across 34 distinct names. The original figures came from grepping `invoke(`
without the generic form and from excluding the `lib/tauri.ts` wrappers — acting
on them would have produced a bogus 16-command "dead code" sweep.

### The strongest result on any boundary

- **Command names, direction Rust→TS: 36/36. Zero misses.** Every command name
  in TS is a plain string literal; **no dynamic invocation exists**, so no name
  can be missing at runtime.
- **Argument names and camelCase conversion: 20/20 correct.** Verified against
  the *installed* Tauri source rather than from memory —
  `tauri-2.11.6/src/ipc/command.rs:78-146` looks up `payload[key]` exactly, and
  `tauri-macros-2.6.3/src/command/wrapper.rs:506-512` lower-camel-cases every key
  by default. `target_device_id`→`targetDeviceId`, `file_path`→`filePath`,
  `from_device`→`fromDevice`, `source_device`→`sourceDevice`. **No
  camelCase/snake_case typo exists anywhere on this boundary.**
- **Event parity: 2/2 both directions, zero orphans.** Rust emits exactly two
  events across all 30 `.rs` files; TS has exactly two `listen()` calls in one file.
- `SettingsData` ↔ `ConduitSettings` key sets identical (19 fields), enforced
  **bidirectionally** — a Rust test parses the TS source and passes. **All 19
  settings are written *and* read, and all 19 round-trip.**

### Findings

| # | Finding | Verdict |
|---|---|---|
| D-F1 | **`relay_enabled` has three definitions of one default.** `storage.rs:1032` says `"false"`; `commands/settings.rs:195` says `default_true`; `settingsTypes.ts:129` says `true`. Fresh install → empty settings table → `read_setting` = `None` → `"false"` → `main.rs:305` returns early and **the relay never starts and never joins**, while the settings UI shows the toggle off and the pinned test says on. Attacked from five angles (migration seed, pre-startup writes, `main.rs` fallback, frontend bootstrap) — all closed. **Stronger than claimed:** the frontend's first Settings save *persists* the wrong value permanently. **Why the suite is green:** `the_relay_is_on_by_default` tests `ConduitSettings::default()` and **never `Storage::get_settings()`** — the only function `main.rs` consults — and `test_get_settings_defaults` enumerates 13 defaults, omitting the one that is wrong. | REAL GAP |
| D-F2 | `save_settings` never restarts the relay, while `RelaySection.tsx:99` promises *"Saving your changes restarts it."* Toggling off → Save → "Settings saved" → **the listener stays bound and peers keep connecting** until restart. | REAL GAP |
| D-F3 | `start_discovery` builds a `DiscoveryService` **without `set_app_handle`**. `discovery.rs:219-229` documents the consequence: *"nothing can receive it — discovery results are dropped."* Stop-then-start and `mdns-device-discovered` is never emitted again. | REAL GAP |
| D-F4 | **`sync_clipboard` has zero production callers, so `clipboard_history` is never written.** The real producer writes a raw WS frame; `mod.rs:909-911` only broadcasts, with no local persistence. Result: `get_clipboard_history` always `[]`, the clipboard half of the Files screen permanently empty, pin/delete/clear permanent no-ops. **Fixing W4.3 as written does not fix this** — the table has no writer. **Second independent bug:** `App.tsx:157` routes `onSynced` to `refreshDevices`, not `refreshClipboard`. | REAL GAP |
| D-F5 | `send_to` returning false ⇒ `warn!` ⇒ `Ok(())`, while `useEncryption.ts:277` converts that into `{ encrypted: true }` and suppresses the toast. Peer drops between lookup and send ⇒ status bar claims handled, no warning. | REAL GAP |
| D-F6 | `resume_file_transfer` verifies a `checksum`; `useFiles.ts` omits it though it is on the wire and in the generated type. A resumed transfer is reported complete with **no integrity check**. | REAL GAP |
| D-F7 | `mdns-device-removed` emits an mDNS fullname; `DiscoveredDevice` has no `service_name`, so all three fallback matchers fail. **A device that leaves the LAN stays in the list for the process lifetime.** | REAL GAP |
| D-F8 | `save_window_state`/`get_window_state`: **zero references** in all 108 TS files. `WindowState::validate()` is dead in production and the live path bypasses every guard the tests pin. | REAL GAP |
| D-F9 | Device cap off by more than one: the backend counts only `status=="paired"`, the UI hardcodes `MAX_DEVICES = 5` and gates on `devices.length`, which **always includes the hub**. | REAL GAP |
| D-F10 | `__mocks__/tauri.ts` is 3 lines and diverges from the real API in 13 ways — see below. | REAL GAP |
| D-F11 | `save_settings_field` accepts any key; an XSS'd webview can write `allowed_commands = ["*"]`, bypassing the only thing between a peer-supplied `RunShellCommand` and `Command::new`. | REAL GAP |
| D-F13 | Eight comments assert facts that are now false, one user-visible: Advanced → Advanced says *"the cloud relay client is disabled in this build"* while the relay runs in-process and joins itself. | STALE |
| D-F15 | `relay_port`/`relay_health_port`/`relay_cert_pin` round-trip with no UI editor. **Operator-level settings with no frontend surface by design.** | INTENTIONAL |

### `__mocks__/tauri.ts` — why every frontend bridge test asserts a fiction

The file is `import { vi } from 'vitest'; export const invoke = vi.fn();`, and
`vitest.config.ts` aliases **two** real modules onto it.

| # | Divergence | Consequence |
|---|---|---|
| 1 | The mock returns **`undefined`**, not a `Promise` | `await invoke(...)` yields undefined; production code hits `undefined.map` |
| 2 | **never rejects** | **no test on this boundary exercises a command's error path** |
| 3 | the aliased `invoke` is **untyped** | `invoke<T>` in test files is checked against `vi.fn()`, not the real `Promise<T>` — **a declared generic that doesn't match Rust is invisible** |
| 4 | `@tauri-apps/api/window` is aliased but only `invoke` is provided | `getCurrentWindow()` is undefined; **`win.show()` never executes in any test**, so `core:window:allow-show` has zero coverage |
| 5 | `@tauri-apps/api/event` is **not** aliased | tests load the *real* module |
| 6 | five registered plugins are **not** aliased | `plugin-fs`/`plugin-dialog` paths are untested against anything |
| 7 | no `restoreMocks`/`clearMocks` | the shared `invoke` mock leaks between tests |
| 8 | `@tauri-apps/api/mocks` ships `mockIPC`, purpose-built for this | unused; the project hand-rolls a strictly worse substitute — `mockIPC` alone would fix #1–#3 |

Also: 9 dead props / unwired UI sites, including `App.tsx:216`'s Ctrl+F
`setSearchFocused(true)` which **does nothing** because `SearchBarProps` has no
ref prop; a **second** `useDevices()` instantiation; `TitleBar.tsx` (144 lines)
entirely unreferenced, and it calls all four window operations
`capabilities/default.json` explicitly forbids; and
`contexts/NotificationContext.tsx` entirely unreferenced — **not mentioned anywhere
in `REMAINING_WORK.md`**.

---

## E. The Android bridge

Both channel names byte-identical. **The "everything is fine" reading is wrong in
a way that is invisible from either side.**

| # | Finding | Verdict |
|---|---|---|
| E-F1 | **Two of three EventChannel subscribers are permanently dead.** `receiveBroadcastStream` registers its Dart handler by **channel name only** (`platform_channel.dart:698` → `channel_buffers.dart:186-188`: *"Only one listener may be set at a time"*). Native-side, `EventChannel` holds **one** `activeSink` per channel name (`EventChannel.java:187`), replaces it on every `listen` (`:207`), and `success()` returns early unless it is still active (`:248`). After startup, the last registrant wins and **`notifications` and `calls` are dead — the phone records zero notifications and zero call-state events, forever.** Only `screen_mirror` survives. | REAL GAP |
| E-F3 | `injectScroll` reads `delta`; the wire carries `dx`/`dy`. Always `0f` → zero-length gesture → `dispatchGesture` no-ops → **`result.success(true)`**. | REAL GAP |
| E-F5 | `isAccessibilityServiceEnabled`/`openAccessibilitySettings` are implemented and **called from nowhere in Dart**. The service is declared correctly, so nothing ever walks the user to enabling it. Every `inject*` returns `NO_ACCESSIBILITY`, swallowed. | REAL GAP |
| E-F6 | Kotlin returns `success(true)` **unconditionally**. `dispatchGesture`'s boolean is discarded; `rootInActiveWindow == null` returns silently; DPAD keys log and return. Dart sees `true`, the desktop sees nothing. **The exact "reports success, delivers nothing" class.** | REAL GAP |
| E-F7 | `injectTouch` reads `actionType` but uses it **only in a log** — every gesture becomes one 100 ms tap; `modifiers` likewise, so `shift+c` types `c`. **Live today** for phone-views-desktop. | REAL GAP |
| E-F8 | `ScreenMirrorService` swallows every exception, then `MainActivity.kt:642` calls `pending?.success(true)` unconditionally ⇒ a failed projection yields `true`, a registered viewer, and **zero frames ever arrive**. | REAL GAP |
| E-F9 | No `MediaProjection.Callback`/`VirtualDisplay.Callback`. On API 34+ the system stops sharing on backgrounding **without telling the app**, while `isStreaming` stays true and the scheduler keeps calling `acquireLatestImage()` at up to 30 Hz forever. | REAL GAP (static, API 34) |
| E-F12 | `getRunningAppProcesses()` returns only the caller's own processes since API 22 ⇒ `app_open` triggers never fire. | REAL GAP |
| E-F13 | Location + `NEARBY_WIFI_DEVICES` declared, **neither runtime-requested** ⇒ `getWifiName` returns `"unknown"` unconditionally; `wifi_change` can never match. | REAL GAP |
| E-F14 | `NotificationService.setSendFunction` **is never called**, and the Kotlin emitter sends no `device_id`. Two independent defects on the same path — notifications never reach the hub even when the stream works. | REAL GAP |
| E-F16 | `ConduitForegroundService.onStartCommand(null,…)` after a process kill needs a qualifying runtime permission that may have been denied ⇒ `ForegroundServiceStartNotAllowedException` + `START_STICKY` restart loop with an undismissable notification. | NEEDS DECISION |
| E-F25 | `MethodCall.argument<Int>("fps")` does **no** `Number` coercion; a JSON `15.0` ⇒ `ClassCastException` and the `?: 15` default never runs. | REAL GAP (latent) |

### Confirmed consistent

**Argument and return-type parity is 100%.** All 19 Dart call sites and 20 Kotlin
arms enumerated; every argument map is flat on both sides; every Dart generic
matches what Kotlin puts in `result.success(...)`, including the awkward cases
(`getRunningApps` returns a JSON *String* and Dart declares `String` +
`jsonDecode`; `getSmsThreads` returns a JSON String and Dart widened to `Object?`).
**No nesting mismatch and no return-type mismatch anywhere on this boundary.**

`registerReceiver` flags are correct for API 33+. Foreground-service types are
correctly declared for API 34+ in both the manifest and `startForeground`. The
accessibility service declaration is valid and its config XML references a string
that exists. **SMS is genuinely repaired** — W2.3 is closed.

**Rotation does not end mirroring** — `AndroidManifest.xml:37` handles
`configChanges`, so the Activity is not recreated. What ends mirroring is Activity
*death*, plus E-F8 and E-F9. (`REMAINING_WORK` states the rotation claim; it is
wrong.)

---

## F. The iOS bridge

**The entire iOS side is one file** against Android's five and Dart's 52.

### The app does not launch on iOS

Three independent reasons, any one of which is fatal:

1. **No Xcode target.** `project.pbxproj` has no `PBXNativeTarget`
   (`targets = ()`), no build phases, no file references, no `INFOPLIST_FILE`, no
   bundle id, no `SWIFT_VERSION`. `pod install` aborts *"Unable to find a target
   named `Runner`"*; `flutter build ios` has nothing to build. Present since the
   initial commit.
2. **No storyboards.** `Info.plist` names `Main` and `LaunchScreen`; `Base.lproj/`
   is **empty**. `AppDelegate.swift:86` force-casts nil ⇒ crash in
   `didFinishLaunchingWithOptions`.
3. **A plugin throws before `runApp`.** `flutter_local_notifications` throws
   `ArgumentError('iOS settings must be set…')` when `settings.iOS == null`, and
   Conduit passes `InitializationSettings(android: …)` only
   (`notification_service.dart:34-40`). The throw escapes into
   `runZonedGuarded`'s `debugPrint`. **`runApp` at `main.dart:367` is never
   reached.** Tests miss it because the integration test runs under
   `TargetPlatform.android`.

### Further findings

| # | Finding | Verdict |
|---|---|---|
| F-F3 | `NEHotspotNetwork` is **iOS 14+** against a declared `platform :ios, '13.0'` with no `@available` guard — a compile error as configured. Independently, the required entitlement is absent, so `_lastWifiName` latches to `"Unknown"` and **`wifi_change` triggers never fire on iOS**. | REAL GAP |
| F-F4 | Native notifications are captured and persisted but **never relayed** — `_send` is never wired. | REAL GAP |
| F-F5 | **`isScreenCapturing` is never reset on a clean stop.** After any clean stop every later `startScreenMirror` hits `guard !alreadyCapturing` ⇒ `result(false)`. **The phone must be force-quit; there is no recovery path.** | REAL GAP |
| F-F7 | `getRunningApps` returns `[]`; Dart does `invokeMethod<String>` ⇒ `TypeError`, and the enclosing `catch` aborts the rest of `_checkTriggers` ⇒ **the entire `time` trigger evaluation is unreachable on iOS.** | REAL GAP |
| F-F8 | `setPhoneProfile` has no iOS case, while the phone reports `automation/triggered` to the hub. **The desktop is told the rule fired while the phone did nothing.** | REAL GAP |
| F-F9 | `getApnsToken` is read **once at startup**, before registration completes ⇒ nil forever. **The APNs token never reaches the desktop.** | REAL GAP |
| F-F10 | `openNotificationListenerSettings` presents a modal "Not Available on iOS" **on every launch** — currently masked by F-F1, so it appears the moment F-F1 is fixed. | REAL GAP |
| F-F11 | iOS refuses SMS correctly, but the UI presents an **unsatisfiable, unresponsive** permission screen: the button is a permanent no-op behind an `_initialized` guard. | REAL GAP |
| F-F16 | Without the `user-assigned-device-name` entitlement, `UIDevice.name` is the generic model name — **every iOS phone shows as "iPhone"** in the desktop's device list. | REAL GAP (cosmetic, real) |
| F-F19 | `UIBackgroundModes` lacks `audio` ⇒ mic capture and desktop audio both die on screen-lock. `processing` is declared, never registered, and `BGTaskSchedulerPermittedIdentifiers` is absent — a hard App Store rejection pairing. | REAL GAP |
| F-F21 | Three `_onError` channels exist and **nothing ever calls `.onError(`**. This is *why* F-F10/F-F11/F-F12 are silent rather than merely broken. | REAL GAP |
| F-F13 | Desktop→phone input is `FlutterMethodNotImplemented` on iOS. **The intent is documented in comments** — iOS has no public touch-injection API outside a Custom Keyboard extension. | INTENTIONAL |

### The root cause of the asymmetry

**No `Platform.isIOS` / `isAndroid` / `kIsWeb` branch exists anywhere in `lib/`.**
There is no place in the Dart code where a platform check *could* have caught
F-F1, F-F7, F-F10 or F-F11. Every iOS-only behaviour is discovered at runtime as a
`TypeError`, a `MissingPluginException`, a thrown `ArgumentError`, a `false` from
a stub, or a modal alert. The single `defaultTargetPlatform` guard in the tree is
the correct one (it stops the SMS listener before subscribing).

### Confirmed consistent — and one important negative

**iOS's EventChannel argument binding is *better* than Android's.** It keys a
dictionary by the listen argument, so three concurrent subscriptions coexist and
each receives only its own type; Android assigns into fixed fields, so a fifth
argument type would be silently dropped. All three Dart subscribers filter on
`type` before acting, so cross-talk is handled on both sides. All 8 notification
payload keys match exactly between Swift and Dart.

**Negative result worth recording:** that agent checked every reachable native API
against `Info.plist` and found **no reachable API missing its usage-description
key** — microphone, camera, local network and Bluetooth keys are all present. The
"iOS terminates the app" failure mode does not occur on any path it could find.
(`REMAINING_WORK` W3.23 claims macOS *does* terminate and that iOS keys are
missing; see H for the disagreement.)

---

## G. The relay and its clients

**This is where the most consequential finding in the audit lives.**

| # | Finding | Verdict |
|---|---|---|
| G-R1 | **Two listeners in one process want TCP 9531.** The relay binds `127.0.0.1:9531` (`service.rs:453`, from `relay.rs:73` `DEFAULT_RELAY_LOCAL_PORT = 9531`); the hub's TLS LAN listener binds `0.0.0.0:9531` (`server/mod.rs:183,195`, `LAN_WSS_PORT`). **Whichever binds second loses and both outcomes are fatal** — either the relay aborts `start()` and nothing can be routed to the desktop, or the LAN TLS listener dies and `PROTOCOL.md:73`'s "only port a mobile client dials" is gone. Documented as intended, untested. | REAL GAP |
| G-R2 | **The phone's entire relay egress is dead.** `_targetDeviceId` is assigned in exactly one place, from `setRelayConfig(url, targetId)`, and **both production call sites pass `null`** (`settings_screen.dart:57-60,77-80`) — with a comment asserting the opposite. The signing guard is therefore never reached, so the phone emits bare `encrypted` envelopes, which `services/relay/src/connection.rs:594-610` explicitly refuses. **Nothing a relay-connected phone sends is routed.** And `websocket_service_relay_test.dart:107` calls `setRelayConfig(relay.url, 'desktop-1')` — so **all five tests in that file, including the one that exists to prove signing works, pin a state the shipping app cannot enter.** | REAL GAP |
| G-R3 | **The relay's binary file path cannot work at all, in either direction** — see B-F3. | REAL GAP |
| G-R4 | **The phone handles none of the relay's answers.** `relay_auth_ok`, `relay_auth_rejected` and all 19 `error` codes fall through with no handler. Auth rejection ⇒ a bare close ⇒ and `_reconnectAttempts` is zeroed on every successful upgrade ⇒ **retries every 3 s forever with no error ever set.** | REAL GAP |
| G-R5 | An unpaired phone sends `device_id: null`; the relay waits to `auth_deadline` then closes **with no frame written.** 10 s of silence, forever, with nothing in any log but a generic timeout line. | REAL GAP |
| G-R6 | Route timestamps are checked against **the relay's** clock in a `−5 s…+30 s` window. A phone >30 s fast or >5 s slow has **every** route refused — and because the phone ignores `error` frames, the failure is invisible. Desktop-side is immune by construction. | NEEDS DECISION |
| G-R7 | The relay re-bases its outbound sequence per connection; the phone never resets its high-water mark on reconnect. **Every** inbound relayed binary frame is then dropped as "replayed", permanently. Latent behind G-R3. | REAL GAP |
| G-R8 | **Every desktop→phone relayed frame is plaintext JSON** inside a signed route. `seal_for_peer` is not applied on the relay egress path. Phone→desktop *is* encrypted. This contradicts `services/relay/src/lib.rs:7-10` and `docs/ARCHITECTURE.md:60`. | REAL GAP |
| G-R9 | `broadcast`/`broadcast_to_others` iterate `ctx.clients`, and the relay connection is never in it ⇒ **no fan-out handler reaches a relay-only peer.** | REAL GAP |
| G-R11 | All phones behind the relay share **one** rate-limit bucket keyed on the literal `"relay_server"`. One relayed 100 MB transfer (~1600 chunks) already exceeds `file`'s budget, and one phone throttles every other phone. | REAL GAP |
| G-R16 | **On a fresh install there is no way at all to point the phone at the relay.** The only writer of `relay_url`/`relay_token` prefs is annotated `// ignore: unused_element` — *"There is no Relay section in the UI yet"* — and `PairingAccept` carries no `relay_token`. **This is the missing feature underneath every phone-side relay symptom above.** | REAL GAP |
| G-R12 | `key_id` is signed but **never validated and not required**, while `PROTOCOL.md` marks it Required and says the verifier resolves the key under it. | STALE (docs) |
| G-R13 | Rust's canonical string is alphabetical; both docs and `sign_with`'s comment claim `SIGNED_FIELDS` order. They agree today. **Enabling `serde_json/preserve_order` anywhere would make every route fail `hmac_invalid`.** | NEEDS DECISION |
| G-R14 | The phone handles `relay_delivery` on **any** connection with no registry check on `from_device_id`; a LAN peer can set the recorded attribution to any id it names. | REAL GAP (latent) |
| G-R17 | The phone cannot produce a real `ping`/`pong` on a relay connection — the heartbeat becomes a routed, encrypted frame landing on the desktop's no-op arm. | REAL GAP (low) |

### The desktop's dual role — answered

The self-join is **load-bearing and correct in intent**, and there is **no
self-delivery loop**: a self-targeted relayed frame would be refused, because the
desktop is never `save_device`d, so `get_client(desktop_uuid)` is `None`. The real
hazard is the one W7.5 already names — an impostor `clients.insert(desktop_uuid, …)`
evicts the desktop permanently and `reconcile_clients` cannot repair it, though it
still cannot forge traffic *as* the desktop. **But G-R1 means the self-join never
completes today.**

### Confirmed consistent

**Route signing is bit-identical across all three implementations and mutually
pinned** — the same three hex constants are asserted in the relay's suite and in
the Dart test. The binary MAC construction is identical on all three sides.
**The relay's 19 emitted error codes match `PROTOCOL.md` §8.2 exactly**, 19/19 by
scripted extraction. The desktop's whole outbound wire set is legal. Route nonces
survive reconnect and restart. A superseded connection cannot evict its
replacement.

**Honest asymmetry:** the vectors pin the *route* mutually, but the *binary
frames* are pinned **one-way only** — `relay_route_test.dart` pins four whole
frames and says so itself; no Rust test asserts them.

---

## H. Declared capability versus used capability

AndroidManifest · `Info.plist`/entitlements · Tauri capabilities ·
`tauri.conf.json`, against usage in three languages.

| # | Finding | Verdict |
|---|---|---|
| H-F6 | **No `com.apple.developer.networking.multicast` entitlement** ⇒ `sendto` to 224.0.0.251 fails and **mDNS discovery silently finds zero desktops on iOS.** The local-network prompt is answered; only the entitlement is missing. | REAL GAP |
| H-F7 | **`RECEIVER_NOT_EXPORTED` blocks incoming SMS on Android 13+.** Android's docs single out exactly this case: such a receiver does not get broadcasts sent on behalf of highly-privileged apps such as telephony. `MainActivity.kt:254-257` registers with that flag and `targetSdk = 36`. **Live SMS mirroring silently no-ops.** | REAL GAP |
| H-F14 | `usesCleartextTraffic="true"` in the **release** manifest; `allowBackup` unset ⇒ true ⇒ **the SQLCipher DB and plaintext `FlutterSharedPreferences` are included in cloud backup** (the Keystore key is not, so a restore yields an unreadable DB *and* a readable token). | REAL GAP |
| H-F17 | **93 Tauri commands are granted; the frontend calls 6.** Including `internal_toggle_maximize` — a *mutating* window permission that `default.json:13-21` claims was removed — plus `internal_toggle_devtools`, `emit`/`emit_to`, all 22 menu and 12 tray commands. Bounded by the CSP, but a genuine over-grant the ACL test cannot see. | REAL GAP (security, low impact) |
| H-F19 | **macOS has no `Info.plist` at all** and opens an audio input stream ⇒ on macOS 10.14+ this **terminates the process**. `REMAINING_WORK` says the opposite (*"fail silently behind a `warn!`"*) and misplaces the keys in `entitlements.plist`. | REAL GAP |
| H-F20 | `tauri.conf.json:50` says `./Entitlements.plist`; the file on disk is lowercase. Real on a case-sensitive volume. CI has no macOS runner, so only a hand-rolled `continue-on-error` grep can see it. **Verified, not stale.** | REAL GAP |
| H-F21 | `tauri.linux.conf.json` and `tauri.windows.conf.json` are referenced **nowhere**; Tauri v2 has no such auto-discovery convention. On Windows and Linux the tray renders the macOS template asset. Two dead declaration surfaces. | REAL GAP |
| H-F22 | `linux.deb.depends` omits `libxcb1`, `libxkbcommon0`, `libwayland-client0`, which `xcap`/`enigo`/`wayland-client` link against. | REAL GAP |
| H-F23 | `fix_firewall` is registered with no `#[cfg(windows)]` and the button renders unconditionally ⇒ a dead, always-visible Windows-only control on macOS and Linux. | REAL GAP |
| H-F27 | **The desktop's LAN TLS identity is regenerated on every launch and never persisted.** The mobile pin bootstrap can only pin a per-session key; **a desktop restart invalidates every phone's pin with no re-pairing affordance.** Contrast `encryption.rs`, where the loopback secret *is* keyring-backed. | REAL GAP |
| H-F31 | **Wayland-only Linux: `Enigo::new(...).unwrap()` panics** inside the tokio task. macOS: no accessibility-grant flow, so synthetic events are dropped by TCC. Every return value is discarded. **Input control works on Windows/X11 and silently does nothing on Wayland and un-granted macOS.** | REAL GAP |
| H-F32 | `xcap` is X11/dbus-only on Linux ⇒ desktop screen mirror returns `Failed to get monitor`. | REAL GAP |
| H-F13 | 4 declared Android permissions with zero callers, combined with SMS/Call Log/location ⇒ squarely inside **Google Play's restricted-permission policy**. A review risk, not just bloat. | REAL GAP (review) |
| H-F11 | `CHANGE_WIFI_MULTICAST_STATE` is absent — but the plugin **never acquires a `MulticastLock`**, so adding the permission alone would fix nothing. Real symptom: inbound multicast is filtered when the device dozes, so the Nearby list silently empties. | REAL GAP (characterised) |
| H-F10 | `PERMISSION_REQUEST_CODE = 1001` **collides with `record_android`'s** ⇒ the mic callback can resolve with the startup permission batch's verdict. | NEEDS DECISION |
| H-F35/36 | `NSPhotoLibraryUsageDescription` declared but unreachable; `NSBonjourServices` lists a service never browsed. | INTENTIONAL (over-declaration) |

**Direction one (used but not granted) for Tauri: none.** All 6 plugin commands
the frontend uses are granted, and `fs:allow-read-text-file`'s empty scope is
populated by `dialog:allow-open`. `shell:allow-open` is correctly absent — the
plugin is in `Cargo.toml` but deliberately not registered.

### Features silently missing or broken on 2+ of 5 platforms

| Capability | Status |
|---|---|
| App builds / launches | **iOS ✖** — no target, no storyboards, throws before `runApp` |
| mDNS discovery | iOS ✖ (no multicast entitlement) · Android ◐ (no MulticastLock) |
| Send / read SMS | **iOS ✖** entirely · **Android live ✖** (`RECEIVER_NOT_EXPORTED`) |
| Remote input into phone | iOS ✖ · Android ✖ (accessibility never enabled) |
| Remote input into desktop | **macOS ✖** (no TCC grant) · **Linux ✖** (X11-only, panics on Wayland) |
| Screen capture, both directions | **Linux ✖** · iOS ◐ |
| System-audio capture | **macOS ✖ terminates** · Android ◐ (silent on deny) |
| Wi-Fi name (automation trigger) | Android always `"unknown"` · iOS broken |

**This is the table the audit's author would have wanted first**, and it is the
part no single boundary could produce.

### The audit's own blind spot

`REMAINING_WORK.md` has a W-section for every layer **and none for "does the
project build, and do the usage keys exist."** No entry for the gutted pbxproj,
the empty `Base.lproj`, the pre-`runApp` throw, the missing multicast
entitlement, `RECEIVER_NOT_EXPORTED`, the unrequested location permissions, or
the dead `tauri.*.conf.json`.

---

## Open disagreements

Two items could not be settled without a device. Both are recorded rather than
resolved.

**1. Are two iOS purpose strings actually required?**
One agent concluded that no reachable API is missing its usage-description key
and that the "iOS terminates the app" mode never occurs. Another concluded that
`NEHotspotNetwork` (`AppDelegate.swift:475`) and `RPScreenRecorder`
(`:309,335`) are reachable and their keys absent, so iOS terminates / capture
never starts. Both cite the same call sites; they disagree on whether ReplayKit
and NEHotspotNetwork are TCC-gated. **Neither could run iOS.** Needs a device.

**2. Whether the Android/iOS API-level conclusions hold.** Every Kotlin and Swift
claim in E, F and H is **static reading only** — there is no Android SDK, no
Xcode, and no device on the machine this ran on. The three that most deserve a
device are H-F7 (`RECEIVER_NOT_EXPORTED` and telephony), E-F9 (MediaProjection
callback on API 34) and H-F31 (Wayland `Enigo::new` error-vs-no-op).

---

## What is worth doing next

Items 2 and 5 below are **done** — see the `Fixed` table and the correction
above. What remains, in dependency order — because several of the rest are one
root cause wearing different clothes.

1. **iOS cannot build or launch.** Three independent blockers: the pbxproj has no
   `PBXNativeTarget` (a stub someone wrote and committed — one commit has ever
   touched it, and it was 112 lines from the start), `Base.lproj` is empty so
   `AppDelegate.swift:86` force-casts nil, and `flutter_local_notifications`
   throws before `runApp` because only `InitializationSettings.android` is
   passed. Restoring the project from the current Flutter template fixes the
   first two and raises the deployment target to 15.0, which also dissolves the
   `NEHotspotNetwork` availability error for free. **This is the single largest
   block of functionality in the project and none of it is subtle once you know
   to look.**
2. ~~**All phones behind a relay share one rate-limit bucket** (G-R11)~~ —
   **done, and the stated mechanism was wrong.** There was no shared bucket;
   relayed frames were charged nothing by anyone. They are now charged against
   the authenticated **sender's** device id, with the transport budget reserved
   for frames no device can be named for. One peer can no longer throttle
   another, and a peer's frames can no longer go unaccounted.
3. **`remote_input` into the phone is unreachable on both platforms** (E-F5,
   F-F13). On Android the accessibility service is declared but nothing ever
   walks the user to enabling it; on iOS there is no public injection API at all.
   Android is a one-line fix plus a settings entry.
4. **Screen capture and remote input into the desktop are broken on Linux**
   (H-F31, H-F32): `xcap` is X11-only, and `Enigo::new(...).unwrap()` *panics*
   inside the tokio task on Wayland.
5. ~~**A relay-only peer cannot be told why it was refused**~~ — **done, and it
   was never a protocol change.** `relay_route.payload` is an arbitrary JSON
   message and the relay forwards it without inspecting it, so an `error` was
   always legal to route; the route simply was not being taken.
6. **The structural finding** (top of this document). Nothing enforces agreement
   between the four representations, which is why three rounds of fixes produced
   ~40 findings. Wiring the generated Dart model up, or deleting it and admitting
   there are three representations, would stop the class recurring.

Two things are *not* worth doing, and the reasons are recorded above: narrowing
`is_desktop_executable` (trades five false successes for seven), and simply
raising the per-type rate limits (the *window* was wrong, not the count — a
60-second window admits a one-second burst and then refuses for 59 seconds).


## A note on the numbers in `REMAINING_WORK.md`

Several entries' `file:line` references now point **inside `#[cfg(test)]`
blocks** — W1.11's `files.rs:588,622,688` and W1.16's `screen_mirror.rs:1249`
among them, and W1.8's `main.dart:339` points at a doc comment. Several more are
30–200 lines off. If entries are being actioned by line number, some will be
actioned against the wrong code. Line numbers are the most perishable part of
that document and the easiest thing to re-derive.
