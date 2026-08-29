# resident

A resident audio-fingerprint matching engine in Rust: successor to a Panako 2.1 deployment's
*matching* side. Ingests pre-extracted fingerprint dumps, serves match / span / crosscheck
questions over JSON-lines at memory speed, any number of concurrent readers, no JVM, no audio
in the loop. Validated behaviorally against the original jar via golden fixtures. AGPL-3.0
(derives understanding, and eventually algorithms, from Panako — see LICENSE).

*(A "resident", in DJ vocabulary, is the DJ who holds down the room every week. Same job.)*

## Building it / working here

At handover, read `REPORT.md` first. For original intent and constraints, continue with
`HANDOFF.md` → `SPEC.md` → `docs/ENGINE-FACTS.md` → `CONTRACT.md` → `AGENTS.md`.
Fixtures: `fixtures/README.md`. Gate: `./check.sh`.

The workspace requires stable Rust and ffmpeg. Run `./check.sh` for the complete local gate.

Core one-shot commands use explicit paths, for example:

```sh
cargo run --release -- ingest --store ./resident-store --dump-dir fixtures/store-dump
cargo run --release -- verify fixtures --store ./resident-store
cargo run --release -- daemon --store ./resident-store
cargo run --release -- extract ./audio.wav
cargo run --release -- validate-extract fixtures --store ./resident-store
cargo run --release -- validate-stream fixtures
cargo run --release -- validate-multiline fixtures --store ./resident-store
cargo run --release -- refingerprint \
  --manifest ./corpus.jsonl --output-dir ./native-dump --jobs 12
cargo run --release -- ab-compare \
  --a-store ./jar-store --b-store ./native-store --probes-dir ./probes
cargo run --release -- span \
  --store ./archive-store --b-store ./reference-store --a-key archive --b-key reference
```

The daemon implements every v0 verb in `CONTRACT.md`, including native `extract` and atomic
`enroll`. Extraction requires ffmpeg at runtime but has no JVM, JNI, or Gaborator dependency.
Its core memory is bounded for multi-hour inputs; `validate-stream` proves exact streamed output
on every real query window and on a stitched boundary/flush fixture.
The `match`, `span`, and `crosscheck` wire verbs accept an off-by-default `multi_line` flag for
ranked residual offset lines; their CLI equivalents use `--multi-line`. Ordinary `verify`
proves 22/22 flag-off parity, while `validate-multiline` also drives a two-line fixture blend
through stored-resource `span`.
`refingerprint` turns a JSONL manifest (`{"key":"...","audio_path":"..."}`) into a resumable
Panako-grammar dump directory. It logs periodic progress to stderr, writes a JSON summary to
stdout, and records non-fatal per-audio decode errors in `failures.jsonl`.
`ab-compare` accepts a recursive directory of `.tdb` probes or a JSONL question manifest
with `{"name":"...","key":"...","window":[start,stop]}` lines. Its pretty JSON report
contains the exact store generations, row-level overlaps and score deltas, aggregate agreement,
the largest divergences, and optional full rows via `--evidence QUESTION`.
`span` and `crosscheck` accept an optional `--b-store`; daemon requests use the
corresponding `"b_store"` field. Probe resources remain in A while targets come from B.
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
