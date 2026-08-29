# ARCHITECTURE — current implementation

Resident is a Rust 2024 workspace:

- `core/` (`resident-core`) owns fingerprint/config types, typed errors, dump parsing, storage,
  matching, and extraction.
- `resident/` is the CLI and JSON-lines process edge. It may use `anyhow`; core does not.

The pinned Panako-compatible configuration lives in `core/src/config.rs`. Time is stored as
integer transform bins and converted to seconds only at an API edge. Fingerprints are the
plain fact tuple `(hash: u64, t: u32, f: u16)`; Panako resource ids are import metadata, not
engine identity.

## Code map

| module | responsibility |
|---|---|
| `core/src/config.rs`, `fingerprint.rs`, `dump.rs` | pinned domain, conversions, dump grammar |
| `core/src/store.rs`, `mmap_view.rs` | generations, shards, forward/inverted access |
| `core/src/matcher.rs`, `span.rs` | oracle voter, evidence, one/cross-store questions |
| `core/src/extract.rs` | bounded native audio analysis and ordered print emission |
| `resident/src/daemon.rs` | concurrent typed JSON-lines process edge |
| `resident/src/refingerprint.rs` | resumable parallel corpus dump production |
| `resident/src/ab_compare.rs` | cutover agreement reports and evidence |
| `resident/src/verify.rs`, `extract_verify.rs` | fixture acceptance commands |

## Store

A store root contains `CURRENT`, immutable JSON generation manifests in `generations/`, and
immutable binary files in `shards/`. `CURRENT` is replaced atomically only after every shard
and the new manifest have been synced. Readers retain their mappings when a later generation
is published.

Resources are assigned deterministically across 64 shards. Each little-endian shard has a
fixed header followed by three directly mapped regions:

1. forward postings sorted by `(resource, time, hash, frequency)`;
2. a distinct-hash index of `(hash, hit start, hit count)`;
3. compact inverted hits sorted by `(hash, resource, time, frequency)`.

The forward range for each resource is recorded in the manifest. A hash lookup binary-searches
each shard index and reads only matching hit ranges. The fixture generation is 168 MiB for
3,208,323 postings, which projects to about 21 GiB at production count.

`core/src/mmap_view.rs` is the sole unsafe-code exception. Its invariant is that published
shard paths are immutable and the backing file remains open for the mapping's lifetime.

Ingest compares canonical per-resource content hashes. Identical resources are no-ops;
replacement and retirement reconstruct and rewrite only affected shards, reusing every other
content-addressed shard. The current and immediately previous manifest are retained, and
unreferenced derived shards are removed after publication. Existing mappings remain valid on
the Linux deployment target even when an old path is unlinked.

## Matching

`core/src/matcher.rs` performs a ±2 hash lookup, groups hits by resource, fits Panako's
dominant offset line, applies factor/residual/duration/coverage gates, and returns stable
score-descending rows. Lookup of distinct probe hashes is parallel; voting per candidate is
parallel. Evidence retains filtered hits, leading offset bins, and per-second density only
when requested.

The off-by-default `multi_line` mode applies that unchanged voter repeatedly to each resource.
Hits accepted by one line are removed by exact evidence identity; the residual cloud is voted
again until it cannot pass or the applicable limit has survived. All surviving rows are
ranked by score, so one reference may appear at multiple offsets. The default `match`, `span`,
and `crosscheck` paths do not enter this loop.

Compatibility-specific behavior—duplicate probe hashes, Java float arithmetic, and Java
`HashMap` tie iteration—is contained in this module. Store ordering and public result ordering
remain deterministic and independent of those quirks.

## Store-to-store queries

`span` and `crosscheck` read A from the forward order and divide long ranges into independent
30-second evidence regions. Their core entry points take separate A and B stores, reject
different config identities, read probe regions only from A, and resolve targets only in B.
The original one-store functions are wrappers passing the same snapshot twice.

Each region uses the same voter; no adjacent results are merged. With `multi_line` absent or
false, one dominant result per reference and region preserves the original behavior. The
opt-in path peels the region's residual cloud and emits its lines score-ranked inside that
chronological region. `crosscheck` queries all targets once per region, in parallel, then
groups segments by reference; `k` still limits final references, not regional lines. A full
7,500-second fixture resource crosschecks the 3.2M-posting store in about 0.5 seconds on the
development Mac. CLI `--b-store` and daemon `"b_store"` open an immutable target generation
for the request; absence keeps the original attached-store behavior.

## Process edge

`resident daemon --store PATH` reads one JSON request per stdin line. Rayon scopes execute
requests concurrently; a mutex writes each complete JSON response as one stdout line. Readers
clone an `Arc<Store>` snapshot under a short lock and do not hold the lock while matching.
Generation writers are serialized, publish a new store, then replace the shared `Arc`; older
requests finish on their prior mappings. Malformed input and unknown verbs produce typed wire
errors without stopping the process. Logs, when added, use stderr only.

## Extraction

`core/src/extract.rs` shells out to ffmpeg only for the pinned mono 16 kHz signed-PCM decode
front. Decoded PCM is streamed into an auto-deleting disk spool, then read through fixed
196,608-sample cores with 12,469 samples of analysis context on either side. The transform
itself is safe Rust and depends on `rustfft`, not Gaborator or JNI. One zero-padded forward
spectrum per core is shared across 510 parallel analysis bands. Each band applies Gaborator
v1's truncated Gaussian response, performs an inverse FFT, and samples on the reference
power-of-two coefficient cadence before the wrapper-compatible 128-sample pooling.
FFT plans are cached process-wide; transform scratch is retained per Rayon worker and zeroed
before reuse. Concurrent extraction requests share immutable plans but never scratch memory.

The event stage intentionally preserves two observable JGaborator/Panako behaviors: the
225-frame circular-buffer delay (analysis support is 12,469 samples for scheduling) and a
max filter constructed for 4096 values but fed a 510-band row. The latter's zero tail and
deque update order are isolated in `lemire_vertical_max_row`; the horizontal pass consumes these
vertical maxima exactly as Panako does. This compatibility sequence is covered by fixture
validation rather than “corrected.” Triplet selection and 34-bit hash packing then follow
Panako directly. The event detector retains 25 frames and the triplet packer retains the
66-frame lookahead needed to finish one first point. Fingerprints are emitted through a sink
in canonical order, so auxiliary RAM is independent of input and output duration; callers
that need one JSON array still choose to collect the result.

`validate-stream` compares the bounded path with whole-file decode byte-for-byte across all
22 real windows. It also stitches three fixture windows into 36 seconds so the gate crosses
an internal core boundary and proves overlap plus final-window flush behavior.

## Batch re-fingerprinting

`resident refingerprint` reads a strict JSONL manifest and assigns each line its stable dump
resource id. A manifest SHA-256 marker binds an output directory to those exact bytes.
Extraction uses a Rayon pool capped by `--jobs`; each worker calls the bounded streaming
extractor and writes prints directly to an id-specific `.tdb.partial`.

The print file is synced and renamed first. Its four-line metadata file is synced and renamed
last, making metadata the completion marker. Resume validates id, key, print grammar, and
declared count before skipping a resource; an orphan partial or print file is overwritten.
Decode failures remain incomplete, are retried on restart, and are written in manifest order
to an atomically replaced `failures.jsonl`. Progress is one stderr line per configured number
of resources; the machine-readable exit summary is one JSON object on stdout.

## Store agreement

`resident ab-compare` opens and identifies two immutable store generations, then replays
either external `.tdb` probes or named store-resource windows against each independently.
One side's unknown source key is a reported question divergence; malformed probes and store
integrity errors abort the run.

For every reference row, the report records presence, query/reference span intersection-over-
union, both scores, and signed B-minus-A score delta. A question agrees when reference sets are
identical, both spans intersect for every row, and score deltas fit `--max-score-delta`
(zero by default). All questions remain in the pretty JSON report; a severity-ranked shortlist
puts missing rows and non-overlapping spans ahead of score drift. `--evidence NAME` reruns
that named question with both complete evidence-bearing row sets embedded.
