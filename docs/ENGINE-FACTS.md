# ENGINE-FACTS — everything known about Panako's data and matcher

Reconstructed from upstream source (JorenSix/Panako @ `e4b0e1d`, the exact commit our jar was
built from — unmodified), from live production scripts, and from direct probes of the real
store. Confidence marks: **[V]** verified against live data/source quote · **[I]** inferred,
verify against source before leaning hard. When a fixture disagrees with this file, the
fixture wins — then log the correction in DECISIONS.md.

## §dump — the fingerprint dump grammar [V]

Per resource, two files (as produced by Panako's file-cache storage):

- `<resourceID>.tdb` — one print per line, space-separated, trailing space:
  `hash resourceID t f`
  e.g. `1082201315 1000181504 224 128`. `hash` fits in 34 bits (values above 2^32 occur —
  parse as u64). `t` = time bin (int), `f` = frequency bin (int, 0..512). The redundant
  per-line resourceID is ignored by Panako's own reader (upstream `dataFromLine()` consumes
  indices 0, 2, 3).
- `<resourceID>_meta_data.txt` — four lines: resourceID · duration-seconds (float) ·
  numberOfPrints · source path. The source path is the caller-facing identity (fixtures use it
  as `key`); resourceID is Panako-internal (a hash of the path string).

## §scale — the real store [V]

Full production store: 411.8M postings, 14,370,768 distinct hashes, mean 28.7 postings/hash,
max ~4,000, zero hashes ≥5,000 (measured 2026-08-27, full scan). ~2,891 resources; a typical
resource: ~3,400 s, ~59k prints ⇒ ~17 prints/sec. Postings-per-hash distribution is a mild
hump around 32–127. Consequence, ruled: **no IDF/rarity weighting** — measured mild, "a modest
sharpener at best, not a design driver."

## §hash — the landmark hash bit layout [V — verbatim upstream `PanakoFingerprint.hash()`]

Three event points (t1,f1,m1), (t2,f2,m2), (t3,f3,m3); t ascending. 34 bits used, in a long:

```java
long ratioT = (long) ((t2 - t1) / (float) (t3 - t1) * 64);   // 6 bits
long f1Range = (f1 >> 5);                                     // 8 bits kept
long df2f1 = (Math.abs(f2 - f1) >> 2);                        // 6 bits kept
long df3f2 = (Math.abs(f3 - f2) >> 2);                        // 6 bits kept
hash =  ((ratioT              & ((1<<6)-1)) << 0 )
      + ((f1LargerThanF2      & 1) << 6 ) + ((f2LargerThanF3 & 1) << 7 )
      + ((f3LargerThanF1      & 1) << 8 ) + ((m1LargerThanm2 & 1) << 9 )
      + ((m2LargerThanm3      & 1) << 10) + ((m3LargerThanm1 & 1) << 11)
      + ((dt1t2LargerThant3t2 & 1) << 12) + ((df1f2LargerThanf3f2 & 1) << 13)
      + ((f1Range             & ((1<<8)-1)) << 14)
      + ((df2f1               & ((1<<6)-1)) << 22)
      + ((df3f2               & ((long)(1<<6)-1)) << 28);
```

Layout bottom-up: ratioT(6) · 3 freq-order bits · 3 magnitude-order bits · Δt-order(1) ·
Δf-order(1) · f1Range(8) · df2f1(6) · df3f2(6). The ONE absolute-ish component is f1Range
(f1>>5 ≈ a ~450-cent frequency bucket); everything else is relative — that is what buys
pitch-shift robustness. (`robustHash()` also exists upstream but is NOT on the store/query
path — ignore it.)

## §nearhash — why queries scan a hash RANGE [V]

The query path looks up each probe hash with a **±PANAKO_QUERY_RANGE integer scan** (ours: ±2),
not an exact lookup. Rationale: the lowest-order hash field is ratioT — quantization jitter at
bin edges perturbs the integer hash by small amounts. Your inverted order must support cheap
"all postings for hashes in [h−2, h+2]".

## §matcher — the query algorithm, step by step [V — from upstream `PanakoStrategy.query()`, lines 263–496]

Given probe prints (hash, t, f) and the store:

1. For each probe hash: collect postings for hashes in ±QUERY_RANGE. Each hit carries
   (resourceID, ref_t, ref_f, query_t, query_f).
2. Group hits by resource. Drop resources with < MIN_HITS_UNFILTERED (10) hits.
3. Sort each resource's hits by query time.
4. Take `partLen = min(HIT_PART_MAX_SIZE=250, max(MIN_HITS_UNFILTERED, len/HIT_PART_DIVIDER=5))`
   hits from each END of the list → `firstHits`, `lastHits`.
5. Modal vote: `y1` = most common (ref_t − query_t) among firstHits; `y2` = same over lastHits.
   (A plain most-common-value vote, NOT a histogram peak-find.)
6. `x1` = query_t of the first hit in firstHits whose deltaT == y1; `x2` = query_t of the last
   hit in lastHits whose deltaT == y2. `slope = (y2−y1)/(x2−x1)`; `offset = −x1·slope + y1`;
   `timeFactor = 1/(1−slope)`.
7. `frequencyFactor = binToHz(ref_f) / binToHz(query_f)` **read off the single hit at x1**
   (yes, one hit decides pitch factor — upstream line 384).
8. Gate: timeFactor within [MIN_TIME_FACTOR=0.8, MAX_TIME_FACTOR=1.2], frequencyFactor within
   [0.8, 1.2]; else drop the resource.
9. Filter hits to those within the line: |actual deltaT − (slope·query_t + offset)| ≤
   threshold, where **threshold = PANAKO_QUERY_RANGE (=2, the same key reused as a residual
   bound in time bins)**.
10. Drop if filtered count ≤ MIN_HITS_FILTERED (5). **score = filtered hit count** (raw int).
11. Duration gate: matched query span ≥ MIN_MATCH_DURATION (5 s).
12. Per-second histogram over ref time of filtered hits → `percentOfSecondsWithMatches`
    (= 1 − emptySeconds/ceil(span)); gate on MIN_SEC_WITH_MATCH. Upstream builds this
    histogram and then throws it away — you keep it (evidence).
13. Emit (query span, ref span, score, timeFactor, frequencyFactor, pct); sort score-desc,
    truncate to NUMBER_OF_QUERY_RESULTS.

Known consequence of steps 5–9: only ONE offset line survives per (query, resource) — a DJ
blend (two records playing) has its second line actively deleted as noise. v0 must reproduce
this (the oracle demands it); the voter should be structured so ranked secondary lines can be
emitted later (SPEC §allowances).

## §time — bins ↔ seconds [V formula, I constant]

`t` bins step by TRANSF_TIME_RESOLUTION=128 audio samples at SAMPLE_RATE=16000 ⇒ 8 ms/bin.
`seconds(t) = t · 128/16000 + latency/16000` where `latency` is the transform's own group
delay in samples, probed by Panako at startup from the Gaborator [I — value not yet measured].
Derive the effective constant by fitting golden output times against raw probe print bins, and
record the fitted value in DECISIONS.md. `binToHz(f)`: bands are log-spaced,
`hz = centToHz(hzToCent(TRANSF_MIN_FREQ=110) + f · 1200/BANDS_PER_OCTAVE)` [I — read
BANDS_PER_OCTAVE from upstream `Config.java`; geometry suggests ~85/octave ⇒ ~512 bins over
110–7040 Hz, ~14 cents/bin].

## §config — the ONE pinned config [V — live production values]

The store, the fixtures, and therefore this engine are pinned to (PANAKO_* prefix omitted):

| key | value | role |
|---|---|---|
| SAMPLE_RATE | 16000 | extraction (context) |
| TRANSF_TIME_RESOLUTION | 128 | 8 ms time bins |
| TRANSF_MIN_FREQ / MAX_FREQ / REF_FREQ | 110 / 7040 / 440 | 6 octaves, log bands |
| FREQ_MAX_FILTER_SIZE / TIME_MAX_FILTER_SIZE | 103 / 25 | peak picking (context) |
| FP_MIN_TIME_DIST / FP_MAX_TIME_DIST | 2 / 33 | triplet pairing bounds (bins) |
| FP_MAX_FREQ_DIST | 128 | triplet pairing bound (bins) |
| QUERY_RANGE | 2 | ±hash scan AND line residual bound |
| MIN_HITS_UNFILTERED / MIN_HITS_FILTERED | 10 / 5 | matcher gates |
| MIN_MATCH_DURATION | 5 | seconds |
| MIN_SEC_WITH_MATCH | (jar default) [I] | coverage gate — confirm from Config.java |
| MIN/MAX_TIME_FACTOR, MIN/MAX_FREQ_FACTOR | 0.8 / 1.2 | factor gates |
| HIT_PART_MAX_SIZE / HIT_PART_DIVIDER | 250 / 5 | end-segment sizing |

Production made **no threshold overrides** — jar defaults govern. The fixtures' manifest
snapshots the effective config; the engine hard-pins these as constants in one module
(`config.rs`), stamps a config identity hash into the store manifest, and rejects mismatched
inputs (`config_mismatch`). Changing config = deliberate edit + store rebuild, never runtime
drift.

## §csv — the jar's query output (what goldens were parsed from) [V]

`;`-separated, ≥12 columns: `idx ; batchTotal ; query_path ; q_start ; q_stop ; ref_path ;
ref_id ; ref_start ; ref_stop ; score ; time_factor ; pitch_factor [; pct_sec_with_match]`.
Factors print as `"1.000 %"` (percent sign is noise). Score is the raw filtered-hit count.
Column 12 (pct) existence: confirm against raw fixture lines (kept verbatim in goldens).

## §quirks — jar behaviors catalogued (context for what NOT to inherit)

- Defaults silently to STRATEGY=OLAF without an explicit arg (a different algorithm — wrong
  answers, no warning).
- Opens its store RW unconditionally; read-only deployment impossible; two concurrent
  processes degrade each other (the production "one JVM law").
- Wrong store path ⇒ silent zero-match answers.
- Re-`store` of the same file doubles its postings (non-idempotent).
- Needs `--add-opens` JVM reflection flags or dies at startup.
- **Matches across silence**: near-zero-amplitude regions fingerprint and cross-match
  unrelated recordings (measured: one 24 s silent head matched 8 unrelated nights). This is a
  property of the print data, not the matcher — reproduce faithfully (the consumer handles
  it); do NOT add cleverness here, but it explains oddities you may see in fixtures.
- Resource ids are a path-string hash (int); collisions theoretically possible, never
  observed at ~2.9k resources. Your engine's caller-key model sidesteps this class.

## §extraction — the Gaborator subset (the second deliverable — SPEC §extraction)

Upstream extraction chain (yours to rebuild, after matcher parity):

1. Decode to mono 16 kHz.
2. **Gaborator** (C++ via JNI): log-frequency Gabor transform — constant-Q-like magnitude
   spectrogram, bands log-spaced from 110 to 7040 Hz around ref 440 (~14 cents/bin, ~512
   bins [I]), one frame per 128 samples (8 ms), with a fixed group-delay latency the code
   compensates in blocksToSeconds().
3. Event points: per-frame vertical max-filter (FREQ_MAX_FILTER_SIZE=103 bins) + horizontal
   max over TIME_MAX_FILTER_SIZE=25 frames; a bin survives iff it equals both maxima and is
   nonzero. Magnitude = 3×3 neighborhood sum.
4. Triplets: every (i<j<k) event combo within FP_MIN/MAX_TIME_DIST=[2,33] bins and
   FP_MAX_FREQ_DIST=128 bins (inner loops break early on time distance).
5. Hash per §hash; emit (hash, t1, f1).

Your extractor (rustfft or any solid CQT/Gabor crate — established libraries welcome; the
Gaborator itself is analysis with log-spaced Gaussian-windowed bands, so a well-built CQT is
the same mathematical object) reproduces steps 2–5 under the §config pin, with **Panako's
exact hash packing** (§hash) so prints stay store-interoperable. Bit-exactness is ruled
not-required — peak picking near ties legitimately differs, and a different-but-equally-good
peak set is acceptable; the two-tier validation in SPEC §extraction is the standard. Decode +
resample to 16 kHz mono is a front-end concern (ffmpeg subprocess is acceptable; pure-Rust
decode/resample crates welcome) — note the resampler choice affects peaks slightly, which the
tolerance absorbs. The fixture windows (`queries/*/window.wav`) are pre-decoded 44.1 kHz mono
WAV: your front resamples them, sidestepping compressed-codec variance for validation.

## §upstream — where to verify [V]

Clone `https://github.com/JorenSix/Panako` at `e4b0e1d` (AGPL — the reason this repo is AGPL).
Load-bearing files: `PanakoStrategy.java` (query path 263–496; line-fit 376; pitch factor 384;
per-second histogram 456) · `PanakoFingerprint.java` (hash 231–273) · `PanakoStorageFile.java`
(.tdb grammar) · `Config.java`/`Key.java` (every default marked [I] above) ·
`PanakoEventPointProcessor.java` (extraction).
