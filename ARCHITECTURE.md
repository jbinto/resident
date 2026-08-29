# ARCHITECTURE — current implementation

Resident is a Rust 2024 workspace:

- `core/` (`resident-core`) owns fingerprint/config types, typed errors, dump parsing, storage,
  matching, and extraction.
- `resident/` is the CLI and JSON-lines process edge. It may use `anyhow`; core does not.

The pinned Panako-compatible configuration lives in `core/src/config.rs`. Time is stored as
integer transform bins and converted to seconds only at an API edge. Fingerprints are the
plain fact tuple `(hash: u64, t: u32, f: u16)`; Panako resource ids are import metadata, not
engine identity.

## Store

A store root contains `CURRENT`, immutable JSON generation manifests in `generations/`, and
immutable binary files in `shards/`. `CURRENT` is replaced atomically only after every shard
and the new manifest have been synced. Readers retain their mappings when a later generation
is published.

Resources are assigned deterministically across 64 shards. Each little-endian shard has a
fixed header followed by three directly mapped regions:

1. forward postings sorted by `(resource, time, hash, frequency)`;
2. a distinct-hash index of `(hash, hit start, hit count)`;
3. compact inverted hits sorted by `(hash, resource, time, frequency)`.

The forward range for each resource is recorded in the manifest. A hash lookup binary-searches
each shard index and reads only matching hit ranges. The fixture generation is 168 MiB for
3,208,323 postings, which projects to about 21 GiB at production count.

`core/src/mmap_view.rs` is the sole unsafe-code exception. Its invariant is that published
shard paths are immutable and the backing file remains open for the mapping's lifetime.

Ingest compares canonical per-resource content hashes. Identical resources are no-ops;
replacement and retirement reconstruct and rewrite only affected shards, reusing every other
content-addressed shard. The current and immediately previous manifest are retained, and
unreferenced derived shards are removed after publication. Existing mappings remain valid on
the Linux deployment target even when an old path is unlinked.

## Matching

`core/src/matcher.rs` performs a ±2 hash lookup, groups hits by resource, fits Panako's
dominant offset line, applies factor/residual/duration/coverage gates, and returns stable
score-descending rows. Lookup of distinct probe hashes is parallel; voting per candidate is
parallel. Evidence retains filtered hits, leading offset bins, and per-second density only
when requested.

Compatibility-specific behavior—duplicate probe hashes, Java float arithmetic, and Java
`HashMap` tie iteration—is contained in this module. Store ordering and public result ordering
remain deterministic and independent of those quirks.

## Store-to-store queries

`span` and `crosscheck` read A from the forward order and divide long ranges into independent
30-second evidence regions. Each region uses the same voter; no adjacent results are merged.
`crosscheck` queries all targets once per region, in parallel, then groups stable raw segments
by reference. A full 7,500-second fixture resource crosschecks the 3.2M-posting store in about
0.5 seconds on the development Mac.
