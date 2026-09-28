<!--
Keep this short. A template nobody fills in is worse than no template.
Delete any line that does not apply to your change, but keep the two
invariants in the checklist - CI enforces both and will fail the pull
request without them.
-->

## What and why

<!-- One paragraph. What changed, and the problem it solves. Link the issue if there is one. -->

## How to verify

<!--
State which suites you ran and their results. Do not write "tests pass" -
"tests pass" has meant at least four different things in this repo's
history. Numbers, or an explicit "I did not run this because ...".

From the repository root:
  cargo fmt --all -- --check
  cargo clippy -p conduit-protocol -p relay --all-targets -- -D warnings
  cargo test -p conduit-protocol -p relay
  cargo build -p conduit-protocol -p relay --release

From apps/desktop:
  npx tsc --noEmit
  npx vitest run
  npm run lint          (exits 1 today: 15 pre-existing errors)
  npx vite build
  npm run test:e2e      (browser shell only; no Tauri IPC)

From apps/mobile:
  flutter test
  dart analyze --fatal-infos
-->

| Suite | Command | Result |
|---|---|---|
|  |  |  |

## Checklist

- [ ] **Exactly one `Cargo.lock`, at the repository root.** No new lockfile
      inside a workspace member. Cargo ignores nested ones, so they are
      invisible to the toolchain while misleading anyone reading the tree.
      The `hygiene` job fails the build if this is wrong. If you added a
      dependency, `Cargo.lock` is part of your diff.
- [ ] **No generated file was hand-edited.** `apps/desktop/src/types/websocket.ts`,
      `apps/mobile/lib/models/protocol.dart` and every file under
      `assets/brand/` are generated. Run the generator and commit its output;
      a hand edit is silently discarded by the next regeneration and then
      fails the drift gate on somebody else's pull request.
- [ ] If `packages/protocol/schema.json` or
      `packages/protocol/src/types.rs` changed, this pull request includes
      the regenerated clients, produced by `npm run generate` in
      `apps/desktop`, and the `codegen drift` job is green.
- [ ] If `PROTOCOL_VERSION` changed, say so here and explain why. It is a
      compatibility decision, not a version bump: an older client may be
      talking to a newer one, and a change that is not backward compatible
      needs to be a deliberate call with a reason attached, not a side
      effect.
- [ ] If a brand asset changed, it was produced by `npm run icons` in
      `apps/desktop` and the `icon drift` job is green.
- [ ] Anything that changes a public behaviour is reflected in the docs in
      the same pull request, not a follow-up.
