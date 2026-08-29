# resident

A resident audio-fingerprint matching engine in Rust: successor to a Panako 2.1 deployment's
*matching* side. Ingests pre-extracted fingerprint dumps, serves match / span / crosscheck
questions over JSON-lines at memory speed, any number of concurrent readers, no JVM, no audio
in the loop. Validated behaviorally against the original jar via golden fixtures. AGPL-3.0
(derives understanding, and eventually algorithms, from Panako — see LICENSE).

*(A "resident", in DJ vocabulary, is the DJ who holds down the room every week. Same job.)*

## Building it / working here

Start with `HANDOFF.md` (the build brief), then `SPEC.md` → `docs/ENGINE-FACTS.md` →
`CONTRACT.md` → `AGENTS.md`. Fixtures: `fixtures/README.md`. Gate: `./check.sh`.

The workspace requires stable Rust and ffmpeg. Run `./check.sh` for the complete local gate.

Core one-shot commands use explicit paths, for example:

```sh
cargo run --release -- ingest --store ./resident-store --dump-dir fixtures/store-dump
cargo run --release -- verify fixtures --store ./resident-store
cargo run --release -- daemon --store ./resident-store
cargo run --release -- extract ./audio.wav
cargo run --release -- validate-extract fixtures --store ./resident-store
cargo run --release -- validate-stream fixtures
```

The daemon implements every v0 verb in `CONTRACT.md`, including native `extract` and atomic
`enroll`. Extraction requires ffmpeg at runtime but has no JVM, JNI, or Gaborator dependency.
Its core memory is bounded for multi-hour inputs; `validate-stream` proves exact streamed output
on every real query window and on a stitched boundary/flush fixture.
See `REPORT.md` for matcher parity, extraction fidelity, scale measurements, and the explicit
performance/capability opportunity ledger.

## Map

| path | what |
|---|---|
| `HANDOFF.md` | build brief for the implementing agent |
| `SPEC.md` | mission, store constraints, acceptance, anti-requirements |
| `CONTRACT.md` | wire protocol v0 |
| `AGENTS.md` | toolchain, style, deps policy, commit rules |
| `docs/ENGINE-FACTS.md` | everything known about Panako's data + matcher |
| `docs/SCOPE.md` | in/out boundaries, awareness behaviors, future lanes |
| `fixtures/` | mini store dump + golden query results (generated from the real corpus) |
| `rig/` | the harness that generated fixtures (runs on the corpus machine, not here) |
