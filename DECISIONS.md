# DECISIONS — rulings and reasons

Append-only. Each entry describes a choice embodied by the same commit.

## 2026-08-28 — pin the initially inferred transform latency remainder

Every golden time lies on the `t * 0.008 + 0.003` grid, within the fixture's three-decimal
formatting precision. This initially identified a 48-sample remainder modulo the 128-sample
time grid. Keep latency with the fingerprint configuration and config identity.

## 2026-08-28 — accept compressed dumps as first-class input

The grammar describes `.tdb`, while the delivered and production-sized fixture seam uses
`.tdb.zst`. Ingest supports either transparently and requires the print count to equal its
metadata declaration. It rejects malformed fields, hashes wider than 34 bits, and frequency
bins above 512 with file and line context.

## 2026-08-28 — keep caller identity separate from Panako ids

The redundant numeric id in each dump posting is ignored, matching Panako's file reader.
Metadata source paths become opaque caller keys. This prevents path-hash collision behavior
from becoming part of Resident's store or protocol.

## 2026-08-28 — use 64 immutable resource shards per generation

A monolithic inverted file would make real retirement rewrite the entire production store;
one inverted file per resource would make every probe perform thousands of binary searches.
Hash each caller key into one of 64 shards and keep both access orders inside that shard. A
retire or replacement can rewrite roughly 1/64 of the corpus, while a query performs 64 cheap
index searches per distinct probe hash. Unchanged content-addressed shard files can be shared
by later manifests.

The data plane is explicit little-endian fixed records. A small JSON manifest is control-plane
metadata; opening postings performs no deserialization pass. The fixture store occupies 168
MiB for 3.2M postings, projecting to roughly 21 GiB for 412M before filesystem overhead.

## 2026-08-28 — atomic publication is the durability boundary

Shard files and a generation manifest are immutable and synced before `CURRENT` is atomically
renamed and the store directory synced. Corrupt or incomplete unpublished files are harmless;
corrupt published files are rejected. Recovery remains rebuild-from-dumps, not a WAL.

## 2026-08-28 — correct full transform latency to 12,464 samples

The first end-to-end matcher run reproduced every golden endpoint exactly 0.776 seconds too
early. The time grid alone cannot distinguish latency values separated by whole 128-sample
bins. Adding 97 bins to the inferred remainder gives 12,464 samples (`0.779 s`), after which
all golden times align. This supersedes the incomplete remainder inference above.

## 2026-08-28 — isolate Java arithmetic and modal tie behavior in the parity voter

Twenty-one fixtures matched after using Java's `float` precision for line fitting; double
precision excluded hits lying on the inclusive two-bin boundary. The last fixture contained
a ten-way modal tie. Panako selects the first entry encountered in Java `HashMap` bucket
order, so the compatibility voter reproduces Java's integer/long hash spreading and stable
bucket order. Probe hash duplicates also retain Panako's observed queue/map behavior.

These are oracle-compatibility details, not general store semantics. A future voter need not
inherit them. Result ordering is explicitly score-descending then caller key, improving on
the jar's unspecified equal-score order.

## 2026-08-28 — make long store comparisons a series of raw 30-second regions

One Panako line fit across a two-hour resource can select only one offset and miss disjoint
shared material. Evaluate non-overlapping 30-second regions independently, return every
passing segment, and do not merge adjacent segments. Thirty seconds provides substantially
more evidence than the five-second minimum while keeping local changes in time/pitch factor
from poisoning an entire recording. `crosscheck` fans each region across the store in one
batched matcher call rather than making one call per target.

This is a conservative v0 region definition, not a claim that 30 seconds is intrinsically
correct. Adaptive hit-cloud segmentation is recorded in REPORT.md as a capability opportunity.

## 2026-08-28 — retain one rollback generation and collect unreferenced shards

Content hashes make identical ingest a true no-op. Replacement and retirement keep existing
internal ids and rebuild only resource shards whose membership changed. After atomic
publication, retain `CURRENT` and its immediate predecessor; remove older manifests and shard
files referenced by neither. Unlinking an old immutable file does not invalidate an existing
Linux mmap, so concurrent readers safely finish on their captured generation without leaving
tombstones or unbounded derived data.

## 2026-08-28 — use scoped blocking concurrency for the stdio daemon

There is no network transport and no need for an async runtime. Read stdin synchronously,
dispatch request work through Rayon, and serialize only the final stdout line. Readers capture
an immutable `Arc<Store>`; writers use a separate mutex and swap the shared snapshot after
publication. Responses may interleave exactly as CONTRACT permits, while bytes within one
response never do.

## 2026-08-28 — port the analysis behavior, not Gaborator

The extraction lane uses a direct safe-Rust frequency-domain formulation: one shared audio
FFT and a truncated Gaussian kernel plus inverse FFT per retained band. This is the small
mathematical subset needed for analysis; synthesis, Gaborator's generic coefficient store,
multirate planner, and C++/JNI surface are absent. The direct formulation leaves a documented
multirate optimization available but makes the compatibility boundary readable and testable.

Panako's observable output also depends on JGaborator's 12,469-sample scheduling support,
225-frame ring delay, reversed retained-band numbering, and a 4096-value max filter receiving
only 510 coefficients. Preserve those quirks inside extraction even though the public match
time conversion remains the fixture-proven 12,464 samples. Correcting the max-filter call
site belongs to a future fingerprint config because it changes the hash population.

Fixture validation is the acceptance boundary: exact event anchors agree at 97.3% recall and
98.0% precision, while exact packed prints reach 87.8%/88.6% and recover 42/44 expected match
references. This satisfies the SPEC's non-bit-exact extraction lane honestly without making
native prints part of the already-exact matcher oracle claim.

## 2026-08-28 — bound extraction with fixed cores and an ordered print sink

Use 196,608-sample analysis cores, the fixture-proven 262,144-point FFT, and 12,469 samples of
Gaborator scheduling context on both sides. A core is exactly the decoded length of each
delivered 12-second window, so this change leaves the established extraction measurements
unchanged. The event and triplet stages retain only their required 25- and 66-frame windows;
the public streaming seam emits completed prints immediately in the same order as collection.

ffmpeg first writes signed PCM into an auto-deleting disk spool. This makes RAM independent of
track duration while providing the known sample count needed for exact wrapper tail behavior.
It intentionally trades temporary disk and a second PCM pass for a simple, testable flush
boundary on the 50 TB deployment target. `validate-stream` requires fingerprint-vector and
duration equality for all 22 fixture windows and for a stitched 36-second input that crosses
an internal core boundary. The existing extraction validation remains 87.8%/88.6%, 97.3%/
98.0% anchors, and 42/44 references.

## 2026-08-28 — make residual line voting explicit and opt-in

Keep `Matcher::match_prints` and an absent/false wire flag on the exact one-vote Panako path.
When `multi_line: true`, request internal evidence from that same voter, remove the accepted
hits by `(query time, reference time, original hash, matched hash)` multiplicity, and vote the
residual cloud again. Stop when the voter rejects the remaining cloud or `k` accepted lines
have been found. Rank all emitted lines score-first, so the line Panako happens to choose need
not remain first when a stronger residual line is recovered.

The fixture proof overlays the real `pair0_t184` and `pair0_t288` cross-match windows at one
query-time origin against `/corpus/wefunk/shows/0789/audio.m4a`. The default voter emits its
30-hit modal line; opt-in residual voting emits two distinct offsets ranked 69 then 30. The
ordinary flag-off oracle verification remains 22/22 before this additive assertion runs.

## 2026-08-28 — share FFT plans and isolate reusable scratch per worker

Cache RustFFT plans behind one process-wide planner and retain transform scratch in thread-local
buffers. Plans are immutable `Arc`s after lookup, while each Rayon worker owns its scratch, so
concurrent extraction requests do not serialize their transforms or alias mutable memory.
Zero scratch before every call to preserve the fresh-allocation precondition conservatively;
the optimization removes planning and allocation churn without changing transform inputs.

The full native extraction validation remains unchanged after caching: 87.8% mean print
recall, 88.6% precision, 97.3%/98.0% anchor recall/precision, and 42/44 expected references.

## 2026-08-29 — make metadata the batch completion marker

Assign dump ids from one-based manifest line numbers and pin each output directory to the
SHA-256 of the exact manifest bytes. This makes ids and filenames independent of worker
completion order and rejects accidental resume with a changed manifest.

Each resource streams prints to a fixed `.partial`, syncs and renames the print file, then
atomically publishes metadata last. A restart trusts nothing merely because it exists:
metadata id/key, print grammar, and declared count are validated before a skip. A print file
without metadata is incomplete and safe to overwrite. Per-audio ffmpeg/decode failures do not
publish metadata and are retried; the final failures JSONL is sorted by manifest id and
atomically replaced. Infrastructure and output I/O errors still abort the batch loudly.

Use one Rayon pool of exactly `--jobs` threads for both file concurrency and nested band work.
This bounds simultaneous extraction state by the operator-selected worker count on the
12-core target. The process-kill integration test publishes at least one resource, kills the
process, resumes, and compares every final output byte with an uninterrupted run.

## 2026-08-29 — define A/B agreement at the observable row boundary

Compare stores by replaying the same external probe, or each store's corresponding named
resource window, through the existing single-line matcher. A question agrees only when the
reference-key sets are identical, every paired query and reference span has positive
intersection, and absolute score delta is within an operator threshold (zero by default).
Report span intersection-over-union and signed B-minus-A scores rather than hiding them behind
the aggregate verdict.

Pretty JSON is the durable report: it identifies both paths, generations, config ids, sizes,
every question and row, plus a severity-ranked divergence shortlist. Missing rows dominate
severity, then non-overlapping spans, then raw score drift. A named evidence option embeds both
complete row sets. Unknown replay keys on one side are data-plane divergences; malformed probe
input or store corruption still aborts.

The fixture acceptance seam compares all 22 probe sets against the same store for 100%
agreement. Rebuilding a second store and retiring `/corpus/wefunk/shows/0789/audio.m4a`
produces exactly three missing rows for that key and zero changes to surviving rows.

## 2026-08-29 — make probe and target store roles explicit

Add two-store core entry points and preserve the existing one-store `span`/`crosscheck`
functions as wrappers that pass one snapshot for both roles. A always owns the named probe
resource and its window; B owns the restricted target or all fan-out targets. This keeps
reference libraries and additional corpora separate without copying them into the archive
generation.

Compare the pinned config identity before reading either role and return `config_mismatch`
with A as expected and B as found. CLI `--b-store` and daemon `"b_store"` are optional and
therefore byte/behavior compatible when absent. The daemon opens B as an immutable snapshot
for the lifetime of that request.

The fixture proof sorts the 16 resources, partitions alternating entries into two equal
eight-resource stores, and selects `1236 → 0789` across the split. Cross-store span over the
real 184–196 second pair window returns one segment exactly equal to the full single-store
answer.

## 2026-08-29 — gate cutover on behavior and preserve rollback

Do not equate a complete native corpus run with permission to cut over. Matcher compatibility
is exact, but native extraction recovers 42/44 expected fixture references and therefore
creates a meaningfully different store. The jar-derived generation remains the rollback and A
side until a representative pilot and then the full native generation pass an explicit
reference-set/span/score policy using `ab-compare`.

Per-file decode failures intentionally leave the batch process successful so thousands of
other resources can finish. Operational automation must inspect the JSON summary and require
`failed == 0`; this is recorded prominently in `REPORT.md` rather than hidden behind an
exit-code convention. No production throughput, RSS, or 412M-posting ingest claim is made
without measuring the target server.

## 2026-08-29 — peel residual lines independently inside query regions

Extend the existing opt-in residual voter to `span` and `crosscheck` without changing their
default functions or absent/false wire behavior. Each 30-second probe region is an independent
hit cloud. In multiline mode its accepted lines remain score-ranked as returned by the
matcher, while regions remain chronological; sorting secondaries by reference offset would
discard the ranking the flag promises.

For `crosscheck`, `k` continues to mean final reference count. It does not cap regional lines:
all accepted lines contribute segments and `score_total`, after which references are ranked
and truncated exactly as before. The cross-store config check and A-probe/B-target ownership
apply before either single- or multiline matching.

The fixture proof persists the real `pair0_t184` + `pair0_t288` overlaid prints as a source
resource, then runs actual cross-store `span` against `0789`. The default path emits score 30;
the opt-in path emits ranked scores 69 and 30 at distinct reference offsets. The ordinary
single-line oracle remains 22/22.

## 2026-08-30 — add a provenance-rich passage lane without changing compatibility verbs

Keep `match`, `span`, and `crosscheck` byte/behavior compatible and add `passages` plus `discover`
as the `passage-v1` profile. Passage mode always asks the multiline voter for filtered-hit evidence,
forms dense support runs, and stitches only geometry-compatible region lines. Its envelope is a
navigation aid; support entries are the only intervals claimed as matched. This prevents a fixed
30-second region or an app-side seam rule from silently turning an unsupported overlay into matched
underlay extent.

Make the profile constants part of the versioned judgment: a support run breaks after a 1.5-second
hit gap; line envelopes may remain in one alignment family across at most 20 seconds only when their
offset changes by at most 2 seconds and time/pitch factors by at most 0.03. A later calibration must
mint a new profile rather than changing what `passage-v1` means.

Derive passage ids from profile, config, endpoint keys/content hashes, support geometry, and factor
ranges—not store generation or the optional diagnostic segment payload. Unrelated ingest therefore
does not churn an observation's identity, while both exact generations remain in every response for
reproducibility and supersession.

## 2026-08-30 — preserve directional observations and reject top-k absence

Label every passage snapshot `a_to_b`. A matcher observation says A was probed against B; a missing
B→A row is neither a veto nor proof of absence. A graph consumer may fuse geometry-compatible forward
and reverse observations as one evidence relation, retaining which direction(s) supported it, but
must take their union rather than intersection and must not double-count a mirrored pair.

`discover` therefore exhausts the stated A→B target snapshot and excludes only the exact self key.
It does not expose `k`, because ranking truncation cannot define absence in a persistent recurrence
graph. Incremental corpus completeness still requires scheduling the reverse corpus→new direction;
resident does not hide that second job behind a misleading symmetric result.

## 2026-08-30 — keep semantic recurrence classes outside resident

Resident owns filtered-hit assignment, support masks, passage splitting/stitching, pairwise passage
identity, and named quality facts. It does not own corpus-global recurrence-class revisions, song
identity, set containment, borrowability, or human rulings. Those depend on canon and on durable graph
history that the fingerprint store deliberately does not contain.

All passage observations are non-exclusive presence evidence. They may prove that a corpus song is
present beneath a drop, but sparse landmark prints cannot prove that the drop is voice, estimate a
reconstructable residual, or show that no simultaneous layer exists. A future residual-audio lane
requires richer signal features and its own validation; the passage API must not fake it from gaps.

## 2026-08-30 — replay production query geometry as passage-v2

Supersede the unaccepted `passage-v1` profile from revision `fcb8add` with `passage-v2`. Keep the
compatibility verbs on their established non-overlapping 30-second regions, but form passages from
12-second regions on an 8-second hop anchored at zero: the geometry used by 2,644 of 2,670 production
Panako identify artifacts. Short five-to-eight-second matches can otherwise straddle a 30-second seam
and fail the match-duration gate in both halves.

Overlapping query regions repeat exact evidence hits. Deduplicate by query time, reference time,
original hash, and matched hash before support construction; report that count as `matched_hits`.
Drop passage `score_total`, because summing residual peels and overlapping windows makes one alignment
look stronger merely because it was asked twice. Retain `score_peak` as a diagnostic voter fact.
Passage identity depends on endpoint fingerprint identities and deduplicated support geometry, not
score or factor extrema.

Bound CLI discovery with repeatable `--target` and accept repeatable `--exclude-key` on CLI and wire.
Different encodings of one logical audio can have different fingerprint vectors and are acoustically
indistinguishable from a whole-recording rebroadcast. Resident must not guess; the corpus owner supplies
known sibling keys, and an unexcluded sibling is honestly reported as a recurrence candidate.

## 2026-08-30 — remove duration from fingerprint identity

Panako's cached-print constructor sets `t3 = -1`, but `PanakoStrategy.store()` rewrites cache metadata
duration from the last fingerprint's `t3`. Under the pinned latency this computes
`(-128 + 12469) / 16000 = 0.7713125`, the exact rotten value found on 1,498 production resources.
The fingerprint vectors and matching index remain valid.

Hash canonical `(hash,t,f)` postings only. Duration is useful resource metadata but is neither a
fingerprint fact nor match identity; repairing it must not churn passage ids. Old manifests retain
their historical hash bytes, so passage output computes the prints-only endpoint hash from forward
postings until a manifest-only rehash publishes the corrected identity profile.

## 2026-08-30 — restrict pair lookups to the target shard

Every resource's forward and inverted postings live in the shard selected by its key. When a pair
query already names the target, look up each probe hash only in that shard and retain that resource's
hits; scanning the other 63 shards cannot add a valid hit. Fan-out keeps the all-shard path. This is
an index-access optimization below the unchanged voter, with a unit comparison against all-shard
lookup, and makes serial evidence-bearing cohort replay practical without changing answers.
