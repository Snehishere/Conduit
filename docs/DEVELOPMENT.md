# Development guide

How to set up, build, test and change Conduit. For the project overview start
with the [root README](../README.md); for system design see
[ARCHITECTURE.md](ARCHITECTURE.md).

- [1. The three working directories](#1-the-three-working-directories)
- [2. Prerequisites](#2-prerequisites)
- [3. The vendored OpenSSL problem](#3-the-vendored-openssl-problem)
- [4. Build and run per component](#4-build-and-run-per-component)
- [5. Linting](#5-linting)
- [6. Code generation](#6-code-generation)
- [7. Code style](#7-code-style)
- [8. Cross-layer change checklist](#8-cross-layer-change-checklist)
- [9. Git workflow](#9-git-workflow)
- [10. Platform-specific notes](#10-platform-specific-notes)

---

## 1. The three working directories

This project has **three distinct roots**, and running a command from the wrong
one is the most common failure. There is no root `package.json` and no root
Flutter project; there *is* a root `Cargo.toml` that owns all three Rust crates.

| What you are working on | Run commands from | Why |
|---|---|---|
| Any Rust crate (`conduit-protocol`, `conduit`, `conduit-relay`) | **repository root** | The root `Cargo.toml` is a virtual workspace whose members are the three crate directories. A crate directory has no `Cargo.toml` workspace of its own. |
| Desktop frontend / codegen | `apps/desktop` | Own `package.json`, `node_modules`, `vite.config.ts`, `vitest.config.ts`, `playwright.config.ts`. |
| Mobile | `apps/mobile` | Own `pubspec.yaml`. |

**Always pass `-p <crate>` from the root.** The crate names are:

| Directory | `-p` name |
|---|---|
| `packages/protocol` | `conduit-protocol` |
| `apps/desktop/src-tauri` | `conduit` |
| `services/relay` | `conduit-relay` |

`scripts/lint-all.ps1` follows this convention: it resolves the repository root
from `$PSScriptRoot` and runs every cargo step from there with an explicit
`-p <crate>`.

`build-desktop.ps1` and `build-mobile.ps1` build their paths from `$PSScriptRoot`
and work from anywhere.

```powershell
# any working directory
.\scripts\lint-all.ps1
```

---

## 2. Prerequisites

| Tool | Required version | Needed for | Verify with |
|---|---|---|---|
| Rust toolchain | 1.98+ | `conduit`, `conduit-relay` (edition 2024) | `cargo --version` |
| Node.js | 24+ | desktop frontend, codegen | `node --version` |
| npm | 11+ | desktop frontend, codegen | `npm --version` |
| Flutter | 3.47+ | mobile app | `flutter --version` |
| Dart | 3.13+ (`>=3.12.0 <4.0.0` per `pubspec.yaml`) | mobile app | `dart --version` |
| Vendored OpenSSL | 3.5.8 win64 | **desktop Rust only** | see [§3](#3-the-vendored-openssl-problem) |

### Rust editions differ between crates

| Crate | Edition | Consequence |
|---|---|---|
| `conduit` (desktop) | 2024 | needs Rust 1.85+ |
| `conduit-relay` | 2024 | needs Rust 1.85+ |
| `conduit-protocol` | 2021 | builds on older toolchains, but shares the root lockfile so the toolchain is effectively workspace-wide |

### Node / npm

`apps/desktop/.npmrc` sets `legacy-peer-deps=true`. Several devDependencies are
peer-conflicting, so use `npm ci` (not `npm install`) where a lockfile is
present, and do not remove that line without testing a clean install.

### Flutter is not on PATH in this environment

On Windows, `C:\flutter\bin` is not on `PATH` by default here. Add it per
session before any `flutter` or `dart` command:

```powershell
$env:PATH = "C:\flutter\bin;" + $env:PATH
```

Without it you get `flutter : The term 'flutter' is not recognized ...`. The
`C:\flutter` path is machine-specific — substitute your own SDK location.

---

## 3. The vendored OpenSSL problem

**This blocks a fresh clone from building the desktop app.** Read this section
before you file "it does not build".

### What happens

`apps/desktop/src-tauri/Cargo.toml:24`:

```toml
rusqlite = { version = "0.40", features = ["bundled-sqlcipher"] }
```

`bundled-sqlcipher` compiles SQLCipher from source, but SQLCipher still links
against a crypto library. `.cargo/config.toml` supplies one by pointing at a
vendored copy:

```toml
# Use prebuilt OpenSSL for SQLCipher builds on Windows.
# Prebuilt OpenSSL is at <workspace>/.tools/openssl-win64/ (downloaded from
# https://github.com/TaurusTLS-Developers/OpenSSL-Distribution/releases).
[env]
OPENSSL_LIB_DIR = { value = ".tools/openssl-win64/lib/static", relative = true }
OPENSSL_INCLUDE_DIR = { value = ".tools/openssl-win64/include", relative = true }
```

The `relative = true` makes these paths resolve relative to the **workspace
root**, so the directory must be `<repo>/.tools/openssl-win64/`.

### What the checkout actually contains

| Property | Value |
|---|---|
| Location | `.tools/openssl-win64/` (80.2 MB) |
| Version | `3.5.8` (from `version.txt`) |
| Static libraries | `lib/static/` — the `OPENSSL_LIB_DIR` the build needs |
| Headers | `include/` |
| Gitignored | yes, `.gitignore` (`/.tools/`) |

`.gitignore` labels `.tools/` as "fetched by scripts/". **There is no such
script.** The list of problems:

| Problem | Consequence |
|---|---|
| No `scripts/fetch-openssl.ps1` | nothing downloads it; a fresh clone has no way to get it |
| No version pin in any tracked file | the 3.5.8 in `.tools/` is unrecorded knowledge |
| No SHA-256 verification | an arbitrary archive from a GitHub release would be accepted |
| `.tools/` is gitignored | a clone never receives it |
| `scripts/build-desktop.ps1` assumes it exists | fails at `npm run tauri -- build` with a link error, not a useful message |

### Manual setup (Windows)

1. Download the win64 package from
   <https://github.com/TaurusTLS-Developers/OpenSSL-Distribution/releases>
   (this checkout used 3.5.8).
2. Extract it to `<repo>/.tools/openssl-win64/` such that
   `lib/static/*.lib` and `include/openssl/*.h` are directly under it.
3. `cd apps/desktop && npm ci && npm run tauri -- dev`

### Linux and CI

`.cargo/config.toml` is unconditional, so on Linux `OPENSSL_LIB_DIR` resolves
to `.tools/openssl-win64/lib/static` — a Windows-only layout. The desktop
build fails there with no installable path. The usual Linux Tauri webview
dependencies (`libwebkit2gtk-4.1-dev`, `libssl-dev`, `libgtk-3-dev`,
`librsvg2-dev`) do not help: the config overrides the system OpenSSL with a
path that does not exist.

**CI exists** (`.github/workflows/ci.yml`, added in `eed58bd`) but its Rust job
does not currently link the desktop crate, so nothing builds the desktop app on
Linux there. Making that work needs either a platform-conditional
`.cargo/config.toml` (or a `target`-specific one) that points at the system
OpenSSL on Linux, or a `fetch-openssl.sh` producing the same layout.

### What is needed to fix it

A `scripts/fetch-openssl.ps1` that:

1. pins a version (3.5.8 to match this checkout),
2. downloads a specific release asset,
3. verifies a SHA-256 checksum of the downloaded archive before extracting,
4. extracts to `.tools/openssl-win64/`,
5. fails with a clear message if the archive's hash does not match.

Plus a Linux equivalent or a platform-conditional cargo config. Tracked in
`docs/REMAINING_WORK.md`.

`conduit-protocol` and `conduit-relay` do **not** have this problem — neither
depends on `rusqlite` or SQLCipher. Both build with no vendored dependencies.

---

## 4. Build and run per component

### `conduit-protocol`

```bash
# repository root
cargo build -p conduit-protocol
cargo test  -p conduit-protocol      # 270 tests
cargo doc   -p conduit-protocol --no-deps
```

Library only. No binary, no server, no I/O. If `cargo doc` emits warnings about
missing intra-doc links to `PROTOCOL_VERSION` / `LAN_WS_PORT`, those are
`[bracket]` references into `types.rs` and are not errors.

### `conduit-relay` (library)

```bash
# repository root
cargo build  -p conduit-relay
cargo test   -p conduit-relay         # 235 tests, 1 ignored
cargo clippy -p conduit-relay --all-targets -- -D warnings
```

**Library only.** There is no `[[bin]]` and no `src/main.rs`, so there is no
`cargo run -p conduit-relay`, no release binary to ship and no container to
build. The desktop app hosts it as a background task in
`apps/desktop/src-tauri/src/relay.rs`, which is the only way to run it.

Configuration comes from the desktop's **Settings** — `relay_enabled`,
`relay_port`, `relay_health_port`, `relay_hostname` — passed to
`Config::resolve` as `conduit_relay::Overrides`. The precedence is
`Overrides` → environment variables → library defaults. The environment is a
fallback for headless and test use only; it is not how the app is configured.
`health_bind` and `ws_bind` are pinned to loopback in `build_config` and are
deliberately **not** inheritable from the environment.

### `conduit` (desktop Rust) — requires the vendored OpenSSL

```bash
# repository root
cargo build -p conduit
cargo test  -p conduit               # 729 tests: all passing
cargo clippy -p conduit -- -D warnings
```

Building the *Tauri app* (frontend bundle + native binary) is driven from
`apps/desktop`:

```bash
# apps/desktop
npm ci
npm run tauri -- dev                  # dev: Vite on :5173 + the Tauri window
npm run tauri -- build                # release bundle (targets: "all")
npm run tauri -- build --debug
```

`npm run tauri -- build` runs `beforeBuildCommand` (`npm run build`, i.e.
`tsc && vite build`) and then links against `../dist`, so the frontend must
build first. `scripts\build-desktop.ps1` wraps the whole sequence
(`-Release`, `-Clean` flags).

### `conduit-desktop` frontend only

```bash
# apps/desktop
npm ci
npm run dev                            # Vite dev server, http://localhost:5173
npm run build                          # tsc && vite build -> dist/
npm run preview
npx tsc --noEmit                      # typecheck without emitting
```

Running the frontend in a plain browser works for layout work. Tauri IPC is
unavailable, so every `invoke()` rejects and the app must degrade. That is the
same environment the Playwright suite runs in — see [TESTING.md](TESTING.md).

### `conduit` (mobile)

```bash
# apps/mobile  (add $env:PATH = "C:\flutter\bin;" + $env:PATH first on Windows)
flutter pub get
flutter analyze
flutter run                           # attached device or emulator
flutter build apk --debug
flutter build apk --release
flutter build ios --debug --no-codesign   # macOS host only
```

`scripts\build-mobile.ps1 -Target android|ios|all [-Release]` runs
`flutter pub get`, `flutter analyze`, then the build.

---

## 5. Linting

```powershell
# any working directory — the script resolves the root from $PSScriptRoot
.\scripts\lint-all.ps1
```

It accumulates per-step results rather than short-circuiting, prints a summary
table, and exits non-zero if any blocking step failed.

| # | Step | Directory | Command |
|---|---|---|---|
| 1 | Format check | repository root | `cargo fmt --all -- --check` |
| 2 | Clippy | repository root | `cargo clippy -p conduit-protocol --all-targets -- -D warnings` |
| 3 | Clippy | repository root | `cargo clippy -p conduit-relay --all-targets -- -D warnings` |
| 4 | Clippy | repository root | `cargo clippy -p conduit --all-targets -- -D warnings` |
| 5–7 | Rust tests | repository root | `cargo test -p <crate> --locked` |
| 8 | Release build | repository root | `cargo build --workspace --release --locked` (skipped by `-Fast`) |
| 9–12 | TypeScript | `apps/desktop` | `tsc --noEmit`, `eslint src/`, `vitest`, `vite build` |
| 13 | Playwright | `apps/desktop` | `npm run test:e2e` — non-blocking; cannot reach Tauri IPC |
| 14 | Dart format | `apps/mobile` | `dart format --set-exit-if-changed` — non-blocking |
| 15 | Flutter analyze | `apps/mobile` | `flutter analyze --fatal-infos` |
| 16 | Flutter tests | `apps/mobile` | `flutter test` |

`-RustOnly` skips everything from step 9 on, for a machine without Node or
Flutter. Node, Playwright and Flutter steps are skipped with a message rather
than silently when their toolchains are absent.

`lint-all.ps1` runs the strict `--fatal-infos` variant only for
`cargo clippy` and `cargo test`; for Dart it uses plain `flutter analyze`. The
stricter analyzer is available directly:

```bash
# apps/mobile
dart analyze --fatal-infos
```

CI (`.github/workflows/ci.yml`) is the merge gate. It runs the same Rust,
Node, codegen and hygiene gates, plus `flutter analyze`. There is no image build
and no compose validation: the relay is a library, so the `rust` job compiles
and tests it like any other crate.

---

## 6. Code generation

Two files are **generated** and must never be hand-edited:

| Generated file | Generator | Source |
|---|---|---|
| `apps/desktop/src/types/websocket.ts` | `scripts/generate_types.js` | `packages/protocol/schema.json` |
| `apps/mobile/lib/models/protocol.dart` | `scripts/generate_dart.js` (quicktype) | `packages/protocol/schema.json` |

```bash
# apps/desktop
npm run generate          # both of the above
npm run generate:types    # websocket.ts only
npm run generate:dart     # protocol.dart only
```

Both files begin with a `GENERATED FILE - DO NOT EDIT` header.

### Why hand-editing is dangerous here

The generated `websocket.ts` once contained a **duplicate
`ClipboardRequestMessage` declaration** that TypeScript silently *merged* into
a single type. `tsc` passed. The type was wrong. Nothing in the toolchain
noticed.

`scripts/generate_types.js` now fails loudly on the classes of error that
previously passed silently:

- a missing `title` on a `oneOf` branch (it becomes the TS interface name and
  the union tag),
- **a duplicate `title`**, which would produce two identical union members,
- a `oneOf` with no object branches.

It also handles `required` + `allOf` as conjunctions and `anyOf`/`oneOf` as
alternatives, because ignoring those combinators previously generated
`Automation Delete` with both `rule_id` and `id` optional, silently discarding
a real schema constraint.

### CI diff-checks the generated files

This is implemented. The `codegen` job in `.github/workflows/ci.yml` runs:

1. `npm ci` in `apps/desktop`,
2. `npm run generate`,
3. `git diff --exit-code` over `apps/desktop/src/types/websocket.ts`,
   `apps/mobile/lib/models/protocol.dart` and `packages/protocol/schema.json`.

Without that step, a schema change can be merged while the generated
TypeScript and Dart silently stay stale — and neither compiler nor test suite
will tell you. This is the single highest-value CI check in the project.

`generate_dart.js` resolves the pinned `quicktype` from
`apps/desktop/node_modules` first and only falls back to `npx --yes quicktype`
(unpinned) if it is missing. Install desktop dev dependencies before running it.

---

## 7. Code style

### Rust

- `conduit` and `conduit-relay`: edition 2024. `conduit-protocol`: edition 2021.
- `cargo fmt` (rustfmt defaults — no `rustfmt.toml` in the tree).
- `cargo clippy -- -D warnings` per crate, from the root with `-p`.
- Test modules are `mod tests { ... }` at the bottom of the file, `#[cfg(test)]`
  is implied by the module, and async tests use
  `#[tokio::test(flavor = "multi_thread", worker_threads = 2)]`.
- Comments explain *why* a decision was made and what broke before it. The
  codebase is unusually dense with these; several are load-bearing (e.g. the
  `add_test_client` contract in
  `apps/desktop/src-tauri/src/server/handlers/mod.rs:205-222`). Match that
  density when the reason is not obvious from the code.
- Constants that duplicate a value in `packages/protocol` must be pinned by a
  test to that source of truth, not by a comment. Examples:
  `ws_bind_addr_matches_ws_port`, `PROTOCOL.md` port assertions, and the
  `kLanWsPort`/`kLanWssPort` mirror in the mobile WebSocket service.

### TypeScript

`apps/desktop/tsconfig.json`:

| Option | Value |
|---|---|
| `strict` | `true` |
| `noUnusedLocals` | `true` |
| `noUnusedParameters` | `false` |
| `noFallthroughCasesInSwitch` | `true` |
| `isolatedModules` | `true` |
| `noUncheckedIndexedAccess` | **absent** |

`noUncheckedIndexedAccess` is not enabled, so `arr[0]` is typed `T` rather than
`T | undefined`. Turning it on will surface a large number of errors at once;
it is worth enabling incrementally per-directory, not in one commit.

ESLint (`apps/desktop/eslint.config.js`) uses `tseslint.configs.strictTypeChecked`
plus `@eslint/js` recommended, `react`, and `react-hooks`. Notable rules:

- `eqeqeq`, `react-hooks/rules-of-hooks` → error.
- `no-console` → warn, `allow: ["warn", "error"]`.
- `@typescript-eslint/no-explicit-any`, `no-unsafe-*`, `no-floating-promises`,
  `no-misused-promises`, `no-non-null-assertion`,
  `no-unnecessary-condition` → warn.
- `react-hooks/exhaustive-deps` → warn (deliberately non-blocking).
- Ignores: `dist/`, `node_modules/`, `src-tauri/`, `**/*.test.*`, `**/*.spec.*`
  — so test files are not linted at all.

#### The `typescript` dependency alias is broken and being fixed

`apps/desktop/package.json` declares two competing compilers:

```json
"@typescript/native": "npm:typescript@^7.0.2",
"typescript": "npm:@typescript/typescript6@^6.0.2",
```

What that resolves to on disk:

| Path | Package | Version |
|---|---|---|
| `node_modules/typescript/` | `@typescript/typescript6` | 6.0.2 |
| `node_modules/@typescript/native/` | `typescript` | 7.0.2 |
| `node_modules/.bin/tsc` → `../@typescript/native/bin/tsc` | `typescript` (native) | **7.0.2** |

So `npx tsc --noEmit` and the `tsc` step inside `npm run build` **both run
TypeScript 7.0.2**, while the package the ESLint config comment describes
("uses TS 6 API via npm alias") is 6.0.2 and is never invoked. The
`typescript` entry is currently dead weight.

Consequence to be aware of while this is being fixed: the typecheck that gates
`npm run build` and `npm test` is TypeScript 7, and `tseslint.configs.strictTypeChecked`
was written against the TypeScript 6 API. A `tsc` version change can move the
pass/fail line on its own. Pin one compiler, delete the other, and re-run
`npx tsc --noEmit`, `npm test` and `npm run lint` together.

### Flutter / Dart

- `apps/mobile/analysis_options.yaml` includes `flutter_lints` plus 8 extra
  rules (`prefer_const_constructors`, `avoid_print`, `prefer_single_quotes`,
  `prefer_const_literals_to_create_immutables`, `prefer_const_declarations`,
  `avoid_unnecessary_containers`, `sized_box_for_whitespace`,
  `use_key_in_widget_constructors`).
- `unused_import`, `unused_local_variable` and `dead_code` are escalated to
  `warning`.
- `dart analyze --fatal-infos` is currently clean: 0 errors, 0 warnings.
- **`dart format` is enforced only as a non-blocking CI step** — `lint-all.ps1`
  runs it with `-Optional` and the CI `flutter` job sets
  `continue-on-error: true`. Measured with
  `dart format --output=none --set-exit-if-changed` over `lib/`, `test/` and
  `integration_test/`: **63 of 74 files are unformatted.** Enabling it as a
  blocking check is a single large mechanical commit; until then, do not assume
  a diff is format-clean.

---

## 8. Cross-layer change checklist

A protocol change is a **three-language** change: Rust (`types.rs`), TypeScript
(`websocket.ts`), Dart (`protocol.dart`). The schema is hand-maintained and
kept in sync by tests, not generated from `types.rs`, so both must be edited and
the tests will tell you if you forget.

Use this checklist for any change to a message shape, a port constant or a
binary frame layout.

1. **Edit `packages/protocol/src/types.rs`.** Add or change the message struct
   and its serde attribute. The convention is an internally tagged enum —
   `#[serde(tag = "type", content = "action")]` where the payload is a nested
   value, or `#[serde(tag = "action")]` where the fields are flat. The
   `protocol_version` field is optional on every message.
2. **Edit `packages/protocol/schema.json` by hand.** It is not generated. Add a
   `title` to the new `oneOf` branch — the title becomes the TypeScript
   interface name and the Dart class name, and it must be unique across all
   branches.
3. **Run the schema-sync tests** and fix whatever they reject:
   ```bash
   # repository root
   cargo test -p conduit-protocol
   ```
   Five tests enforce the invariants:
   | Test | Enforces |
   |---|---|
   | `schema_oneof_branches_are_unambiguous` | no two branches accept the same JSON |
   | `schema_oneof_branch_titles_are_unique` | no title collision (this is the duplicate-declaration bug) |
   | `schema_sample_set_covers_every_oneof_branch` | every branch has a sample |
   | `schema_accepts_every_sample_in_sample_set` | every sample validates |
   | `schema_accepts_every_serialized_rust_type` | **every `types.rs` message serializes to something `schema.json` accepts** — this is the `types.rs` ↔ `schema.json` link |
4. **Regenerate the clients:**
   ```bash
   cd apps/desktop && npm run generate
   ```
5. **Update `packages/protocol/PROTOCOL.md`.** It is normative. It carries its
   own table of contents and its statements are asserted against the code by
   tests (port tables, discovery examples). Mark anything you could not verify
   in code as `[unverified]` rather than asserting it.
6. **Bump `PROTOCOL_VERSION` if the change is not backward compatible.** It is
   `pub const PROTOCOL_VERSION: u32 = 1;` in
   `packages/protocol/src/types.rs:17`. A test pins it to 1
   (`protocol_version_is_one`), so a bump breaks that test deliberately.
7. **Check who enforces the version.** Currently only the desktop hub does: it
   rejects an inbound `protocol_version` greater than `PROTOCOL_VERSION` and
   answers `unsupported_protocol_version`. **The relay does not check it at
   all, and neither does the mobile app.** Do not assume a peer has rejected a
   too-new version on your behalf.
8. **Update both consumers.** `services/relay` and `apps/desktop/src-tauri`
   depend on `conduit-protocol`; if either dispatches on the message type,
   the match must be extended or it will fail to compile (good) or fall through
   to an unhandled branch (bad — check explicitly).
9. **Run the full suite** from the correct directories:
   ```bash
   # repository root
   cargo test -p conduit-protocol
   cargo test -p conduit-relay
   cargo test -p conduit          # 729 tests, all passing
   cd apps/desktop && npx tsc --noEmit && npm test
   cd apps/mobile && flutter test && dart analyze --fatal-infos
   ```
10. **Add a cross-language check.** There is none today: nothing verifies that
    `types.rs`, `websocket.ts` and `protocol.dart` agree on the same message.
    The three regeneration steps are the entire safety net, and step 3 only
    covers Rust ↔ schema. A conformance test is in `docs/REMAINING_WORK.md`.

### Ports and constants

Port numbers live in `packages/protocol/src/types.rs`
(`LAN_WS_PORT = 9527`, `LAN_WSS_PORT = 9531`). If you change one:

- update `PROTOCOL.md` (asserted by test),
- update `kLanWsPort` / `kLanWssPort` in
  `apps/mobile/lib/services/websocket_service.dart` (asserted by test — the
  tests also assert that **no call site** hard-codes either number, and that no
  call site builds a `wss://` URL against the plaintext port),
- update `WS_BIND_ADDR` in `apps/desktop/src-tauri/src/main.rs`,
- update `relay_port`, `relay_health_port` and `DEFAULT_RELAY_LOCAL_PORT` in
  `apps/desktop/src-tauri/src/relay.rs` for the relay ports. Note that 9531 is
  deliberately shared: it is both the desktop's LAN TLS listener and the relay's
  loopback-only plaintext listener (`DEFAULT_RELAY_LOCAL_PORT`) that the
  desktop joins its own relay over,
- run `cargo test -p conduit-protocol`.

---

## 9. Version control workflow

This working copy is not under version control. The published repository is a
separate, freshly initialised snapshot — see the Status section of the README.

Once a remote exists:

- work on short-lived branches off `main`,
- use conventional commit prefixes (`feat:`, `fix:`, `docs:`, `chore:`, `refactor:`,
  `test:`) in the imperative mood,
- do not rewrite history on `main`.

Two invariants worth defending in CI:

1. **Exactly one `Cargo.lock`, at the repository root.** `.gitignore` enforces
   `**/Cargo.lock` + `!/Cargo.lock`. Nested lockfiles are how two crates end up
   building against different versions of the same dependency, and Cargo ignores
   them silently — so nothing would tell you it had happened. Do not commit a
   nested one.
2. **Generated files are never hand-edited.** See
   [§6](#6-code-generation).

---

## 10. Platform-specific notes

### Windows

- Flutter is not on `PATH`: `$env:PATH = "C:\flutter\bin;" + $env:PATH`.
- `scripts\lint-all.ps1`, `build-desktop.ps1` and `build-mobile.ps1` all derive
  their paths from `$PSScriptRoot` and run from any working directory.
- `.tools/openssl-win64/` is the only known working OpenSSL layout.
- Tauri bundles with `webviewInstallMode: { "type": "skip" }` on Windows, so
  the machine must already have the WebView2 runtime.

### Linux

- Tauri desktop needs `libwebkit2gtk-4.1-dev`, `libssl-dev`, `libgtk-3-dev`
  and `librsvg2-dev` (the last for the tray icon). `tauri.conf.json` also
  declares `libasound2` (deb) / `alsa-lib` (rpm) as bundle dependencies.
- **The vendored-OpenSSL config blocks the Linux desktop build.** See
  [§3](#3-the-vendored-openssl-problem). There is no installable path today.
- `conduit-relay` and `conduit-protocol` build on Linux with no extra system
  packages.

### macOS

- `tauri.conf.json` sets `bundle.macOS.minimumSystemVersion: "12.0"`, with
  `signingIdentity: null` and `entitlements: "./Entitlements.plist"`. Note the
  case mismatch: the file on disk is `apps/desktop/src-tauri/entitlements.plist`
  (lowercase `e`). macOS's default case-insensitive filesystem hides this; a
  case-sensitive filesystem will fail the build. An unsigned build is the
  default; notarisation is not configured.
- iOS builds require a macOS host. `flutter build ios --no-codesign` avoids
  signing.

### Android

- `flutter_launcher_icons` is configured with `min_sdk_android: 21`; regenerate
  launcher icons with `dart run flutter_launcher_icons` after changing
  `assets/icon.png`.
- `apps/mobile/android/local.properties` is gitignored and must be regenerated
  by the Flutter tool (`flutter pub get` / `flutter run`).

### iOS

- `apps/mobile/ios/Pods`, `.symlinks` and `Flutter/ephemeral` are gitignored.
  Run `flutter pub get` after a clone; `pod install` runs from it.
