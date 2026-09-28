# ADR-0001: One Cargo lockfile, and workspace-level dependency declarations

- **Status:** Accepted
- **Date:** 2026-09

## Context

`Cargo.toml` declares a virtual workspace with three members
(`apps/desktop/src-tauri`, `services/relay`, `packages/protocol`) and a
`[workspace.dependencies]` table (`Cargo.toml:1-22`). Because the root manifest is
virtual, Cargo resolves the whole graph from the root and **ignores any
`Cargo.lock` inside a member directory**. Three such files existed. Cargo never
read them; they were inert.

They were not merely inert, they were wrong. `services/relay/Cargo.lock` did not
contain `conduit-protocol` — the relay's own mandatory path dependency
(`services/relay/Cargo.toml`) — so the file was a snapshot of a graph that
could never have been the relay's graph. Anyone reading it to answer "what does
the relay depend on?" got a confidently wrong answer that no tool would ever
contradict.

The failure mode is the same one that runs through this whole remediation: a
claim that is checkable-looking, is not checkable, and is not contradicted
because the thing that would contradict it is not looking.

## Decision

Only `Cargo.lock` at the workspace root is authoritative.

- The three nested lockfiles are deleted.
- `.gitignore` enforces the invariant going forward: `**/Cargo.lock` with
  `!/Cargo.lock` (`.gitignore:5-6`), so a nested lockfile cannot be committed
  even by accident.
- Shared third-party versions continue to be declared once, in
  `[workspace.dependencies]` at the root, and members reference them with
  `workspace = true`.

## Consequences

**Easier.** Dependency versions for all three crates are visible in one place and
cannot disagree. `cargo tree` and `cargo build` agree with the file on disk. The
relay's dependency set is now derivable from `services/relay/Cargo.toml` plus
the root lockfile, which is the truth rather than a stale snapshot.

**Unwelcome.** A nested lockfile is sometimes deliberately used to pin a
*binary* or a *library consumer* to a fixed graph. This workspace is three
co-deployed binaries plus one shared library, so there is no such consumer, and
the pattern is now impossible to express. If a downstream integrator ever vendors
a member independently, this rule has to be revisited for that subtree.

**Unwelcome.** The enforcement is a `.gitignore` rule, not a build check. A
nested lockfile created locally is silently ignored by both Cargo *and* git;
there is no error. A CI step that fails on a committed nested lockfile would be
stricter than this. That step does not exist yet.

## Alternatives considered

**Keep the nested lockfiles for reference.** Rejected: they cannot be kept
correct, because nothing keeps them in sync and nothing validates them. A file
that is wrong and looks authoritative is worse than no file.

**Pin member dependencies inside each member manifest and keep per-member
lockfiles.** Rejected: in a virtual workspace Cargo still ignores the nested
lockfiles, so this produces the same three stale files while also fragmenting
version control. It only works with a non-virtual root.

**Convert the root to a non-virtual (package) manifest.** Rejected: there is no
root package to describe, and inventing one purely to own a lockfile adds a
member that builds nothing.
