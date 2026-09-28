# ADR-0007: Refuse to ship end-to-end encryption claims the code cannot support

- **Status:** Accepted
- **Date:** 2026-09

## Context

The desktop frontend was handed a bare identifier, `encryptionEnabled`, which was
always truthy, and rendered an "E2E" status indicator unconditionally. Nothing
backed the claim. `sendEncrypted` had zero call sites, so every payload left in
the clear. Meanwhile `Onboarding.tsx` told the user, twice, that everything is
end-to-end encrypted (`Onboarding.tsx:17`, `Onboarding.tsx:31`).

Investigating the claim rather than the symptom produced a more important
finding: **the encrypted path is hop encryption and could not be end-to-end as
designed.** Verified against the current code:

- **The relay terminates the envelope.** An inbound `encrypted` frame is
  decrypted in the desktop process and the inner JSON is what gets dispatched
  (`server/mod.rs:595-620`). The shared secret is a per-pair secret held by this
  process. An envelope is therefore encryption between the app and the relay
  process, never a channel between two devices.
- **The outbound wrap is best-effort.** `seal_for_peer`
  (`server/mod.rs:321-349`) returns the frame **verbatim** when the peer has no
  shared secret (`:325-333`), and `send_encrypted_message` logs a warning and
  still resolves `Ok(())` when the target is not connected
  (`commands/pairing.rs:236-247`) — a payload reported as sent that never left.
- **The desktop's own webview is plaintext by necessity.** `local_desktop` has no
  key pair, so it is not in the sync engine and never receives a sealed frame
  (`server/mod.rs:316-320`).
- **There is no inbound decrypt in the Tauri surface.** `main.rs:230` registers
  `send_encrypted_message`; there is no `decrypt_message`. The frontend can
  therefore not open an envelope it receives, and every inbound sensitive
  payload is handled in the clear.
- **File bytes are base64 in plaintext JSON.** `commands/file.rs:60-76` routes
  each frame to exactly one peer, and that is a real fix, but the payload is
  unencrypted and the only integrity check is a SHA-256 `checksum` field.

Two further defects found during the investigation have since been fixed and are
noted so the record is not read as claiming they are still open: the outbound
path used to double-encrypt into `encrypted(encrypted(…))` which the peer dropped
after peeling one layer (`server/mod.rs:307-314` and its regression test at
`:2444-2446`), and `send_to` used to look a device id up in a map keyed by
per-connection UUID, so the send missed the local client table
(`server/mod.rs:936-973`).

The conclusion is unchanged: **no message type in this build is end-to-end
encrypted.** The only honest options were to build real end-to-end encryption —
a key exchange between devices, an envelope the relay cannot open, a new inbound
decrypt command — which is a protocol revision and not a fix, or to stop claiming
it.

## Decision

Ship the weaker, true claim, and make the false claim unrepresentable.

1. **`MessageProtection` is a closed TypeScript union:**
   `export type MessageProtection = 'envelope' | 'plaintext'`
   (`apps/desktop/src/hooks/useEncryption.ts:46`).
   **A comparison to `'e2e'` is a compile error.** The original lie cannot be
   reintroduced without `tsc` failing, which is the point: the guarantee is
   enforced by the type system rather than by remembering.
2. **`MESSAGE_PROTECTION` is a per-type table with a cited reason for each row**
   (`useEncryption.ts:58+`). Every entry names the code that decides it. Adding a
   message type without a row is a visible omission.
3. **The runtime counters record what was observed leaving the app in this
   session**, so the status bar reports a measurement rather than an assumption
   (`useEncryption.ts:255`, `buildStatus`).
4. **`sendSensitive` is the single entry point for sensitive payloads.** It tries
   the envelope path, and on failure records the type as having left in the
   clear, warns once, and returns `encrypted: false` so the caller decides. The
   downgrade is *visible* rather than silent
   (`useEncryption.ts:293-312`). `sendEncrypted` returns a `SendResult` with a
   reason instead of a bare boolean, so the failure reason reaches the caller
   (`useEncryption.ts:265-283`).
5. **The module carries the evidence inline.** The comment block at
   `useEncryption.ts:11-43` is the audit, with `path:line` for every claim, so
   the next reader does not have to re-derive it. **Its line numbers have since
   gone stale** — the double-encryption and address-lookup defects it cites were
   fixed and the code moved — so the table's `reason` fields should be
   re-verified against the current source before being trusted. The closed union
   is the durable part of this decision; the comment block is not.

## Consequences

**Easier.** The UI can no longer lie by accident. When real end-to-end encryption
is built, adding `'e2e'` to the union is a one-line change that immediately
surfaces every place that has to learn to handle it. The status bar now shows a
measurement, so a regression in the encrypted path is visible during a session
rather than in a bug report.

**Unwelcome, and the point of the record: the product now advertises weaker
protection than its marketing implied.** That is the correct outcome, but it is
a real cost — users who chose Conduit for an E2E claim chose it for a reason, and
they are owed a clear statement rather than a quietly weaker status bar. That
communication is not this record's job.

**Unwelcome: `Onboarding.tsx` still contains the false claim.**
`Onboarding.tsx:17` — "with end-to-end encryption" — and `Onboarding.tsx:31` —
"All communication is end-to-end encrypted. We don't store your data on any
servers — it's pure peer-to-peer." Both are wrong today, in a component that is
reachable (`App.tsx:410` renders it when `conduit_onboarded` is unset). Fixing
it requires editing a file another agent owns; it is tracked in
Still outstanding; see the project Limitations section. Until it is fixed, the first-run
experience contradicts the status bar.

**Unwelcome.** `sms` and `file` are recorded as `'plaintext'` in the table. Those
are not oversights to be quietly upgraded later; they are the accurate answer,
and the `reason` field on each row cites the broadcast that causes them.

**Unwelcome.** `sendSensitive` warns once per distinct reason. A user on a
misconfigured relay is told their SMS went out in the clear, once, and then
silently thereafter for the rest of the session. That is a deliberate noise
trade and it is arguable.

## Alternatives considered

**Fix the encryption path and keep the E2E claim.** Rejected for this change, not
in principle. It requires a key exchange between devices, a change to the
envelope format, removal of the relay's decryption, a new inbound decrypt
command, and a fix to the UUID/device-id address lookup. That is a protocol
revision, and it cannot be done credibly in the same change that discovered the
problem. The honest label costs one line; the dishonest label cost every user
who relied on it.

**Remove the encrypted path entirely and delete `sendEncrypted`.** Rejected: the
hop encryption is real encryption and it does reduce exposure. Deleting working
protection to avoid having to describe it is not a trade, it is damage.

**Keep `MessageProtection` as an open `string`.** Rejected: that is the lie with
extra steps. The whole value of the decision is that `'e2e'` is a type error.

**Model the real state as a richer enum (`'e2e' | 'hop' | 'none' | 'unknown'`)
with the unavailable values present but unused.** Rejected: a union member that
is never produced is an invitation. Two members, both produced, is a smaller
surface to get wrong.

## Related

- The relay's key and signing separation: [ADR-0004](0004-domain-separated-relay-signing-key.md).
- The threat model this claim sits inside: `docs/SECURITY.md`.
