# SPEC — the resident matching engine

## Background (all you need)

The corpus: ~2,900 recordings ("mixes"), each ~1–2 h of continuous DJ radio. Every mix was
fingerprinted by Panako 2.1 (STRATEGY=PANAKO): a log-frequency spectral transform → spectral
peak events → 3-event landmark hashes. The full posting set is ~412M entries over ~14.4M
distinct hashes (mean ~29 postings/hash, max ~4k — hash skew is MILD; do not build IDF/rarity
weighting, it was measured and ruled a non-driver).

Today every question requires running the Java jar with *audio in*: decode, re-extract, then
match — seconds to minutes per question, one JVM at a time. But both sides of nearly every
question we ask are ALREADY fingerprinted. This engine exists to exploit that: ingest the
extracted prints once, then answer matching questions at memory speed, concurrently, with no
audio and no extraction in the loop.

## What you are building

A Rust workspace: a core library plus a `resident` binary that runs as a long-lived daemon
speaking JSON-lines over stdin/stdout (CONTRACT.md), plus one-shot CLI subcommands for ingest
and verification. Input format: Panako's plain-text fingerprint dumps (grammar in
ENGINE-FACTS.md §dump). Store format: **yours to design**, under these constraints:

- mmap-friendly and instantly warm: open = map + read header, no deserialization pass.
- Both access orders served: by-hash (inverted: global identification) and by-resource-by-time
  (forward: range-scan one mix's prints over a time window — this order is the new capability;
  Panako discards it).
- Retire of a resource must be cheap and real (contiguous-range drop or equivalent — not
  tombstone-forever).
- Versioned header + explicit generation identity. Rebuild-from-dumps is the recovery story:
  the store is a derived artifact; corruption handling = refuse loudly + rebuild, NOT
  journaling/WAL ceremony.
- Immutable generations: readers map a generation; writers build the next and atomically swap
  (manifest rename). Concurrent readers are trivially safe by construction.
- Store compatibility means: ingests the provided dumps + reproduces the oracle's answers.
  The on-disk layout is private and **expected to diverge from Panako permanently**. Design
  for our access patterns, not for Panako's layout.

## The oracle

`fixtures/` contains a mini-store dump (a few dozen real resources) and golden query results:
for each query, the exact fingerprints Panako extracted (so your probe set is identical) and
the exact rows the jar returned against a store built from those same dumps. The matcher
algorithm is documented step-by-step in ENGINE-FACTS.md §matcher — implement those semantics,
then verify against the goldens. When doc and fixture disagree, the fixture wins.

## Acceptance

1. **`verify` (the gate):** for every fixture query, run your `match` with the provided probe
   prints against your store built from `fixtures/store-dump/`. Parity target: identical match
   sets (same references matched, none missing, none extra), scores exactly equal (they are
   integer hit counts), all times within ±0.02 s (float/latency slack), factors within ±0.001.
   Ship a `verify` subcommand that prints a per-query and aggregate report. Any residual
   divergence: root-cause it in DECISIONS.md — a tolerance you had to widen is a finding, not
   a fudge.
2. **`span`/`crosscheck` (behavioral):** no jar goldens exist for store-vs-store (the jar
   cannot ask the question — that is the point). Required checks: (a) `span(A, window, A)`
   returns the identity match; (b) for fixture pairs known to share material (listed in
   `fixtures/manifest.json`), `span`/`crosscheck` find overlap consistent with the `match`
   goldens' cross-references; (c) results are deterministic and stable-ordered.
3. **Latency (informal):** report `match` and `span` timings on the dev store in REPORT.md.
   Target class: milliseconds, not seconds. Criterion benches welcome but not required.

## Output doctrine (rulings from the consuming system — honor exactly)

- **Emit raw facts; never interpret.** No merging of adjacent matches, no score floors beyond
  Panako's own internal gates, no dedup across windows. The consumer merges; "the lab states
  facts; merging is interpretation." A single-window hit is kept — never dropped as noise.
- **Emit every column Panako computes**, including time_factor, pitch_factor, and
  seconds-with-match coverage — precision downstream comes from these, not from grid tuning.
- **Evidence on request** (`evidence: true`): the filtered hit list [(q_t, ref_t) …], the
  offset histogram's top bins, per-second match density. Panako computes these and throws them
  away; you keep them available. Do not bloat default responses with them.
- **Errors are errors.** A dead or absent store, a bad request, a version mismatch — typed
  errors on the wire, never an empty result set. (A predecessor silently returned "no matches"
  from a wrong store path; that class of bug cost real data and must be unexpressible here.)

## Papercuts you must NOT reproduce (anti-requirements)

Catalogued from production scar tissue; each is a design input:

1. **Single-process law**: the jar cannot share its store — two concurrent processes degrade
   both. You: any number of concurrent readers, always.
2. **Read-only impossible**: the jar opens its store RW unconditionally (fails on ro mounts).
   You: reads never require write access to anything.
3. **Silent empty store**: wrong path ⇒ the jar happily reports zero matches. You: absent /
   empty / version-mismatched store = loud typed error.
4. **Config drift hazards**: behavior steered by env-var/config-file archaeology, silent
   strategy defaults. You: explicit args; the store manifest pins the fingerprint-config it
   was built under; a probe or dump claiming a different config is rejected (`config_mismatch`).
5. **Non-idempotent ingest**: the jar's re-store doubles postings. You: ingest of an
   already-present resource key with identical content is a no-op; with different content, an
   explicit error unless `replace: true`.
6. **JVM ceremony**: no reflection flags, no GC tuning, no startup cost worth caching around.

## Extraction — the second deliverable (shed the dependency across the board)

The mission is full independence from Panako AND the Gaborator, not just the matcher. After
the matcher gate is green, build the extraction lane per ENGINE-FACTS.md §extraction: audio in
(decode/resample may shell to ffmpeg, or use established pure-Rust crates — your call) →
log-frequency transform → event-point picking → triplet pairing → **Panako's exact hash
packing** (§hash — keeping the hash and config pin is what keeps stores interoperable and
jar-A/B possible). New verbs: `extract` (audio path → prints) and `enroll` (audio path →
extract + ingest).

**Bit-exactness with Panako's prints is explicitly NOT required** (ruled by the owner: a full
re-fingerprint of the corpus is an acceptable cutover). The acceptance standard is two-tier,
against the fixture windows' audio (`queries/*/window.wav`, 44.1 kHz mono, pre-decoded):

- **Print-tier**: extract each window; compare against the jar's own prints for the same file
  (`prints.tdb`). Report per-window print-set overlap (a hash+t-proximity match fraction).
  Target: high overlap (think majority-to-most, not all); report the number, don't chase 100%.
- **Match-tier (the one that matters)**: feed YOUR extracted prints into YOUR matcher against
  the fixture store; compare answers with golden.json at the level that decides real
  questions: same references found, spans overlapping the golden spans, scores the same order
  of magnitude. Perfect row-equality is not expected (different peaks ⇒ different hit counts).

Timebox honesty: matcher parity is the hard requirement of this build; extraction is built
second and reported honestly in REPORT.md if unfinished — a clean, tested extract lane at 80%
overlap beats an unfinished everything.

## Scope boundaries (see docs/SCOPE.md for the full list)

Only the subset we use: STRATEGY=PANAKO semantics under the one pinned config in
ENGINE-FACTS.md §config. No OLAF, no monitor/sync mode, no IDF. **Boundary awareness is a
feature**: paths outside the subset return `unsupported` errors naming what was asked; nothing
silently approximates.

One structural allowance for known future work (design for, don't build):
- **Multi-line matches**: the jar votes ONE dominant time-offset line per (query, ref) and
  deletes the rest of the hit cloud as noise — which erases DJ blends/doubles. v0 emits
  jar-parity single lines (the oracle demands it), but structure the voter so ranked secondary
  lines can be emitted later behind a flag; don't bake single-line-ness into wire shapes
  (rows are already a list per reference).

## Iteration ethos

This engine will be worked on hard after handover. Optimize for change: small modules, plain
code on the hot path, abstractions only where two call sites already demand them, and every
non-obvious choice logged in DECISIONS.md. No CI yet (it will come) — `./check.sh` is the gate.
