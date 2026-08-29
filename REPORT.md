# REPORT — final handover

## Read this first

Resident is ready for a production **cutover pilot**, not an unmeasured blind cutover.

- The matcher is the hard compatibility boundary and is exact on all 22 Panako oracle
  fixtures.
- The native extractor is operational, bounded-memory, resumable at corpus scale, and free of
  Java/JNI/Gaborator. It is deliberately not described as bit-exact: native prints recover
  42/44 expected fixture references.
- The cutover controls now exist: deterministic batch re-fingerprinting, inspectable A/B store
  reports, and cross-store query support. No production corpus run or production acceptance
  threshold has happened in this repository.
- Keep the jar-derived generation intact until a representative native pilot passes an
  operator-approved A/B policy. Reference-set retention should matter more than exact score
  equality during that policy decision.

The recommended next action is a 50–100 recording pilot on the target x86_64 server, including
the longest and most troublesome recordings, followed by `ab-compare` over real consumer
questions. Measure throughput, peak RSS, temporary-disk use, and row agreement before starting
the 2,900-file run.

## What shipped

### Matching and storage

- Safe-Rust Panako fingerprint domain, dump parser, and pinned config identity.
- Immutable, mmap-backed, 64-shard generations with atomic publication, idempotent ingest,
  replacement, retirement, rollback retention, and typed corruption failures.
- Exact Panako-compatible single-line matcher, including Java float and modal-tie behavior.
- Opt-in ranked residual lines behind `multi_line: true`; the default path remains the oracle
  path.
- Regionized `span` and `crosscheck`, with optional distinct probe-A and target-B stores in
  core, CLI, and daemon.

### Extraction and corpus production

- Native 16 kHz log-frequency analysis, event selection, triplet construction, and exact
  34-bit hash packing without a Gaborator dependency.
- Fixed overlap cores, bounded event/triplet windows, ordered print emission, shared FFT plans,
  and reusable per-worker scratch.
- `refingerprint`: strict JSONL manifest, stable manifest-line ids, bounded `--jobs` pool,
  atomic per-resource completion, restart validation, deterministic failures JSONL, and
  periodic stderr progress.

### Cutover controls

- `ab-compare`: external print probes or named store-resource windows, exact store-generation
  identity in the report, row presence, query/reference span IoU, signed score delta,
  aggregate agreement, ranked divergences, and named full-evidence output.
- Cross-store `span`/`crosscheck`: A supplies probe prints, B supplies targets, and different
  fingerprint config identities fail with `config_mismatch`.
- Fixture commands for matcher parity, extraction fidelity, streaming identity, multiline
  recovery, A/B agreement, and split-store behavior.

## Final acceptance evidence

All measurements below are from the development Mac on 2026-08-29 unless explicitly noted.
They are fixture measurements, not production-server forecasts.

| gate | result |
|---|---|
| `./check.sh` | green: fmt, clippy with warnings denied, 18 unit tests, and 2 process-level batch tests |
| Panako matcher oracle | **22/22** exact; 20.608 ms total, 0.937 ms mean in the final optimized run |
| Multiline additive proof | real pair0 overlay: flag off score 30; flag on ranked scores 69, 30 at distinct offsets |
| Native print fidelity | **87.8% recall / 88.6% precision** |
| Native anchor fidelity | **97.3% recall / 98.0% precision** |
| Native downstream matching | **42/44** expected references |
| Native extraction timing | 22 × 12-second windows in **3.907 s** total; short-window measurement only |
| Stream identity | **22/22** windows exact plus an exact stitched 36-second boundary/flush case |
| Batch recovery | parent process killed after publication; resume output byte-identical to an uninterrupted 40-resource run |
| Batch failure isolation | one valid + one missing audio: done=1, failed=1, successful exit, deterministic failure record |
| A/B identity | fixture store against itself: **22/22, 100.0%** |
| A/B retirement sensitivity | retiring one resource: exactly **3** missing rows for that key, **0** other deltas |
| Cross-store span | 8+8 split fixture stores: one real cross-pair segment, exactly equal to full-store span |
| Fixture store | **16 resources, 3,208,323 postings, 168 MiB** |
| Production store projection | about **21 GiB** for 411.8M postings, before filesystem overhead |
| Long crosscheck baseline | about **0.5 s** for a 7,500-second fixture resource against the fixture store |

Earlier extraction validation was also identical on an emulated x86_64 Debian/Rust 1.93
target, but its 11.84-second timing was emulation overhead and is not useful for capacity
planning.

## Cutover runbook

### 1. Freeze the corpus manifest

One JSON object per line:

```json
{"key":"/opaque/caller/key","audio_path":"/absolute/path/to/audio"}
```

Manifest bytes determine resource ids and pin the output directory. Do not reorder, reformat,
or append lines while resuming. To change the manifest, use a new output directory.

### 2. Run a representative pilot

Place temporary PCM on a volume with several GiB free. The current extractor spools about
115 MB per decoded audio hour per concurrent worker.

```sh
TMPDIR=/large-volume/resident-tmp \
cargo run --release -- refingerprint \
  --manifest pilot.jsonl \
  --output-dir pilot-native-dump \
  --jobs 12 \
  --progress-every 10
```

Progress goes to stderr. The final JSON summary goes to stdout. Per-file decode failures do
not make the process exit nonzero; automation must require `failed == 0` and an empty
`failures.jsonl`. A restart retries failed/incomplete files and validates completed ones.

### 3. Build the native candidate store

```sh
cargo run --release -- ingest \
  --store pilot-native-store \
  --dump-dir pilot-native-dump
```

Keep the jar-derived store read-only and untouched. It is both the A side and rollback.

### 4. Compare fixed probes first

Fixed external probes isolate target-store changes from probe-extraction changes.

```sh
cargo run --release -- ab-compare \
  --a-store jar-store \
  --b-store pilot-native-store \
  --probes-dir acceptance-probes \
  --k 100 \
  > pilot-ab.json
```

Start with `--max-score-delta 0` so all drift is visible. Decide the production threshold
from the report rather than choosing one in advance. Inspect a named divergence with:

```sh
cargo run --release -- ab-compare \
  --a-store jar-store \
  --b-store pilot-native-store \
  --probes-dir acceptance-probes \
  --k 100 \
  --evidence QUESTION_NAME
```

Then replay stored-resource windows to measure the combined effect of changing both probe and
target fingerprints.

### 5. Shadow cross-store questions

Use the existing archive generation as A and a native/reference corpus as B:

```sh
cargo run --release -- span \
  --store archive-store \
  --b-store pilot-native-store \
  --a-key SOURCE \
  --b-key TARGET \
  --start 100 --stop 130
```

Only after the pilot agreement policy passes should the full manifest run begin. Preserve the
jar-derived generation through the full native ingest and a second A/B pass.

## Failure and recovery semantics

| condition | behavior |
|---|---|
| malformed/changed manifest | aborts before extraction; changed bytes cannot reuse the directory |
| missing ffmpeg installation | global abort before work |
| one unreadable/undecodable audio | failure JSONL entry; other resources continue; retry on restart |
| process or machine interruption | metadata-absent work is incomplete; completed pairs are validated and skipped |
| corrupt completed dump pair | resume aborts loudly rather than trusting or overwriting it |
| output I/O failure | global abort; never converted into a per-audio failure |
| missing A/B replay key on one side | recorded as a question divergence |
| corrupt/unopenable A/B store | global abort |
| cross-store config mismatch | typed `config_mismatch` error |

Metadata publication is the per-resource commit point. The print file is synced and renamed
first; metadata is synced and renamed last. The process-kill test compares all final filenames
and bytes, including the manifest marker and failures file.

## Honest gaps and risks

1. **Native extraction is not the Panako print population.** Two of 44 expected downstream
   references are missed on fixtures. The full native store must therefore be accepted by
   behavior, not by an expectation of 100% exact row agreement.
2. **The production run is unmeasured.** No 2,900-file extraction, 412M-posting native ingest,
   12-core throughput curve, peak RSS, or peak temp-disk measurement was performed here.
3. **Short-window speed does not predict multi-hour throughput perfectly.** FFT work is fixed
   per core, but ffmpeg formats, concurrent workers, filesystem behavior, and thermal limits
   matter. Benchmark 1, 6, and 12 jobs on real two-hour inputs.
4. **Temporary PCM uses the system temp directory.** Set `TMPDIR` to the large volume. At
   roughly 230 MB for a two-hour mono s16 file, 12 simultaneous spools need roughly 2.8 GB
   before overhead.
5. **Ingest is not streaming end-to-end.** It loads dump resources into memory, then bounds
   duplicate sorting to one shard. This is expected to fit 128 GB but has not been measured on
   412M postings.
6. **Daemon B stores are opened per request.** This is simple and snapshot-safe, but repeated
   high-rate use should cache explicitly configured immutable B stores.
7. **A/B severity is diagnostic, not policy.** Missing rows rank above non-overlap, then score
   drift. The operator must decide acceptable reference retention and score/span drift.
8. **Multiline is only on `match`.** Regionized `span`/`crosscheck` still emit their
   default single voter line per region.

## Recommended next steps, ranked

1. **P0 — production pilot and capacity measurement.** Run representative two-hour files at
   jobs 1/6/12; record wall time, CPU, RSS, temp peak, output size, and failures.
2. **P0 — establish the A/B acceptance policy.** Use fixed real probes; review every missing
   reference and non-overlapping span. Prefer recall/set retention over exact score parity.
3. **P0 — run the full resumable manifest with rollback preserved.** Require failed=0, ingest
   into a new store root, then rerun the full A/B suite before switching consumers.
4. **P1 — validate production-scale ingest and query latency.** Confirm projected store size,
   build RSS, cold/warm match latency, full-resource crosscheck latency, and generation
   publication/rollback.
5. **P1 — add operational packaging/CI after cutover semantics settle.** `./check.sh` is
   intentionally ready to become CI verbatim; deployment/service wiring remains external.
6. **P2 — optimize extraction only if the pilot misses its window.** The multirate transform
   is the largest likely gain. Keep `validate-extract` and `validate-stream` as hard gates.
7. **P2 — remove avoidable I/O and allocation costs.** Direct one-core-lookahead decode,
   bounded dump ingest, batched shard lookup, and cached configured B stores are the useful
   semantics-preserving targets.
8. **P3 — expand additive capability.** Residual lines in regionized queries and adaptive
   hit-cloud segmentation have more product value than further Panako emulation.

## Opportunity ledger

### A — performance left with identical observable semantics

| opportunity | relative payoff |
|---|---:|
| Gaborator-style octave/multirate analysis with fixture-identical output | high, plausibly 4–8× extraction throughput |
| Stream dump ingest into bounded shard spools | high peak-RAM reduction; modest speed gain |
| Batch probe hashes into one ordered scan per shard | medium–high at production scale |
| Keep the distinct-hash index in compact resident RAM | medium for cold/random queries |
| Decode directly with one-core lookahead instead of a PCM spool | low–medium latency and temporary-I/O reduction |
| Cache configured immutable B stores in the daemon | medium for frequent cross-store traffic |
| Partition multiline residual hits without cloning/evidence materialization | medium on dense blends; negligible default impact |
| Typed mapped-record views and fixed-plan specialization | low |

### B — performance available only by changing Panako-compatible answers

| opportunity | payoff | compatibility cost |
|---|---:|---|
| Exact-hash lookup instead of ±2 | up to roughly 5× fewer candidate postings | loses ratio-quantization tolerance and recall |
| Fewer bands or fingerprint-dense bands only | high–very high | creates a new fingerprint population |
| Rarity/IDF pruning | low–medium | changes scores and can remove oracle rows |
| Remove Java-float/modal-tie behavior | low | changes boundary scores and rare tied lines |

### C1 — additive capability without changing defaults

| opportunity | relative payoff |
|---|---:|
| Residual multiline output through `span`/`crosscheck` | high for simultaneous material in DJ recordings |
| Adaptive hit-cloud regions behind a new mode | medium–high for long/non-contiguous recordings |
| Explicit daemon registry for multiple named reference stores | medium operational simplification |

### C2 — capability requiring deliberate compatibility departure

| opportunity | relative payoff |
|---|---:|
| A v1 fingerprint config with corrected 510-band max filtering | medium quality/maintainability; full re-fingerprint required |
| Joint line clustering that can peel gate-failing modes | medium–high on noisy blends |
| A new post-Panako native fingerprint population | high long-term ceiling; migration and new oracle required |

## Explicitly not built

Network/socket transport, mixmd integration, extraction-quality chasing beyond the measured
native result, CI, packaging, and deployment/service management remain out of scope. No
external systems were changed and no commits were pushed.
