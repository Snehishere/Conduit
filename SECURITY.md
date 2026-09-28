# Security Policy

## Scope

Conduit is a local-first, peer-to-peer app. It links a phone to a desktop over
your local network, and can optionally be pointed at a relay you host yourself.
It has no cloud service, no account system, and no telemetry.

What this project defends against:

- **Unauthenticated LAN peers.** A connection must present a pairing token, or
  the per-launch local capability the desktop's own webview uses, before any
  privileged message is accepted. Pairing tokens are single-use and expire.
- **Drive-by access from a web page.** A browser page that reaches the local
  WebSocket port is refused; see [ADR 0003](docs/decisions/0003-per-launch-local-capability-token.md).
- **Untrusted file paths and capabilities.** Files are only opened from paths the
  app itself recorded, canonicalised and confirmed to be inside the downloads
  root. Tauri's capability surface is deliberately narrow, with tests that fail
  if it grants anything the frontend cannot use.
- **Arbitrary code execution via automation.** Shell commands run from
  automation rules are deny-by-default and gated by an explicit allowlist; see
  [ADR 0002](docs/decisions/0002-deny-by-default-shell-command-allowlist.md).
- **Forged relay messages.** Relay routes are signed with a key that is
  domain-separated from the relay token, and the sender identity is bound to the
  authenticated connection rather than to the message body; see
  [ADR 0004](docs/decisions/0004-domain-separated-relay-signing-key.md).

What this project explicitly does **not** defend against, by design:

- **Code running as your own user account.** A process that can run as you can
  read the app's database, read its config, or write to it. Conduit is not a
  sandbox and does not attempt to be one. See
  [ADR 0005](docs/decisions/0005-same-user-local-access-is-not-a-boundary.md).
- **A compromised operating system, or a keyboard logger.**
- **Traffic analysis.** The existence and timing of messages is not hidden.

## Encryption — please read this before relying on it

Conduit uses X25519 key agreement and XChaCha20-Poly1305, with HMAC-SHA256 for
message authentication. **This is not end-to-end encryption.** The desktop hub
decrypts each message and dispatches the inner payload, and a self-hosted relay
authenticates, verifies signatures on, and routes messages. Traffic is
protected from a passive third party on your local network, and from tampering
in transit. It is **not** protected from the desktop itself, nor from an operator
who controls the relay.

The app's status bar reports this honestly rather than claiming "E2E"; see
[ADR 0007](docs/decisions/0007-refuse-to-ship-end-to-end-encryption-claims.md).

## Certificate pinning

The relay's TLS certificate pin is the SHA-256 hash of the certificate's
SubjectPublicKeyInfo (SPKI), not of the whole certificate. SPKI is used so that
the pin survives certificate renewal as long as the key is reused. See
[ADR 0006](docs/decisions/0006-spki-pinning-not-whole-certificate.md) and
[docs/relay-tls.md](docs/relay-tls.md).

## Reporting a vulnerability

**There is no published security contact address yet.** If you have found a
vulnerability and want to report it privately, open a GitHub issue marked as a
draft and ask for a private reporting channel to be set up, or contact the
maintainer through the email address associated with their GitHub profile. Do
not open a public issue with working exploit detail in it.

Please include:

- what the issue is, and which component (desktop app, mobile app, relay, or the
  shared protocol crate)
- the version or commit you tested
- reproduction steps, ideally minimal
- the impact you believe it has, and who is affected
- whether it requires local network access, a paired device, or a relay

## Response

No response time is committed to, because this is a small project maintained in
spare time. What you can expect:

1. An acknowledgement that the report was read.
2. An assessment of severity, and whether it is accepted as a defect.
3. A fix, or a clear statement of why not, if you want to wait for one.
4. Credit in the release notes, if you want it.

If a report is declined, you will be told why. If a fix is needed before a
release, that will be prioritised over feature work.

## Disclosure

There is no formal disclosure policy yet. The intent is straightforward:
report privately, allow time for a fix, and publish details only after a fixed
version is available. Please do not publish working exploits before then.

## Security-relevant tests

The repository treats these as security controls and they are expected to keep
passing:

- `rce_chain_*` in `apps/desktop/src-tauri/src/server/mod.rs` — an
  unauthenticated client must not reach privileged operations
- `capability_file_grants_nothing_the_frontend_cannot_use` and
  `capability_file_does_not_reintroduce_the_dangerous_grants`
- `wrong_key_leaves_an_intact_database_untouched` — a credential-store failure
  must never destroy local data
- `v2_broadcast_skips_unpaired_and_keeps_paired` — an unpaired peer must receive
  nothing
- `e2e_client_cannot_forge_route_claiming_another_from_device_id`
- `spki_extraction_matches_openssl_byte_for_byte`

If you change any of these and they start failing, that is a finding, not a test
to update.
