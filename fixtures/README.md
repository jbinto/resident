# fixtures — the oracle

Generated on the corpus machine by `rig/golden.py` against the REAL jar and REAL corpus audio.
Regenerate only there; never hand-edit.

## Layout

- `store-dump/` — `<resourceID>.tdb.zst` (zstd — decompress or stream) + `<resourceID>_meta_data.txt` per resource
  (grammar: ENGINE-FACTS §dump). Build your store from these. Keys = the source paths in the
  meta files.
- `queries/<name>/prints.tdb` — the EXACT fingerprints the jar extracted for that query
  window. This is your probe input: identical probe set ⇒ matcher parity is apples-to-apples.
- `queries/<name>/golden.json` — the jar's answer for those prints against a store built from
  `store-dump/` (the same mini store). `rows` carry parsed fields plus each raw output line
  verbatim (13th-column check, formatting questions: consult `raw`).
- `queries/<name>/window.wav` — the window's actual audio (44.1 kHz mono WAV, pre-decoded).
  The extraction lane's validation input: extract from this, compare against `prints.tdb`
  (print tier) and against `golden.json` via your own matcher (match tier) — SPEC §extraction.
- `manifest.json` — jar sha, upstream commit, full config snapshot, resource list,
  `known_pairs` (which resources share material, with production-measured span sizes — the
  behavioral targets for `span`/`crosscheck`).

## What parity means

See SPEC §acceptance. `prints.tdb` in, `golden.json` out. Query names: `pairN_25/50/75` are
windows from a mix known to share material with a partner in the store (expect cross-matches
plus a perfect self-match); `loneN_50` are distractors (expect self-match only, usually).
Every row's `score` is an integer hit count — exact match required. The self-match is a
degenerate-but-legal case: same resource, offset ~window start, timeFactor ~1, pitchFactor ~1.
