# SPEC — behavioral specification

This document states the behavior Resident is expected to preserve. `ARCHITECTURE.md` describes the
implementation, `CONTRACT.md` defines the public interface, and fixtures decide any disagreement
about Panako compatibility.

## Mission

Resident answers cross-recording audio questions over fingerprints stored once. It supports a
Panako-compatible matching lane and a higher-level passage lane without requiring audio decode or
extraction during stored-resource queries.

The reference corpus used to shape the engine contains roughly 2,900 long-form recordings and 412
million postings. Both sides of a question are commonly already fingerprinted. The store therefore
serves two access orders:

- inverted `(hash → resource/time/frequency)` lookup for identification;
- forward `(resource/time → hash/frequency)` scans for stored-resource windows.

Native extraction makes new stores independent of Java, JNI, and Gaborator. Imported Panako dumps
remain supported as a compatibility and migration path.

## Store requirements

- Opening an existing store maps immutable files; it performs no posting deserialization pass.
- Readers need no write access and may safely share a generation across processes.
- A writer builds immutable shards and a manifest before atomically publishing `CURRENT`.
- Existing readers finish against their captured snapshot after a new generation is published.
- Identical-content ingest at an existing key is a no-op. Changed content requires explicit
  replacement.
- Retirement removes a resource from rebuilt affected shards; tombstones do not accumulate.
- Every generation pins a fingerprint configuration identity and rejects incompatible input.
- Missing, corrupt, empty, or version-incompatible stores fail distinctly and never masquerade as
  an empty match result.
- The store is a derived artifact. Its recovery mechanism is a rebuild from dumps or audio, not a
  journal.
- Mmap plus the operating-system page cache is both the modest-machine and large-RAM strategy.

The on-disk format is private to Resident and is not expected to match Panako storage.

## Compatibility matching

The default voter reproduces the pinned Panako 2.1 behavior documented in
`docs/ENGINE-FACTS.md`:

- ±2 hash lookup;
- the oracle's modal offset-line fit, gates, Java float arithmetic, and tie behavior;
- score as the accepted filtered-hit count;
- stable score-descending output with a key tie-break;
- every computed factor and coverage field retained;
- optional raw accepted hits, offset peaks, and per-second density.

`verify` must preserve exact reference sets and integer scores on all 22 oracle questions. Times may
differ by at most 0.02 seconds and factors by at most 0.001. A fixture disagreement is a regression
unless a deliberate compatibility change is recorded in `DECISIONS.md` with a replacement oracle.

`multi_line` is off by default. When enabled, the unchanged voter repeatedly peels accepted lines
from the residual hit cloud and returns every surviving line in stable score order. This is
additive evidence for blends and repeated offsets, not part of default jar parity.

## Stored-resource questions

`span` compares one stored A window with one B resource. `crosscheck` compares A with a target store
and may rank references. Their compatibility geometry is independent, non-overlapping 30-second A
regions. They emit local segments and do not stitch them.

A and B may be separate immutable stores only when their configuration identities agree. Source
keys resolve in A; target keys resolve in B. An unknown key is an error, not an empty answer.

The required behavioral checks are:

1. a resource window matches itself on the identity diagonal;
2. known shared fixture material agrees with the match oracle's cross-references;
3. split-store and same-store answers agree when their relevant fingerprints are identical;
4. pair-restricted shard lookup emits the exact segment set of the all-shard reference path;
5. every result is deterministic and stable-ordered.

## Passage observations

`passages` and `discover` turn local multiline evidence into record-grained, directional
observations. They use the versioned `passage-v3` profile:

- 12-second A regions on an 8-second hop, anchored at zero;
- exact hit deduplication across overlapping regions;
- support runs split when either clock lacks an accepted hit for more than 1.5 seconds;
- ordinary stitching across at most 20 seconds when offset and factor tolerances hold;
- locked-offset stitching across at most 30 seconds under tighter tolerances;
- explicit support spans, so bridged holes are never reported as matched audio;
- contained weak residual lines retained as alternates instead of top-level questions;
- strong or non-contained concurrent alignments retained as primary passages;
- conservative `same_audio_candidate` marking for near-total, zero-offset, unit-factor diagonals.

Passage output proves presence only. It does not prove exclusivity, classify an overlay, name the
audio, merge human identities, or bless same-audio revisions. The request remains directional;
absence of B→A cannot reject a surviving A→B observation.

A passage ID is derived from the passage profile, config identity, endpoint keys and prints-only
content hashes, and support geometry. It excludes mutable metadata and volatile quality summaries.
Changing identity-affecting geometry requires a new passage profile.

`discover` exhaustively evaluates its stated target set without a top-*k* cutoff. The exact source
key and caller-supplied exclusions are omitted. A consumer that wants direction-unioned corpus
observations schedules the inverse enrollment fan-out separately and preserves both provenances.

## Extraction

The native extraction lane performs:

1. ffmpeg decode/resample to signed mono 16 kHz PCM;
2. the pinned log-frequency transform;
3. Panako-compatible event selection;
4. triplet construction;
5. exact 34-bit landmark hash packing.

Decoded PCM is spooled to an auto-deleting file and processed in overlapping bounded cores.
Auxiliary memory is independent of audio duration. Events and prints are emitted in canonical
order. Concurrent requests share immutable FFT plans but not mutable scratch.

Bit identity with Panako extraction is not required. Acceptance is answer-faithful and measured at
three levels:

- print-set overlap against the same decoded windows;
- time/frequency anchor overlap;
- downstream reference and span recovery through Resident's matcher.

`validate-stream` additionally requires the bounded implementation to equal its whole-input
reference byte-for-byte on every fixture window and across an internal core boundary/final flush.

## Batch production and comparison

`refingerprint` consumes a strict JSONL corpus manifest. It binds the output directory to the exact
manifest bytes, writes each print file atomically before its metadata completion marker, validates
completed work on resume, retries incomplete resources, and emits deterministic failure records.
`--jobs` is a hard extraction concurrency bound.

`ab-compare` records both immutable store generations and compares answer sets, span overlap, and
score drift for external print probes or stored-resource windows. Evidence can be attached to a
named question. Store replacement is accepted by answer-faithful cohort review, not by shard or
print byte equality.

## Output doctrine

- Emit measurements, not human conclusions.
- Preserve raw local segments in compatibility APIs; passage stitching belongs only to the
  explicitly versioned passage API.
- Do not turn scores into probabilities.
- Do not infer negative evidence from a missing reverse match.
- Do not force single occupancy: simultaneous alignments are legal.
- Do not hide duplicate-encoding candidates before a caller blesses their revision relation.
- Make granular evidence opt-in so default and corpus-wide responses remain tractable.
- Return typed errors for every failure mode; empty results mean the question ran successfully.
- Keep stdout machine-readable and reserve stderr for diagnostics.

## Unsupported work

Resident supports only the pinned Panako strategy/configuration. It does not implement OLAF,
monitor/sync mode, rarity weighting, song metadata, human canon, recurrence-class governance,
set/track containment, overlay classification, source separation, or a multi-user review workflow.
Out-of-scope protocol requests fail as `unsupported`; they are never silently approximated.

The detailed boundary is maintained in `docs/SCOPE.md`.
