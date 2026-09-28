# ADR-0005: Same-user local access is not a boundary

- **Status:** Accepted
- **Date:** 2026-09

## Context

The remediation added a per-launch capability token to the local WebSocket
([ADR-0003](0003-per-launch-local-capability-token.md)), an enforced
shell-command allowlist ([ADR-0002](0002-deny-by-default-shell-command-allowlist.md)),
and a derived relay signing key
([ADR-0004](0004-domain-separated-relay-signing-key.md)). It is easy to read
that work as a claim that the desktop app is now hardened against local attack.
It is not, and this record exists so that a future reader does not mistake the
absence of certain defences for an oversight.

## Decision

**A process running as the same OS user as the Conduit app is inside the trust
boundary. Conduit does not defend against it. This is a deliberate non-goal.**

Concretely, and by design, the following all work today for any process running
as the user:

- Reading the encrypted SQLite database file directly. `storage.rs` has no
  protection against a same-user reader of `conduit.db`; SQLCipher protects the
  file at rest against *other accounts* and against offline recovery, not against
  code that runs as the owner and can also read the key material (which the
  keyring fallback deliberately makes easy — see
  [ADR-0008](0008-keyring-fallback-over-hard-exit.md)).
- Calling the Tauri IPC command `get_local_ws_token`
  (`commands/settings.rs:250`) and reading the capability token out of it.
- Writing the `automation_rules` table directly, producing a rule that fires on
  the next minute boundary via the time trigger (`main.rs:330-382`).
- Reading the `0600` `local_ws_token` file (`security.rs:958-963`).

**None of that is defended against, and none of it should be.** Anyone able to run
code as the user could have written the automation rule directly, edited the
database, or read the clipboard sync out of memory. The automation engine is not
a privilege boundary within the user's own session; it is a feature.

What the hardening *does* remove is a specific set of boundaries:

| Removed boundary | Closed by |
|---|---|
| A web page in any browser, on any origin, reaching the desktop's local WebSocket | [ADR-0003](0003-per-launch-local-capability-token.md) — the token is not in a web page's reach |
| A process on another machine, including one on the LAN, reaching the local socket | [ADR-0003](0003-per-launch-local-capability-token.md) — loopback membership is no longer an identity |
| A process in another OS account on the same host | [ADR-0003](0003-per-launch-local-capability-token.md) and [ADR-0008](0008-keyring-fallback-over-hard-exit.md) — `0600` and the per-user ACL |
| A LAN peer with no pairing token, forging traffic as another device | [ADR-0004](0004-domain-separated-relay-signing-key.md) |
| Any peer, running any shell command at all, through an automation rule | [ADR-0002](0002-deny-by-default-shell-command-allowlist.md) — deny-by-default, so a hostile rule author is confined to the executables the user named |

## Consequences

**Easier.** The threat model is honest and short enough to state in one breath.
Nobody has to reason about what "partially protected against a local attacker"
means, because the answer is: nothing, and that is the design.

**Unwelcome.** The allowlist and the capability token are not a defence against
malware. A keylogger, a same-user trojan, or a malicious Rust extension in the
user's own build is unaffected by all of it. A reader who assumes otherwise will
make bad decisions about what to install alongside Conduit.

**Unwelcome.** `get_local_ws_token` is reachable over IPC by anything running in
the user's session. That is why the token's value is bounded: it authorises a
*local socket connection*, and the party that could steal it is already able to
do everything the socket can do directly.

**Unwelcome.** Anyone arguing for a defence here has to argue for OS-level
separation — a different service account, a per-user keyring entry the app
cannot read, a sandboxed broker process. That is a real design, and it is a
different product. It is not this one.

## Alternatives considered

**Same-user process as a boundary: enforce per-rule provenance.** Rejected:
provenance is only meaningful if the attacker cannot write the database, and they
can. A signed-rule scheme would need a key the same-user attacker does not have,
which is the OS keyring, and the keyring is explicitly allowed to be unavailable
([ADR-0008](0008-keyring-fallback-over-hard-exit.md)) — so the same-user
attacker would be able to mint keys too. The scheme would be security theatre
with extra moving parts.

**Same-user process as a boundary: refuse to run automation rules the local
webview did not author.** Rejected: the same-user attacker can drive the webview
too, and this would break the phone-pushed-rule workflow that the feature exists
for.

**Document the boundary and move on.** That is this decision.

## How to tell if this record is wrong

If a future change makes the local socket unreachable from same-user code — for
example by moving the local transport to a per-user named pipe with an OS-enforced
peer credential, or by running the backend under a separate service account —
then this record is superseded and the allowlist and capability token should be
re-evaluated against a boundary that actually exists. Until then, any document
that describes Conduit as protected from local malware is wrong, and
`docs/SECURITY.md` should be corrected rather than this record relaxed.
