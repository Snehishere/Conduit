# ADR-0003: A per-launch capability token identifies the local desktop

- **Status:** Provisional
- **Date:** 2026-09

## Context

`ws_to_device_id` is not merely a routing table — it *is* the identity store.
Handlers ask it who a connection is (`server/mod.rs:601`,
`server/mod.rs:956`, `server/mod.rs:1141`) and that answer decides whether the
`automation/rule`, `automation/triggered`, `screen_mirror` and `remote_input`
dispatch arms run at all.

Every loopback peer used to be inserted into that map at accept time. Because
"came from 127.0.0.1" was the whole test, **any process on the machine** was
authenticated as the trusted local desktop and could fire automation rules and
drive the screen and the keyboard. Combined with the unenforced command allowlist
([ADR-0002](0002-deny-by-default-shell-command-allowlist.md)) that was a
remote-code-execution chain reachable from an unprivileged local process, and
from a web page in any browser on the same host, because a browser can open a
WebSocket to `ws://127.0.0.1:9527` and set whatever headers it likes.

## Decision

A connection is granted an identity by presenting a **per-launch capability
token**, and nothing else. Not loopback membership, not a socket, not a header.

1. **43 alphanumeric characters, 256 bits.** Generated from
   `rand::distr::Alphanumeric` via `rand::rng()` (ChaCha12 CSPRNG),
   `security.rs:887-899`. 43 characters of a 62-symbol alphabet is
   `log2(62^43) ≈ 256` bits — not guessable within any plausible number of
   attempts.
2. **Regenerated every launch.** `LocalCapability::generate`
   (`security.rs:922-927`) is called from a `OnceLock` initialiser
   (`security.rs:949-955`). It exists only in this process's memory, so a leaked
   copy dies with the process. There is no persistence path that could resurrect
   an old token.
3. **Compared in constant time.** `constant_time_eq` (`security.rs:905-914`)
   accumulates the XOR difference across all bytes rather than returning early.
   Length is not hidden — the token is fixed-length by construction — but the
   contents do not leak through timing.
4. **Delivered by a `pairing` / `action: "local_auth"` frame.** The dispatcher
   routes that pair to `handle_local_auth`
   (`server/mod.rs:743-745`), which verifies the presented `token` and, only on
   success, calls `mark_local_desktop`
   (`handlers/pairing.rs:290-309`). A failure answers with an `error` frame whose
   code is `invalid_local_capability`. The message validator requires the token
   field and bounds its length (`security.rs:548-555`).
5. **Handed to the webview over Tauri IPC.** `get_local_ws_token`
   (`commands/settings.rs:250`) returns the token to the in-process webview only;
   a remote peer cannot reach the IPC surface.
6. **A connection is unpaired from the instant it is accepted.** The accept path
   explicitly logs and does nothing else for loopback peers
   (`server/mod.rs:377-389`). The unpaired window is harmless because broadcasts
   are filtered on identity, so nothing reaches the socket before the frame
   lands.
7. **Persisted to a `0600` file for diagnostics and for the webview, never
   consulted for authorisation.** `local_capability_file_path`
   (`security.rs:958-963`) and `persist_local_capability` (`security.rs:970-995`)
   write `local_ws_token` under the app data directory, owner read/write only on
   POSIX, truncated (never appended) on every launch. The authoritative copy is
   the in-process one. Reading the file grants nothing.

## Consequences

**Easier.** A browser page, a process in another OS account, and anything on the
LAN are all back to being anonymous until they present the token. The invariant
is one sentence long: *an identity is a token, not an address*.

**Unwelcome: the webview must now send the token, or it is unpaired against its
own server.** `useWebSocket.tsx:100` still does `new WebSocket(WS_URL)` with no
token and no `local_auth` frame, so as of this record the desktop's own webview
does **not** have an identity and the privileged dispatch arms are closed to it.
The command exists, the server arm exists, and the two-line frontend change that
consumes it does not. Tracked in
Still outstanding; see the project Limitations section. This is a real
first-breaks-then-works sequence, not a no-op.

**Unwelcome: residual exposure in a file and in the JS heap.** The token is
written to `local_ws_token` in the user's data directory and it lives in the
webview's JavaScript heap. A process running as the same user can read either.
That is not a defect in the design — it is the boundary. See
[ADR-0005](0005-same-user-local-access-is-not-a-boundary.md).

**Unwelcome: `local_desktop` has no shared secret, so everything it receives is
plaintext by necessity.** The local capability authenticates a *connection*, it
does not establish a session key, and no per-peer key is negotiated for the
loopback path. A local peer that is authorised sees exactly what the relay would
see. Accepting that is what makes the fix deployable; a key exchange here would
be a much larger protocol change with a much smaller threat-model payoff.

**Unwelcome.** A `0600` file is a POSIX concept. On Windows there is no per-file
mode, so the token file's protection rests on the per-user `%LOCALAPPDATA%` ACL.

**Unwelcome.** Rotating per launch means a crash-restart invalidates nothing
persistent — there is nothing to invalidate — but it also means an operator
cannot pre-provision a token for a headless or auto-started deployment. There is
no supported unattended-pairing story for the local socket.

## Alternatives considered

**Use the browser `Origin` header as the check for "is this the webview".**
Rejected, and this is the important rejection. A local process sets `Origin`
freely; `curl`, a script, or a page served from a local file all control it. A
check that a hostile local process can satisfy does not close the hole it was
introduced to close. It would have looked like a fix and left the threat exactly
where it was.

**Keep loopback trust but require a Tauri IPC handshake first.** Not possible:
IPC is reachable only from the webview, so it cannot be the credential a
*connection* presents over a socket.

**Issue a capability at pair time and persist it.** Rejected: a long-lived token
is a durable credential on disk, which is a strictly larger blast radius than a
per-launch one, and the threat being closed is *local process* access — a
persisted token is readable by exactly the attacker in question.

**Use a named pipe / Unix socket instead of TCP loopback.** Rejected as out of
scope: it is the correct end state (it removes the loopback surface entirely) but
it changes the transport for the mobile clients, the relay, the discovery
service, and the Playwright tests. Recorded here as the direction to take, not
the decision taken.

**Derive the token from a machine secret.** Rejected: a derived token is
reproducible, so it stops being per-launch. Randomness with no derivation input
is what makes "a leaked copy dies with the process" true.

## Why Provisional

The design is sound but the deployment is not finished: the webview half of the
handshake is not written, and there is no unattended-pairing story. This record
should be re-examined once the frontend consumes `get_local_ws_token`, and it
should be superseded outright if the transport moves off TCP loopback.
