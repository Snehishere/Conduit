# Architecture Decision Records

This directory records the decisions taken during the 2026-09 remediation of the
Conduit monorepo. Each file is one decision, numbered and never renumbered or
rewritten in place: a decision that is later replaced gets a new file, and the
old one is marked `Superseded by ADR-NNNN` with a link.

## Index

| ADR | Status | One-line summary |
|---|---|---|
| [0001](0001-single-cargo-lockfile-and-workspace-dependencies.md) | Accepted | Only the workspace-root `Cargo.lock` is authoritative; nested lockfiles are deleted and forbidden by `.gitignore`. |
| [0002](0002-deny-by-default-shell-command-allowlist.md) | Accepted | `CommandAllowlist` becomes real: a deny-by-default `allowed_commands` setting, one process-wide instance, enforced at every call site and at the dispatch gate. |
| [0003](0003-per-launch-local-capability-token.md) | Provisional | A 43-character, per-launch, in-memory capability token replaces "any loopback peer is trusted". |
| [0004](0004-domain-separated-relay-signing-key.md) | Superseded by ADR-0011 | The relay token is no longer the message-signing key; a labelled KDF derives a distinct key, `from_device_id` is signed, and a `key_id` rotation window is added. |
| [0005](0005-same-user-local-access-is-not-a-boundary.md) | Accepted | Same-user local access is explicitly **not** defended against; this ADR records the threat model, not a control. |
| [0006](0006-spki-pinning-not-whole-certificate.md) | Accepted | Pinning is SPKI on both sides, plus a `GET /pin` discovery endpoint; the documented workflow could never previously have matched. |
| [0007](0007-refuse-to-ship-end-to-end-encryption-claims.md) | Accepted | `MessageProtection` is a closed union of `'envelope' \| 'plaintext'`, so a future `'e2e'` claim is a compile error. |
| [0008](0008-keyring-fallback-over-hard-exit.md) | Accepted | A secret is never returned before a verified read-back; the keyring falls back to a `0600` key file instead of silently resetting the database. |
| [0009](0009-untrusted-fields-must-fail-closed.md) | Provisional | "Absent" and "failed to read" are different states (`Result<Option<T>>`), corrupt values fail closed, and SQL limits are clamped at the storage layer. |
| [0010](0010-schema-json-is-hand-maintained-and-test-enforced.md) | Accepted | `types.rs` is the single source of truth; `schema.json` is hand-maintained and pinned by five invariant tests. |
| [0011](0011-per-device-relay-route-keys.md) | Accepted | Every device signs relay routes with its own key derived from its pairing secret; the relay resolves keys through its host, and a re-pair is the rotation mechanism. |

## Format

Each record uses the same five sections, in this order:

- **Status** — `Accepted`, `Provisional`, or `Superseded by ADR-NNNN`.
  `Provisional` means the decision is real and in force, but is expected to be
  revisited once the open question named in the record is answered. Do not
  upgrade a record's status without changing what the code does.
- **Context** — the situation that forced a decision, with `path:line`
  references wherever a claim is checkable.
- **Decision** — what was decided, stated in the present tense as current fact.
- **Consequences** — what this makes easier *and* harder. Unwelcome
  consequences are labelled as such and are not to be quietly dropped; an ADR
  that lists only the benefits is not a record, it is a press release.
- **Alternatives considered** — each option and the concrete reason it was
  rejected. "Too complex" is only an acceptable reason when the complexity is
  named.

## Conventions

- **Every claim is checkable.** A statement that cannot be verified against the
  tree at the stated `path:line` does not belong in an ADR.
- **No marketing language.** "Robust", "seamless", "military-grade" and
  "best-in-class" are not evidence. Name the mechanism.
- **Records describe decisions, not instructions.** If a decision is later
  reversed, write a new record; do not edit the old one's Context.
- **Unfinished work is not hidden.** Where a decision is in force but a piece of
  it is not yet implemented, the record says so and links to
  see the Limitations section of the README. An ADR that implies more
  shipped than shipped is the same failure mode the 2026-09-26 audit was
  commissioned to fix.
- **Siblings, not duplicates.** `README.md`, `../DEVELOPMENT.md`,
  `../TESTING.md`, `../ARCHITECTURE.md`, `../SECURITY.md`, `../HANDOFF.md` and
  `../REMAINING_WORK.md` are owned elsewhere. Link to them; do not restate
  their content here.

## Scope note

These records cover the remediation work, not the whole product. Decisions
predating it (transport, persistence, protocol versioning) are described where
they are load-bearing for a remediation decision, but they have no record here
yet. Writing them is an open task; see `../REMAINING_WORK.md`.
