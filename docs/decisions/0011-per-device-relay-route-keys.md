# ADR-0011: Every device signs relay routes with its own key, derived from the pairing secret

- **Status:** Accepted
- **Date:** 2026-09

## Context

[ADR-0004](0004-domain-separated-relay-signing-key.md) fixed a real forgery
bug: the relay's bearer token had been doubling as the HMAC key for
`relay_route`, so any authenticated client could sign a route claiming
`from_device_id` of any other device. It separated the two roles with a
labelled KDF and added a `{current, previous}` rotation ring.

The separation was correct, and it produced two dead ends.

1. **The derived key was not a key any client could hold.** It came from
   `HMAC_SECRET` — the relay's own master secret, resolved from
   `HMAC_SECRET_FILE` or bootstrapped at startup
   (`services/relay/src/config.rs:103-137`). No client has it, by design: it is
   the relay's, not the user's. So no client, on either platform, could produce
   a route the relay would accept, and the whole relay path was unreachable
   from both ends. This is why ADR-0004's own *Consequences* records that "an
   old client signs with whatever it was configured with; a new relay verifies
   only the derived key … Every existing deployment must deploy the new client
   and the new relay together", and why the previous release shipped with the
   relay client disabled.
2. **The one alternative that makes clients work is the forgery bug again.**
   Hand a client the bearer token and it can sign as anyone, because the token
   is a shared secret by definition. Hand it the relay-wide signing key and the
   same is true — ADR-0004 says so itself in its Decision: "every client holds
   the signing key, so the signature proves 'a legitimate client produced this',
   not 'this client produced it'". The check that made it survivable was a
   second check, comparing the signed `from_device_id` against the connection
   identity `relay_auth` established.

ADR-0004 records its own version of this in *Consequences*, and it is the
sentence this decision exists to act on:

> **Unwelcome.** `RELAY_SIGNING_KEY` is unset by default, in which case the key
> is derived from `HMAC_SECRET`. That is convenient and it means the master
> secret is effectively still the root of trust — the separation is between
> *roles*, not between *custodians*.

The root problem underneath all three points is that a **shared** key has to be
shared. Held by one relay it is unreachable by clients; held by every client it
is forgeable by every one of them. Any design that keeps one key has to pick
which failure to accept.

A rotation window does not help, because its whole purpose is an overlap during
which two keys are both accepted for the same signer. That is only necessary
when a signer has more than one key. If each device has exactly one, there is
nothing to overlap and the ring is complexity in search of a problem.

## Decision

**Each device's route key is derived from that device's own pairing secret.**

1. **The derivation.** `packages/protocol/src/lib.rs:98`, `:136`:

   ```
   derive_key(master, label)     = HMAC-SHA256(master, "conduit-protocol/v1/derive:" || label)
   derive_route_key(secret, id)  = derive_key(secret, "conduit-relay/v1/route-key:" || id)
   ```

   `secret` is the 32-byte X25519 secret the device already established with the
   desktop during pairing (`PROTOCOL.md` §4.2) — a secret the client has and the
   hub stored, so nothing new has to be provisioned or handed out. `id` is the
   device id the hub filed the device under, and it is inside the label
   (`ROUTE_KEY_LABEL`, `lib.rs:75`), so two devices sharing one pairing secret
   get unrelated keys. The `device_id` is not secret; it is the lookup key.
2. **`key_id` must equal `from_device_id`.** The key is looked up under
   `key_id`, and the attributed sender is then required to match
   (`services/relay/src/route.rs:89-137`), so neither can be rewritten to point
   a valid signature at another device. `key_id` stays a **mandatory** signed
   field (`hmac::SIGNED_FIELDS`, `packages/protocol/src/lib.rs:193-201`): it is
   what the verifier resolves against, so omitting it must fail closed rather
   than fall back to "try something".
3. **The relay holds no keys. It asks its host.** Route verification takes a
   `&dyn RouteKeys` and calls `key_for(device_id)`
   (`conduit_protocol`-shaped trait in `services/relay/src/state.rs`, used from
   `route.rs:125`). The relay is a library with no key store, no key file and no
   environment variable naming a key. An unknown device is `unknown_device`,
   not a failed signature.
4. **Where the host gets them** — `apps/desktop/src-tauri/src/relay.rs`:

   | Device | Source of its route key |
   |---|---|
   | A paired phone | `derive_route_key(device.shared_secret, device.id)`, from the `devices` row, recomputed on every refresh (`DeviceRouteKeys::refresh`, `relay.rs:161-201`). Only `status == "paired"` rows are registered. |
   | The desktop itself | Generated once as 32 random bytes and kept in the OS keyring under account **`relay_route_key`** (`relay.rs:54`, `:130-144`). It has no pairing with itself, so there is no secret to derive from. |
   | The bearer token | Generated once and kept in the OS keyring under account **`relay_token`** (`relay.rs:48`, `:417-426`). Never configured by hand. |

   Both credentials live beside the SQLCipher key and the X25519 identity under
   service `conduit_app`, and are covered by the same
   resolve-mint-prove-durability policy (`ADR-0008`).
5. **Re-pair is the rotation mechanism, and it is immediate.** A new pairing
   produces a new shared secret, so the next refresh yields a different key for
   the same device id, and the old one is simply no longer resolvable. Nothing
   is kept "for a while" in case a peer is slow to update, because there is
   nothing to coordinate: both sides read the one key that exists.
6. **No `RELAY_SIGNING_KEY*` variables exist.** Nor does `HMAC_SECRET` have any
   route-signing role left: the master secret now backs only the `/health`
   bearer-token default (`config.rs:382-404`).

## Consequences

**Easier.** A device can sign for itself and for nothing else, by construction
rather than by a follow-up comparison. A stolen relay token yields a connection
that can be authenticated but cannot forge a route, because the token is not in
the verification path at all. Revocation becomes real: revoking or unpairing a
device drops its key within `KEY_REFRESH_INTERVAL` (5 s) via the refresh task
(`relay.rs:79`, `:364-381`), and no key is ever accepted in a second slot
alongside its replacement. There is no operator to provision a key, no `.env`
to get wrong, and no deployment where the host and the relay can disagree about
secrets. And the host is the only party that holds keys, so the desktop's own
route key is derived, stored and re-derived in exactly one place.

**Unwelcome, and carried over from ADR-0004: this is still a breaking wire
change.** A client that signs with anything else is rejected, and there is no
compatibility shim, because the legacy path is the vulnerability. `PROTOCOL.md`
§9 owns the rollout note.

**Unwelcome.** The desktop's own route key is the one key that is *not* derived
from a pairing secret, because a desktop does not pair with itself. It is
therefore generated and stored rather than recomputed, which means it is the one
value whose loss takes routing with it — and the keyring fallback in ADR-0008
is what keeps that from being a data-loss event.

**Unwelcome.** Revocation is only as fast and only as complete as the `devices`
table is correct. A row with `status == "paired"` and an empty
`shared_secret` is skipped rather than guessed, so it silently loses routing
rather than routing with a key derived from nothing — fail-closed, but a
corrupt row is a silent outage for that one device rather than a loud one.
The refresh loop logs that case by name.

**Unwelcome.** `key_id` is now redundant with `from_device_id`, and it stays in
the wire format anyway. It is in `SIGNED_FIELDS`, so removing it would be a
second breaking change made for no security gain, and it is the field a
verifier resolves against. Keeping it costs a few bytes and buys one less thing
to get wrong later; the redundancy is deliberate and documented, not an
oversight.

**Unwelcome.** The relay cannot verify a frame on its own. It is now a component
that is only correct when paired with a host implementing `RouteKeys`
correctly — a real coupling, accepted in exchange for the relay holding no key
material at all.

**The old record's "unwelcome" paragraph is the motivation.** ADR-0004's
statement that the master secret was "effectively still the root of trust" is
not a caveat that can be documented away; it is the reason this record exists.
Separating *roles* under one custodian buys nothing once the clients cannot hold
the role's key. Separating *custodians* — giving each device a key only that
device can produce — is the thing that actually holds.

## Alternatives considered

**Ship the relay's signing key to clients in `relay_auth`.** Rejected: this is
the forgery bug ADR-0004 was written about, unchanged. Every client would hold
the ability to sign as every other client, and the `sender_mismatch` comparison
would be the only thing standing between that and open impersonation.

**Sign with the bearer token, per device, without a shared MAC key.** Rejected
for the same reason. Domain-separating the token does not help: any client that
can derive a valid MAC can derive it for any `from_device_id`.

**Let the relay issue per-device keys and sign them with a relay-side identity.**
Rejected as reintroducing exactly what is being removed: a signing identity to
provision, a key to distribute, and a revocation list to keep. It is ADR-0004
with more moving parts.

**Keep a rotation window, per device.** Rejected as incoherent. A window exists
so a signer can hold two keys at once; per-device derivation guarantees it holds
exactly one. The `conduit-relay` test
`a_re_pair_replaces_the_key_rather_than_adding_a_second_one` asserts that
exactly one key remains registered per device after a re-pair, so the property
is pinned rather than assumed.

**TLS client certificates, one per device.** Rejected for the reason given in
ADR-0004 and unchanged by this record: it moves the identity problem to
certificate issuance and revocation, which is a larger operational surface than
a derived key, and it would not remove the need to sign the sender claim.

**Use the device's X25519 identity key directly rather than a derived key.**
Rejected: the route key must be usable by a peer that shares the *pairing*
secret, and binding it to the long-term identity key would mean the relay needs
each device's public key and the ability to verify an X25519 signature, which is
a different verification algorithm with a different implementation to keep
correct on two platforms.

## Related

- The record this supersedes: [ADR-0004](0004-domain-separated-relay-signing-key.md).
- Relay TLS and the pin: `docs/relay-tls.md`, [ADR-0006](0006-spki-pinning-not-whole-certificate.md).
- Keyring resolution policy: [ADR-0008](0008-keyring-fallback-over-hard-exit.md).
- Threat model for the relay boundary: [`../../SECURITY.md`](../../SECURITY.md).