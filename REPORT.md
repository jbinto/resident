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

### B — performance available by departing from Panako answers

| opportunity | expected payoff | compatibility cost |
|---|---:|---|
| Exact-hash lookup instead of ±2 integer range | up to roughly 5× fewer candidate postings | loses ratio-quantization tolerance and therefore recall |
| Rarity/IDF candidate pruning | low–medium on the measured mildly skewed corpus | changes scores and can remove oracle rows; production measurement already ruled it a non-driver |
| Replace Java-float and HashMap-tie compatibility | low | changes boundary scores and rare tied modal lines; useful mainly as cleanup, not acceleration |

### C — capability left on the table

| opportunity | compatibility | relative payoff |
|---|---|---:|
| Emit ranked secondary offset lines for DJ blends/doubles | additive wire capability; departs from jar's one-line deletion when enabled | high |
| Adaptive hit-cloud region finding instead of fixed independent span regions | can preserve existing raw rows behind a new mode | medium–high for long recordings |
| Re-fingerprint with an improved native transform/config after migration | intentionally creates a new config/store identity | high; ends legacy extraction constraints |

## Results

Pending final gate and extraction report.
