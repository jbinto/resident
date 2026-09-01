# Fixture capture rig

`golden.py` is the provenance harness for the bundled compatibility fixtures. It is not part of the
Resident runtime and is not a portable fixture downloader.

The script was run on an isolated corpus machine against a locally built Panako 2.1 jar from commit
`e4b0e1d`. It records the jar SHA-256 and effective configuration in `fixtures/manifest.json` so the
oracle can be attributed to exact code and settings rather than to the vague label “Panako.”

## What it captures

In one run the harness:

1. selects known shared-audio pairs and unrelated distractors from local corpus measurements;
2. builds a temporary Panako LMDB store from cached prints, without re-extracting the references;
3. exports the selected resources through Panako's file-cache dump grammar;
4. cuts 12-second WAV query windows with ffmpeg;
5. asks the jar to extract and cache each query's exact fingerprints;
6. queries the temporary store with those same prints;
7. preserves parsed rows and the jar's raw output lines;
8. writes a manifest and a compressed fixture bundle.

That separation matters. `prints.tdb` fixes the probe population for matcher parity, while
`window.wav` lets the native extractor be evaluated independently against the same sound.

## Environment assumptions

The checked-in script intentionally contains absolute paths and a corpus-specific pair-selection
input. They document the original run; they are not defaults for another machine. Before reusing
the harness, review and replace at least:

- the Java and Panako jar paths;
- Panako's file-cache directory;
- the known-pair input;
- the output and lock paths;
- the ffmpeg command and available CPU limit.

Run it only on an isolated capture host. The script replaces its configured output directory and
uses a best-effort lock shared with the original query runner.

## Reproduction standard

A replacement fixture set must retain:

- the Panako upstream commit and exact jar digest;
- the full effective configuration snapshot;
- source keys, resource IDs, durations, and print counts;
- exact cached query prints;
- raw jar response lines as well as parsed fields;
- query audio sufficient to rerun native extraction validation.

Do not hand-edit generated fixtures. If a new oracle intentionally changes answers, publish it as a
new fixture profile and explain the compatibility boundary in `DECISIONS.md`.

See [fixtures/README.md](../fixtures/README.md) for the checked-in layout and validation commands.

## Upstream credit and redistribution

The oracle exists because [Panako](https://github.com/JorenSix/Panako), developed by Joren Six at
IPEM, Ghent University, exposes a reproducible fingerprint cache and query path. Panako 2.1 obtains
its log-frequency analysis through JGaborator and [Gaborator](https://www.gaborator.com/). Their
software is not vendored by this harness.

The repository's AGPL license covers Resident's code; it does not automatically grant rights to
redistribute third-party audio used in fixtures. Anyone publishing a regenerated fixture bundle
must separately verify the rights and privacy status of its audio and metadata.
