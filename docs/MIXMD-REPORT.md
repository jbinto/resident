# MIXMD REPORT — product layers above resident

Date: 2026-08-30

## Executive ruling

Resident is the right foundation, but its current result unit is below mixmd's product unit.
It returns independent matcher facts; mixmd needs one record-grained question backed by every
occurrence and every partner context.

Use three explicit ownership layers:

```text
audio revisions
    ↓
resident — fingerprints, layered passage geometry, match evidence
    ↓
concordance — occurrences, recurrence graph, classes, proposals, incremental analysis
    ↓
mixmd — canon, playback/context UI, human decisions
```

Resident should not learn artist/title semantics. Mixmd should not learn how to stitch offset
lines. A dedicated corpus service, called `concordance` in this report, should turn resident's
passage facts into persistent evidence classes and review questions.

Resident needs an additive passage-matching profile; its v0 compatibility verbs should remain
unchanged.

## 1. Resident: signal evidence

Keep `match`, `span`, and `crosscheck` as raw compatibility surfaces. Add a versioned
`passage-v1` profile whose result is a supported passage rather than one fixed-region voter row.

A representative result is:

```json
{
  "alignment_id": "aln_...",
  "profile": "passage-v1",
  "provenance": {
    "a_generation": "...",
    "b_generation": "...",
    "a_content_hash": "...",
    "b_content_hash": "...",
    "config_id": "...",
    "algorithm_version": 1
  },
  "a": {"key": "show/2012-04-14", "start": 113.2, "stop": 267.9},
  "b": {"key": "show/2010-09-03", "start": 842.7, "stop": 997.3},
  "mapping": {
    "ref_seconds": "0.9997 * query_seconds + 729.54",
    "time_factor": 1.0003,
    "pitch_factor": 1.0
  },
  "support": [
    {
      "a_window": [113.2, 171.8],
      "b_window": [842.7, 901.3],
      "matched_hits": 812
    },
    {
      "a_window": [179.1, 267.9],
      "b_window": [908.6, 997.3],
      "matched_hits": 1184
    }
  ],
  "quality": {
    "matched_hits": 1996,
    "supported_seconds": 147.4,
    "query_coverage": 0.91,
    "reference_coverage": 0.89,
    "largest_unsupported_gap": 7.3,
    "residual_p50_bins": 0.4,
    "residual_p95_bins": 1.7,
    "time_factor_drift": 0.0008,
    "boundary_confidence": {"start": 0.72, "stop": 0.84}
  },
  "evidence_ref": "evidence/aln_..."
}
```

Quality should remain a vector of named facts. Raw score, duration, coverage, contiguity,
residual error, drift, and ambiguity answer different questions; reducing them to one magic
confidence number would conceal rather than resolve uncertainty.

### Proposed resident verbs

`discover_passages(source, targets, profile, cursor)` finds every qualifying passage alignment,
with snapshot-pinned pagination. It must not silently truncate the evidence graph by top `k`.

`align_pair(a, b, windows, profile)` returns every compatible concurrent line and stitched
passage for one recording pair.

`explain_alignment(alignment_id)` returns full hits, density, rejected fragments, competing
lines, and the reason fragments were split or stitched.

`analyze_layers(source_window, candidate_refs)` produces simultaneous matched-layer hypotheses
and residual evidence for the overlay capability.

Every persistent result must identify both store generations, both resource content hashes,
the fingerprint configuration, and the matcher profile/version. An `alignment_id` is
deterministic only within that provenance.

## 2. Concordance: the corpus evidence graph

Concordance is a backend analysis component, not browser logic and not resident-core. It owns
the lifecycle between immutable signal facts and human-authored canon.

| record | purpose |
|---|---|
| `AudioRevision` | logical recording, immutable audio revision, resident key, content hash |
| `AnalysisRun` | store generations, config, algorithm/profile version, completion state |
| `PassageAlignment` | immutable resident fact joining two recording-local passages |
| `Occurrence` | derived recording-local span representing one appearance of some material |
| `CandidateClassRevision` | versioned grouping of occurrences supported by the graph |
| `Proposal` | machine-authored name borrowing, unknown surfacing, class resolution, or conflict |
| `HumanDecision` | append-only accept, split, merge, reject, or classify action |
| `CanonAssignment` | human-authored identity attached to occurrences or a ruled class |

A candidate class is not a connected component. Shared samples, transitions, medleys, drops,
and overlays create bridge edges. The model must support:

- one occurrence participating in multiple layered hypotheses;
- overlapping occurrences on one recording timeline;
- human `must-link` and `cannot-link` constraints;
- stable class lineage across recomputation, with immutable revisions;
- class split and merge without rewriting old evidence.

A review question may look like:

```json
{
  "question_id": "q_...",
  "kind": "borrow_name",
  "class_id": "class_...",
  "class_revision": 7,
  "summary": {
    "occurrences": 19,
    "recordings": 4,
    "date_range": ["2010-09-03", "2026-02-12"],
    "total_supported_seconds": 1643.2
  },
  "suggestion": {
    "identity_id": "track_gang_starr_skillz",
    "supporting_canon_entries": ["entry_123", "entry_818"]
  },
  "best_occurrence": {
    "recording_id": "practice-2026-02-12",
    "window": [113.2, 267.9],
    "listen_window": [103.2, 277.9]
  },
  "partner_contexts": [
    {"recording_id": "show-2010-09-03", "window": [832.7, 1007.3]}
  ],
  "conflicting_identities": []
}
```

The answer uses optimistic concurrency and freezes its scope:

```json
{
  "question_id": "q_...",
  "expected_class_revision": 7,
  "action": "accept_identity",
  "identity_id": "track_gang_starr_skillz",
  "apply_to_occurrence_ids": ["occ_1", "occ_2", "occ_3"]
}
```

One human action may transactionally resolve every occurrence in the reviewed class revision.
If another occurrence joins later, it becomes an "extend this ruled class?" proposal. The
machine does not silently create a new canon entry.

## 3. Forming product capabilities

### Name borrowing

Intersect candidate-class occurrences with human tracklist entries. If all named members
support one identity, propose it to the unnamed members and cite the supporting canon entries.
If named members disagree, produce a conflict question; never majority-vote canon. The question
opens on the best clean occurrence and provides doors into every supporting context.

### Unknown surfacing

Retain short or weak matches as evidence, but rank ordinary review questions using separate
signals:

- occurrence count, including repeated drills inside one recording;
- distinct recording count;
- date and year spread;
- total and longest contiguous supported duration;
- number of strong partner contexts;
- overlay and ambiguity burden.

A seven-second sliver can remain in the graph without becoming a curator question. That is a
workflow-eligibility rule, not evidence deletion.

### Recurrence classes

A human may classify a ruled identity as `record`, `theme_bed`, `station_id`, `dj_drop`,
`practice_drill`, or another product taxonomy. Existing occurrences resolve together. New
members of the ruled class are routed to an extension review instead of being asked as fresh
identities.

## 4. Invariants

1. Machine output is evidence or proposal, never canon.
2. Every persisted evidence row names its exact audio revisions, store snapshots, config, and
   algorithm version.
3. An empty result is distinct from an analysis failure or incomplete scan.
4. Discovery pagination never changes the meaning of absence; top-`k` results cannot build the
   recurrence graph.
5. Classes are versioned hypotheses, not permanent equivalence relations.
6. Concurrent layers and overlapping occurrences are legal. No timeline has a single-record
   occupancy constraint.
7. Human decisions are append-only and retain the evidence revision that was reviewed.
8. Reanalysis supersedes evidence; it does not rewrite decisions or canon history.
9. Same inputs, resident snapshots, and analysis profile produce deterministic facts and class
   proposals.
10. A single answer applies atomically to the explicit reviewed membership. Future machine
    discoveries require another human authorization unless the product later defines a clear
    standing-rule mechanism.

## 5. Incremental update story

New recording: enroll it once, then discover it against all existing resources. Only edges
involving that audio revision need analysis.

Replaced audio: create a new `AudioRevision`, supersede alignments involving the old revision,
and scan the replacement against the corpus. Preserve decisions and flag timestamped canon for
review rather than shifting it automatically.

Retired audio: retire the resident resource and mark dependent evidence unavailable. Never
delete human decisions.

Canon edit: re-evaluate only proposals and classes whose occurrences overlap the changed canon
entry. No fingerprint work is required.

Matcher/profile change: create a new immutable `AnalysisRun` and re-query the resident forward
store. Keep the old graph available for diff and rollback.

Class decision: update the affected class lineage and downstream proposals transactionally.

Discovery must define directional semantics centrally. Resident's compatibility matcher is
query/reference-sensitive, so concordance must not guess whether A→B, B→A, or a fusion of both
constitutes one passage fact.

## 6. Required lower-level resident changes

### Adaptive passage formation

Current `span` and `crosscheck` deliberately divide audio into independent 30-second regions
and do not merge them. Add hit-cloud-driven segmentation, explicit support intervals, gap
splitting, and geometry-aware stitching behind `passage-v1`.

Two fragments may stitch only when reference identity, predicted offset, time factor, pitch
behavior, and intervening support are compatible. Matching the same resource in adjacent
windows is insufficient.

### Joint multi-line clustering

Current multiline mode greedily peels one accepted line and stops when the next dominant cloud
fails the compatibility voter. That can conceal a valid weaker layer. Passage mode needs joint
or robust line clustering, and it must allow lines with overlapping query-time support.

The v0 voter and default behavior remain exact and unchanged.

### Rich match quality

Passage results should report:

- supported duration and support intervals;
- query and reference coverage;
- largest and typical unsupported gaps;
- offset-residual distribution;
- time and pitch drift across the passage;
- boundary confidence;
- competing-reference and competing-line ambiguity;
- distinct probe fingerprints explained, not only duplicated hit count.

Current `score` and `sec_with_match` remain useful facts, but cannot determine record identity
or safe stitching by themselves.

### Durable provenance and complete discovery

Every result meant for persistence carries store generations, resource content hashes, config
identity, and matcher profile. Passage discovery is snapshot-pinned and exhaustible through
stable pagination or a persisted result artifact; ranking cannot discard graph edges.

### Re-query without re-extraction

The store already has the essential mechanism: forward fingerprint ranges by resource and
time. New segmentation, clustering, and ranking profiles can re-read those prints without
audio decode or fingerprint extraction. Resource revision tokens and analysis-run provenance
need to become explicit at the API boundary.

## 7. Overlay and underlay separation

The data model and passage API must treat simultaneous layers as ordinary:

```text
query 113–268
  ├─ corpus song X: supported 113–268, with gaps
  ├─ corpus station bed Y: supported 181–194
  └─ additional component: unresolved
```

Fixed regions must not glue an overlay into the underlay's passage merely because matching
support exists on both sides. Joint lines and temporal support masks should show exactly where
each corpus source is supported.

Fingerprint evidence alone cannot honestly prove "unknown voice on top." A landmark match
proves that song X is present. Sparse `(hash, t, f)` prints do not contain magnitude or enough
signal information to reconstruct or subtract X.

Implement overlay capability in two stages:

1. `passage-v1` returns concurrent alignments, support masks, jointly assigned fingerprint
   evidence, and a neutral unexplained-fingerprint result.
2. A validated audio-domain lane retains richer time-frequency features or reopens the audio,
   estimates masks/residual audio, and optionally supplies an auditionable residual. Only this
   layer can support a calibrated claim that an additional component is present.

The human may classify that component as a DJ drop. Resident must not label it as voice without
a separate validated classifier.

Resident owns passage identity and alignment geometry. It does not own durable semantic
recurrence classes, titles, or the distinction between a theme and a record.

## 8. Refusals at the application boundary

Do not build any of the following in mixmd UI or ordinary application code:

- merging 30-second rows into records;
- inferring continuity from matching resource keys alone;
- interpreting raw hit counts as confidence;
- residual-line peeling or overlay detection;
- thousands of point queries followed by union-find;
- treating pairwise matching as a transitive equivalence relation;
- inferring unknown voice from a coverage dip;
- deciding whether directional evidence is equivalent;
- copying a machine label directly into canon;
- hiding generation or algorithm changes behind in-place evidence updates.

Signal geometry belongs in resident. Corpus graph formation belongs in concordance. Canon
belongs exclusively to the human-facing product.

## 9. Recommended build order

1. Add `passage-v1`: adaptive, multilayer, provenance-rich resident output while preserving
   all v0 compatibility behavior.
2. Backfill the concordance evidence graph and validate passage/class formation on known
   tracklisted recordings.
3. Ship name borrowing with conflict-first behavior and one atomic answer per reviewed class.
4. Add unknown-class ranking and recurrence-class rulings.
5. Build an overlay benchmark containing clean underlays, real DJ drops, blends, and false
   silence matches; decide from that evidence whether fingerprint residuals suffice or richer
   audio features are required.

Once passage evidence exists, name borrowing, unknown surfacing, and recurrence workflow become
mostly graph and review problems rather than more audio matching algorithms.
