# MIXMD ROUND 4 REPORT — replay repair and full-bank schedule

Date: 2026-08-30

This is a new report. It does not edit `docs/MIXMD-REPORT.md` or
`docs/MIXMD-ACTUAL-REPORT.md`.

## Executive ruling

The original production divergence is closed. It was a query-geometry recall bug, not a
fundamental incompatibility with the Panako-derived prints. Resident must use the production
12-second window / 8-second hop geometry for passage discovery. On revision `88ab8df`, the practice
night→Wefunk 0417 pair moved back to the jar's fixed 5509–5524 reference locus and produced 37
record-grained occurrences across the 55-minute source.

Round 4 closes the remaining resident work needed before a full evidence bank:

- `passage-v3` turns dominated residual lines into preserved alternates instead of additional
  top-level questions;
- a tightly locked offset ramp may bridge a support hole up to 30 seconds, while the hole remains
  explicitly unsupported;
- near-total, zero-offset, unit-factor matches are marked as proposal-grade same-audio candidates;
- pair queries materialize postings from only the target shard while preserving the global lookup
  shape needed for exact Panako modal-tie behavior;
- legacy duration-coupled endpoint hashes can be repaired by an atomic, shard-preserving manifest
  rehash before passage banking.

The full bank should run as one strictly serial, resumable mixmd job over 2,891 source fan-outs. At
the measured 20-second mean it is approximately 16.1 hours. One complete pass already observes both
directions because every resource becomes A once. The two-job rule applies only when one new resource
is enrolled later: new→all and all-existing→new.

Do not wait for bit-perfect agreement with the legacy suggestion rows before running the bank. Bank
under a new immutable analysis run, compare at the answer level, and activate it only after the
cohort and corpus-recall gates below pass. The legacy mesh remains available throughout.

## 1. Defined revisions and acceptance material

| revision | role |
|---|---|
| `fcb8add09f5e52e21759711d713f8a7604fd329e` | frozen round-2 `passage-v1` starting point |
| `88ab8df31d48bf332a92d4c9d2c00057f3f77957` | production 12s/8s replay geometry, honest hit counts, prints-only dynamic identity |
| `4e5c891` | first target-shard pair optimization; later found incomplete under modal ties |
| `b553e55` | `passage-v3`: alternates, locked-hole bridge, same-audio candidate |
| `6e6daff` | profiled, manifest-only `rehash-identities` operation |
| `3bea7f7` | unchanged ingest remains idempotent on a legacy unprofiled manifest |
| `84cd3fd` | corrected target-shard optimization with exact global lookup-shape preservation |

The verified Round 3 JSONs are on `mixmap-rig` at `~/resident-r3/replay/`. Fresh Round 4 JSONs are
at `~/resident-r4/replay-r4/`. All matching reads used the byte-identical replay twin
`~/resident-store`; `~/.panako/resident-store` was not mutated.

The local gate on `84cd3fd` is green:

```text
./check.sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test

resident binary unit tests: 6 passed
resident process tests:     2 passed
resident-core tests:       21 passed
doc tests:                  0 failed
```

The isolated rig checkout also passed all Rust tests and a release build. The rig lacks the
`cargo-fmt` component, so formatting/clippy were enforced by the complete Mac gate rather than
silently skipped as a claimed rig gate.

## 2. Replay findings

### C7: the centerpiece divergence is closed

On `88ab8df`, practice005→Wefunk 0417 produced 37 passages, all at reference 5509–5524 seconds,
with source occurrences scattered from 157 to 3164 seconds. `HEAD~1` found no passage there. The
cause was non-overlapping 30-second query regions starving five-to-eight-second crumbs at seams.
The production-compatible 12-second windows every 8 seconds recover them.

This is one record repeatedly re-cued/backspun against one continuous partner play. It is not a
drop-over-song example and not one continuous 55-minute source occurrence. The passage result is N
occurrences of one candidate recurrence class; concordance may ask one class-level question.

### C1 and P2: bridge the locked talk-over hole in resident

The Wefunk 0694→0232 accepted matcher lines include the intended continuous offset ramp. Under
`passage-v3`, its main primary is:

```text
A envelope       7872.971–8540.515
B envelope        863.659–1529.475
matched hits                    1,957
supported seconds             336.864
largest explicit hole          20.512
```

The ramp is one passage even though support disappears for about 21 seconds. This is resident's
job: offset continuity and factor continuity are signal geometry, and the app cannot reconstruct
them from finished edges. The rule is narrow:

- ordinary stitch: gap ≤20s, endpoint-offset jump ≤2s, time/pitch-factor jump ≤0.03;
- locked stitch: gap ≤30s, endpoint-offset jump ≤0.5s, time/pitch-factor jump ≤0.01.

The 20.512-second interval is still absent from `support`; the engine does not claim the talk-over
hole matched. Gaps beyond 30 seconds or without a locked ramp remain separate passages. Concordance
may later join separate passages semantically, but it does not get to reinterpret pairwise signal
continuity.

The earlier C1 support sliver no longer becomes a separate question. Depending on track membership,
such a line is either absorbed into the same alignment's deduplicated support or retained as an
alternate; it is never silently discarded.

### C3 and P1: primary occupancy plus preserved alternates

For Wefunk 0229→0232, the 0–130-second cohort now contains exactly two primary passages:

```text
A 0.963–60.035    → B 0.963–60.035    2,001 hits
A 105.179–116.723 → B 105.547–117.171   156 hits
```

The roughly 45-second break stays split, so the falsifiable gap rule holds. The three residual lines
inside the first query occupancy are retained as alternates under the first primary:

```text
A 24.955–35.939 → B 100.803–111.779   70 hits
A 46.339–58.659 → B  41.571–53.891  113 hits
A 52.091–59.411 → B  42.571–49.899   47 hits
```

The deterministic dominance rule is: if at least 80% of a candidate's A envelope lies inside a
primary that has at least four times its matched hits, expose it in `alternates` with the primary id
instead of in the top-level `passages` array.

This is not a single-occupancy assertion. The complete alternate alignment survives for diagnostics
and later layer analysis. Equal-strength competitors, partial overlaps, and lines outside the
primary query occupancy remain top-level passages. Exact repeated hits created by overlapping query
windows are deduplicated by their query/reference bins and hash pair.

The full C3 recording has a third legitimate primary at A 1939–2100; the “two passages” predicate
applies to the stated 0–130-second acceptance cohort, not the whole recording.

### C6 and P3: mark same-audio; do not declare it

The mp3→flac twin now produces one primary and 27 preserved alternates. The primary is a 1:1
diagonal:

```text
A/B envelope                 0.995–5095.099
matched hits                         143,609
supported seconds                   5,056.096
A fingerprint-extent coverage        0.992520
B fingerprint-extent coverage        0.992514
start delta                               0s
stop delta                                0s
```

Resident emits `same_audio_candidate` pointing to this primary. The predicate requires at least 90%
of both resources' fingerprint extents, start and stop deltas within 2 seconds of zero, and every
time/pitch-factor extremum within 0.005 of 1.

It remains a candidate. The same signal facts can mean duplicate encodings, a deliberate complete
rebroadcast, or a copied master. Only mixmd can bless an `audio_revision` relation. Once blessed,
the job supplies all sibling keys through `exclude_keys`; resident must not let an unreviewed
heuristic hide evidence. Because discovery has no top-K cutoff, an unexcluded sibling cannot crowd a
real recurrence out of the bank.

### C2, C4, C5: carry the verified Round 3 readings forward

- C2, two takes: 12 passages describe a performance diff rather than one forced whole-mix glue.
  The intro maps 1:1; A 553–684 maps to B 180–311 after material was cut between takes.
- C4, stamped end: Wefunk 0688 has resident support at A 3361.2–3372.1, crossing the machine-stamped
  3369.04 end, and at 3392.8–3403.4, filling the manufactured gap to the next cue. Resident evidence
  directly contradicts treating `tle_39147083`'s machine end as a canon boundary.
- C5, fogged partner: practice→Circle Research set1 produces 19 passages at the partner's fogged
  `[10:48?] gang starr - skillz` locus versus two legacy jar edges. Fog is a canon-lending policy,
  not absence of signal evidence.

Mixmd Phase 0—stop the in-place `t_end` write and add typed bounds—remains intentionally paused. No
mixmd code or snapshot was modified in this round.

## 3. P4: pair-query optimization, including the failed first attempt

The store puts all postings for one resource in its key-selected shard. The first optimization
therefore read only that shard for a named pair. A unit test showed the target hits equal to
all-shard hits filtered by resource id, but production replay caught a subtler compatibility fact:

1. the voter emulates Java `HashMap` iteration order;
2. its capacity depends on the number of query hashes whose ±2 lookup range matched anywhere;
3. hits from unrelated resources are later filtered, but their mere presence can change a modal tie;
4. target-only lookup therefore added one line and changed one line on C1.

Revision `84cd3fd` materializes postings only from the target shard but probes every shard's compact
hash index for range presence. Globally present hashes remain in the compatibility ordering even
when they have no target postings.

The acceptance comparison used the fresh exact command on both binaries, sorted every accepted raw
segment by its geometry and factors, and hashed the canonical JSON:

```text
88ab8df all-shard accepted-line SHA-256
ad68c781b8b471e80c096fe8bfba9e2996c5e8ace01c31e5c838884bfca24145

84cd3fd target-postings/global-presence SHA-256
ad68c781b8b471e80c096fe8bfba9e2996c5e8ace01c31e5c838884bfca24145
```

The old evidence-bearing C1 pair took approximately 5.5 minutes. The corrected optimized pair took
2.817 seconds. C3 took 1.082 seconds and the 5,056-second C6 diagonal took 0.272 seconds. These are
pair-query measurements, not fan-out cost projections.

The lesson is operational as well as technical: fixture-scale filtered-hit equivalence was
necessary but insufficient. Production answer equivalence found a corpus-global ordering dependency
that the hot path must preserve.

## 4. Duration rot and the repair sequence

### Cause

The `0.7713125` value is not a dump-grammar ambiguity. Panako's cached-print constructor leaves
`t3 = -1`; `PanakoStrategy.store()` later rewrites duration from the last cached fingerprint's `t3`.
With the pinned transform latency this becomes:

```text
(-128 + 12469) / 16000 = 0.7713125
```

That explains the exact rotten duration on 1,498 of 2,891 resources. Matching uses the stored
fingerprint time extent and is unaffected.

### Identity ruling

Duration does not belong in audio fingerprint identity. An endpoint `content_hash` now covers only
canonical `(hash,t,f)` postings. `passage_id` uses that prints-only endpoint identity, so later
duration correction cannot churn passage ids.

### Explicit repair order

1. Stop the resident writer for the store being repaired; keep the reference store read-only.
2. Run `resident rehash-identities --store PATH` on the operational store/twin before banking.
3. Verify that `stats.fingerprint_identity_profile == "prints-v1"`, retain the previous manifest,
   and record old/new generations in the analysis-run preflight.
4. Expect legacy duration-coupled hashes to change for all resources, not only the 1,498 rotten
   ones: the hash formula changed for every endpoint. Fingerprint shards are reused byte-for-byte.
5. Independently derive trusted durations from authoritative audio probes, validate complete key
   coverage and plausible agreement with `t_min/t_max`, then publish a duration-metadata-only
   generation under an expected-generation precondition.
6. Confirm that store generation changed but prints-only `content_hash` and passage ids did not.

Step 2 is implemented and idempotent in `6e6daff`. It releases the previous 64 mmap views before
opening the new generation, avoiding file-descriptor exhaustion. Revision `3bea7f7` also ensures
unchanged ingest remains a no-op on an unprofiled legacy manifest before rehash.

Step 5 is deliberately not implemented here: resident has no authoritative duration input in this
task, and inventing one from rotten metadata would be worse than leaving metadata visibly wrong.
Duration repair is not a passage-bank blocker because matching, same-audio coverage, and passage
identity use fingerprint extents; the identity rehash is the required pre-bank step.

## 5. Phase 4 full-bank schedule

### Deployment shape

Keep the middle layer as a mixmd in-app job-system module backed by the existing SQLite database.
Resident is a release subprocess. Do not add a separately deployed concordance service: on one
self-hosted box it would add another database/protocol/recovery boundary without improving signal or
product ownership. The hard invariant remains: after process or host restart, the archive and every
committed source checkpoint are intact.

The job module owns durable graph history. Browser reads consume banked primary passages,
alternates, occurrence/class revisions, and proposals; they do not reconstruct identity from the
legacy suggestion mesh.

### Run records and checkpoints

Create one immutable `analysis_run` with at least:

```text
run_id
resident_revision = 84cd3fd
passage_profile = passage-v3
store_generation and fingerprint_identity_profile
config_id
source_snapshot_hash and source_count = 2891
started_at, completed_at, activated_at
state = preparing | running | validating | accepted | failed | superseded
```

Create one unique source checkpoint per `(run_id, source_revision_id)`:

```text
state = pending | leased | complete | failed
attempt, lease_until, started_at, completed_at
stdout_sha256, byte_count
primary_count, alternate_count, target_count, same_audio_candidate_count
typed error payload
```

For each source, the single worker does this:

1. claim one pending row with a renewable lease;
2. invoke `resident discover --evidence=false` for A→all under the pinned store generation;
3. stream stdout to a run-scoped temporary artifact and hash it;
4. reject a response whose snapshot/profile/config/generation differs from the run;
5. in one SQLite transaction, insert immutable observations with uniqueness keys, write the raw
   artifact checksum/counts, and mark the source complete;
6. atomically publish or rename the raw artifact after the transaction protocol chosen by mixmd;
7. release memory before claiming the next source.

On restart, expire only stale leases. A source whose transaction committed is complete; a source
whose subprocess or transaction did not commit is rerun. Passage ids plus run/source/target
uniqueness make retries idempotent. Never advance a global numeric cursor before the source
transaction commits.

An empty successful fan-out is a completed zero-result source. A subprocess error, malformed JSON,
snapshot mismatch, or missing raw artifact is failed/incomplete and can never masquerade as
absence.

### Serial schedule and cost envelope

Strictly one resident discovery subprocess runs at a time. The rig has 10 GB free RAM, no swap, and
measured peak RSS around 3.9 GB; parallel fan-outs risk host failure for little predictable gain.

Nine production fan-outs measured 4.0–43.1 seconds with a mean near 20 seconds. Therefore:

```text
2,891 sources × 20 seconds = 57,820 seconds = 16.06 hours
planning range from observed variance: 10–35 hours
```

The 12s/8s geometry is about 2.5× the old 30-second geometry on practice005 (43s versus 17s), and
cost tracks corpus connectivity more than source length or a simple sparse/dense band. Do not build
a scheduler that predicts cost from duration alone.

Recommended operation:

- take the normal SQLite/archive backup and run the identity-rehash preflight;
- verify disk headroom for the measured raw bank plus SQLite/WAL growth and one failed-run cleanup
  margin; stop rather than guessing if the measured projection does not fit;
- start one continuous overnight run, but make it safely pausable between source checkpoints;
- interleave sources by legacy degree/result-size strata only to improve ETA calibration, not to
  change semantics or priority;
- update ETA from completed wall time and bytes, reporting p50/p90 as well as the mean;
- retain stderr and typed failures per source; retry deterministic failures only after diagnosis;
- keep `evidence=false` corpus-wide. It costs similar CPU but reduces output about 140×. Use full
  evidence only for named cohort pairs or on-demand explanation.

Measured output was 163 KB–2.2 MB per source, giving a single-digit-GB raw discovery bank. Raw
artifacts are useful replay receipts, but normalized immutable SQLite observations remain the
product read model.

### Directional work count

A full pass is exactly 2,891 A→all fan-outs. Since every resource becomes A, it covers every ordered
pair in both directions once. Do not double it to 5,782.

For one later enrollment N, schedule exactly the new work:

- N→all-existing in one discovery fan-out;
- each existing A→N through bounded target discovery, batched by the job module as supported.

The legacy 24.5% one-way mesh was caused by `topK:4`, not proof of intrinsic matcher directionality.
The bank must still retain directional observations and fuse their union defensively: absence in
one direction is never a veto, especially for partial windows, future profiles, or interrupted runs.

### Acceptance gate before activation

Activation is answer-faithful, not bit-perfect.

1. **Mechanical completeness:** 2,891 source checkpoints complete; zero unresolved snapshot
   mismatches; source/target keys belong to the pinned snapshot; checksums and row counts reconcile.
2. **Resident cohorts:** C1 is one locked ramp with an explicit hole; C3 is two primaries in 0–130
   plus three alternates; C4 crosses and fills the stamped-end gap; C5 reaches the fogged name locus;
   C6 is one same-audio candidate; C7 remains at Wefunk 0417's 5509–5524 locus.
3. **Legacy mesh accounting:** classify every pending legacy suggestion as primary support,
   primary-envelope hole, alternate alignment, same-audio sibling, or absent. Stratify absent rows by
   legacy engine, top-K truncation, score/span, direction, named state, and partner. Do not flatten an
   explicit v3 hole into a false match just to improve row recall.
4. **Question-level comparison:** on cohort nights, compare distinct record questions, recurrence
   membership, best-listen occurrence, partner doors, and borrowable canon evidence—not raw row
   counts. The expected success is fewer duplicated asks with no unexplained loss of a curator-heard
   identity.
5. **Curator ear gate:** audition the strongest occurrence, transitions, the largest support hole,
   and at least one partner context for each changed high-value class. The confirmed practice-night
   identity remains one end-to-end acceptance datapoint.
6. **Supersession:** only after those checks, mark the new run active. Keep suggestions,
   dismissals, old observations, and human decisions unchanged; reads switch by active run id. A
   failure supersedes nothing and resumes from source checkpoints.

Full-mesh recall accounting belongs inside this phase. It is not a precondition that blocks the
bank whose output is needed to perform the comparison.

## 6. Reconciliation with the first two reports

### What the blind report got right

`MIXMD-REPORT.md` correctly put pairwise support geometry in resident, persistent recurrence/class
history in concordance, and canon/human decisions in mixmd. It correctly rejected top-K discovery,
single timeline occupancy, global machine confidence, and app-side offset stitching.

It also correctly described overlay support as non-exclusive: resident can say corpus song X is
present underneath a window without saying the unmatched component is voice or that nothing else is
present.

### What changes after seeing internals and replay

- The blind `passage-v1` sketch was too aspirational in places. Resident now has concrete
  `passage-v3` primaries, alternates, support masks, and same-audio candidates; corpus-global class
  identity still stays outside.
- A generic `analyze_layers` verb would overclaim what sparse prints contain. Concurrent known
  alignments and non-exclusive underlay support are legitimate; reconstructable residual audio or
  `unknown_voice_present` still requires a new audio/features lane.
- The practice night is repeated re-cueing/backspinning, not a drop-over-song example. The overlay
  requirement remains real, but C4/prose-mark cohorts—not the practice lattice—are its receipts.
- The 24.5% one-way production mesh is a `topK:4` artifact. Union fusion remains the safe invariant,
  but the earlier empirical directional interpretation was wrong.
- The store is resident-native. “Jar-derived” describes the origin of its fingerprints, not a
  foreign format. `Store::open` compatibility is proven; no conversion is required.
- The 30-second non-overlapping passage geometry in the first implementation was wrong for the
  production question. `passage-v2` restored 12s/8s recall; `passage-v3` adds the replay-ruled
  dominance and locked-hole semantics.
- `score_total` was not an honest quality fact because overlapping windows and residual peels could
  double-count. It is gone; `matched_hits` is exact-hit deduplicated and `score_peak` remains a
  diagnostic voter fact.
- Duration was wrongly included in endpoint identity. It is now prints-only, with an explicit
  manifest rehash before banking.
- “Caller must know every duplicate encoding” was too weak. The caller still owns the final audio
  revision, but resident now emits a strong same-audio candidate so store-only orphan siblings can
  be surfaced rather than silently rank as ordinary recurrence.

### What remains unchanged from the actual-layers report

- Concordance should be a resumable in-app SQLite job module, not a separate service.
- Machine evidence must never write canon or overwrite a human-shaped range field.
- Typed, overlapping canon ranges must distinguish set containment from simultaneity.
- Unknown state needs independent axes: unplaylisted, human-unidentifiable, machine singleton,
  machine recurrer, and borrowable are not one enum value.
- Name/entity normalization and spelling merges belong above resident.
- Legacy suggestions and dismissals are superseded, not rewritten; every read consumer moves by an
  active analysis-run boundary rather than a destructive migration.

## 7. Native re-fingerprint branch

The current evidence does not justify a full native re-fingerprint as a prerequisite. The store is
readable, the divergence was fixed by query geometry, and answer-level cohorts are stronger than the
legacy mesh in the expected places.

If later answer-faithfulness fails for extraction reasons, treat native extraction as one immutable
store-generation experiment. Freeze the existing pinned extraction identity: 16 kHz mono,
128-sample/8 ms time step, measured 12,464-sample latency, 110–7040 Hz, 85 bands/octave, 103×25 peak
filters, triplet time distance 2–33 bins, frequency distance ≤128 bins, and exact 34-bit Panako hash
packing. Run resumable extraction with one output per audio revision, validate decode failures and
durations before store build, then bank the same passage-v3 cohorts against both stores. Promote only
on curator-auditioned answer fidelity, not print equality.

That branch creates a new store and analysis run. It never mutates the read-only Panako-print
reference, and it does not alter the Phase 4 checkpoint/migration design.

## Final boundary

Resident now owns everything that requires filtered-hit and alignment geometry: accepted competing
lines, exact support, holes, stitching/splitting, primary-versus-alternate presentation dominance,
pairwise passage identity, same-audio signal candidates, and reproducible re-query from stored
prints.

Mixmd concordance owns durable observations, directional union, occurrence/class lineage, revision
relations, name borrowing, question eligibility, and atomic human decisions. Typed canon owns sets,
containment, simultaneity, identities, borrowability, and every blessed range.

Do not move any of the resident geometry back into TypeScript reads. Do not move semantic class or
canon authority down into resident.
