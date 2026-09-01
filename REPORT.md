# REPORT — current validation brief

Resident is an active audio-fingerprint engine with a production-scale compatibility store, a
native extraction path, and a versioned passage-observation layer. This report records what has
actually been measured. It is not a release promise or a claim that machine matches are identities.

## Current verdict

- The default matcher exactly reproduces all 22 bundled Panako oracle questions.
- Stored-resource pair lookup, exhaustive discovery, residual multiline evidence, and passage-v3
  have been replayed against a 2,891-resource production-derived store.
- Native extraction is bounded and operational, but intentionally answer-faithful rather than
  bit-identical to Panako. It recovers 42 of 44 expected downstream fixture references.
- Immutable generations, typed failures, read-only query access, resumable batch extraction, and
  A/B comparison provide a credible operating and migration path.
- Passage observations reduce local point/segment output to inspectable alignments while preserving
  holes and alternate lines. They remain directional, non-exclusive evidence.

The Panako-derived store remains a useful behavioral reference. A future native corpus should be
accepted by ear-checkable answer cohorts and full-mesh recall accounting, not print-byte equality.

## Validation matrix

The fixture measurements were made on the development Mac on 2026-08-29. Production-store replay
was independently reproduced on the corpus rig on 2026-08-30 from defined revisions.

| area | measured result |
|---|---|
| Panako matcher oracle | **22/22 exact** reference sets and scores |
| Native print fidelity | **87.8% recall / 88.6% precision** |
| Native anchor fidelity | **97.3% recall / 98.0% precision** |
| Native downstream answers | **42/44** expected references |
| Bounded streaming | **22/22** windows byte-identical to the whole-input path, plus one exact stitched 36-second boundary/flush case |
| Multiline evidence | fixture blend returns distinct residual offset lines only when enabled |
| A/B identity | fixture store against itself: **22/22, 100%** |
| A/B retirement sensitivity | one retirement removes exactly the three expected rows and changes no others |
| Split-store span | equal to the corresponding full-store pair answer |
| Pair lookup optimization | canonical evidence-bearing segment sets exactly equal to the all-shard path |
| Fixture store | **16 resources · 3,208,323 postings · 168 MiB** |
| Production-derived store | **2,891 resources · about 412M postings · about 21 GiB** |

The native extractor's 42/44 result is the important limitation in this table. The two misses make
an unreviewed full-corpus replacement inappropriate; they do not prevent native stores from being
built and compared under an explicit answer-level policy.

## Production passage replay

The passage work was driven by failures that do not appear in a short-clip benchmark: overlapping
query windows, repeated re-cueing, talk-over holes, several residual lines at one offset, and
duplicate encodings.

The replay used a byte-identical, separately mounted twin of the production-derived store. The
read-only reference store was not modified.

### Geometry recall

An earlier non-overlapping 30-second passage experiment entirely missed the known locus of one
record repeatedly re-cued over roughly 55 minutes. The fixed 12-second window / 8-second hop
passage geometry recovered **37 passages**, all at the reference locus around 5509–5524 seconds,
with query occurrences scattered from roughly 157–3164 seconds. This closed the geometry
divergence against the legacy answer at revision `88ab8df`.

The interpretation matters: these are repeated occurrences of the same small reference passage,
not one continuous 55-minute play and not evidence of a voice overlay.

### Continuous alignment with a hole

One long pair contains a smooth offset ramp interrupted by a talk-over-sized evidence hole. The
passage-v3 locked-continuation rule produced a main passage with:

- **1,957** deduplicated matched hits;
- **336.864 seconds** of supported audio;
- one explicit **20.512-second** unsupported hole.

The envelope remains one alignment, while the support list states that Resident did not hear the
underlay inside the hole. Locked continuation is capped at 30 seconds and requires tighter offset
and factor stability than ordinary stitching.

### Residual-line dominance

For the 0–130-second test window of another pair, passage-v3 reports two primary questions with
**2,001** and **156** hits. Three contained weaker lines with **70**, **113**, and **47** hits remain
available as alternates rather than appearing as three more top-level questions.

This is presentation dominance, not deletion. Equal-strength concurrent lines, non-contained
lines, and partial overlaps remain primary because they may describe genuine layering.

### Duplicate encodings

An MP3/FLAC sibling pair produces one primary diagonal with **143,609** hits and **5,056.096
supported seconds**, plus 27 residual alternates. Resident marks it as a same-audio candidate with
about **0.99252 / 0.99251** endpoint coverage and zero offset/factor deltas.

The engine does not silently suppress the pair. The same signal geometry can also describe an
intentional full rebroadcast; a consumer must bless a revision relation before passing the sibling
key as a discovery exclusion.

### Honest timeline evidence

In a measured tracklist case, machine-derived application metadata had written an end at 3369.04
seconds and thereby manufactured a gap before the next authored cue. Resident independently reports
one passage crossing that boundary (about 3361.2–3372.1) and another filling the later gap (about
3392.8–3403.4). The evidence contradicts treating a measured match end as an authored hard boundary.

That observation motivates typed provenance and overlapping ranges in consuming systems. Resident
itself reports support geometry and does not mutate human timeline data.

## Pair-query correctness and speed

A first shard optimization incorrectly assumed that a pair query could ignore the corpus-global
matching-hash population. Panako's iteration/tie behavior still depends on that population even
though only the selected target's hits enter its voter. The corrected implementation:

1. reads target postings only from the target key's shard;
2. probes all 64 compact hash indexes for global range presence;
3. feeds only target hits to the voter in the compatibility order.

On the evidence-bearing long-pair cohort, the all-shard and optimized binaries each accepted 73
segments. Their canonical segment-set SHA-256 was identical (`c5144ef7…`). Runtime fell from more
than five minutes to **2.817 seconds** on that rig. This equivalence was independently rerun after
revision `84cd3fd`.

## Exhaustive-discovery capacity

Nine full-length, evidence-off discovery fan-outs measured **4.0–43.1 seconds**, with a mean near
20 seconds. Cost tracked match connectivity more strongly than audio duration. At 2,891 source
resources, the serial estimate is:

```text
2,891 × 20 seconds ≈ 16.06 hours
```

Raw evidence-off output ranged from roughly 163 KiB to 2.2 MiB per source, projecting to
single-digit GiB for one complete observation bank. Peak memory observed on the rig made concurrency unsafe;
the intended schedule is strictly serial.

A durable banking job should therefore checkpoint each source independently, pin source/target
generation plus passage profile, write output before advancing the checkpoint, and resume from the
first incomplete source after restart. New enrollment requires two logical lanes:

- new A → all existing targets;
- every existing A → the new target, using bounded target discovery.

One A→all fan-out already covers every ordered pair whose source is A. A full initial observation bank is 2,891
fan-outs, not twice that number.

## Store identity and duration repair

Legacy imported metadata exposed a Panako dump pathology: 1,498 of 2,891 resources carried the
same bogus duration, `0.7713125`. Matching remained correct because fingerprint extents come from
stored time bins, but older endpoint hashes included the metadata duration and would churn passage
IDs after correction.

Resident now separates the operations:

1. `rehash-identities` publishes prints-only endpoint hashes and reuses every shard byte;
2. an external authoritative decoder produces a complete key/duration JSONL map;
3. `set-durations` requires the expected post-rehash generation, complete exact key coverage, and
   plausible agreement with fingerprint extents;
4. publication changes duration metadata and generation only;
5. the command reopens the store and proves content hashes unchanged and shards reused.

The bogus duration is explained by a metadata-parser/cache value derived from extraction scheduling:
`(-128 + 12469) / 16000 = 0.7713125`. The **12,469-sample value** is analysis scheduling support.
The separate **12,464-sample value** is the fixture-proven public fingerprint timestamp latency.
Both are intentionally named and tested.

`rehash-identities` must run before any legacy passage IDs are treated as bankable. Re-running either
maintenance operation against already-correct state is a no-op.

## Operating model

Resident works best as one supervised local daemon per application process:

- open the configured immutable store once;
- use request IDs because responses can complete out of order;
- keep stdout protocol-only and stderr diagnostic-only;
- cap callers in front of Rayon rather than creating unbounded pending work;
- make overload, timeout, crash, missing binary/store, protocol refusal, and successful no-match
  distinct states;
- expose engine version, store generation, config identity, resource count, and posting count in
  health;
- restart unexpected exits with bounded backoff;
- run store mutation through one controlled writer and serve from read-only mounts.

Corpus-scale discovery should be an application job with durable checkpoints, not an HTTP request
and not a second always-on service. The hard recovery invariant is that an application restart can
reopen the archive and resume derived work without rewriting prior observations.

## Native-store acceptance

When a native full-corpus build is warranted:

1. freeze a strict JSONL manifest and a new output/store root;
2. benchmark real long recordings at conservative `--jobs` values, including temporary storage and
   peak RSS;
3. run resumable `refingerprint`, requiring zero unresolved failures;
4. ingest into a new immutable store while retaining the imported reference generation;
5. compare fixed external probes first, then stored-resource cohorts, with `ab-compare`;
6. bank passages for ear-checkable difficult cohorts: ordinary recurrence, repeated re-cueing,
   overlays, tempo/pitch changes, silence, evidence holes, and duplicate encodings;
7. run full-mesh recall accounting against the legacy observation mesh;
8. switch consumers only after a human-approved answer-faithfulness policy passes.

Exact scores and prints are diagnostic. Missing useful references and non-overlapping passage
answers deserve the highest review priority.

## Known limits

1. **Native extraction changes the print population.** Its current fixture misses require a
   reviewed migration.
2. **Fingerprint evidence cannot classify residual audio.** Resident can establish a known
   underlay while leaving overlay presence possible; voice/source separation is another system.
3. **Passage stitching is profile-specific.** A 30-second locked gap cap is a judgment encoded in
   passage-v3, not a universal law.
4. **Residual peeling is greedy.** A gate-failing dominant residual mode can obscure a weaker valid
   mode behind it.
5. **Same-audio remains proposal-grade.** Human/application governance owns revision sets.
6. **Daemon target stores are opened per request.** High-rate cross-store use may warrant an
   explicit immutable store registry.
7. **Corpus ingest peak memory is not fully characterized.** Query-store scale is measured; a fresh
   412M-posting native build still needs operator capacity receipts.
8. **No network transport is included.** The stable boundary is the local JSON-lines daemon.

## Revision receipts

The production replay evolved through these defined revisions:

| revision | change |
|---|---|
| `fcb8add` | initial passage evidence lane |
| `88ab8df` | overlapping production query geometry; recovered the known re-cue locus |
| `4e5c891` / `84cd3fd` | target-shard lookup and compatibility-correct global lookup shape |
| `b553e55` | weak contained residual lines retained as alternates |
| `6e6daff` | prints-only endpoint identity publication |
| `c40568c` | authoritative duration metadata publication |

Independent Round-5 replay from revision `959d18e` reproduced the passage cohorts byte-for-byte and
confirmed that both the protected reference store and its replay twin remained untouched.
