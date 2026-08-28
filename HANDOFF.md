# HANDOFF — build brief for the engine agent

You are building a **resident audio-fingerprint matching engine in Rust**, from scratch, in this
repo. You have full judgment. This document is your mission; the rest of the repo is your
material. Nobody will answer questions mid-build — decide, log the decision, keep moving.

## The mission in one paragraph

A 20-year radio-DJ archive (~2,900 recordings, ~2 hours each) was fingerprinted with
[Panako](https://github.com/JorenSix/Panako) (Java, AGPL). The archive's owner needs the
*matching* side of that system as a fast, long-lived, footgun-free native engine: it ingests the
already-extracted fingerprints (plain-text dumps, provided), builds its own on-disk store, and
answers match questions over a simple JSON protocol — including question shapes the Java tool
cannot ask, like "compare stored recording A's window against stored recording B" with no audio
in the loop. It also grows its own extraction lane (audio → fingerprints, same hash family) so
the Java tool and its C++ transform dependency can exit the system entirely.
**This is not a port.** Panako is the *oracle*: golden fixtures in `fixtures/` are
its recorded answers, and your engine must reproduce them. How you get there — data structures,
file formats, internals — is yours to design. Where Panako's code has papercuts (they are
catalogued), you are explicitly required NOT to reproduce them.

## Read order

1. `SPEC.md` — what to build, acceptance criteria, boundaries.
2. `docs/ENGINE-FACTS.md` — everything reverse-engineered about Panako's data and matcher.
   This saves you hours; verify load-bearing claims against upstream source where cheap.
3. `CONTRACT.md` — the wire protocol your daemon speaks.
4. `AGENTS.md` — how to work in this repo (toolchain, style, deps, commits).
5. `fixtures/README.md` — the golden fixtures and what parity means.

Reference source: clone upstream Panako for reading (it is AGPL — which is why THIS repo is
AGPL): `git clone https://github.com/JorenSix/Panako && git -C Panako checkout e4b0e1d`.
The files that matter: `PanakoStrategy.java` (matcher, lines 263–496), `PanakoFingerprint.java`
(hash, lines 231–273), `PanakoStorageFile.java` (.tdb grammar), `Config.java`/`Key.java`
(defaults). Read to understand semantics; do not transliterate structure.

## Build order (suggested, not mandated)

1. **Ingest + store**: parse the dump format, design your on-disk store (see SPEC constraints),
   build it from `fixtures/store-dump/`.
2. **`match` + the verify harness**: implement the matcher, then `cargo run -- verify fixtures/`
   until parity is green. This is the heart of the build — everything else is plumbing.
3. **`span` and `crosscheck`**: the store-vs-store verbs (the reason this engine exists).
   Behavioral checks, no jar golden — see SPEC §acceptance.
4. **The daemon loop**: JSON-lines over stdin/stdout per CONTRACT.md.
5. **`ingest`/`retire`/`stats`**, generation swap.
6. **The extraction lane** (SPEC §extraction): `extract`/`enroll`, validated two-tier against
   the fixture windows' audio + prints. This is what makes the system fully independent of
   Java and the Gaborator — build it after the matcher gate is green, and report honestly how
   far it got.
7. **REPORT.md** — see below.

## Definition of done

- `./check.sh` green (fmt + clippy -D warnings + tests).
- `verify` reports parity per SPEC §acceptance, with every divergence either fixed or
  root-caused in `DECISIONS.md`.
- The daemon runs, answers every CONTRACT verb, and survives malformed input with typed errors.
- `ARCHITECTURE.md` describes what IS (store layout, module map, data flow) — written for a
  maintainer who was not present.
- `DECISIONS.md` logs every judgment call you made and why (format choices, tolerances,
  deviations from ENGINE-FACTS, anything you found that contradicts the docs).
- `REPORT.md` — your closing brief: what works, what diverged from the oracle and why, what you
  did not build, what you would do next, and anything in the fixtures or facts that smelled
  wrong. Assume the maintainer reads REPORT.md first.

## When uncertain

Prefer: decide + log in DECISIONS.md. If a fixture contradicts ENGINE-FACTS.md, trust the
fixture (it is observed behavior; the doc is reconstruction). If the oracle's behavior is
genuinely ambiguous, match it where cheap, and where matching it would mean reproducing a
catalogued papercut, don't — document the intentional divergence instead.
