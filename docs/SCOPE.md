# SCOPE — what this engine is and is not

Slim and built-to-purpose: this engine implements **only the subset of Panako's behavior the
archive actually uses**, and it knows its own boundaries. If the subset assumption ever turns
out wrong, the engine says so loudly instead of approximating.

## In (v0)

- STRATEGY=PANAKO matching semantics under the single pinned config (ENGINE-FACTS §config).
- Ingest of Panako plain-text fingerprint dumps; own versioned store (forward + inverted
  orders); generation swap; retire.
- Verbs: ping · match · span · crosscheck · ingest · retire · stats (CONTRACT.md).
- Fixture verification harness (`verify`).
- Conversion layer: time bins↔seconds, freq bins↔Hz.

## Out (deliberately, v0) — asked-for = `unsupported` error, never approximation

- Audio decode and fingerprint **extraction** (prints arrive pre-extracted; the extraction
  subset is specced in ENGINE-FACTS §extraction for the future in-engine lane).
- OLAF strategy, monitor/sync modes, any Panako CLI surface beyond the matcher semantics.
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

1. **In-engine extraction** — kills the Java + Gaborator dependency end to end.
2. **Multi-line blend emission** — surface the hit-cloud lines Panako deletes.
3. **Unix-socket transport** for multi-client serving.
4. **CI** — `check.sh` becomes the pipeline verbatim.
5. **Full-corpus scale** — dev store is ~20 resources; production is ~2,900 / ~412M postings.
   Nothing in the design may assume the dev size (mmap + binary search scales; document any
   place you knowingly traded scale for simplicity).
