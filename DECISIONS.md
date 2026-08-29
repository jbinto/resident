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
