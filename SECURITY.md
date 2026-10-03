# Security Policy

## Scope

Conduit is a local-first, peer-to-peer app. It links a phone to a desktop over
your local network, and the desktop hosts a relay itself for when the two
devices are not on the same network. There is no relay for you to deploy, no
cloud service, no account system, and no telemetry.

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
- **Forged relay messages.** Each device signs its routes with its **own** key,
  derived from the pairing secret it already shares with the desktop by a
  labelled KDF, and `key_id` must equal `from_device_id` — so a device can sign
  for itself and for nothing else. The relay holds no keys: it asks its host,
  which resolves them from the pairing registry. A re-pair rotates the key, so
  there is no rotation window and no window in which two keys are both accepted.
  See [ADR 0004](docs/decisions/0004-domain-separated-relay-signing-key.md) and
  its successor,
  [ADR 0011](docs/decisions/0011-per-device-relay-route-keys.md).

What this project explicitly does **not** defend against, by design:

- **Code running as your own user account.** A process that can run as you can
  read the app's database, read its config, or write to it. Conduit is not a
  sandbox and does not attempt to be one. See
  [ADR 0005](docs/decisions/0005-same-user-local-access-is-not-a-boundary.md).
- **A compromised operating system, or a keyboard logger.**
- **Traffic analysis.** The existence and timing of messages is not hidden.
- **Disclosure to anyone who reads the pairing code.** A pairing code — the QR
  image, or the six characters a person reads aloud — is a bearer credential for
  the whole relationship, and it is worth being blunt about what that now yields.
  A peer that presents it becomes **a paired device**: trusted for every message
  type the hub serves, including the whole automation surface and every user-facing
  settings gate, and able to receive everything a paired device receives.
  It also learns the **relay bearer token**, because the desktop delivers the
  relay's address, token and certificate pin in `pairing/accept` — that delivery
  is strictly downstream of a successful pairing, which requires a valid,
  unexpired, single-use token and consumes it, so the token cannot be harvested
  by guessing. The relay token matters because it is the way to reach the hub
  from outside the local network.

  This widening is inherent to hosting the relay in the same app that pairs
  devices, and it is bounded in three specific ways:

  - The relay token authenticates a **connection**, not a sender. Routing a frame
    additionally requires a signature under the **sender's own route key**,
    derived from that device's pairing secret and bound to its device id, so a
    token holder cannot route as anyone else and cannot forge a frame as the
    desktop. See [ADR 0011](docs/decisions/0011-per-device-relay-route-keys.md).
  - A peer that reads the code and does not complete pairing is told nothing; the
    token is delivered on the accept, not on the request.
  - `relay_url` is always the relay's **TLS** listener and is never a loopback
    address — a loopback host is refused outright and the field is omitted rather
    than filled with a syntactically valid value that names the phone from the
    phone.

  If the pairing code is exposed, the remedy is to unpair the device from the
  desktop (Settings → General). That deletes its `devices` row and calls
  `DeviceRouteKeys::forget` synchronously, which is what both halves of its access
  rest on: with no row it is not in the pairing registry, so it is neither a relay
  fan-out recipient nor a trusted peer from that moment, and its route key is
  dropped from the hub's copy immediately — the relay drops it from its own copy
  at its next refresh, and answers `unknown_device` rather than routing for it.
  With no row there is also no stored shared secret, so nothing it was sent is
  decryptable. Rotating the relay token alone would not help against a peer that
  already paired — which is why unpairing is the right primitive here and not
  token rotation.

  Protocol-level detail is in
  [PROTOCOL.md §4.2](packages/protocol/PROTOCOL.md) under *The relay fields*,
  and the egress that gives a paired device its reach is described below.

## Broadcast scope

The desktop sends **every broadcast it originates to every paired device in its
registry** — not only to the devices that happen to be connected to it over the
LAN right now.

That is a real change in exposure and it is worth stating rather than leaving to
be discovered:

- **The recipient set is the pairing registry, not the set of live sockets.** A
  phone that paired on the LAN and then walked out of range is still a recipient.
  "It is not on my network at the moment" no longer implies "it does not get my
  clipboard, my notifications, my SMS or my call state".
- **Each recipient's copy is sealed for that recipient**, with that device's own
  pairing secret, inside its own signed route. The relay forwards opaque
  envelopes and holds no key material, so it cannot read any of them. The hub
  still terminates each one — this is the hop encryption described below, not
  end-to-end encryption, and the hub is a party to every clipboard body and every
  SMS it forwards.
- **What does not change:** no *kind* of data became reachable that a paired
  device could not already obtain by asking. A paired device could always address
  the hub directly. Fan-out changes who is *told*, not what is *knowable*.
- **A device that is not `paired` is not a recipient.** The recipient set comes
  from the pairing registry, and a row whose status is anything else — revoked, or
  deleted entirely — is not routable at all, so withdrawing a device genuinely
  withdraws it from every future broadcast. Unpairing takes effect **synchronously**
  (`WsServer::disconnect_client` calls `DeviceRouteKeys::forget` directly rather
  than waiting for the registry's 5 s refresh), because the same registry is the
  fallback the auth gate reads — so a revoked device is not merely unroutable, it
  is untrusted, from the moment the revoke lands. The relay keeps its **own** copy
  of the key set and refreshes that on its own schedule, so for up to 5 s it will
  still accept a route signed by a device this hub has already revoked.
- **No device is served twice.** A device that is on the LAN *and* joined to the
  relay — which is what a network transition looks like — is served by one path
  only. Duplicated notifications are a nuisance; duplicated file chunks corrupt a
  transfer.
- **The hub gained a pre-auth-adjacent egress.** Routing a *refusal* back to a
  peer that has no local socket means the desktop will sign and encrypt on behalf
  of an id it is otherwise refusing. That path is gated on the same trust
  predicate as the authentication gate, so an id that is not a paired device gets
  no signed route — otherwise "you are not paired" would be a cheap way to make
  the hub mint one per refused frame.

## Encryption — please read this before relying on it

Conduit uses X25519 key agreement and XChaCha20-Poly1305, with HMAC-SHA256 for
message authentication. **This is not end-to-end encryption.** The desktop hub
decrypts each message and dispatches the inner payload, and the relay
authenticates, verifies signatures on, and routes messages. Traffic is
protected from a passive third party on your local network, and from tampering
in transit. It is **not** protected from the desktop itself.

The threat model changed when the relay moved in-process, and it changed in both
directions, so both halves are worth stating:

- **There is no third-party relay operator any more.** The relay runs in the
  same process as the hub, on the desktop's own machine, as a background task
  (`apps/desktop/src-tauri/src/relay.rs`). There is no separate deployment, no
  operator, and therefore no operator who can read the relay's traffic or swap
  its certificate out from under you. The bearer token, the route keys and the
  TLS certificate all live in the app's own keyring and app data directory,
  generated on your machine and never configured by hand.
- **There is also no process isolation from the hub.** The old model at least
  had the relay in a different process, possibly on a different machine, which
  meant a compromise of one did not automatically give you the other. Now a
  compromise of the desktop process is a compromise of both at once. That is
  the same boundary as [ADR 0005](docs/decisions/0005-same-user-local-access-is-not-a-boundary.md)
  says for everything else: anything running as your own user can read the
  relay's state too.

What the relay still cannot do is read the payloads: it forwards opaque
`encrypted` envelopes and has no key material of its own, only a reference to
its host's key resolver.

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

Added with the relay fan-out, because each of these is a control that the change
could have removed:

- `a_relay_only_frame_is_sealed_and_never_routed_in_the_clear` — a frame routed
  to a device with **no local socket** must still be sealed for that device. This
  is the one that matters most: the recipient set is the pairing registry, so the
  secrets it needs are not in the map the egress used to read them from, and the
  failure mode is cleartext.
- `the_fan_out_recipient_set_is_exactly_the_routable_devices` — the recipient set
  is the routing table and nothing else, so an unpaired device, a revoked one and
  an id a peer made up are all unnameable. It replaced an earlier test that
  asserted the `refresh()` `status != "paired"` filter, which `relay.rs`'s own
  `an_unpaired_or_revoked_device_cannot_route` already pins against the real
  registry; restating it in the fan-out suite tested the fixture.
- `an_oversized_broadcast_is_refused_before_it_can_kill_the_relay_leg` — a paired
  peer can pad `sms`, `call`, `clipboard` or a `file` control frame to any size,
  because `validate_message` closes no field set for those types. Sealing
  hex-encodes the payload, so an oversized frame crosses the relay's 1 MiB read
  ceiling — and crossing that ceiling **closes the connection** rather than
  dropping one frame, taking every relay-only peer offline for a reconnect
  backoff. The fan-out refuses anything over 256 KiB. Closing the field sets is
  the better fix and is still open (`REMAINING_WORK.md` W3.31).
- `a_refusal_for_an_unknown_id_is_not_given_a_signed_relay_route` — routing a
  refusal must not become a pre-auth signing oracle. `send_error` is reachable
  from `not_authenticated`, so without this gate a stranger could make the hub
  mint a signed, encrypted route per refused frame by sending junk.
- `the_desktop_is_never_its_own_relay_fan_out_recipient` and
  `a_device_with_both_a_socket_and_a_relay_route_is_served_once` — the first is
  an amplification loop (the relay delivers a self-addressed route straight
  back and it fans out again, multiplicatively, until the queue fills); the
  second is duplicate delivery of file chunks, which corrupts a transfer.
- `one_relay_peer_exhausting_its_budget_does_not_throttle_another` — relayed
  frames are metered per authenticated sender. Before this the path was not
  metered at all, so this asserts both halves of the fix.
- `a_relayed_binary_frame_is_charged_to_the_transport_budget` — a relayed v2
  frame names its recipient and not its sender, so the transport budget is the
  only one available to it; it previously had none. This one is a **guard, not a
  regression test**: the charge happens inside the relay read loop, that loop
  needs a live WebSocket, so the accounting was extracted into
  `admit_relay_binary` to be reachable — and a test on the extracted function
  passes whether or not the loop calls it. It is listed here because the property
  is a security control, not because it would catch the call site being deleted.
- `answer_channel_still_answers_sockets_only` — pins that the socket lookup was
  deliberately *not* widened. "This map cannot reach a relay-only device" is
  still true and is not the bug; routing around it is.

Two of the controls above have **no** test, and both are named in
`REMAINING_WORK.md`: `DeviceRouteKeys::forget`, which makes a revoke take effect
immediately instead of at the next 5 s registry refresh (W3.29 — with the
`peer_secret` fallback, the stale window was a trust window, not a routing one),
and the order in which `refresh` publishes its two maps, `secrets` before `keys`,
which is what keeps a concurrent reader from seeing a routable device whose secret
is not yet published (W3.30).

If you change any of these and they start failing, that is a finding, not a test
to update.
