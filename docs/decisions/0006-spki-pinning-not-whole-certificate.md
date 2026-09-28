# ADR-0006: Certificate pinning is SPKI, on both sides, and is discoverable

- **Status:** Accepted
- **Date:** 2026-09

## Context

Pinning was specified but not achievable end to end, because the two sides
hashed different things.

- The pin tooling hashed the certificate's **SubjectPublicKeyInfo**:
  `scripts/generate-cert-pin.sh:133-138` does
  `openssl x509 -pubkey -noout | openssl pkey -pubin -outform DER | openssl dgst -sha256 -binary | openssl base64 -A`,
  and the relay's own pin is SPKI too
  (`services/relay/src/tls.rs:390-397`).
- The only client that implemented pinning hashed the **whole DER certificate**:
  `apps/mobile/lib/services/websocket_service.dart:191` calls
  `_computeSha256Sync(cert.der)`.

`sha256(SPKI-DER) != sha256(cert-DER)`. The documented workflow therefore
produced a pin that the client could never match, and **every pinned connection
was rejected**. Pinning was not "not enabled" — it was a hard failure, which is
worse, because the failure looks like an attack and trains operators to disable
the check.

The relay's test suite states the divergence explicitly and pins the correct
invariant: `spki_pin_is_not_the_whole_certificate_hash`
(`services/relay/src/tls.rs:855-871`).

## Decision

**Both sides hash the SPKI.**

1. **The relay computes the pin from the certificate it actually loaded**, at
   startup, from the parsed certificate rather than from the PEM text
   (`services/relay/src/tls.rs:159-163`, `spki_pin` at `tls.rs:28`).
2. **SPKI extraction is a self-contained DER walk with no new dependency.**
   `read_tlv` / `spki_der_from_certificate` (`tls.rs:266-388`) parse the
   certificate as DER, descend `Certificate` → `TBSCertificate`, skip the six
   fields that precede the key (optional version, serial, signature algorithm,
   issuer, validity, subject), and **slice the original bytes** rather than
   re-encoding, so canonical DER in gives canonical DER out. Only the short and
   long definite-length forms are handled, which is all DER permits.
3. **The two sides are cross-checked against third parties, not against each
   other.** `spki_extraction_matches_the_generating_key_pair`
   (`tls.rs:789-803`) checks the extracted SPKI against the key pair that signed
   the certificate, and `spki_extraction_matches_openssl_byte_for_byte`
   (`tls.rs:1280-1327`) pipes the extracted SPKI through the real `openssl` and
   requires the same digest. A self-consistent bug would pass a
   self-consistency test.
4. **The pin is discoverable, not out of band.** `GET /pin` on the relay's health
   port returns `{"sha256": "sha256/…", "algorithm": "spki-sha256"}`, computed
   from the loaded certificate (`services/relay/src/main.rs:917-941`; the value
   reaches the health handler as `state.tls_pin`, `main.rs:1000`). When TLS is
   not initialised it returns `"sha256": null` with `"error": "tls_unavailable"`
   rather than a wrong answer. `scripts/generate-cert-pin.sh:72-92` prefers this
   endpoint and falls back to `openssl` only if it is unreachable, and it refuses
   to print a pin for a certificate that fails `-checkhost`
   (`generate-cert-pin.sh:119-127`).
5. **The client side is documented as a requirement, and the requirement is
   written where the implementer will see it.** `generate-cert-pin.sh:148-165`
   states that the Dart client must hash the SPKI and not the whole DER, and
   says plainly that hashing `cert.der` produces a value that never matches.

## Consequences

**Easier.** Pinning works. It survives certificate renewal as long as the key
pair is reused, so an operator renews without redistributing pins — which is the
difference between a pin that is maintained and a pin that gets disabled. The pin
can be discovered from the running relay, so it can never again be computed from
the wrong file. The relay's DER walk is validated against `openssl`, so a
mistake in it is a test failure rather than a production outage.

**Unwelcome: the Dart client still hashes the whole certificate.**
`websocket_service.dart:191` is unchanged. The relay and the tooling are on SPKI;
the mobile client is not. The fix here made the *correct* answer computable and
correctly documented; it did not make the client correct. This is the single
outstanding half of this decision and it is tracked in
see the Limitations section of the README. Until it is done, a pin obtained
through the documented workflow still will not match on Android or iOS.

**Unwelcome.** The desktop has no configuration target for a pin at all.
`VITE_RELAY_CERT_SHA256` was deliberately removed (see
`apps/desktop/src/vite-env.d.ts`), so the desktop client does no pinning. This
ADR does not change that; the gap is stated in
`generate-cert-pin.sh:157-161` and belongs in
[`../../SECURITY.md`](../../SECURITY.md).

**Unwelcome.** The `openssl` cross-check test depends on `openssl` being on
`PATH`. It is a test, not a build dependency, but it will skip or fail in an
environment without it — which is itself a small hole in the assurance argument.

**Unwelcome.** SPKI pinning does not protect against a key compromise, and it
does not distinguish two certificates that share a key. A renewal that reuses the
key is accepted, which is the intended behaviour, but it also means a key reused
across two unrelated hosts inherits the union of both hosts' trust.

## Alternatives considered

**Change the script to hash the whole certificate, and document that.** Rejected
as simpler but wrong. A whole-certificate pin pins the *certificate*, not the
*key*, so it must be redistributed on every renewal. That is a recurring
operational tax, and recurring operational taxes on security controls are how
security controls get turned off. It is also strictly less robust: any change to
the certificate body — a new SAN, a changed validity window — breaks every
client, including changes the operator did not intend as key changes.

**Add a DER-parsing crate as a dependency.** Rejected: the walk is about sixty
lines, handles only the two length forms DER requires, and being
self-contained means it cannot be broken by a transitive bump. The cost is that
we own the correctness, which is why the `openssl` cross-check exists.

**Use `webpki` / the TLS stack's own SPKI extraction.** Rejected: rustls does not
expose the server certificate's SPKI as a value. Re-parsing the DER we already
have is the only option without a new dependency.

**Serve the pin over the relay's own authenticated channel instead of a separate
`/pin` endpoint.** Rejected: a client that needs the pin in order to
authenticate cannot fetch it from behind the authentication it has not yet
performed. Discovery has to be out of band by construction.

**Drop pinning and rely on the system trust store.** Rejected: that is a
different, weaker posture — public CAs can issue for a name, and on a LAN a
device name is not a secret. `docs/SECURITY.md` owns the argument; the short form
is that pinning is the only thing standing between a relay hostname and anyone
who can get a certificate for it.

## Related

- Relay TLS configuration: `docs/relay-tls.md`.
- Threat model for the relay boundary: `docs/SECURITY.md`.
