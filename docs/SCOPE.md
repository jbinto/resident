# SCOPE — what this engine is and is not

Slim and built-to-purpose: this engine implements **only the subset of Panako's behavior the
archive actually uses**, and it knows its own boundaries. If the subset assumption ever turns
out wrong, the engine says so loudly instead of approximating.

## In (v0)

- STRATEGY=PANAKO matching semantics under the single pinned config (ENGINE-FACTS §config).
- Ingest of Panako plain-text fingerprint dumps; own versioned store (forward + inverted
  orders); generation swap; retire.
- Verbs: ping · match · span · crosscheck · passages · discover · ingest · retire · stats · extract · enroll
  (CONTRACT.md).
- Offline `rehash-identities` maintenance publishes prints-only endpoint identities without
  rewriting fingerprints or exposing a concurrent daemon mutation.
- Offline `set-durations` maintenance accepts a complete authoritative duration map, validates it
  against fingerprint extents, and atomically reuses identity and shard bytes.
- Fixture verification harness (`verify`).
- Opt-in ranked multi-line emission for `match`, `span`, and `crosscheck`; absent/false retains
  jar single-line parity and original region behavior.
- Resumable, bounded-parallel native re-fingerprinting from JSONL audio manifests to the
  Panako dump grammar, with deterministic failures and atomic per-resource completion.
- A/B agreement reports for two store generations over external probes or stored-resource
  windows, including row spans, score deltas, ranked divergences, and named evidence dumps.
- Cross-store `span` and `crosscheck`: probe resources in A, explicit targets or an entire
  reference/additional-corpus store in B, under one required config identity.
- Additive `passage-v3` observations: production-compatible 12-second/8-second query geometry,
  deterministic pairwise passage ids, explicit dense-support
  intervals and holes, quality vectors, endpoint revisions, geometry-aware region stitching,
  dominated-alignment alternates, same-audio candidates, and exhaustive directional discovery
  without top-`k` graph loss.
- Conversion layer: time bins↔seconds, freq bins↔Hz.
- **Extraction** (SPEC §extraction — built AFTER matcher parity is green): decode/resample
  front (ffmpeg subprocess or established crates) → log-frequency transform → event points →
  triplets → Panako's exact hash packing. Two-tier validation; bit-exactness ruled
  not-required. Bounded-memory chunk analysis and ordered print emission support multi-hour
  inputs. This lane sheds the Java + Gaborator dependency entirely.

## Out (deliberately, v0) — asked-for = `unsupported` error, never approximation

- OLAF strategy, monitor/sync modes, any Panako CLI surface beyond the subset above.
- IDF/rarity weighting (measured mild; ruled out).
- Network transport (stdio only; unix socket is a planned iteration).
- Reading Panako's LMDB directly (dumps are the compat seam).
- CI, packaging, distribution (will come; `./check.sh` is the gate meanwhile).
- Corpus-global recurrence-class identity, title/name resolution, canon mutation, and human review
  workflow. Resident supplies immutable signal observations; a persistent middle layer owns these.
- Claims that an unmatched overlay is voice or that a matched underlay is the only active layer.
  Sparse landmark fingerprints prove presence, not exclusivity or reconstructable residual audio.

## Boundary awareness (required behaviors)

- Store manifest pins the fingerprint config identity; mismatched dumps/probes → `config_mismatch`.
- Unknown verb / out-of-subset request → `unsupported`, naming what was asked.
- Absent/empty/mismatched store → typed error; never zero-matches-as-answer.
- Hash values must fit 34 bits; a dump violating the grammar or ranges fails ingest loudly
  with file+line context.

## Named future lanes (design for, don't build)

1. **Unix-socket transport** for multi-client serving.
2. **CI** — `check.sh` becomes the pipeline verbatim.
3. **Run the full-corpus re-fingerprint and cutover** — the resumable tool and A/B gate now
   exist; the 10–20 hour production execution remains an operator action.
4. **Full-corpus scale validation** — dev store is ~20 resources; production is ~2,900 /
   ~412M postings.
   Nothing in the design may assume the dev size (mmap + binary search scales; document any
   place you knowingly traded scale for simplicity).
