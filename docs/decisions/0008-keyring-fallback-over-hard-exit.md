# ADR-0008: Fall back to a key file rather than exiting when the keyring is unavailable

- **Status:** Accepted
- **Date:** 2026-09

## Context

Two bugs compounded into unbounded, silent data loss.

**First:** `Storage::new` read the SQLCipher key from the OS keyring. On a read
failure it generated a *new* key and then **discarded the result of
`set_password`** (`apps/desktop/src-tauri/src/storage.rs:660-668` describes the
old code verbatim). The discarded write is the whole bug: the app is now holding
a key that exists nowhere.

**Second:** `open_or_recover` renamed the database to `corrupt_<ts>.bak` and
created a fresh, empty one on *any* open failure (`storage.rs:588-646`). A
SQLCipher database opened with the wrong key fails to decrypt **exactly the way a
corrupt file does**, so a lost key was indistinguishable from corruption, and the
recovery path destroyed the user's data every time.

Combined: on any Linux host with no running Secret Service — headless servers,
minimal containers, most CI images, and this repository's own
`docker-compose.yml` — `keyring` 4.2's `zbus-secret-service` backend returns
`PlatformFailure` for *every* operation including writes
(`encryption.rs:19-25`). A new key was minted every launch, the user's database
was renamed to `.bak`, and an empty one was created. Every launch. Settings,
paired devices, notifications and clipboard history: gone, repeatedly, with no
error shown.

`set_password` was also hard-coded to `keyring::Entry` and its result discarded,
which is *why the path was untestable* — there was no seam to fake.

## Decision

Four rules, together.

1. **A secret is never returned to a caller until it has been read back from a
   durable store.** `KeyStore` is a trait with `load`, `store` and `confirm`
   (`encryption.rs:78-160`). `resolve_secret` writes, then `confirm`s by reading
   back, and only then returns the secret. A keyring that reports success but
   holds nothing — the lying-writes case, which `set_password` returning `Ok`
   makes real — is caught here. The rule is stated as the module's contract at
   `encryption.rs:33-38`.
2. **The database file is classified before any destructive action.**
   `DbFileKind` is `Empty | PlainSqlite | SqlCipher | Unrecognised`
   (`storage.rs:134-138`), determined from the header bytes
   (`classify_db_file`, `storage.rs:178-187`). `open_or_recover` then branches:
   - `SqlCipher` / `PlainSqlite` — the file is **intact**; the cause is the key,
     not the data. The file is left untouched and a specific error is returned
     naming both recovery sources (`storage.rs:611-625`). **An intact file is
     never moved.**
   - `Empty` — nothing to lose; recreate.
   - `Unrecognised` — genuinely unreadable; quarantine.
3. **`.bak` files are capped at three.** `MAX_QUARANTINE_BACKUPS`
   (`storage.rs:191`) prunes the oldest `conduit.corrupt_<ts>.bak` files, and
   only those — the pruner matches the prefix and a numeric stamp and leaves
   anything else alone (`storage.rs:241-255`).
4. **When the keyring is unavailable the app falls back to a `0600` key file
   rather than exiting.** Only a failure to write *that* is fatal
   (`encryption.rs:320-403`).
5. **The fallback is self-healing.** `resolve_secret` reads the key file first
   (`encryption.rs:281-303`), which is what makes the fallback idempotent: on a
   machine that has previously fallen back, the file is used, so the key is
   stable across launches. When the keyring becomes reachable again,
   `migrate_key_file_into_keyring` (`encryption.rs:413-483`) writes the secret
   into the keyring, verifies it, and **only then deletes the key file**. If the
   keyring holds a *different* secret, the file wins and the keyring entry is
   left alone for investigation (`encryption.rs:913-928`).

The old `set_password` is replaced rather than merely checked:
`encryption.rs:80-102` explains that the trait seam exists specifically so the
"keyring unavailable" paths are exercisable by tests, and the suite covers the
lying-writes case (`keyring_that_ignores_writes_falls_back_to_the_key_file`,
`encryption.rs:827-837`) and the key-stability case
(`key_is_stable_across_launches_while_the_keyring_is_broken`,
`encryption.rs:839-854`).

## Consequences

**Easier.** The data-loss chain is broken at both ends. A lost key produces a
specific, actionable error that tells the user their data is intact and where
the old key is. The pathological host — a Linux box with no Secret Service — now
works, with a stable key, and migrates itself into the keyring when one appears.
The bug is now testable, which is the precondition for it not coming back.

**Unwelcome, and this is the trade, stated plainly: this is
availability-over-confidentiality.** The reduced protection is real:

- The key file is a **plaintext** file in the user's own profile directory. It
  is readable by **any process running as the same user**, including malware.
- On **Windows** there is no POSIX mode. There is no per-file DACL; protection
  rests on the per-user `%LOCALAPPDATA%` ACL (`encryption.rs:70-76`). Anything
  that can read the user's profile can read the key.
- There is **no keychain auditing** and no OS access prompt. A keychain read can
  be logged by the OS; a file read cannot.
- The reduction is logged at `warn` on every use, naming the reduced protection
  (`encryption.rs:48-51`, `:398-403`).

**Unwelcome.** A key file on disk means a second copy of the secret exists, and
an operator restoring a backup of the database without the corresponding key
file gets the intact-file error rather than a silent reset. That is the intended
behaviour, but the error message is long and it is a support surface.

**Unwelcome.** The self-healing migration has a failure mode that must not be
simplified away: if the keyring write cannot be confirmed, the key file is
**kept** (`keyring_backup_key_file_is_kept_when_the_migration_cannot_be_confirmed`,
`encryption.rs:897-910`). A refactor that deletes the file on the strength of a
`store` call that was never confirmed would reintroduce the original bug.

**Unwelcome.** `DbFileKind::PlainSqlite` is a real state: a database that is
structurally intact but *unencrypted* is left alone and reported. For a user who
had a plaintext database from an older build, Conduit will now refuse rather than
migrate. Migration is not attempted.

## Alternatives considered

**Exit loudly when the keyring is unavailable.** Rejected: it is the safe
choice and it makes the product unusable on every headless Linux host and in
this repository's own `docker-compose.yml`. Refusing to start is not a security
control, it is a denial of service the user administers themselves. Availability
of the user's own data on their own machine is the thing being protected.

**Fall back to a key file with a restrictive ACL on every platform.** Rejected as
incomplete: it would be correct on POSIX and would do nothing on Windows, where
setting a per-file DACL needs explicit Win32 work. Rather than ship a
platform-dependent guarantee that is silently absent on one platform, the
guarantee is stated as "per-user directory ACL on Windows" and the reduction is
documented. Tightening this is a real future task.

**Keep the quarantine behaviour but only for genuinely corrupt files, and no key
file at all.** Rejected: this fixes the destructive half and leaves the
unusable-host half. The two fixes are independent and both are needed.

**Auto-generate a new key and re-encrypt the database when the old key is
missing.** Rejected: re-encryption requires decrypting the file, which is exactly
what is impossible when the key is gone. There is no recovery path; the honest
answer is the intact-file error.

**Delete the keyring entry and mint fresh on any read failure.** Rejected: that
is the original bug. The invariant "never return a secret you have not read
back" exists to make this class of mistake impossible to write.

## Related

- What the key file does and does not protect against:
  [ADR-0005](0005-same-user-local-access-is-not-a-boundary.md).
