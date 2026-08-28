# DECISIONS — rulings and reasons

Append-only. Each entry describes a choice embodied by the same commit.

## 2026-08-28 — pin the observed transform latency at 48 samples

Every golden time lies on the `t * 0.008 + 0.003` grid, within the fixture's three-decimal
formatting precision. The effective Gaborator latency is therefore 48 samples at 16 kHz. Keep
it with the other fingerprint configuration constants and include it in the config identity.

## 2026-08-28 — accept compressed dumps as first-class input

The grammar describes `.tdb`, while the delivered and production-sized fixture seam uses
`.tdb.zst`. Ingest supports either transparently and requires the print count to equal its
metadata declaration. It rejects malformed fields, hashes wider than 34 bits, and frequency
bins above 512 with file and line context.

## 2026-08-28 — keep caller identity separate from Panako ids

The redundant numeric id in each dump posting is ignored, matching Panako's file reader.
Metadata source paths become opaque caller keys. This prevents path-hash collision behavior
from becoming part of Resident's store or protocol.

## 2026-08-28 — use 64 immutable resource shards per generation

A monolithic inverted file would make real retirement rewrite the entire production store;
one inverted file per resource would make every probe perform thousands of binary searches.
Hash each caller key into one of 64 shards and keep both access orders inside that shard. A
retire or replacement can rewrite roughly 1/64 of the corpus, while a query performs 64 cheap
index searches per distinct probe hash. Unchanged content-addressed shard files can be shared
by later manifests.

The data plane is explicit little-endian fixed records. A small JSON manifest is control-plane
metadata; opening postings performs no deserialization pass. The fixture store occupies 168
MiB for 3.2M postings, projecting to roughly 21 GiB for 412M before filesystem overhead.

## 2026-08-28 — atomic publication is the durability boundary

Shard files and a generation manifest are immutable and synced before `CURRENT` is atomically
renamed and the store directory synced. Corrupt or incomplete unpublished files are harmless;
corrupt published files are rejected. Recovery remains rebuild-from-dumps, not a WAL.
