# REPORT — closing brief

This report is maintained during the build and finalized at handoff.

## Opportunity ledger

### A — performance left on the table with identical semantics

| opportunity | expected payoff | current reason |
|---|---:|---|
| Batch all probe hashes into one ordered scan per shard | medium–high at production scale | v0 performs one binary search per distinct probe hash per shard; already sub-millisecond warm on fixtures |
| Keep the distinct-hash index resident in a compact RAM structure | medium for cold/random workloads | mmap/page cache is the required baseline and needs production measurement first |
| Stream dumps into bounded shard spools during ingest | high reduction in peak build RAM, modest speed gain | current build loads input prints then bounds duplicate sorting memory to one shard |
| Decode mapped fixed records as validated typed slices | low–medium | explicit safe byte reads keep the format obvious; bounds checks remain in the hot loop |
| Replace 510 full-rate inverse FFTs with Gaborator-style octave downsampling | high (roughly 4–8× extraction throughput) | the direct full-rate formulation made the compatibility transform small and auditable; a multirate rewrite needs its own fixture proof |
| Cache FFT plans and worker scratch across extraction requests | medium | current extraction plans once per file and allocates one spectrum per band; correctness was established before pooling state |
| Analyze directly behind ffmpeg with one-core lookahead instead of a PCM disk spool | low–medium latency and temporary-I/O reduction | the spool makes total length and exact tail flushing explicit while already bounding RAM; the 50 TB target makes the trade safe |

### B — performance available by departing from Panako answers

| opportunity | expected payoff | compatibility cost |
|---|---:|---|
| Exact-hash lookup instead of ±2 integer range | up to roughly 5× fewer candidate postings | loses ratio-quantization tolerance and therefore recall |
| Rarity/IDF candidate pruning | low–medium on the measured mildly skewed corpus | changes scores and can remove oracle rows; production measurement already ruled it a non-driver |
| Replace Java-float and HashMap-tie compatibility | low | changes boundary scores and rare tied modal lines; useful mainly as cleanup, not acceleration |
| Reduce bands/octave or analyze only fingerprint-dense bands | high to very high extraction gain | creates a new hash population and requires a full-corpus re-fingerprint |

### C — capability left on the table

| opportunity | compatibility | relative payoff |
|---|---|---:|
| Emit ranked secondary offset lines for DJ blends/doubles | additive wire capability; departs from jar's one-line deletion when enabled | high |
| Adaptive hit-cloud region finding instead of fixed independent span regions | can preserve existing raw rows behind a new mode | medium–high for long recordings |
| Re-fingerprint with an improved native transform/config after migration | intentionally creates a new config/store identity | high; ends legacy extraction constraints |
| Correct the legacy 4096-vs-510 max-filter call-site quirk | incompatible fingerprint population, best introduced as a v1 config | medium quality/maintainability payoff; removes an accidental oracle constraint |

## Results

- Matcher fixture parity: **22/22**, including exact rows, scores, factors, and formatted time
  endpoints when fed the oracle prints.
- Store fixture: **3,208,323 postings / 168 MiB**. This projects to about **21 GiB** for the
  412M-posting production corpus. A warm fixture verify pass is sub-millisecond per query;
  a full 7,500-second store-to-store crosscheck is about 0.5 seconds on the development Mac.
- Extraction (22 real 12-second windows): mean print recall **87.8%**, precision **88.6%**;
  anchor recall **97.3%**, precision **98.0%**; downstream matcher recovery **42/44** expected
  references. Results were identical on the Mac builder and an emulated x86_64 Debian/Rust
  1.93 target. Native Mac extraction took 4.05 seconds total for all 22 windows; the emulated
  x86_64 run took 11.84 seconds and is not a server throughput estimate.
- The native extractor has no Gaborator/JNI/JVM dependency. It ports only the analysis
  behavior needed here: Gaussian log-frequency bands, native coefficient cadence, the legacy
  wrapper delay/max-filter behavior, event selection, and exact 34-bit triplet packing.
- Extraction RAM is bounded by one overlapped 196,608-sample core plus parallel band work and
  the 25/66-frame event windows. Prints can be consumed incrementally in canonical order.
  `validate-stream` is exact on 22/22 real windows and a stitched 36-second boundary/flush
  case; the established extraction and downstream-match measurements are unchanged.
- `extract` and `enroll` are live in the concurrent daemon. Enrollment was exercised from an
  absent store through generation publication and immediate snapshot visibility.

The extraction result clears SPEC's stated honest-reporting bar but is not bit-exact Panako.
Nearly all event locations agree; residual print differences are dominated by floating-point
magnitude ordering and a few unstable local maxima. Matcher parity remains the hard,
fully-green compatibility boundary.
