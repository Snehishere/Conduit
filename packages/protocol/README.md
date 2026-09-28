# conduit-protocol

Rust crate defining the Conduit wire protocol, plus the normative
[specification](PROTOCOL.md).

## What This Package Is

`conduit-protocol` holds the canonical Rust types for every message on the wire,
the HMAC / replay-protection helpers shared by the relay and the desktop, and the
binary-frame constants. It is a **library only** — it has no binary, no server
and no I/O.

The desktop app and the relay server both depend on it, so the two cannot drift
apart on message shape, signing or framing. The Dart client does not depend on
it; it is generated from the hand-maintained `schema.json`, which tests hold in
agreement with `types.rs` (see below).

## Scope of the Encryption

This crate defines the message shapes and the signing scheme. It does not
provide end-to-end encryption, and nothing here should be described that way:
the `encrypted` envelope (§4.16 and §6 of [PROTOCOL.md](PROTOCOL.md)) is keyed
to a secret shared between the desktop hub and one peer, and the hub terminates
it. A relay can route those bytes but cannot open them; either endpoint of a LAN
hop sees the plaintext. See
[ADR-0007](../../docs/decisions/0007-refuse-to-ship-end-to-end-encryption-claims.md).

## Why This Package Exists

Without a shared protocol definition, each component defines its own message
types ad-hoc. Over time, fields drift, new types get missed, and the mobile
app (Dart) and desktop/relay (Rust) disagree on wire formats. This package
eliminates that class of bugs.

## The Three Crates

This is a **Cargo virtual workspace** with exactly one `Cargo.lock`, at the
repository root. There is no per-crate lockfile and there must not be one — a
second lockfile is how two crates end up building against different versions of
the same dependency.

| Crate | Path | Role |
|-------|------|------|
| `conduit-protocol` | `packages/protocol` | The protocol library. Message types, HMAC/signing, replay cache, frame constants. No I/O. |
| `conduit` | `apps/desktop/src-tauri` | The Tauri desktop app. A **consumer**: it depends on `conduit-protocol` and implements the server side of the protocol (pairing, screen mirror, remote input, automation, file transfer). |
| `relay` | `services/relay` | The relay server. A **consumer**: it depends on `conduit-protocol` and implements the router (auth, `relay_route` verification, binary v2 framing, health/metrics endpoints). |

```
┌──────────┐    JSON / binary frames    ┌──────────┐
│  Mobile  │◄──────────────────────────►│  Desktop │
│  (Dart)  │                            │  (Rust)  │
└────┬─────┘                            └────┬─────┘
     │                                       │
     │          ┌─────────────┐              │
     └─────────►│   Relay     │◄─────────────┘
                │  (Rust)     │
                └─────────────┘
```

The mobile app is Dart and does not consume this crate; it mirrors the port
constants by hand, and `packages/protocol/src/types.rs` has tests that assert
the Rust constants, `PROTOCOL.md` and the Dart constants all agree.

## What's Included

| File | Purpose |
|------|---------|
| `PROTOCOL.md` | **The wire protocol specification.** Normative; read this for the message catalogue, the `relay_route` signing scheme, the binary frame layout, the error codes and the compatibility notes. |
| `src/types.rs` | The message types and binary-frame constants. Authoritative Rust definition. |
| `src/lib.rs` | Crate entry point; the `hmac` module (signing, keyring, `NonceCache`, key derivation). |
| `schema.json` | JSON Schema for every message type, for validating messages in other languages. **Hand-maintained.** |
| `Cargo.toml` | Rust package manifest. |

## Message Types

**There is no message-type table in this file, by design.**

A hand-maintained duplicate of `src/types.rs` is a second source of truth that
drifts silently — the previous version of this README had one, and it had
already fallen behind the code on several types. The catalogue lives in
[PROTOCOL.md §4](PROTOCOL.md#4-message-types) and the authoritative definition
lives in `src/types.rs`. `src/types.rs` currently defines **57 message types**
across 20 distinct `type` string values (`WireMessage` is an untagged envelope,
not a wire type). Do not add a table here; add the type to `src/types.rs` and
`schema.json`, then add its section to `PROTOCOL.md`.

There is no struct for `pairing/local_auth` (the desktop's loopback handshake,
documented in `PROTOCOL.md` §4.2); it is dispatched from raw JSON.

## Usage in Rust

```toml
# In your Cargo.toml
[dependencies]
conduit-protocol = { path = "../../packages/protocol" }
```

The crate is re-exported flat, so `conduit_protocol::*` brings in the message
types as well as `conduit_protocol::hmac`.

```rust
use conduit_protocol::*;

// Authenticate to a relay
let auth = RelayAuth::new("device-id-123", "relay-token-abc");
let json = serde_json::to_string(&auth)?;

// Route a message through the relay. The signature is mandatory: the relay
// rejects an unsigned route with `incomplete_relay_route`. Use the relay's
// message-signing key, never the relay token.
let signing_key = conduit_protocol::hmac::derive_signing_key(&master_secret);
let route = RelayRoute::signed(&signing_key, "device-id-123", "device-id-456", payload);
let json = serde_json::to_string(&route)?;

// Receiving side: verify against a keyring so key rotation keeps working.
let keyring = SigningKeyring::new("v1", signing_key.to_vec());
assert_eq!(keyring.verify(&incoming_json).as_deref(), Some("v1"));
```

`PROTOCOL.md` is the specification for the wire behaviour these helpers
implement — read §4.15 before writing a client.

## Validating Messages in Another Language

`schema.json` can validate messages from any language:

```bash
# With ajv (Node.js)
npx ajv validate -s schema.json -d message.json

# With jsonschema (Python)
jsonschema -i message.json schema.json
```

Two caveats:

* **`schema.json` is hand-maintained.** It is not generated from `src/types.rs`.
  It must stay in 1:1 agreement with the Rust types — one `oneOf` branch per
  wire message, no duplicates — and that invariant is enforced by tests in
  `src/types.rs`:

  | Test | Enforces |
  |------|----------|
  | `schema_sample_set_covers_every_oneof_branch` | one sample per branch, so branches and samples cannot drift apart |
  | `schema_oneof_branches_are_unambiguous` | no two branches share a `(type, action)` discriminator |
  | `schema_oneof_branch_titles_are_unique` | titles are unique (they key the generated types) |
  | `schema_accepts_every_serialized_rust_type` | every Rust type serialises to something the schema accepts |
  | `schema_accepts_every_sample_in_sample_set` | every branch accepts at least one real message |

  > A duplicated `oneOf` branch makes the union ambiguous: a *valid* message
  > matches more than one branch, which is an error for any conforming
  > validator. `schema_oneof_branches_are_unambiguous` exists to catch exactly
  > that regression.

  Run `cargo test -p conduit-protocol` after touching either file.

* **The schema is not the spec.** Where the schema and the code disagree, the
  code is right and the schema is a bug. `sms/new` and `sms/sent` are a
  documented, unresolved case where `schema.json` deliberately accepts two
  shapes because the phone and the desktop still disagree — see `PROTOCOL.md`
  §4.11.

## Editing the Protocol

1. Edit `src/types.rs`.
2. Edit `schema.json` to match.
3. Add or update the message's section in `PROTOCOL.md`.
4. `cargo test -p conduit-protocol` — 266 tests, including the schema/Rust sync
   tests and three tests that read `PROTOCOL.md` directly.
5. If the wire format changed incompatibly, add a Compatibility entry to
   `PROTOCOL.md` §9 and bump `PROTOCOL_VERSION` (or
   `BINARY_FRAME_VERSION`) as the change requires.
6. Regenerate the client models (§ below).

## Regenerating the Client Models

The TypeScript and Dart models are generated from `schema.json` by the npm
`generate` scripts:

```bash
cd apps/desktop
npm install        # once; pins quicktype
npm run generate   # runs both generators
```

| Script | Output |
|--------|--------|
| `npm run generate:types` | `apps/desktop/src/types/websocket.ts` |
| `npm run generate:dart` | `apps/mobile/lib/models/protocol.dart` |

**Never hand-edit the generated client models.** Edit `src/types.rs` and
`schema.json`, then regenerate — a hand-edit is silently lost the next time
anyone runs `npm run generate`, and in the meantime it disagrees with every
other client.

`quicktype` is pinned as a devDependency of `apps/desktop`, so codegen is
reproducible. `scripts/generate_types.js` and `scripts/generate_dart.js` are
hand-written projectors and **fail loudly** rather than guessing — they error on
a duplicate branch title and preserve `anyOf`/`oneOf`/`allOf` required-property
constraints instead of flattening them into optional fields.

## Versioning

The protocol is versioned independently of any application.
Current version: **1** (`PROTOCOL_VERSION` in `src/types.rs`).

Enforcement is asymmetric: only the desktop hub checks inbound
`protocol_version`, and the relay does not check it at all. See
`PROTOCOL.md` §1.1. The relay binary frame is versioned separately by
`BINARY_FRAME_VERSION` (currently `0x02`).

See `PROTOCOL.md` §7 for the versioning strategy and §9 for the current
breaking changes.

## License

MIT
