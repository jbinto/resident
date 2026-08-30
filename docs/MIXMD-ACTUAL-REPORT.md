# MIXMD ACTUAL-LAYERS REPORT — critique, resident changes, and migration

Date: 2026-08-30

This is the second report. It leaves `docs/MIXMD-REPORT.md` unchanged and reconciles that blind
design with the mixmd implementation and the supplied production snapshot.

## Executive ruling

Mixmd has rebuilt an evidence graph repeatedly inside a page read. That is the central defect.
The individual rules are often thoughtful, but they are being asked to recover passage identity,
directional fusion, span geometry, and class membership from an immutable pile of lossy point edges.
The result is not merely slow or inelegant. It changes the meaning of evidence according to which
read path, bucket width, union order, title spelling, or machine-stamped endpoint happens to see it.

The corrected ownership is:

```text
resident
  directional signal observations, dense support, gaps, geometry, quality, endpoint revisions
       ↓
mixmd concordance job module
  immutable observation bank, occurrence graph, class revisions, proposals, human constraints
       ↓
typed canon and review UI
  overlapping ranges, set containment, identities, one reviewed question and one atomic answer
```

The middle layer should be an in-app job-system module backed by the existing SQLite database, not
a separately deployed service. Resident remains the compute subprocess. The hard operational rule
is one application and one archive database that can restart and resume; another daemon, protocol,
database, and recovery story would make that invariant worse without creating an ownership benefit.

I implemented the resident half that is justified now: additive `passages` and `discover` verbs
under a versioned `passage-v1` profile. They emit deterministic directional passage identities,
explicit support intervals and holes, endpoint content hashes and generations, and named quality
facts. The existing parity verbs are unchanged. I did not put semantic recurrence classes in
resident: class revision, canon intersection, sets, borrowability, and human constraints require
the persistent product graph and do not belong in the fingerprint store.

The first application fix is not a new grouping heuristic. It is to stop writing match-derived ends
into `tracklist_entries.t_end`. That single column currently means both “the human authored this
range” and “a machine edge happened to stop here,” and identity lending treats either as a hard
boundary. This is machine evidence contaminating the canon-shaped read model in place.

## 1. Snapshot receipts

The snapshot opened read-only at:

`/Users/jbinto/dev/mixmd-snapshot-2026-08-30-for-codex.db`

Its relevant counts are:

| fact | snapshot value |
|---|---:|
| mixes | 2,641 |
| suggestions | 207,969 |
| pending suggestions | 207,891 |
| pending `panako` edges | 202,032 |
| pending `panako-cascade` proposals | 5,859 |
| rejected suggestions | 78 |
| suggestion dismissals | 94 |
| tracklist entries | 46,270 |
| set ranges | 973 across 569 mixes |
| tracklist entries whose cue lies in a set | 13,553 |

### Directionality reproduces; the raw denominator differs slightly

For distinct pending `panako` pairs with a non-null partner, this query finds 128,721 directed
pairs. Of those, 97,216 have the reverse pair: 75.5%. Therefore 24.5% are one-way.

```sql
WITH pairs AS (
  SELECT DISTINCT mix_id, matched_mix_id
  FROM suggestions
  WHERE decision = 'pending'
    AND engine = 'panako'
    AND matched_mix_id IS NOT NULL
)
SELECT count(*) AS directed_pairs,
       sum(EXISTS (
         SELECT 1 FROM pairs r
         WHERE r.mix_id = p.matched_mix_id
           AND r.matched_mix_id = p.mix_id
       )) AS reciprocal_pairs
FROM pairs p;
```

The addendum gives 128,986 rather than 128,721. I could not obtain that denominator from the
snapshot's soft partner columns, including or excluding pending cascade rows, but the reported 75.5%
rate reproduces exactly. The discrepancy is a predicate or snapshot-point difference, not a basis
for pretending the mesh is symmetric.

Concrete ruling: bank the **union** of A→B and B→A observations. If reverse observations have
compatible clocks, factors, and support geometry, fuse them into one undirected evidence relation
whose `directional_support` is `a_to_b`, `b_to_a`, or `both`; retain both source observations and
their separate quality vectors. Never require intersection, never treat missing reverse evidence
as a veto, and never double-count a mirrored pair as two recurrences.

This changes incremental ingestion. A new audio revision needs new→corpus **and** corpus→new jobs.
Only those pairs are new, but one directional fan-out is not a complete scan. The implemented
resident `discover` verb says `direction: "a_to_b"` explicitly; a future bulk reverse-scan verb may
make the second job cheaper, but the scheduler must not infer symmetry in the meantime.

### Concurrent evidence is ordinary, not exceptional

Using strict positive interval overlap on pending `panako` edges, and requiring a different partner
mix, I get 177,784 of 202,032 rows, or 88.0%:

```sql
WITH pending AS (
  SELECT id, mix_id, t_start, coalesce(t_end, t_start) AS t_end, matched_mix_id
  FROM suggestions
  WHERE decision = 'pending'
    AND engine = 'panako'
    AND matched_mix_id IS NOT NULL
), flagged AS (
  SELECT s.id, EXISTS (
    SELECT 1 FROM pending o
    WHERE o.mix_id = s.mix_id
      AND o.id <> s.id
      AND o.matched_mix_id <> s.matched_mix_id
      AND o.t_start < s.t_end
      AND o.t_end > s.t_start
  ) AS overlapping
  FROM pending s
)
SELECT count(*), sum(overlapping) FROM flagged;
```

The addendum's 166,886/202,536 (82%) does not reproduce with this interval predicate; its denominator
also exceeds the 202,032 pending paired `panako` rows by 504. Both measurements rule out single
occupancy. Neither proves 82% or 88% true audio layering: the same underlay matching two different
reference nights creates the same SQL shape. The model must allow overlap first, then let shared
class identity collapse redundant partner evidence without deleting genuinely simultaneous lines.

### The “backspin drill” explanation was wrong; the edge wall is real

The night `mix_eec016ce-de66-48df-b170-ac3a897d8884` currently has 359 pending rows:

- 252 unnamed `panako` edges, average span 7.03 seconds;
- 107 named `panako-cascade` proposals.

The inherited 252+25 count is stale, and the “one record drilled by backspinning” interpretation is
discarded. The measured phenomenon is a DJ drop/bumper over Gang Starr's record. The curator's ear
has separately confirmed the underlay across 01:53–54:41. That human confirmation is canon-grade
input; the edge wall is only the fragmented machine evidence that led to the question.

The supplied 67-claim residual census is consistent with the rows and code paths inspected:
18 partner-unplaylisted, 17 landing on prose marks that name the underlay but are classified
`DECLARED_UNKNOWN`, 16 just past a stamped end, seven `skillz`/`skills` song-id ties, seven fog
quarantines, two genuine ties, and zero coverage-floor failures. The important result is causal:
48 of 67 are lost to coverage/type/end semantics around the overlay, not weak fingerprint evidence.
Seven more are entity-resolution failures dressed as cross-source disagreement.

### A machine endpoint manufactures a real false gap

`evidence-end-sweep.ts` parses a bare cue, selects a nearby edge end, and writes it directly to
`tracklist_entries.t_end`. `identityEndOf` then treats any non-null value in that column as the row's
own measured bound. The snapshot contains this exact sequence:

```text
tle_39147083-a4db-412b-b644-61b79ec9e12e
[0:55:58] gang starr - skills
t_start = 3358.00, t_end = 3369.04

next authored cue:
[0:56:43] gang starr - 2 deep
t_start = 3403.00
```

The machine stop at 3369.04 becomes a hard identity end and manufactures an approximately 34-second
gap before the next human cue. There are 16,114 tracklist rows with non-null `t_end`, including
12,928 bound song rows. The schema cannot state which ends are authored and which are machine
evidence; the display bytes must be reparsed to recover that provenance.

This is plain wrong. Evidence may narrow a machine proposal or produce a boundary question. It may
not silently overwrite the effective reach of a human line in the same field as authored range data.

### Sets already prove containment is not optional

The 973 set ranges contain 13,553 tracklist cues. Yet `tracklist_entries` has no `set_id`; runtime
code assigns a line to a set by cue containment. At least 2,070 song vocabulary rows include strings
such as “(start of … set)” or “(end of … set),” although only a small referenced subset remains in
current tracklist rows. This is the leaked-boundary failure: a recording structure annotation was
allowed into musical identity text because containment had nowhere durable to live.

## 2. The engine/application boundary

### Resident must own

- assignment of filtered fingerprint hits to competing offset lines;
- pairwise directional passage identity and endpoint audio revision provenance;
- conversion of fixed analysis regions into dense support runs;
- geometry-aware stitching and splitting across region seams;
- explicit holes, non-exclusive support, factors, drift/coverage ingredients, and diagnostic hits;
- exhaustive re-query from stored forward fingerprints without re-extraction;
- a stable explanation of why two fragments did or did not stitch.

These are signal semantics. Reimplementing them from accepted edges in TypeScript necessarily loses
the rejected/residual hit cloud, factor continuity, and exact support mask needed to decide correctly.

### The persistent mixmd concordance module must own

- audio-revision and analysis-run lifecycle;
- fusion of compatible directional observations without discarding one-way evidence;
- recording-local occurrences supported by multiple partner observations;
- versioned candidate recurrence classes and their lineage;
- intersection with typed human canon, name-borrowing proposals, conflicts, and eligibility ranking;
- human must-link/cannot-link constraints and append-only reviewed decisions;
- stable record-grained questions and atomic application to an explicit reviewed membership set.

This is not browser/display grouping. It is durable derived product state. It belongs in SQLite and
must be incrementally recomputed by jobs, not rebuilt on each page view.

### Typed canon must own

- musical identities and spelling/entity merges;
- track, talk, drop, freestyle, station ID, bed, and set ranges;
- set containment versus simultaneous audio relationships;
- human coverage and human judgments about identifiability;
- the final decision to attach any identity to any range or class.

Resident never writes canon. Concordance never promotes a proposal merely because the graph changed.

## 3. Resident changes implemented here

All old matching surfaces remain intact. `passage-v1` is additive.

### `passages`: one directional pair

The new verb accepts A, an optional A window, B, an optional separate B store, and an optional raw
evidence flag. It returns:

```json
{
  "snapshot": {
    "profile": "passage-v1",
    "direction": "a_to_b",
    "config_id": "...",
    "a_generation": "...",
    "b_generation": "...",
    "a_key": "...",
    "a_content_hash": "..."
  },
  "b_key": "...",
  "b_content_hash": "...",
  "passages": [{
    "passage_id": "psg_...",
    "a_envelope": [184.13, 196.00],
    "b_envelope": [4316.92, 4328.43],
    "support": [{
      "a_start": 184.13,
      "a_stop": 186.20,
      "b_start": 4316.92,
      "b_stop": 4318.29,
      "hits": 7
    }],
    "quality": {
      "score_total": 43,
      "score_peak": 43,
      "matched_hits": 43,
      "supported_seconds": 6.52,
      "query_coverage": 0.55,
      "largest_gap": 2.06,
      "segment_count": 1,
      "support_count": 4,
      "time_factor_min": 0.9703,
      "time_factor_max": 0.9703,
      "pitch_factor_min": 0.9679,
      "pitch_factor_max": 0.9679,
      "sec_with_match_min": 0.667
    }
  }]
}
```

Internally the verb requests evidence-bearing multiline matches, splits support when either clock
has a filtered-hit gap over 1.5 seconds, and stitches region lines only when both clocks move forward,
envelope gaps are at most 20 seconds, endpoint offset changes by at most 2 seconds, and time/pitch
factors move by at most 0.03. Those thresholds are the frozen `passage-v1` profile, not app knobs.

The envelope is explicitly not matched occupancy. Only `support` is. An overlay may create a gap
inside an otherwise consistent alignment without being glued into the underlay. The observation is
also explicitly non-exclusive: it proves B is present where supported, not that B is the only sound.

`passage_id` hashes the profile, config, both endpoint keys and content hashes, support geometry, and
factor range. It excludes unrelated store-generation churn and the optional diagnostic payload; the
response still carries both generations. Consumers should key immutable banked facts by analysis run
plus passage id, because quality may evolve under a future profile.

### `discover`: exhaustive A→targets passages

The second verb fans one source over all or named target resources and returns passage matches sorted
by supported seconds, score, and key. It deliberately exposes no top `k`. `crosscheck` remains a
ranked UI/diagnostic query; it cannot define absence in a corpus graph.

`discover` is exhaustive for the stated A→B snapshot only. It does not pretend to answer B→A.
Scheduling both directions remains a concordance responsibility. A bulk reverse verb is a sensible
future optimization after production cost is measured, not a reason to lie about the current result.

### Validation performed

- `cargo check --all-targets`: green during implementation;
- `cargo test --all-targets`: 16 core, six binary-unit, and two process tests green;
- fixture oracle verification: 22/22 questions green;
- multiline fixture validation: green;
- a real fixture pair (`1236`, 184–196 seconds → `0789`) produced one deterministic passage with
  four explicit support intervals, 6.52 supported seconds, 0.55 envelope coverage, and a 2.06-second
  largest gap.

The requested read-only A/B store was not present at `/Users/jbinto/.panako/resident-store` in this
execution environment, so no production-store acceptance claim is made. Before mixmd banks new
observations, the same verbs must be replayed against that jar-derived store and compared with the
existing suggestion evidence. Missing access is distinct from an empty match result.

### What is intentionally not implemented in resident

Resident does not emit a corpus-global `class_id`. A stable pairwise observation id is a signal fact;
a stable recurrence class is a versioned graph judgment affected by one-way evidence, canon changes,
entity merges, human cannot-links, and later arrivals. Putting that in the fingerprint engine would
force it to own SQLite history and human semantics.

Resident also cannot honestly emit `unknown_voice_present`. Sparse landmarks can establish the known
song underlay, concurrent known lines, and unsupported holes. They do not retain magnitude or enough
signal to subtract the underlay. A true residual/overlay lane must reopen audio or persist richer
time-frequency features, produce an auditionable residual where possible, and be validated on clean
underlays, real drops, blends, and false positives. Until then the correct output is “corpus song X
is present; exclusivity unknown.”

## 4. Concrete middle-layer shape

Names are illustrative; the invariants are not.

### Durable analysis tables

```text
audio_revisions
  id, mix_id, resident_key, content_hash, duration, created_at, retired_at

analysis_runs
  id, profile, config_id, a_generation, b_generation,
  target_set_hash, state, cursor, started_at, completed_at, error

passage_observations
  id, run_id, resident_passage_id, direction,
  a_revision_id, b_revision_id, a_envelope_start, a_envelope_end,
  b_envelope_start, b_envelope_end, quality_json,
  superseded_by_run_id, imported_suggestion_id

passage_support
  observation_id, seq, a_start, a_end, b_start, b_end, hit_count

evidence_relations
  id, a_revision_id, b_revision_id, directional_support,
  current_revision_id

evidence_relation_members
  relation_revision_id, observation_id

occurrence_revisions
  id, mix_id, audio_revision_id, envelope, support_union, lineage_id, derived_by_run

candidate_class_revisions
  id, lineage_id, revision, membership_hash, status, supersedes_id

candidate_class_members
  class_revision_id, occurrence_revision_id, role, support_summary

proposals
  id, kind, class_revision_id, payload, state, supersedes_id, created_at

human_decisions
  id, action, reviewed_class_revision_id, reviewed_membership_hash,
  target_identity_id, payload, actor, created_at
```

Observations and class revisions are append-only. “Current” is an indexed projection, never an
in-place mutation of the evidence that a human reviewed. If a ruled class gains an occurrence, the
new membership produces an extension proposal. It does not inherit the old authorization silently.

### Class construction is not union-find over a five-second place key

Candidate construction may use graph algorithms, but a connected component is not an identity.
Transitions, reused samples, talk over instrumentals, station beds, and common drops form bridges.
The class builder needs edge compatibility, support overlap, conflicting concurrent lines, human
must/cannot links, and lineage from the previous revision. It should propose splits and merges, not
erase the distinction by transitive closure.

The record-grained question is a projection of a class revision. It carries one best audition,
partner contexts, every occurrence in the reviewed membership, supporting canon ranges, conflicts,
and the precise atomic effect of yes/no/split. Display order or page-view input order must never mint
its identity.

### Unknown is four axes, not one bucket

At minimum represent independently:

```text
human_coverage:       unplaylisted | covered
human_identification: unruled | identified | ruled_unidentifiable
corpus_status:         unseen | singleton | recurrer
borrowability:         eligible | fogged | prose_underlay | contested | prohibited
```

A prose range saying “live freestyles over Gang Starr - Skillz instrumental” is human-covered,
identifies an underlay, and also describes an overlay. It is not equivalent to “unknown.” A recurring
machine class with no human range is unplaylisted and a recurrer, not human-unidentifiable. Keeping
these axes removes much of the current quarantine logic rather than adding another exception to it.

## 5. Canon ranges, sets, containment, and simultaneity

Move toward typed, overlapping ranges. A minimal shape is:

```text
timeline_ranges
  id, mix_id, kind, start, end, identity_id?, text?, authored_by, created_at

range_relations
  parent_range_id, child_range_id, relation
  relation ∈ { contains, simultaneous_with, derived_from }
```

Rules:

1. A Wefunk guest set is a `set` range. Track ranges are children through `contains`.
2. Set names and boundaries never alter a song identity. “End of Ruby Jane's set” is range metadata,
   not part of the recording's title.
3. An acapella, freestyle, or DJ drop over an instrumental is a second overlapping range connected
   by `simultaneous_with`; it does not truncate or rename the underlay.
4. Canon track occurrences should eventually be real ranges. A bare cue can migrate as a cue/draft,
   but should not masquerade as a fully measured track range.
5. Machine support and boundary proposals reference canon ranges but live in evidence tables. They
   never write authored endpoints.

This structure separates “the set contains this track” from “this voice is simultaneous with this
instrumental.” A single `t_end` and one payload enum cannot represent both relationships.

## 6. Supersede-don't-rewrite migration

The order matters because present `t_end` semantics can corrupt every newly derived class.

### Phase 0 — make legacy state immutable enough to audit

Deploy schema and job code before changing reads. Stop `evidence-end-sweep` from writing
`tracklist_entries.t_end`; replace it with writes to a new evidence-bound table. Keep the existing
207,969 suggestions, 94 dismissals, weld tables, and tracklist rows untouched for audit and rollback.
Record a migration watermark and snapshot hash.

### Phase 1 — decontaminate endpoint provenance side by side

Create a typed bounds table rather than rewriting old rows:

```text
tracklist_range_bounds
  tracklist_entry_id,
  authored_end,
  legacy_machine_end,
  provenance,
  derived_from_suggestion_id?,
  supersedes_id?
```

Reparse `display_text` through the one line grammar. If the bytes contain a range, copy that end to
`authored_end`. If the bytes are a bare cue and legacy `t_end` is non-null, preserve it only as
`legacy_machine_end`. New identity lending reads authored bounds and typed canon relations; machine
bounds become cited proposal evidence. This recovers provenance without mutating legacy rows.

### Phase 2 — establish audio revisions and import legacy edges

For every current mix part/artifact, create an `audio_revision` with the best available resident key
and content hash. Where old evidence lacks a source content hash, mark it explicitly unknown; do not
manufacture one from a path.

Import each suggestion as an immutable `legacy-edge-v1` observation, retaining the original
suggestion id, engine, decision, evidence JSON, soft endpoint columns, artifact id, timestamps, and
direction. Cascade rows remain proposals derived from their seed edge, not independent signal
observations. The original tables continue serving the old read path.

### Phase 3 — preserve dismissals as scoped human constraints

Import each dismissal with its exact target song, mix window, source, and legacy grouping convention.
A dismissal of a point/claim is not a class-wide cannot-link unless the human actually reviewed that
class membership. Keep a legacy suppression adapter during shadow reads. Existing weld rulings may
be imported only against the explicit member ids present when ruled; later members require review.

This avoids the current weld failure in which `bankWeldClass` replaces members under a persistent
class ruling. A verdict cannot have mutable scope.

### Phase 4 — bank `passage-v1` beside legacy evidence

Run resident against the read-only jar-derived reference store. Create analysis-run rows, schedule
both directions, bank support intervals transactionally in short batches, and checkpoint completion
in SQLite. Restarting the app resumes from the last completed source/target partition. Failures and
incomplete target sets are first-class run states, not empty results.

Build directional relations, occurrence revisions, and candidate classes from the new bank. Compare
legacy and new questions on named, unnamed, prose-overlay, fog, set, and one-way cohorts. Do not cut
over on row-count similarity; use curator audition and preserved partner doors.

### Phase 5 — add one new read-model boundary

Expose a server `MixQuestions`/`ClassQuestions` shape produced from persisted class revisions. Keep a
temporary adapter to the existing `MixClaims` type so rendering can move independently from writes.
The migration surface includes these real production importers and their downstream type consumers:

- `api/app.ts` and `api/shared.ts`;
- `app/data/mix.ts`, `mixClaims.ts`, and `quizDeck.ts`;
- `useClaims`, familiarity paint, the claim rail/model, main route, and cascade-arrival workbench;
- game contenders/desk/quiz types and claim keys;
- `features/mixes/service.ts`;
- server write/read paths in `editorial.ts`, `proposals-read.ts`, and `unheard-sweep.ts`.

Move the direct `listClaimsForMix` callers first, then the adapter-backed UI types, then writes that
currently rederive `edgeCoords`, source votes, or claim runs. During shadow mode, log old/new question
ids and effects but keep only legacy actions enabled.

### Phase 6 — cut writes, then reads

After A/B acceptance, enable decisions against explicit class revisions and membership hashes.
Transactionally write canon only after the human action passes optimistic concurrency. Then switch
the claim rail, game, census, cascade/name borrowing, song facts, and familiarity views to the new
read model.

Disable new legacy suggestion/cascade/weld minting only after every consumer has moved. Leave legacy
tables read-only as historical receipts. Do not delete or rewrite them in this migration. A later
retention decision can archive them after all dismissals and human actions have traceable successors.

## 7. Deployment and incremental operation

Use an in-app module with durable jobs such as:

```text
analysis_jobs(id, kind, audio_revision_id, target_revision_id?, profile,
              state, attempt, cursor, lease_until, error, created_at, updated_at)
```

The web process starts one bounded worker after migrations. It leases a job in a short transaction,
runs resident as the existing blocking subprocess/host outside a DB transaction, banks one bounded
result batch atomically, advances the cursor, and releases the lease. On restart, expired leases return
to pending. A completed run is published by one pointer/update only after all expected directional
partitions are present.

No network service is required. CPU-heavy fingerprint work is already isolated in resident and
parallelized there. If TypeScript graph construction becomes CPU-heavy, a worker thread may execute
the same job module against the same SQLite file, but that is an implementation detail, not another
deployed service or source of truth.

Incremental cases:

- new audio: enroll once; schedule new→all and all→new; derive affected occurrences/classes;
- replaced audio: new revision, new directional jobs, supersede old observations; never shift canon;
- canon edit/entity merge: re-evaluate overlapping class proposals without fingerprint extraction;
- profile change: new analysis run over stored prints; retain old run for A/B and rollback;
- retired audio: mark dependent evidence unavailable; preserve decisions and class lineage;
- new class member: mint a new class revision and extension proposal; do not inherit old authority.

## 8. What will not survive corpus growth—or is wrong now

1. **Read-time graph reconstruction.** It repeats expensive work, has no completeness state, no
   provenance boundary, and can change stable-looking question ids with input order.
2. **Three incompatible notions of a place.** Server claims, five-second UI buckets, and seam/weld
   tolerances do not describe one identity contract.
3. **Transitive union-find on a bucketed partner offset.** One bridge can fuse unrelated records;
   the result is a display accident, not a recurrence class.
4. **Song-id voting before entity resolution.** `skillz` and `skills` are evidence agreement turned
   into a false tie by vocabulary fragmentation. A genuine conflict and an unresolved alias are not
   the same state.
5. **One-source-one-vote after geometry loss.** Voting cannot repair a window glued across an overlay
   or clipped by a machine endpoint. Source authority is useful only after honest support exists.
6. **Machine evidence in `tracklist_entries.t_end`.** It mutates effective canon reach in place and
   cannot be traced or superseded cleanly. This is wrong today, regardless of scale.
7. **No audio-revision validity on edges.** A path or artifact generation is insufficient to decide
   whether an old observation still describes current bytes.
8. **Implicit symmetry.** Discarding one-way evidence would erase 24.5% of the measured pair mesh;
   counting both directions independently inflates recurrence.
9. **Single occupancy.** Most pending rows overlap a different partner row under reasonable interval
   semantics. Overlap must be legal even before deciding whether it is redundant underlay evidence.
10. **Weld heuristics as identity.** A 40–75-second duration band, fanout floor, 300-member cap, one
    strongest occurrence per night, and representative-derived id encode presentation convenience,
    not recurrence truth. Dropping repeated same-night occurrences directly contradicts the product's
    practice/drop use cases.
11. **Mutable membership under a ruling.** Replacing weld members while preserving class status
    changes what a human supposedly authorized.
12. **Top-`k` discovery.** A ranked shortlist can drive UI, never corpus absence or singleton status.
13. **One `unknown` enum.** It confuses no human coverage, a human declaration, machine singleton,
    machine recurrer, fog, prose overlay, and borrowability.
14. **Set text inside song identity.** Structural boundaries must be containment facts, not title
    suffixes that poison cross-night identity.

SQLite can hold 207,969 rows comfortably. The scaling failure is not raw row count; it is repeatedly
interpreting those rows as if the interpretation had stable identity, provenance, and human scope.

## 9. Refusals: do not rebuild these in the app

I would reject any app-layer implementation of:

- stitching 30-second/8-second matcher fragments into claimed audio spans;
- deciding support continuity from endpoint gaps alone;
- assigning residual hits to simultaneous offset lines;
- turning score into confidence or inferring overlay voice from low coverage;
- trimming a known underlay to a fixed window that also contains a drop;
- deciding that missing reverse evidence invalidates a forward match;
- issuing thousands of point matches and reconstructing passages with time buckets;
- re-querying audio when resident's forward fingerprint store can answer a new profile;
- mutating authored range reach from machine evidence.

The engine should own the signal facts listed above. Conversely I would reject resident owning song
titles, set membership, borrowability, class rulings, or final canon. The hard boundary is not
“Rust versus TypeScript”; it is signal interpretation versus durable corpus judgment versus human
authorship.

## 10. Reconciliation with the blind report

Seeing mixmd changes some recommendations and strengthens others:

| blind-report position | ruling after inspection |
|---|---|
| three layers: resident, concordance, mixmd canon | correct; retained |
| a dedicated concordance service | changed: use an in-app durable job module on this solo box; resident remains the subprocess |
| passage support/gaps and non-exclusive layers belong in resident | correct; a first additive implementation now exists |
| semantic recurrence classes belong outside resident | correct; they must be persistent, not page-view grouping |
| new recording needs one corpus discovery | incomplete: measured one-way evidence requires both new→corpus and corpus→new |
| canon edits only re-evaluate proposals | incomplete today: first separate machine-stamped `t_end` from authored bounds, because current evidence has already changed lending reach |
| candidate classes are not connected components | confirmed by bucketed union-find, overlays, sets, and weld behavior |
| future members of a ruled class require extension review | confirmed and made urgent by current weld membership replacement |
| overlap must be legal | confirmed at 82% by the maintainer predicate and 88% by strict interval overlap here; neither number alone identifies layers |
| unknown is a workflow state | expanded into independent coverage, identification, corpus-recurrence, and borrowability axes |
| canon assignments attach to occurrences | expanded: typed overlapping ranges plus explicit set containment and simultaneity are required |
| stable paginated discovery | narrowed for the implementation: `discover` is exhaustive in one response now; snapshot-pinned pagination/artifacts become necessary only after production response size is measured |

The largest correction is operational, not conceptual: “concordance” should not become another
service. The largest newly visible data-model issue is containment. The largest newly visible bug is
the in-place endpoint write. The largest resident correction is directional completeness.

## Closing recommendation

Ship no further claim-grouping heuristics until endpoint provenance is separated and passage evidence
can be banked side by side. Then run `passage-v1` against the read-only jar store, build class revisions
in a resumable in-app job, and shadow the one real overlay night plus named/unnamed/set cohorts.

The success metric is not fewer rows. It is one honest question per distinct record or recurrent
class, with a best audition, every partner door, visible conflicts and gaps, and one answer whose
explicit reviewed scope never changes underneath the curator.
