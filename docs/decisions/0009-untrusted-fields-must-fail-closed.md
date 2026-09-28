# ADR-0009: Untrusted and unresolvable fields must fail closed

- **Status:** Provisional
- **Date:** 2026-09

## Context

A recurring pattern ran through the settings and query layer: **a read failure
resolved to the permissive default.** Three instances, of increasing severity.

1. **`auto_accept_files` defaulted to accept.** An inbound file request from any
   authenticated peer was accepted unless the user had explicitly turned the
   setting off (`server/handlers/files.rs:276-283`). Accept-by-default for
   inbound file writes is a privacy decision made by a missing row.

2. **`get_setting` collapsed two different states into one.** It returned
   `Option<String>` from a three-link `.ok()` chain, so `QueryReturnedNoRows`
   ("the user never set this") and a real SQL error — locked database, I/O fault,
   corrupt page — produced the same `None` (`storage.rs:1055-1068` documents the
   old code). Every caller that treats `None` as "use the default" then silently
   did the wrong thing, and because `get_settings` writes the whole struct back
   on save, **the next save wrote the default over the user's real value.** A
   transient SQL error destroyed a preference.

3. **A negative `LIMIT` meant "no limit".** SQLite reads a negative `LIMIT` as
   unlimited, so an unclamped limit — and these arrive from JavaScript and from
   the LAN protocol — turned a bounded query into a silent full-table dump.

The common shape: a value that could not be obtained was indistinguishable from a
value that did not exist, and the code picked the convenient branch.

## Decision

1. **"Absent" and "failed to read" are different states.** The fallible read is
   `Storage::try_get_setting(&str) -> Result<Option<String>>`
   (`storage.rs:1048`), where `Ok(None)` means *only* "the key has never been
   written". Any SQL error is `Err`.
2. **The fallible read is the one to use for anything security- or privacy-
   relevant.** `Storage::get_setting` is kept for the existing call sites — its
   `Option` return shape is unchanged so no caller has to change at once — but it
   now logs loudly instead of vanishing (`storage.rs:1069-1079`), and its doc
   comment says explicitly to prefer `try_get_setting`. (That doc comment's own
   justification for keeping the shape — that `main.rs`'s `generate_handler!`
   list and the protocol dispatch depend on it — is inaccurate:
   `get_setting` is a `Storage` method, not a Tauri command, and appears in
   neither list. The shape is kept for call-site compatibility, not because
   anything requires it.) `Storage::get_settings` propagates the failure rather
   than returning defaults (`setting_or_default` returns `Result`;
   `storage.rs:975-1030`).
3. **A present-but-unusable value fails closed.** `setting_string_list`
   (`server/mod.rs:1199-1204`) treats a *missing* row as the documented default
   and a *corrupt* row as an empty list — an allowlist fails closed, so an
   empty `notification_apps` mirrors nothing
   (`server/mod.rs:1327-1334`, and the test at `server/mod.rs:1730-1747`).
   `parse_allowed_commands_setting` (`security.rs:1088-1096`) does the same for
   the command allowlist: missing, unparsable and explicitly-emptied all mean
   "block everything".
4. **Query limits are clamped at the storage layer.** `clamp_limit`
   (`storage.rs:20-23`) maps any `i64` into `1..=MAX_QUERY_LIMIT` (1000), where
   0 and every negative value collapse to 1. It is applied to all five
   call sites — clipboard, notifications, file transfers, automation logs
   (`storage.rs:824`, `:894`, `:1303`, `:1332`) — as defence in depth behind the
   per-handler bounds check. The test names the trap explicitly:
   `clamp_limit(-1) == 1, "LIMIT -1 means UNLIMITED in SQLite"`
   (`storage.rs:3054-3065`).

## Consequences

**Easier.** A transient database error can no longer be mistaken for "the user
never configured this", so a locked database no longer silently resets
preferences on the next save. A corrupt allowlist cannot widen. A hostile or
careless limit cannot dump a table. The clamps are in one function, so the rule
is stated once.

**Unwelcome, and the reason this record is Provisional: a missing row still gets
the permissive default on the two paths that matter most.**
`auto_accept_files_enabled` (`server/handlers/files.rs:279`) reads through
`get_setting`, so a *hard SQL failure* still resolves to `None`, which maps to
`true` — accept. And `setting_enabled` (`server/mod.rs:1187-1192`) has the same
shape. The fail-closed property established here is for **corrupt and unparsable
values**, not for **failed reads on the WebSocket path**. Closing that last gap
means moving those two helpers onto `try_get_setting` and deciding what a failed
read means for each gate — a real decision, not a mechanical change, because
"the database is locked, therefore accept every inbound file" and "the database
is locked, therefore accept nothing" are both defensible and have different
consequences. Tracked in
See the project Limitations section.

**Unwelcome.** `auto_accept_files` still *defaults to accept on a fresh
install* (`storage.rs:1020`; the behaviour is pinned by
`file_request_valid_accepts_when_auto_accept_files_unset`,
`server/handlers/files.rs:296-314`). Changing that to deny-by-default would be
more defensible, but it changes behaviour for every existing user who never
touched the setting, and it is a product decision rather than a bug fix. It was
deliberately left as-is and is called out here so it is not mistaken for an
oversight.

**Unwelcome.** `get_settings` deliberately keeps its *parse* fallbacks —
`.parse().unwrap_or(5)`, `serde_json::from_str(…).unwrap_or_default()`
(`storage.rs:996-1009`). Those handle a value that is present but unusable, and
the contract is pinned by an existing test
(`save_settings_field_edge_non_numeric_max_devices_silently_falls_back_to_default`).
Only *query* failures are surfaced. A reader may reasonably think that is
inconsistent; the comment at `storage.rs:984-989` explains why.

**Unwelcome.** `MAX_QUERY_LIMIT = 1000` is a policy constant with no
configuration. A user with more than 1000 clipboard entries cannot page past
1000 in one query, and the callers have to do the paging.

**Unwelcome.** Clamping to 1 rather than to a default means a caller that
computes `limit - 1` for "all but the last" silently gets one row. That is
intentional — fail closed, and the smallest closed value — but it is a silent
result, not an error.

## Alternatives considered

**Make every read fallible and propagate errors to the UI.** Rejected as the
whole fix: it does not answer what a *failed* read should mean. A gate that
returns an error is not a gate. The distinction between "read failed" and "not
configured" has to be carried into the decision, not thrown away by the type.

**Treat a failed read as "not configured" everywhere, and document it.**
Rejected: that is the bug. It is indistinguishable from a missing row precisely
because the type says so.

**Treat a failed read as "deny" everywhere.** Rejected as a blanket rule: a
locked database would then refuse all inbound clipboard and notification syncing
for as long as the lock lasts, which is a self-inflicted outage on a
correctness fix. Fail closed should be decided per gate, not applied uniformly.

**Validate limits only in the WebSocket handlers.** Rejected: the storage layer
is the chokepoint every caller passes through, and the second layer costs three
lines. Handler-only validation is exactly the omission that let the trap through
in the first place.

**Reject a negative limit with an error instead of clamping.** Rejected: it turns
a bad caller into a failure visible to the user instead of a bounded, correct
result. Clamping keeps the read working and keeps the table private.

**Reuse a `NonZeroU32` / `NonNegative` newtype for limits.** Rejected: it would
catch negatives at the type level but not at the boundary where a value crosses
from JSON, and it would not protect a caller that deliberately passes `-1`. The
clamp is the check that actually holds.

## Why Provisional

The absent/failed distinction and the clamping are settled. Whether the two
remaining permissive-on-failure gates should flip is not, and this record should
be revisited when that decision is made rather than treated as closed.
