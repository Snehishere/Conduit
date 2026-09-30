# ADR-0004: The relay message-signing key is domain-separated from the relay token

> ## Status: SUPERSEDED by ADR-0011
>
> **This record is kept as written.** Its Context and Decision are not edited;
> the reversal is recorded in a new record, per the rule in
> [`README.md`](README.md) that a decision that is later replaced gets a new
> file rather than a rewritten one.
>
> **What replaced it:** [ADR-0011](0011-per-device-relay-route-keys.md).
> Instead of one relay-wide signing key derived from the relay's master secret,
> **every device has its own route key**, derived from the pairing secret that
> device already shares with the desktop. `RELAY_SIGNING_KEY`,
> `RELAY_SIGNING_KEY_ID`, `RELAY_SIGNING_KEY_PREVIOUS` and
> `RELAY_SIGNING_KEY_PREVIOUS_ID` no longer exist, the `{current, previous}`
> ring is gone, and there is no rotation window.
>
> **Why:** the objection this record raises against itself — quoted verbatim
> under *Consequences* below, that an unset `RELAY_SIGNING_KEY` means "the
> master secret is effectively still the root of trust — the separation is
> between *roles*, not between *custodians*" — is the defect ADR-0011 fixes.
> Separating roles inside one custodian is not enough when the clients cannot
> hold the key at all: see ADR-0011's Context for the two dead ends this
> record's design leads to.

- **Status:** Superseded by [ADR-0011](0011-per-device-relay-route-keys.md)
- **Date:** 2026-09

## Context

The relay's bearer token and the key that authenticated `relay_route` messages
were the same secret. `RELAY_TOKEN` was what a client presented to authenticate,
and the same value was the HMAC key for signed routes.

Two consequences, both bad:

1. **Every authenticated client could impersonate every other.** A client knows
   the relay token by construction — it had to send it. If that value is also the
   MAC key, then any client can compute a valid MAC over a route that claims
   `from_device_id` of a different device. Message-level authenticity was
   therefore worth nothing.
2. **The operator instructions told operators to make it so.**
   `.env.example` instructed that `HMAC_SECRET` be set equal to `RELAY_TOKEN`.
   The documented configuration was the maximally-collapsed one.

The blast radius of (1) is one authenticated client on the relay forges traffic
in any other client's name.

## Decision

Four changes, together.

1. **A distinct, domain-separated signing key.** The message-signing key is
   derived from the master secret by a labelled KDF:

   ```
   derive_key(master, label) = HMAC-SHA256(master, "conduit-protocol/v1/derive:" || label)
   derive_signing_key(master) = derive_key(master, "conduit-relay/v1/message-signing-key")
   ```

   (`packages/protocol/src/lib.rs:91`, `:110`; the prefix is
   `KDF_PREFIX` at `lib.rs:73`, the label `SIGNING_KEY_LABEL` at `lib.rs:68`.)
   The label is part of the KDF input, so a key derived with this label cannot
   equal the master secret, cannot equal the relay token, and cannot equal any
   other derived key. `derive_key` rejects an empty label outright
   (`lib.rs:94`). Bumping `KDF_PREFIX` retires every previously derived key at
   once.
2. **`from_device_id` is signed.** It is in the canonical signing string
   (`lib.rs:167`), so rewriting it invalidates the MAC
   (`lib.rs:899-913`, `types.rs:887-897`).
3. **The signature is bound to the authenticated connection identity, not just to
   the message body.** The relay requires
   `route.from_device() == authenticated_device_id`, where
   `authenticated_device_id` is the identity established by `relay_auth` on
   *this* connection (`services/relay/src/main.rs:1863-1900`). A valid signature
   alone is not accepted: every client holds the signing key, so the signature
   proves "a legitimate client produced this", not "this client produced it". A
   mismatch is rejected as `sender_mismatch`.
4. **A `key_id` with a `{current, previous}` rotation window.** `SigningKeyring`
   (`lib.rs:254-315`) carries a current key and id and, optionally, a previous
   key and id. Signing always uses `current`; verification accepts either, but
   only when the message's `key_id` names it (`lib.rs:311-315`). A message with
   no `key_id` is rejected (`lib.rs:1063-1070`) — rotation state has to be
   explicit on the wire. The relay resolves the ring from
   `RELAY_SIGNING_KEY[_ID]` and `RELAY_SIGNING_KEY_PREVIOUS[_ID]`
   (`services/relay/src/main.rs:374-411`); a previous key without an id, or two
   identical ids, is a startup error. `.env.example:65-92` documents the
   re-keying procedure and the `/metrics` counter to watch
   (`conduit_relay_messages_dropped_total{reason="hmac_failed"}`).

The relay token remains the *authentication* credential. It is no longer the
*signing* credential. `HMAC_SECRET` is used for exactly two things: seeding the
derived signing key, and backing the replay-nonce cache
(`.env.example:28-31`).

## Consequences

**Easier.** A stolen relay token no longer yields message forgery. Compromise of
one secret no longer implies compromise of two roles. Key rotation is a
supported operation with a documented overlap window, instead of a coordinated
outage. The single implementation lives in `conduit_protocol::hmac`
(`services/relay/src/hmac.rs:6` re-exports it) so the relay and the desktop
cannot drift.

**Unwelcome, and the most important line in this record: this is a breaking wire
change. Deployed clients are rejected.** An old client signs with whatever it was
configured with; a new relay verifies only the derived key and the `{current,
previous}` ring. Every existing deployment must deploy the new client and the
new relay together. There is no compatibility shim and no legacy path — the
legacy path is precisely the vulnerability. `packages/protocol/PROTOCOL.md` and
`README.md` are the operational notes; rollout procedure is in
See the project Limitations section.

**Unwelcome.** `RELAY_SIGNING_KEY` is unset by default, in which case the key is
derived from `HMAC_SECRET`. That is convenient and it means the master secret is
effectively still the root of trust — the separation is between *roles*, not
between *custodians*. An operator who wants independent custodians must set
`RELAY_SIGNING_KEY` explicitly.

**Unwelcome.** The `{current, previous}` window is a two-slot ring. A key
compromised during its previous-slot window is still accepted, and there is no
revocation: dropping a key means restarting the relay without it in the ring. A
full revocation mechanism (a fetched key list, a signed revocation set) is not
built. `docs/SECURITY.md` owns that discussion.

**Unwelcome.** An operator who re-keys in the wrong order gets a total outage,
and the failure surfaces as `hmac_invalid` on every route, which is
indistinguishable from a client misconfiguration unless you read the metrics.
The rejection message says so explicitly (`services/relay/src/main.rs:1902-1909`).

## Alternatives considered

**A separate `key_id` namespace held in a service crate.** Rejected: the shared
implementation must live in `conduit_protocol` so the desktop and the relay
cannot diverge, and `conduit-protocol` cannot depend on a service crate — that
would invert the workspace dependency direction and make the protocol crate
unusable by third-party clients. A protocol crate that cannot name a service's
configuration is the correct boundary; the label is the whole of the namespace
that is needed.

**A full JWE-style encrypted envelope for routes.** Rejected as
disproportionate. It would add a nested encryption layer to messages that are
already transport-protected by TLS, at the cost of a new envelope format, a new
negotiation, and a new set of failure modes, to solve an authenticity problem
that a signed `from_device_id` plus a connection-identity comparison solves
completely.

**Sign only the body and keep `from_device_id` unsigned.** Rejected: this is
precisely the original bug in a subtler form. The claim that names the sender
must be inside the MAC.

**Rely on TLS client certificates per device.** Rejected for now: it moves the
identity problem to certificate issuance and revocation, which is a larger
operational surface than a shared secret plus a rotation window, and it would not
remove the need to sign the sender claim.

**Keep one key and rotate by restarting clients and relay simultaneously.**
Rejected: it turns a routine key change into a coordinated outage of every
client. The `{current, previous}` ring exists precisely so that window does not
have to be instantaneous.

---

## What changed

Everything below describes the relay **as it is not**. None of it is in the code.

1. **`RELAY_SIGNING_KEY[_ID]` and `RELAY_SIGNING_KEY_PREVIOUS[_ID]` are
   gone.** There is no relay-side signing key to configure, and no `.env` to
   configure it in.
2. **`SigningKeyring` and its `{current, previous}` ring are gone.** Verification
   resolves the one key belonging to the device that claims to have sent the
   route, by asking the host through the `RouteKeys` trait
   (`services/relay/src/route.rs:89-137`). A device can sign for itself and for
   nothing else, so there is no ring and no rotation window.
3. **The derivation is per device, not per relay.** ADR-0004 derived one key from
   the relay's master secret; the current scheme derives each device's key from
   that device's own pairing secret
   (`conduit_protocol::hmac::derive_route_key`, `packages/protocol/src/lib.rs:136`).
   The master secret no longer sits in the route-verification path at all — it
   only backs the `/health` bearer-token default.
4. **`key_id` is still mandatory, but it is no longer a rotation id.** It must
   equal `from_device_id`. That is what closes the "rewrite the signature onto
   another device" hole without a second, separate id to remember.
5. **Revocation is immediate** rather than impossible. A revoked device's row
   stops being registered as a key source, and the desktop drops its key within
   `KEY_REFRESH_INTERVAL` (5 s), with no overlap window in which the old key is
   still accepted.

The wire-level guarantee this record was reaching for — a valid signature alone
must not be enough, because every client holds the signing key — is preserved
and tightened. Under ADR-0011 a valid signature is *never* alone enough: the key
that verifies a route is the key belonging to the authenticated connection, so
no other client can produce a signature this one can.

See [ADR-0011](0011-per-device-relay-route-keys.md) for the replacement
decision.
