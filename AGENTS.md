# AGENTS.md — how to work in this repo

## Toolchain & gates

- Latest stable Rust, edition 2024. Workspace: `core/` (lib) + `resident/` (bin).
- The gate is `./check.sh`: `cargo fmt --check && cargo clippy --all-targets -- -D warnings &&
  cargo test`. Green before every commit. No CI yet (deliberate; it will come — keep the gate
  scriptable and fast so it can become CI verbatim).
- `#![forbid(unsafe_code)]` everywhere except the one mmap-view module; every `unsafe` block
  there carries a `// SAFETY:` comment stating the invariant.

## Dependencies

Use established crates freely — do not reinvent wheels — **especially off the core path**:
clap (CLI), serde/serde_json (wire), thiserror (core errors), anyhow (bin edge), memmap2 +
zerocopy (store views), rayon (parallel verbs), tracing + tracing-subscriber (stderr logs),
zstd (fixtures/dumps), criterion/proptest (dev). The one place to stay spartan is the hot read
path: the store layout and the voter should be plain, explicit code a maintainer can read
top-to-bottom — no framework magic, no async runtime (blocking I/O + rayon covers this
workload), no premature generics.

## Style

- Small modules named for what they hold. Let abstractions emerge: introduce a trait/generic
  only when a second real call site exists, not for symmetry or "we'll need it".
- Errors: typed (`thiserror`) in core, `anyhow` only at the binary edge. Every error path
  distinguishable from empty results — this is a SPEC requirement, not taste.
- Comments state invariants and constraints the code can't show ("postings within a resource
  are t-sorted; binary search depends on it"), never narration of what the next line does.
- Tests must be able to fail: assert on real values from real inputs, no tautologies. The
  fixture verify harness is the anchor test; unit tests cover the voter's edge cases (empty
  hit lists, single-hit lists, ties in the modal vote, factor gates).

## Commits

Small, each one building and green. Message style: `area: what changed` with a body line for
why when non-obvious. Log judgment calls in DECISIONS.md in the same commit that embodies them.

## Docs (budgeted — agents are the only readers)

`README.md` (orientation) · `ARCHITECTURE.md` (what IS — update in the same commit that
changes what it claims) · `DECISIONS.md` (rulings + why, append-only) · `docs/SCOPE.md`
(boundary list — update when a boundary moves) · `REPORT.md` (closing brief). Nothing else
unless something genuinely needs a home. Delete stale prose rather than layering corrections.
