# ADR-0002: The shell-command allowlist is deny-by-default, and it is actually consulted

- **Status:** Accepted
- **Date:** 2026-09

## Context

`CommandAllowlist` (`apps/desktop/src-tauri/src/automation.rs:112`) shipped with
roughly forty unit tests and `#[allow(dead_code)]` markers on `new` and
`is_empty` (`automation.rs:118`, `automation.rs:163`). It had **zero** production
callers. `execute_action_with_allowlist` takes `Option<&CommandAllowlist>` and
skips enforcement entirely on `None` (`automation.rs:660`), and every production
site passed `None`:

- `main.rs:358` — the 30-second timer that fires time-triggered rules.
- `server/mod.rs:539` — the `automation/rule` dispatch arm.
- `server/mod.rs:1502` — the `automation/triggered` dispatch arm.

The type's own doc comment claimed "an empty allowlist means ALL shell commands
are BLOCKED (deny-by-default)" (`automation.rs:94`). That was true of the type
and false of the program. A peer that could open the loopback WebSocket — which
at the time was any process on the machine — could persist an
`automation_rules` row whose action was `run_shell_command`, and it would
execute. The rule fired, the command ran, and the allowlist's forty tests stayed
green throughout.

## Decision

The allowlist becomes a real, deny-by-default control.

1. **A setting, deny-by-default.** A new `allowed_commands` setting holds a JSON
   array of executable names. An empty list — which is what a fresh install has —
   blocks all shell execution. A missing row, an unparsable row and an explicit
   `[]` are all read as an empty list
   (`security.rs:1088-1096`, `commands/settings.rs:191-208`). A corrupt row can
   never widen the list.
2. **One process-wide instance.** A single `CommandAllowlist` lives in a
   `OnceLock<Arc<RwLock<…>>>` (`security.rs:1019`) and is initialised at startup
   from the setting (`security.rs:1036`). There is one value and one accessor
   (`security.rs:1054`), so "the allowlist exists" and "the allowlist is
   consulted" cannot drift apart. It is a `std::sync::RwLock` rather than a tokio
   lock because the guard is only ever held long enough to clone a `Vec<String>`
   and is never held across an `await`, which also lets startup initialise it
   with a plain synchronous call from `main`.
3. **Enforced at every call site.** All three `execute_action_with_allowlist`
   production call sites now pass `Some(..)` from that instance
   (`main.rs:358`, `server/mod.rs:539`, `server/mod.rs:1502`).
4. **Enforced again at the dispatch gate.** Before a rule is stored
   (`shell_rule_gate`, `server/mod.rs:1386`) and before an already-stored rule is
   fired (`triggered_rule_gate`, `server/mod.rs:1440`), the command is checked
   against the live allowlist and refused with a protocol `error` frame naming
   the reason. This is belt-and-braces on purpose: the execution path is
   checked even for rules persisted by a build from before the allowlist existed.
5. **Live, not restart-scoped.** `set_allowed_commands` writes the row and
   replaces the live allowlist in the same call (`commands/settings.rs:216-239`),
   so there is no window in which the persisted setting and the enforced list
   disagree. Entries are trimmed, de-duplicated and capped at
   `MAX_ALLOWED_COMMANDS = 256` (`security.rs:1063`).
6. **Stored in the settings KV table, behind its own commands, not as a
   `ConduitSettings` field.** See "Storage shape" below.
7. **The UI states the residual.** The Advanced settings panel says "Executable
   name only — arguments are not matched" and warns that `*` allows every
   command (`AdvancedSection.tsx:317-321`).

### Storage shape

`allowed_commands` is a row in the `settings` table read and written through
`get_allowed_commands` / `set_allowed_commands`
(`commands/settings.rs:189-239`), and is deliberately **not** a field on
`ConduitSettings`.

The reason is a test that already exists:
`frontend_settings_type_key_set_matches_conduit_settings`
(`commands/settings.rs:414-431`) parses
`apps/desktop/src/components/settings/settingsTypes.ts`, extracts the member
names of `export interface SettingsData`, and asserts the key set is exactly
equal to what `ConduitSettings` serialises to. A Rust-only field would have
turned that test red, and that file was owned by a different agent during the
remediation. Storing the value as a KV row keeps the cross-language contract
untouched while still making the setting persist and be editable.

This is a deferral, not a preference. The natural end state is a real
`ConduitSettings` field plus a matching TypeScript field, in one pass. Tracked in
see the Limitations section of the README.

## Consequences

**Easier.** A shipped allowlist no longer requires four independent call sites to
remember a parameter. The safety property is now a property of the process, not
of each caller's discipline. Rules asking for a non-allowlisted command are
refused at the boundary with a message that tells the user what to do
(`automation.rs:672-676`).

**Unwelcome, and important: the allowlist matches the executable name only.**
`is_allowed` extracts the first token and compares that against the entries
(`automation.rs:151-159`). Arguments are never inspected. So allowlisting `sh`,
`bash`, `cmd`, `powershell`, `python`, `perl`, `ruby`, `node` or `env` authorises
arbitrary code execution by definition — `sh -c <anything>` is what the
executor literally does (`automation.rs:681-685`). The metacharacter rejection
(`;` `|` `&` `$` backtick `>` `<` newline, `automation.rs:181-185`) prevents
`ls; rm -rf /` from being smuggled past an `ls` entry, but it does nothing to
constrain the arguments of an allowed interpreter. **There is no configuration
of this allowlist that is both useful and a real boundary against a hostile rule
author.** The allowlist raises the bar from "any command" to "one of the
executables you named"; it does not make a hostile rule author harmless.

The current UI copy states the executable-name-only rule and the `*` warning
(`AdvancedSection.tsx:317-321`) but does **not** yet name the interpreter
entries specifically. The copy must be extended to say that allowlisting a shell
or interpreter is equivalent to allowing arbitrary code; that change is
still outstanding; see the Limitations section of the README.

**Unwelcome.** The allowlist is a flat name list, so a legitimate rule that runs
`git status` requires the user to allowlist `git` — and therefore authorises
`git` for any arguments. There is no per-rule or per-argument granularity. The
`trusted_source_only` flag on a rule (`automation.rs:90`) restricts who may
*fire* a rule; it does not restrict what the rule may run.

**Unwelcome.** Deny-by-default is a behaviour change for any existing user with
automation rules containing shell commands: those rules stop working until the
user allowlists the executable. That is the intended outcome, and it will be
reported to users as a broken feature rather than a security fix.

**Unwelcome.** `allowed_commands` is now a third shape of persisted settings
alongside `ConduitSettings` and the ad-hoc `get_setting` / `save_setting` pairs.
Settings are no longer described by a single struct, and the cross-language key
set test only covers one of the two shapes.

## Alternatives considered

**Make `execute_action_with_allowlist` take a non-optional reference.** Rejected
as the primary fix: it would have forced the four sites to have an allowlist in
hand, but it does nothing about the *dispatch gate*, where a hostile rule can be
written long before anything tries to execute it. Persisting `ls; rm -rf /` and
refusing at execution time is worse than refusing at write time. The `Option` is
still there because a genuine desktop-initiated path may want it; there is no
such caller now.

**Delete `CommandAllowlist` and refuse all shell automation outright.** Rejected:
it would have been simpler and strictly safer, but it removes a documented
feature that some users rely on, and the feature is defensible when the user
knows exactly which executables they have authorised. The residual risk is
disclosed above rather than hidden.

**Filter the executable list in the frontend.** Rejected: the trust boundary is
the process, not the webview. A filter in the UI is advisory; the UI is the
component that can be replaced by anyone with filesystem access to the frontend
bundle.

**Store the allowlist as a `ConduitSettings` field.** Rejected for now, for the
reason in "Storage shape". This is a real trade, not an oversight, and it is
reversible: moving the value into the struct later requires no migration, since
it is already a settings-table row.

## Open items

- The Advanced settings copy must explicitly warn that allowlisting a shell or
  interpreter authorises arbitrary code. `AdvancedSection.tsx:317-321`.
- Promote `allowed_commands` into `ConduitSettings` together with a matching
  `settingsTypes.ts` field.
- CI has no check for a committed nested-`Cargo.lock` (see
  [ADR-0001](0001-single-cargo-lockfile-and-workspace-dependencies.md)).
