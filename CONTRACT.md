# CONTRACT — wire protocol v0

JSON-lines over stdin/stdout. One request object per line in; one response object per line out.
Responses may interleave across requests (concurrent execution) — `id` correlates. stderr is
for logs only (never protocol). A future unix-socket transport will reuse these shapes.

## Envelope

Request: `{"id": "<caller string>", "verb": "<verb>", ...params}`
Success: `{"id": "...", "ok": true, ...result}`
Error:   `{"id": "...", "ok": false, "error": {"kind": "<kind>", "message": "<human text>"}}`

Error kinds: `store_missing` · `store_version_mismatch` · `config_mismatch` · `bad_request` ·
`unsupported` · `internal`. A malformed line (unparseable JSON, missing id) gets an error with
`"id": null`. The process never exits on bad input; it exits on EOF of stdin or a fatal store
condition (after emitting an error).

## Identity

Resources are keyed by **caller-supplied opaque strings** (`key`). The fixtures use corpus
paths as keys (from the dump metadata). Internal integer ids never appear on the wire.

## Time units

All times on the wire are **seconds** (f64), converted from internal time bins at the edge
(conversion spec: ENGINE-FACTS.md §time). Windows are `[start, stop)` seconds.

## Verbs

### ping
`{}` → `{"engine": "<name>", "version": "<semver>", "store": {"path": "...", "generation": "...",
"resources": N, "postings": N, "config_id": "..."} | null}`
Store block is null iff no store is attached (which is itself only legal for `ping`/`ingest`).

### match — jar-equivalent question: probe prints vs the whole store
```
{"prints": [[hash, t, f], ...] | "prints_path": "<path to .tdb>",
 "k": 10, "evidence": false, "multi_line": false}
```
→ `{"rows": [{"ref_key": "...", "q_start": s, "q_stop": s, "ref_start": s, "ref_stop": s,
"score": int, "time_factor": f, "pitch_factor": f, "sec_with_match": f, "evidence": {...}?},
...]}`
Rows sorted score-desc then ref_key; truncated to `k`. Semantics = ENGINE-FACTS §matcher.
No self-exclusion (callers do that), no merging, no score floor. `multi_line` is off by
default and preserves jar parity when absent/false. When true, accepted lines are removed from
each reference's residual hit cloud and the same voter is applied again; ranked secondary rows
may therefore repeat a `ref_key` at distinct offsets.

### span — store-vs-store: A's stored prints over a window, against B
```
{"a_key": "...", "a_window": [t0, t1] | null, "b_key": "...",
 "b_store": "/optional/target/store", "evidence": false, "multi_line": false}
```
→ `{"segments": [{"a_start": s, "a_stop": s, "b_start": s, "b_stop": s, "score": int,
"time_factor": f, "pitch_factor": f, "evidence": {...}?}, ...]}`
Probe = A's stored prints in the window (forward order), matched against B only. `null` window
= all of A. The probe is divided into independent 30-second regions. Absent/false
`multi_line` emits the single Panako-compatible voter result from each region. When true, the
same residual peeling as `match` emits all accepted lines, ranked score-first within their
region; region order remains chronological and adjacent results are not merged.
When `b_store` is absent/null, both resources come from the daemon's attached store. When
present, A is still read from the attached store and B is read from that target store.
Both stores must have the same fingerprint config identity or the request fails with
`config_mismatch`.

### crosscheck — store-vs-store fan-out: A against many
```
{"a_key": "...", "a_window": [t0, t1] | null, "targets": "all" | ["key", ...],
 "b_store": "/optional/target/store", "k": 25, "evidence": false, "multi_line": false}
```
→ `{"matches": [{"ref_key": "...", "segments": [...as span...], "score_total": int}, ...]}`
Internally batched/parallel; one request, one response. Sorted score_total-desc, truncated to
`k`. This verb is why the protocol is coarse: the N-way loop lives inside the engine, never as
N wire calls.
`b_store` has the same probe-A/target-B and config-identity semantics as `span`. Target
keys and `"all"` are resolved only in B. `multi_line` has the same per-region semantics as
`span`; `k` remains the final reference limit, while every accepted line contributes a
segment and to that reference's `score_total`.

### passages — geometry-bearing evidence for one resource pair
```
{"a_key": "...", "a_window": [t0, t1] | null, "b_key": "...",
 "b_store": "/optional/target/store", "evidence": false}
```
→ `{"snapshot": {"profile":"passage-v3", "direction":"a_to_b", "config_id":"...",
"a_generation":"...", "b_generation":"...", "a_key":"...", "a_content_hash":"..."},
"b_key":"...", "b_content_hash":"...", "passages":[...], "alternates":[...],
"same_audio_candidate": {...} | absent}`

Each passage has a deterministic `passage_id`, first-to-last `a_envelope`/`b_envelope`, explicit
`support` spans on both clocks, and a `quality` vector. The vector reports peak line score,
deduplicated matched hits, supported seconds, coverage within the A envelope, largest unsupported A gap,
segment/support counts, factor ranges, and minimum `sec_with_match`. Envelopes do **not** claim
that holes matched; only `support` does. All observations are presence evidence and non-exclusive:
a passage saying B is present underneath A does not say that no overlay is present.

Passage mode replays the production identify geometry: 12-second query regions on an 8-second hop,
anchored at zero. Overlapping regions may repeat exact hits; the quality count deduplicates them.
The engine always uses evidence-bearing multiline matching internally. `evidence:true` additionally
returns the accepted raw region `segments` for diagnosis; passage identity is unchanged by the flag.
The request remains directional—A was probed against B—and absence of B→A is not negative evidence.

`passages` contains the primary record-grained questions. If a line occupies at least 80% of its
query envelope inside a primary with at least four times as many matched hits, it is retained in
`alternates` with that primary's id instead of becoming another top-level question. This is
presentation dominance, not rejection: equal-strength concurrent lines and lines outside the
primary occupancy remain primary. Exact repeated hits from overlapping windows are deduplicated.

Ordinary lines stitch across at most 20 seconds with the documented factor/offset tolerances. A
tightly locked alignment (offset within 0.5 seconds and time/pitch factor changes within 0.01) may
bridge up to 30 seconds, preserving the hole between `support` spans. This represents one continuing
alignment with missing evidence, never matched audio inside the hole.

`same_audio_candidate` marks a primary passage only when it supports at least 90% of both resources'
fingerprint extents, starts and stops within 2 seconds at offset zero, and all time/pitch factor
extrema are within 0.005 of 1. It is proposal-grade: the same signal facts can describe duplicate
encodings or a deliberate full rebroadcast, so resident does not retire, merge, or exclude either
resource automatically.

### discover — exhaustive directional passage fan-out
```
{"a_key": "...", "a_window": [t0, t1] | null, "targets": "all" | ["key", ...],
 "exclude_keys": ["known-sibling-key", ...],
 "b_store": "/optional/target/store", "evidence": false}
```
→ `{"snapshot": {...as passages...}, "matches":[{"ref_key":"...",
"ref_content_hash":"...", "passages":[...], "alternates":[...],
"same_audio_candidate": {...} | absent, "matched_hits":N,
"supported_seconds":s}, ...]}`

Discovery applies no top-`k` truncation. Matches sort by supported seconds, then deduplicated hits
and key; the exact self key plus caller-supplied exclusions are omitted. The CLI spells a bounded
target set as repeated `--target KEY` and exclusions as repeated `--exclude-key KEY`. Resident
cannot prove that differently encoded fingerprints are one logical audio revision; it marks strong
same-audio candidates and the caller must exclude every sibling key it has already blessed.
Discovery is exhaustive only for the stated A→B
snapshot and target set.
A bounded request may name every key in the pinned B snapshot; resident imposes no smaller numeric
chunk limit, so hundreds of repeated CLI `--target KEY` flags are supported and job-side chunking is
only for checkpoint granularity.
A corpus job that needs the union of both directions must also schedule corpus→A probes; reverse
absence must never erase a surviving forward observation.

### ingest
`{"dump_dir": "<dir with *.tdb + *_meta_data.txt>"}` or
`{"resources": [{"key": "...", "prints_path": "...", "duration": s}, ...], "replace": false}`
→ `{"generation": "...", "resources_added": N, "postings_added": N}`
Identical-content re-ingest of a present key: no-op (reported). Different content without
`replace: true`: error `bad_request`.

### retire
`{"key": "..."}` → `{"generation": "...", "postings_removed": N}`
Unknown key: error `bad_request` (never a silent no-op).

### stats
`{}` → store totals + per-resource `{key, duration, postings, t_min, t_max}` list.

### operator-only identity rehash

`resident rehash-identities --store PATH` is deliberately a CLI maintenance operation, not a daemon
verb. It atomically republishes an unmarked legacy manifest with `prints-v1` hashes derived from
stored `(hash,t,f)` postings, reusing all shard files. Output is
`{"previous_generation":"...","generation":"...","resources_changed":N}`. Re-running it is a
no-op with identical generations and zero changed resources. It does not change duration metadata.

### operator-only duration publication

```
resident set-durations --store PATH --durations durations.jsonl \
  --expected-generation GENERATION
```

The strict JSONL input has exactly `{"key":"...","duration_seconds":s}` per line. It must cover
every manifest key exactly once and no unknown key. Durations must be finite and positive, no more
than 1 second before the final fingerprint timestamp, and may leave at most the greater of 10 seconds
or 1% of duration after it. Zero-posting resources have no extent check.

The store must already carry the `prints-v1` identity profile. A generation mismatch or invalid row
is `bad_request` and publishes nothing. Success atomically changes only duration metadata and the
store generation, reuses every shard, and verifies that fingerprint identity inputs are unchanged:

```json
{"previous_generation":"...","generation":"...","resources_changed":N,
 "content_hashes_unchanged":true,"shards_reused":true}
```

Re-running the complete file against the returned generation is a no-op. This is deliberately an
offline CLI maintenance operation rather than a concurrent daemon verb.

### extract — audio in, prints out (the extraction lane; SPEC §extraction)
`{"audio_path": "<file>"}` → `{"prints": [[hash, t, f], ...], "duration": s}`
Decode+resample to the pinned 16 kHz mono front, then transform → events → triplets → hash.
No store interaction.

### enroll — extract + ingest in one verb
`{"audio_path": "<file>", "key": "...", "replace": false}` →
`{"generation": "...", "postings_added": N, "duration": s}`

## Determinism

Same store generation + same request ⇒ byte-identical response (modulo `id`). Stable sort
orders everywhere; ties broken by key. This is load-bearing for fixture verification and for
the consumer's snapshot-diff workflows. A `passage_id` is derived from the profile, config,
both endpoint keys and content hashes, support geometry, and factor ranges. It therefore remains
stable across unrelated store-generation changes while the response still records both snapshots.
