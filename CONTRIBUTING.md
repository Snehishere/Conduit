# Contributing to Conduit

Thank you for considering a contribution. This document covers how to get the
project running, how to run the checks, and the few conventions that a change
is expected to follow.

[`docs/DEVELOPMENT.md`](docs/DEVELOPMENT.md) and [`docs/TESTING.md`](docs/TESTING.md)
are the long form and are worth reading before your first change. This file is
the working summary.

## What this project is

Conduit links a phone to a desktop over your local network. It relays clipboard
history, files, SMS threads and notifications, call signalling, screen
mirroring, remote input as a trackpad and keyboard, and trigger-to-action
automation. Transport is a local WebSocket hub on the desktop with mDNS
discovery. An optional relay is included that you can host yourself, for when
the two devices are not on the same network. There is no account system, no
hosted service, and no telemetry.

The repository is a Cargo virtual workspace of three Rust crates plus two
non-Rust frontends:

| Directory | Crate or package | What it is |
|---|---|---|
| `packages/protocol` | `conduit-protocol` | Shared wire format. Library only, no binary, no I/O. |
| `apps/desktop` | `conduit-desktop` (npm) and `conduit` (Rust, in `src-tauri/`) | Tauri v2 desktop app. React 19 + TypeScript + Vite 8 + Tailwind 4 in `src/`, Rust backend in `src-tauri/`. |
| `apps/mobile` | `conduit` (Flutter) | Android and iOS app. |
| `services/relay` | `relay` | Optional self-hostable relay. |
| `scripts` | — | PowerShell build and lint wrappers, and the code generation scripts. |

## Before you start: this is a spare-time project

Conduit is maintained by one person, outside working hours. Two things follow
from that, and it is better to know them now than to discover them in week
three:

- **A first response to an issue or a pull request may take a while.** Open an
  issue early if you are considering something large. An early "I am thinking
  about this" comment is cheaper for you and for the maintainer than a finished
  pull request that turns out to conflict with a plan already in motion.
- **Review will be detailed but slow.** Expect several rounds. Comments tend to
  be about correctness and about the two invariants in
  [Two invariants worth defending](#two-invariants-worth-defending-in-review),
  not about formatting, which the tools handle.

Small, well-scoped pull requests get through faster than large ones. If you
want to change something structural, open an issue first and agree on the shape
of the change before writing it.

## Getting set up

### Prerequisites

| Tool | Version | Needed for | Check with |
|---|---|---|---|
| Rust | 1.98, pinned by `rust-toolchain.toml` | all three Rust crates | `cargo --version` |
| Node.js | 24 or later (`.nvmrc` pins 24) | desktop frontend, code generation | `node --version` |
| npm | 11 or later | desktop frontend, code generation | `npm --version` |
| Flutter | 3.47 or later | mobile app | `flutter --version` |
| Dart | 3.13 or later | mobile app | `dart --version` |
| Docker | any recent | relay container only | `docker --version` |
| Vendored OpenSSL | 3.5.8, win64 | **desktop backend only** | see below |

Node, Flutter, and Docker are each needed for one part of the project only. If
you are working on the desktop frontend you do not need Flutter. If you are
working on `relay` or `conduit-protocol` you need none of them.

On Windows, Flutter is often not on `PATH`. If `flutter` is not recognised, add
it for the current session:

```powershell
$env:PATH = "C:\flutter\bin;" + $env:PATH
```

Substitute your own SDK location if it is not there.

### The vendored OpenSSL directory is a required manual step

**A fresh clone cannot build the desktop backend until you supply this
directory yourself.** Read this before you file a bug saying the desktop app
does not build.

The desktop backend depends on `rusqlite` with the `bundled-sqlcipher` feature.
That feature compiles SQLCipher from source, but SQLCipher still has to link
against a crypto library. [`.cargo/config.toml`](.cargo/config.toml) points at
a vendored copy:

```toml
# Use prebuilt OpenSSL for SQLCipher builds on Windows.
# Prebuilt OpenSSL is at <workspace>/.tools/openssl-win64/ (downloaded from
# https://github.com/TaurusTLS-Developers/OpenSSL-Distribution/releases).
[env]
OPENSSL_LIB_DIR = { value = ".tools/openssl-win64/lib/static", relative = true }
OPENSSL_INCLUDE_DIR = { value = ".tools/openssl-win64/include", relative = true }
```

Because both paths are `relative = true`, they resolve against the **workspace
root**, so the directory must be `<repo>/.tools/openssl-win64/`.

If it is missing, `cargo build -p conduit` and `cargo test -p conduit` fail with
a C compiler error rather than a useful message:

```
sqlcipher/sqlite3.c(113738): fatal error C1083: Cannot open include file:
'openssl/crypto.h': No such file or directory
```

The directory is roughly 80 MB, is gitignored, and nothing in the repository
fetches it. There is no download script, no recorded version, and no checksum.
`.gitignore` describes `.tools/` as "fetched by `scripts/`", which is
aspirational rather than accurate.

To set it up by hand on Windows:

1. Read [`.cargo/config.toml`](.cargo/config.toml) for the exact
   `OPENSSL_LIB_DIR` and `OPENSSL_INCLUDE_DIR` values the build expects, and for
   the download URL recorded in its comment.
2. Download a win64 build of OpenSSL 3.5.8 from that URL.
3. Extract it so that the layout matches what the config expects: static
   libraries directly under `lib/static/` and headers under `include/openssl/`,
   inside `<repo>/.tools/openssl-win64/`.
4. Verify the build with `cargo build -p conduit` from the repository root.

On Linux there is currently no working path. `.cargo/config.toml` is
unconditional, so on Linux it points at the Windows-only layout above and
overrides the system OpenSSL. The usual Tauri packages
(`libwebkit2gtk-4.1-dev`, `libssl-dev`, `libgtk-3-dev`, `librsvg2-dev`) do not
help. This is also why the CI `rust` job does not link the desktop crate.

**A `scripts/fetch-openssl.ps1` that pins a version, verifies a SHA-256
checksum, and extracts to `.tools/openssl-win64/` is a wanted contribution.** It
is the highest-value onboarding fix available. A Linux equivalent, or a
platform-conditional `.cargo/config.toml`, would be needed to unblock the
desktop build on Linux and in CI.

`conduit-protocol` and `relay` do not need this. Neither depends on `rusqlite` or
SQLCipher, and both build with no vendored dependencies.

## The three working directories, and the crate-name trap

This project has three distinct roots, and running a command from the wrong one
is the single most common failure.

| What you are working on | Run commands from |
|---|---|
| Any Rust crate | repository root |
| Desktop frontend, code generation | `apps/desktop` |
| Mobile | `apps/mobile` |

The root [`Cargo.toml`](Cargo.toml) is a virtual workspace that owns all three
Rust crates. A crate directory has no `Cargo.toml` workspace of its own, so
Cargo commands go from the root and select the crate with `-p`.

**Crate directory names are not crate names.** This catches everyone once:

| Directory | `-p` argument |
|---|---|
| `packages/protocol` | `conduit-protocol` |
| `apps/desktop/src-tauri` | `conduit` |
| `services/relay` | `relay` |

So `apps/desktop/src-tauri` builds a crate called `conduit`, and the mobile app
in `apps/mobile` is a Flutter package also called `conduit`. `-p` is required,
not optional. Getting this wrong produces "no such package" rather than an
error that names the directory you typed.

## Building and running

### Protocol

```bash
# repository root
cargo build -p conduit-protocol
cargo test  -p conduit-protocol
cargo doc   -p conduit-protocol --no-deps
```

Library only. This is the fastest crate to build, and a good place to start if
you are still getting the toolchain to work.

### Relay

```bash
# repository root
cargo build -p relay
cargo test  -p relay
cargo run   -p relay
```

Configuration comes from **process environment variables only**. A bare
`cargo run -p relay` does not read `.env`; that is what `docker compose` does.
For a local run, export the variables yourself. [`\.env.example`](.env.example)
is the annotated list of what exists, and the relay refuses to start without
`RELAY_TOKEN` and its HMAC secret.

The relay container:

```bash
# repository root - the build context must be the repository root
docker build -t conduit-relay -f services/relay/Dockerfile .
```

### Desktop

The Rust backend and the Tauri app:

```bash
# repository root
cargo build -p conduit          # requires .tools/openssl-win64/, see above
```

```bash
# apps/desktop
npm ci                         # prefer npm ci over npm install: .npmrc sets legacy-peer-deps
npm run tauri -- dev           # Vite dev server plus the Tauri window
npm run tauri -- build         # release bundle
```

`npm run tauri -- build` runs the frontend build first and then links against
`../dist`, so the frontend must compile before the Tauri build succeeds.
[`scripts\build-desktop.ps1`](scripts) wraps the sequence and accepts `-Release`
and `-Clean`.

Frontend work needs neither Rust, Tauri, nor the vendored OpenSSL:

```bash
# apps/desktop
npm run dev                    # Vite dev server on http://localhost:5173
npm run build                  # tsc then vite build, output to dist/
npx tsc --noEmit               # typecheck without emitting
npm run lint                   # eslint src/
```

The frontend runs in a plain browser, which is useful for layout work. Tauri
IPC is not available there, so every `invoke()` rejects and the app has to
degrade. That is the same environment the Playwright suite runs in.

### Mobile

```bash
# apps/mobile
flutter pub get
flutter analyze
flutter run
flutter build apk --debug
flutter build apk --release
flutter build ios --debug --no-codesign    # macOS host only
```

[`scripts\build-mobile.ps1`](scripts) runs `flutter pub get`, `flutter analyze`,
and the build. iOS builds need a macOS host.

## Running the tests

Every command is run from the directory shown. `cargo` commands are run from
the **repository root** with `-p <crate>`.

The results below were measured on a fresh checkout of this tree.

| Suite | Directory | Command | Result |
|---|---|---|---|
| relay | repository root | `cargo test -p relay` | 189 passed |
| protocol | repository root | `cargo test -p conduit-protocol` | 266 passed |
| desktop backend | repository root | `cargo test -p conduit` | not run, see note |
| format | repository root | `cargo fmt --all -- --check` | clean |
| clippy | repository root | `cargo clippy -p <crate> --all-targets -- -D warnings` | clean for `conduit-protocol` and `relay` |
| typecheck | `apps/desktop` | `npx tsc --noEmit` | exit 0 |
| unit | `apps/desktop` | `npx vitest run` | 216 passed, 20 files |
| build | `apps/desktop` | `npm run build` | ok |
| codegen | `apps/desktop` | `npm run generate` | ok, 55 message types |
| mobile unit | `apps/mobile` | `flutter test` | fails, see note |
| mobile analyze | `apps/mobile` | `dart analyze --fatal-infos` | fails, see note |
| mobile integration | `apps/mobile` | `flutter test integration_test` | needs a device or running emulator |

Notes on the ones that do not pass.

**`cargo test -p conduit` could not be run.** It needs `.tools/openssl-win64/`,
which no script in the repository provides. On a machine where that directory is
present the suite is expected to pass; it has not been re-measured here.

**The mobile Dart tree does not currently compile.** The generated file
`apps/mobile/lib/widgets/conduit_logo.dart` uses `Offset`, `Canvas`, `Size`,
`Paint`, and `Path` but carries no import that provides them, because
`scripts/icons/emit_sources.py` does not emit one. The measured results are 41
unit tests passing with `test/screens/home_screen_test.dart` failing to load,
and 34 analyzer errors, of which 31 are in `conduit_logo.dart` and 3 cascade
into `lib/screens/home_screen.dart`. Running `npm run icons` does not fix it,
because the generator is the source of the defect. The fix is an import line in
the Dart header in `scripts/icons/emit_sources.py`, after which
`npm run icons` regenerates a compiling file. The protocol, relay, and desktop
suites are unaffected, which is why the tree is worth publishing as it is.

A single test by name:

```bash
cargo test -p conduit remote_input_unauthenticated_rejected_by_dispatcher
cargo test -p conduit server::handlers::remote_input::tests
cargo test -p conduit-protocol schema_accepts_every_serialized_rust_type
```

[`docs/TESTING.md`](docs/TESTING.md) documents what each suite actually covers
and, more usefully, what it does not. Three gaps are structural and worth
knowing before you trust a green run: there are no Rust integration tests
anywhere, the Tauri IPC mock returns `undefined` for every command, and there
is no cross-language conformance test.

## Running all the linters

```powershell
# any working directory - the script resolves the root from $PSScriptRoot
.\scripts\lint-all.ps1
```

[`scripts/lint-all.ps1`](scripts) accumulates per-step results rather than
stopping at the first failure, so one run reports everything that is broken,
and it exits non-zero if a blocking step failed. It accepts `-RustOnly` for a
machine without Node or Flutter, and skips steps whose toolchain is absent
rather than passing them silently.

This script is Windows only. The equivalent gate is
[`.github/workflows/ci.yml`](.github/workflows/ci.yml), which is the merge gate
and runs on Linux.

## Code style

### Rust

- `conduit` and `relay` are edition 2024. `conduit-protocol` is edition 2021.
  The workspace mixes editions on purpose.
- [`rustfmt.toml`](rustfmt.toml) exists and sets `edition = "2021"`. It is
  deliberately minimal, and its comment explains why: adding `style_edition`
  at the root would apply to all three crates and reformat the 2021 crate
  against rules it was not written for. Do not add it. If one crate ever needs
  a different one, put a `rustfmt.toml` next to that crate's own
  `Cargo.toml`.
- `cargo clippy -p <crate> -- -D warnings` must be clean.
- Tests live in a `mod tests { ... }` block at the bottom of the file they
  test, with `use super::*;`. There is no `tests/` directory anywhere in the
  repository, and adding one means adding a crate-level test that can reach the
  public API, which nothing currently does.
- Async tests use
  `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]`.
- Comments in this codebase explain **why** a decision was made and what broke
  before it, not what the next line does. That density is deliberate. Match it
  when the reason is not obvious from the code.
- A constant that duplicates a value in `conduit-protocol` must be pinned to
  that source of truth **by a test**, not by a comment. Several are already,
  including the mobile port mirror and the desktop bind address.

### TypeScript

[`apps/desktop/tsconfig.json`](apps/desktop) sets `strict`, `noUnusedLocals`,
`noFallthroughCasesInSwitch`, and `isolatedModules`.
`noUncheckedIndexedAccess` is deliberately absent, so `arr[0]` is typed `T` and
not `T | undefined`. Turning it on surfaces a large number of errors at once;
it should be enabled incrementally, per directory, and not in a single commit.

ESLint uses `tseslint.configs.strictTypeChecked` plus the recommended, `react`,
and `react-hooks` configurations. `eqeqeq` and
`react-hooks/rules-of-hooks` are errors. The `no-unsafe-*` family,
`no-explicit-any`, `no-floating-promises`, `no-misused-promises`, and
`no-unnecessary-condition` are warnings. `react-hooks/exhaustive-deps` is a
warning on purpose. Test files are not linted.

### Dart

[`analysis_options.yaml`](apps/mobile) includes `flutter_lints` plus extra
rules, and escalates `unused_import`, `unused_local_variable`, and `dead_code`
to warnings. `dart analyze --fatal-infos` is the target, and it is clean apart
from the compile failure described above.

`dart format` is **not** enforced. It runs in the lint script and in CI, but
non-blocking in both, and most of the Dart tree is unformatted. Do not assume
a Dart diff is format-clean, and do not reformat unrelated files to make yours
look tidy. Making the check blocking is a single mechanical change, but it has
to be its own commit.

## Code generation

`packages/protocol/schema.json` is the hand-maintained source for the client
models. Code generation runs from `apps/desktop`:

```bash
npm run generate          # both outputs
npm run generate:types    # apps/desktop/src/types/websocket.ts only
npm run generate:dart     # apps/mobile/lib/models/protocol.dart only
```

Two files are generated and carry a `GENERATED FILE - DO NOT EDIT` header:

| Generated file | Generator |
|---|---|
| `apps/desktop/src/types/websocket.ts` | `scripts/generate_types.js` |
| `apps/mobile/lib/models/protocol.dart` | `scripts/generate_dart.js` |

**Never hand-edit them.** The reason is specific, and it is the reason a
generated-file diff in CI is worth having.

`websocket.ts` once declared the same interface twice. TypeScript **merges**
interface declarations with the same name instead of reporting an error, so
`tsc` passed while the effective type was wrong and carried fields that do not
exist in Rust. No type checker can catch that, and no test caught it either.
The only thing that sees it is a regenerate-and-diff, which is why the CI
`codegen` job runs `npm run generate` and then `git diff --exit-code` over both
generated files and the schema. If you edit a generated file by hand, that job
fails and it is correct that it does.

The generator now rejects the classes of error that previously passed
silently: a missing `title` on a `oneOf` branch, since the title becomes the
interface name, a duplicate `title`, and a `oneOf` with no object branches.

`generate_dart.js` resolves the pinned `quicktype` from
`apps/desktop/node_modules` and only falls back to an unpinned `npx --yes
quicktype` if it is absent, so install the desktop dependencies before running
it.

## Cross-layer change checklist

A protocol change is a change across three languages: Rust, TypeScript, and
Dart. The schema is hand-maintained rather than generated from the Rust types,
so both must be edited and the tests will tell you if you forget one. Use this
checklist for any change to a message shape, a port constant, or a frame
layout.

1. **Edit `packages/protocol/src/types.rs`.** This is the source of truth. Add
   or change the message struct and its serde attribute. The convention is an
   internally tagged enum: `#[serde(tag = "type", content = "action")]` when
   the payload is a nested value, `#[serde(tag = "action")]` when the fields
   are flat. `protocol_version` is optional on every message.
2. **Edit `packages/protocol/schema.json` by hand.** It is not generated. Give
   the new `oneOf` branch a `title`, because that becomes the TypeScript
   interface name and the Dart class name, and it must be unique across all
   branches.
3. **Run the protocol tests** and fix whatever they reject:

   ```bash
   # repository root
   cargo test -p conduit-protocol
   ```

   Five tests hold the schema and the Rust types together. The important one
   is `schema_accepts_every_serialized_rust_type`, which serialises every
   message from `types.rs` and asserts the schema accepts the result. The
   others enforce that no two `oneOf` branches accept the same JSON, that
   branch titles are unique, that every branch has a sample, and that every
   sample validates.
4. **Regenerate the clients:**

   ```bash
   # apps/desktop
   npm run generate
   ```

5. **Handle the new message in every consumer.** The desktop server dispatch in
   `apps/desktop/src-tauri/src/server/mod.rs` matches on the `(type, action)`
   pair and has a fallthrough arm, so a missing arm compiles and silently drops
   the message. Extend the dispatch in the desktop server, in the relay, and in
   the mobile client. Check each one explicitly rather than assuming the
   compiler found it.
6. **Update `packages/protocol/PROTOCOL.md`.** It is normative, and tests
   assert parts of it against the code. Mark anything you could not verify in
   code as `[unverified]` rather than asserting it.
7. **Bump `PROTOCOL_VERSION` if the change is not backward compatible.** It is
   `pub const PROTOCOL_VERSION: u32 = 1;` in
   `packages/protocol/src/types.rs`, and a test pins it to 1, so a bump
   deliberately breaks that test. Update the test in the same commit.
8. **Check who enforces the version.** Today only the desktop hub does: it
   rejects an inbound `protocol_version` above `PROTOCOL_VERSION` and answers
   `unsupported_protocol_version`. The relay does not check it, and neither
   does the mobile app. Do not assume a peer has rejected a too-new version on
   your behalf.
9. **Run the full suite** from the correct directories:

   ```bash
   # repository root
   cargo test -p conduit-protocol
   cargo test -p relay
   cargo test -p conduit
   # apps/desktop
   npx tsc --noEmit && npx vitest run
   # apps/mobile
   flutter test && dart analyze --fatal-infos
   ```

### Ports and constants

Port numbers are defined once, in `packages/protocol/src/types.rs`:
`LAN_WS_PORT = 9527` (plaintext) and `LAN_WSS_PORT = 9531` (TLS). If you change
one, update all of:

- `packages/protocol/PROTOCOL.md`, which asserts the port table.
- `kLanWsPort` and `kLanWssPort` in
  `apps/mobile/lib/services/websocket_service.dart`. Tests assert that **no call
  site** hard-codes either number, and that no call site builds a `wss://` URL
  against the plaintext port. The mobile app once did exactly that at three
  call sites, which broke LAN pairing entirely.
- `WS_BIND_ADDR` in `apps/desktop/src-tauri/src/main.rs`, which a test pins to
  the plaintext port and asserts does not parse as the TLS port.
- `docker-compose.yml` and `.env.example` for the relay ports.

Then run `cargo test -p conduit-protocol`.

## Two invariants worth defending in review

These are the two things a reviewer should push back on hardest, because
nothing in the toolchain will tell you that you broke them.

**1. Exactly one `Cargo.lock`, at the repository root.** `.gitignore` enforces
`**/Cargo.lock` with a single `!/Cargo.lock` exception, and a CI job checks it.
Nested lockfiles are how two crates end up building against different versions
of the same dependency, and Cargo ignores a nested lockfile silently, so
nothing reports it. If you see one, it is a bug, not a local convenience. There
is also deliberate version skew between crates recorded in the root
`Cargo.toml`; when you move a dependency into `[workspace.dependencies]`,
confirm which version all three crates should converge on and check whether the
bump needs code changes.

**2. Generated files are never hand-edited.** The full reasoning is in
[Code generation](#code-generation). In short: TypeScript merges duplicate
interface declarations instead of erroring, so a hand-edited or stale generated
file produces a wrong type that `tsc` accepts. Only the CI diff catches it.

## Commits and pull requests

- Branch off `main`, keep the branch short-lived, and delete it after merge.
- Use conventional commit prefixes: `feat:`, `fix:`, `docs:`, `chore:`,
  `refactor:`, `test:`. A one-line subject in the imperative mood, then a body
  explaining why the change is needed if the reason is not obvious from the
  diff.
- Do not rewrite published history. Do not force-push to `main`.
- Keep unrelated changes in separate commits. A commit that mixes a codegen
  regeneration with a behavioural change makes the drift check impossible to
  read.
- If your change touches `packages/protocol`, say so in the pull request and
  list which of the nine checklist steps applied. It is the one place a
  reviewer will ask.
- Run the lint script and the relevant tests before you open the pull request.
  CI is the merge gate, and a red run costs a review round.

## Where to ask questions

- **Bug reports and feature requests:** GitHub issues on this repository.
- **Questions and design discussion:** GitHub Discussions if the project has
  them enabled, otherwise an issue. Please do not open a pull request to ask a
  question.
- **Before starting anything structural:** open an issue. The
  [architecture decision records](docs/decisions) record decisions already made
  and the reasoning behind them. Reading the relevant one first will often
  answer the question and save a round trip.
- **Security issues:** do not open a public issue. See
  [`SECURITY.md`](SECURITY.md) for the current reporting route.
- **Conduct:** see [`CODE_OF_CONDUCT.md`](CODE_OF_CONDUCT.md).

Everyone participating is expected to follow the
[Code of Conduct](CODE_OF_CONDUCT.md).

## Licence

Conduit is released under the MIT licence. See [`LICENSE`](LICENSE).
