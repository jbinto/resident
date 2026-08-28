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
 "k": 10, "evidence": false}
```
→ `{"rows": [{"ref_key": "...", "q_start": s, "q_stop": s, "ref_start": s, "ref_stop": s,
"score": int, "time_factor": f, "pitch_factor": f, "sec_with_match": f, "evidence": {...}?},
...]}`
Rows sorted score-desc then ref_key; truncated to `k`. Semantics = ENGINE-FACTS §matcher.
No self-exclusion (callers do that), no merging, no score floor.

### span — store-vs-store: A's stored prints over a window, against B
```
{"a_key": "...", "a_window": [t0, t1] | null, "b_key": "...", "evidence": false}
```
→ `{"segments": [{"a_start": s, "a_stop": s, "b_start": s, "b_stop": s, "score": int,
"time_factor": f, "pitch_factor": f, "evidence": {...}?}, ...]}`
Probe = A's stored prints in the window (forward order), matched against B only. `null` window
= all of A. Multiple disjoint segments are expected output (same voter, applied per
offset-line/region — v0 may emit the single dominant segment per contiguous match region, but
the shape stays a list).

### crosscheck — store-vs-store fan-out: A against many
```
{"a_key": "...", "a_window": [t0, t1] | null, "targets": "all" | ["key", ...],
 "k": 25, "evidence": false}
```
→ `{"matches": [{"ref_key": "...", "segments": [...as span...], "score_total": int}, ...]}`
Internally batched/parallel; one request, one response. Sorted score_total-desc, truncated to
`k`. This verb is why the protocol is coarse: the N-way loop lives inside the engine, never as
N wire calls.

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

## Determinism

Same store generation + same request ⇒ byte-identical response (modulo `id`). Stable sort
orders everywhere; ties broken by key. This is load-bearing for fixture verification and for
the consumer's snapshot-diff workflows.
