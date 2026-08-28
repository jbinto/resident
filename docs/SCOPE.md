# SCOPE — what this engine is and is not

Slim and built-to-purpose: this engine implements **only the subset of Panako's behavior the
archive actually uses**, and it knows its own boundaries. If the subset assumption ever turns
out wrong, the engine says so loudly instead of approximating.

## In (v0)

- STRATEGY=PANAKO matching semantics under the single pinned config (ENGINE-FACTS §config).
- Ingest of Panako plain-text fingerprint dumps; own versioned store (forward + inverted
  orders); generation swap; retire.
- Verbs: ping · match · span · crosscheck · ingest · retire · stats · extract · enroll
  (CONTRACT.md).
- Fixture verification harness (`verify`).
- Conversion layer: time bins↔seconds, freq bins↔Hz.
- **Extraction** (SPEC §extraction — built AFTER matcher parity is green): decode/resample
  front (ffmpeg subprocess or established crates) → log-frequency transform → event points →
  triplets → Panako's exact hash packing. Two-tier validation; bit-exactness ruled
  not-required. This lane sheds the Java + Gaborator dependency entirely.

## Out (deliberately, v0) — asked-for = `unsupported` error, never approximation

- OLAF strategy, monitor/sync modes, any Panako CLI surface beyond the subset above.
- IDF/rarity weighting (measured mild; ruled out).
- Multi-line (blend) match emission — structured-for, not built (SPEC §allowances).
- Network transport (stdio only; unix socket is a planned iteration).
- Reading Panako's LMDB directly (dumps are the compat seam).
- CI, packaging, distribution (will come; `./check.sh` is the gate meanwhile).

## Boundary awareness (required behaviors)

- Store manifest pins the fingerprint config identity; mismatched dumps/probes → `config_mismatch`.
- Unknown verb / out-of-subset request → `unsupported`, naming what was asked.
- Absent/empty/mismatched store → typed error; never zero-matches-as-answer.
- Hash values must fit 34 bits; a dump violating the grammar or ranges fails ingest loudly
  with file+line context.

## Named future lanes (design for, don't build)

1. **Multi-line blend emission** — surface the hit-cloud lines Panako deletes.
2. **Unix-socket transport** for multi-client serving.
3. **CI** — `check.sh` becomes the pipeline verbatim.
4. **Full-corpus re-fingerprint** — once the extract lane holds, the whole corpus gets
   re-extracted by THIS engine (~1–2 days parallel compute on the corpus machine, ruled
   acceptable) and Panako exits the system entirely.
5. **Full-corpus scale** — dev store is ~20 resources; production is ~2,900 / ~412M postings.
   Nothing in the design may assume the dev size (mmap + binary search scales; document any
   place you knowingly traded scale for simplicity).
