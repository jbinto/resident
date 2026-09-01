# AGENTS.md — contributor rules

These rules apply to humans and coding agents. Read `README.md` for orientation,
`ARCHITECTURE.md` for the implementation, `CONTRACT.md` for the public interface, and `SPEC.md` for
behavior that must survive a refactor.

## Source lineage is part of correctness

Resident is an independent implementation built from behavioral study of
[Panako](https://github.com/JorenSix/Panako) 2.1 at commit `e4b0e1d`, fixture answers produced by
that version, and the analysis behavior Panako obtains through JGaborator and
[Gaborator](https://www.gaborator.com/). Preserve the acknowledgements and license explanation in
`README.md`.

Be exact in claims:

- the default matcher is Panako-compatible under one pinned configuration;
- imported dumps and golden answers come from Panako;
- native extraction is answer-faithful, not bit-identical;
- Resident does not vendor, link, or wrap Panako, JGaborator, or Gaborator at runtime;
- observed upstream quirks belong only in compatibility paths and must have fixture coverage.

When compatibility behavior changes, cite the relevant upstream file/commit or fixture, add a test
that would have failed before the change, and record the ruling in `DECISIONS.md`.

## Toolchain and gate

- Latest stable Rust, edition 2024. Workspace members: `core/` (library) and `resident/` (binary).
- `./check.sh` is the complete gate: `cargo fmt --check`, warning-free Clippy across all targets,
  then all tests. It must pass before every commit.
- Keep the gate non-interactive, deterministic, and suitable for direct use in CI.
- Extraction tests require `ffmpeg`; stored-print matching does not.

## Architecture rules

- `resident-core` owns fingerprint/config types, typed errors, dump parsing, storage, matching,
  passage construction, and native extraction.
- The `resident` binary owns CLI parsing, JSON-lines transport, process I/O, batch orchestration,
  and human-facing error context.
- Keep modules small and named for what they contain. Introduce a trait or generic only after a
  second real call site needs it.
- The hot read path—the store layout, mapped views, hash lookup, and voter—must remain plain code a
  maintainer can read from top to bottom. Avoid framework machinery and premature abstraction there.
- Blocking I/O plus Rayon is the chosen concurrency model. Do not introduce an async runtime merely
  to wrap the stdio daemon.

## Safety and errors

- `#![forbid(unsafe_code)]` applies everywhere except the isolated mmap-view module.
- Every unsafe block in that module requires an adjacent `// SAFETY:` comment stating the lifetime,
  alignment, bounds, and immutability invariants it relies on.
- Core errors are typed with `thiserror`; `anyhow` is only for adding context at the binary edge.
- Every failure must remain distinguishable from a successful empty result. Missing stores,
  incompatible generations, bad requests, and unsupported behavior never collapse to “no match.”
- Query paths must work against read-only stores. Generation publication remains an explicit writer
  operation.

## Dependencies

Use established crates freely away from the hot path. Current choices include:

- `clap` for the CLI;
- `serde` and `serde_json` for wire/control-plane data;
- `thiserror` in core and `anyhow` at the binary edge;
- `memmap2` and `zerocopy` for immutable store views;
- `rayon` for bounded CPU parallelism;
- `rustfft` for native analysis;
- `tracing` and `tracing-subscriber` for stderr diagnostics;
- `zstd`, `tempfile`, and the existing test tooling for fixtures and batch work.

Do not add a dependency that hides store layout or voting behavior behind opaque framework code.

## Tests

- Tests must assert real outputs from real inputs; tautological serialization or “does not panic”
  tests are insufficient for algorithmic behavior.
- `verify` against the 22 Panako-derived oracle questions is the matcher anchor.
- Unit tests cover empty/single-hit cases, modal ties, factor gates, generation invariants, passage
  stitching, alternates, and same-audio candidates.
- Process tests cover interruption, resume, atomic publication, and failure isolation.
- Native extraction changes must preserve both `validate-extract` measurements and byte-identical
  `validate-stream` output unless a deliberate new fingerprint profile is introduced.
- Optimizations on compatibility paths require equivalence against the straightforward reference
  path, not merely similar aggregate counts.

## Documentation

Keep each fact in one authoritative home:

- `README.md` — public orientation, examples, FAQ, acknowledgements;
- `ARCHITECTURE.md` — what the implementation is now;
- `CONTRACT.md` — public CLI/daemon shapes and invariants;
- `SPEC.md` — behavioral requirements and acceptance standards;
- `docs/ENGINE-FACTS.md` — verified Panako/Gaborator compatibility facts;
- `docs/SCOPE.md` — engine/application boundary;
- `DECISIONS.md` — append-only rulings and their reasons;
- `REPORT.md` — measured validation and known limits;
- `fixtures/README.md` and `rig/README.md` — fixture contents and provenance.

Update architecture or contract documentation in the same commit as the behavior it describes.
Append a decision only for an actual ruling; do not use the file as a changelog. Delete stale prose
instead of layering a correction report on top of it.

## Commits and worktree care

- Preserve unrelated and untracked user work.
- Keep commits small, building, and green.
- Use `area: what changed` messages, with a body explaining non-obvious reasons.
- Never rewrite or discard a published store as part of a test. Use fixtures or an explicit
  temporary directory.
