//! RAM-INDEX: the store's inverted side (hash -> postings) held in process memory, partitioned by
//! hash instead of by resource, at 8 bytes a posting.
//!
//! The mapped store answers one probe hash with a binary search in each of 64 resource-sharded
//! indexes, then reads ~64 short posting runs scattered across the hits regions (24 + 12 bytes a
//! posting, mostly in pages the process has not touched yet: minor faults). Here a probe hash
//! names one partition and one bucket directly, and every posting for it sits in one sorted run.
//! The answers are the mapped store's, in the mapped store's order, so every caller (matcher,
//! span, crosscheck) is unchanged; `tests` hold the two equal on a segmented, tombstoned store.
//!
//! Layout. A 34-bit hash splits into partition (top 6 bits), bucket (next 20) and suffix (low 8).
//! Each partition keeps a bucket offset table and one `u64` key per posting, sorted:
//! `suffix:8 | shard:6 | rank:16 | t:24 | f:10`, where `rank` orders resource ids within their
//! shard. Sorting keys therefore sorts a bucket by (hash, shard, resource id, t, f), which is the
//! mapped order once the shards are interleaved back (see `lookup_range`).

use rayon::prelude::*;

use crate::config::MAX_HASH;
use crate::store::{StoredHit, shard_count};
use crate::{Error, Result, Store};

const PARTITION_BITS: u32 = 6;
const BUCKET_BITS: u32 = 20;
const SUFFIX_BITS: u32 = 34 - PARTITION_BITS - BUCKET_BITS;
const BUCKETS: usize = 1 << BUCKET_BITS;
const SHARD_BITS: u32 = 6;
const RANK_BITS: u32 = 16;
const T_BITS: u32 = 24;
const F_BITS: u32 = 10;

/// Summary of a built index, for logs and capacity planning.
#[derive(Clone, Debug, serde::Serialize)]
pub struct RamIndexStats {
    pub postings: u64,
    pub bytes: u64,
    pub build_seconds: f64,
}

pub(crate) struct RamIndex {
    partitions: Vec<Partition>,
    /// Resource id by (shard << RANK_BITS | rank).
    ids: Vec<u32>,
}

struct Partition {
    /// `keys[offsets[b]..offsets[b + 1]]` is bucket `b`.
    offsets: Vec<u32>,
    keys: Vec<u64>,
}

struct Split {
    partition: usize,
    bucket: usize,
    suffix: u64,
}

const fn split(hash: u64) -> Split {
    Split {
        partition: (hash >> (BUCKET_BITS + SUFFIX_BITS)) as usize,
        bucket: ((hash >> SUFFIX_BITS) as usize) & (BUCKETS - 1),
        suffix: hash & ((1 << SUFFIX_BITS) - 1),
    }
}

const fn key_suffix(key: u64) -> u64 {
    key >> (64 - SUFFIX_BITS)
}

const fn key_internal(key: u64) -> usize {
    ((key >> (T_BITS + F_BITS)) & ((1 << (SHARD_BITS + RANK_BITS)) - 1)) as usize
}

const fn key_shard(key: u64) -> u64 {
    key >> (T_BITS + F_BITS + RANK_BITS) & ((1 << SHARD_BITS) - 1)
}

impl RamIndex {
    pub(crate) fn build(store: &Store) -> Result<(Self, RamIndexStats)> {
        let started = std::time::Instant::now();
        if shard_count() != 1 << SHARD_BITS {
            return Err(Error::Unsupported("RAM index expects 64 shards".into()));
        }
        // Rank resources within their shard by id, so key order follows the mapped order.
        let mut by_shard: Vec<(u32, u32)> = store
            .resources()
            .iter()
            .map(|resource| (resource.shard, resource.id))
            .collect();
        by_shard.sort_unstable();
        let mut internal_of = vec![u32::MAX; store.resource_id_limit()];
        let mut ids = vec![u32::MAX; 1 << (SHARD_BITS + RANK_BITS)];
        let mut rank = 0_u32;
        for (index, &(shard, id)) in by_shard.iter().enumerate() {
            if index > 0 && by_shard[index - 1].0 != shard {
                rank = 0;
            }
            if rank >= 1 << RANK_BITS {
                return Err(Error::Unsupported(format!(
                    "RAM index holds at most {} resources per shard",
                    1 << RANK_BITS
                )));
            }
            let internal = (shard << RANK_BITS) | rank;
            internal_of[id as usize] = internal;
            ids[internal as usize] = id;
            rank += 1;
        }

        let partitions = (0..1_usize << PARTITION_BITS)
            .into_par_iter()
            .map(|partition| build_partition(store, partition as u64, &internal_of))
            .collect::<Result<Vec<_>>>()?;
        let postings: u64 = partitions.iter().map(|p| p.keys.len() as u64).sum();
        let bytes = postings * 8
            + partitions
                .iter()
                .map(|p| p.offsets.len() as u64 * 4)
                .sum::<u64>()
            + ids.len() as u64 * 4;
        let stats = RamIndexStats {
            postings,
            bytes,
            build_seconds: started.elapsed().as_secs_f64(),
        };
        Ok((Self { partitions, ids }, stats))
    }

    /// The keys holding exactly `hash`.
    fn run(&self, hash: u64) -> &[u64] {
        let Split {
            partition,
            bucket,
            suffix,
        } = split(hash);
        let partition = &self.partitions[partition];
        let bucket = &partition.keys
            [partition.offsets[bucket] as usize..partition.offsets[bucket + 1] as usize];
        let from = bucket.partition_point(|&key| key_suffix(key) < suffix);
        let to = from + bucket[from..].partition_point(|&key| key_suffix(key) == suffix);
        &bucket[from..to]
    }

    /// Append the live postings for `[start, stop]` exactly as the mapped store emits them:
    /// shard by shard, and within a shard by (hash, resource id, t, f).
    pub(crate) fn lookup_range(&self, start: u64, stop: u64, out: &mut Vec<StoredHit>) {
        // A lookup range is a handful of hash values (QUERY_RANGE each side), one run apiece,
        // each run sorted by shard: merge them shard-major, hash-minor.
        let mut runs: Vec<(u64, &[u64])> = (start..=stop.min(MAX_HASH))
            .map(|hash| (hash, self.run(hash)))
            .filter(|(_, run)| !run.is_empty())
            .collect();
        while !runs.is_empty() {
            let shard = runs
                .iter()
                .map(|(_, run)| key_shard(run[0]))
                .min()
                .expect("nonempty runs");
            for (hash, run) in &mut runs {
                let take = run.partition_point(|&key| key_shard(key) == shard);
                for &key in &run[..take] {
                    out.push(StoredHit {
                        resource_id: self.ids[key_internal(key)],
                        t: ((key >> F_BITS) & ((1 << T_BITS) - 1)) as u32,
                        f: (key & ((1 << F_BITS) - 1)) as u16,
                        matched_hash: *hash,
                    });
                }
                *run = &run[take..];
            }
            runs.retain(|(_, run)| !run.is_empty());
        }
    }

    pub(crate) fn has_range(&self, start: u64, stop: u64) -> bool {
        (start..=stop.min(MAX_HASH)).any(|hash| !self.run(hash).is_empty())
    }
}

/// Count, then place, then sort each bucket: two reads of the partition's slice of every shard.
fn build_partition(store: &Store, partition: u64, internal_of: &[u32]) -> Result<Partition> {
    let low = partition << (BUCKET_BITS + SUFFIX_BITS);
    let high = (partition + 1) << (BUCKET_BITS + SUFFIX_BITS);
    let mut offsets = vec![0_u32; BUCKETS + 1];
    let mut total = 0_u64;
    store.for_each_live_posting(low, high, |hash, _, _, _| {
        offsets[split(hash).bucket + 1] += 1;
        total += 1;
    })?;
    if total > u64::from(u32::MAX) {
        return Err(Error::Unsupported(
            "RAM index partition exceeds 2^32 postings".into(),
        ));
    }
    for bucket in 0..BUCKETS {
        offsets[bucket + 1] += offsets[bucket];
    }
    let mut cursor = offsets.clone();
    let mut keys = vec![0_u64; total as usize];
    let mut failure = None;
    store.for_each_live_posting(low, high, |hash, resource_id, t, f| {
        let internal = internal_of
            .get(resource_id as usize)
            .copied()
            .unwrap_or(u32::MAX);
        if internal == u32::MAX || t >= 1 << T_BITS || u32::from(f) >= 1 << F_BITS {
            failure.get_or_insert((resource_id, t, f));
            return;
        }
        let Split { bucket, suffix, .. } = split(hash);
        keys[cursor[bucket] as usize] = (suffix << (64 - SUFFIX_BITS))
            | (u64::from(internal) << (T_BITS + F_BITS))
            | (u64::from(t) << F_BITS)
            | u64::from(f);
        cursor[bucket] += 1;
    })?;
    if let Some((resource_id, t, f)) = failure {
        return Err(Error::Unsupported(format!(
            "RAM index cannot hold posting (resource {resource_id}, t {t}, f {f})"
        )));
    }
    for bucket in 0..BUCKETS {
        keys[offsets[bucket] as usize..offsets[bucket + 1] as usize].sort_unstable();
    }
    Ok(Partition { offsets, keys })
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::{DumpResource, Fingerprint, ResourceMeta};

    fn resource(key: &str, prints: Vec<Fingerprint>) -> DumpResource {
        DumpResource {
            meta: ResourceMeta {
                source_id: "1".into(),
                key: key.into(),
                duration: 100.0,
                declared_prints: prints.len() as u64,
            },
            prints,
            prints_path: PathBuf::new(),
        }
    }

    fn prints(seed: u64, count: u64) -> Vec<Fingerprint> {
        let mut state = seed | 1;
        (0..count)
            .map(|index| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                // Crowd a few hash neighbourhoods, including both ends of the hash space and the
                // partition and bucket edges, so +-2 ranges cross them.
                let hash = match state % 5 {
                    0 => state % 3_000,
                    1 => MAX_HASH - state % 40,
                    2 => (1 << 28) - 20 + state % 40,
                    3 => (1 << 8) * 7 - 3 + state % 6,
                    _ => state % (MAX_HASH + 1),
                };
                Fingerprint::new(hash, (index * 3 + state % 3) as u32, (state % 513) as u16)
            })
            .collect()
    }

    fn assert_same_lookups(store: &Store, ram: &Store) {
        let mut hashes: Vec<u64> = store
            .resources()
            .iter()
            .flat_map(|r| store.forward(&r.key, None).unwrap())
            .map(|print| print.hash)
            .collect();
        hashes.extend([
            0,
            1,
            2,
            3,
            MAX_HASH,
            MAX_HASH - 1,
            (1 << 28) - 1,
            1 << 28,
            12_345,
        ]);
        hashes.sort_unstable();
        hashes.dedup();
        let mut nonempty = 0;
        for &hash in &hashes {
            let mapped = store.lookup(hash).unwrap();
            assert_eq!(ram.lookup(hash).unwrap(), mapped, "hash {hash}");
            assert_eq!(ram.has_match(hash).unwrap(), store.has_match(hash).unwrap());
            nonempty += usize::from(!mapped.is_empty());
        }
        assert!(nonempty > 1_000, "only {nonempty} hashes had hits");
        let mapped = store.lookup_table(&hashes).unwrap();
        let resident = ram.lookup_table(&hashes).unwrap();
        for &hash in &hashes {
            assert_eq!(resident.entry(hash), mapped.entry(hash));
        }
        let targets: Vec<u32> = store.resources().iter().step_by(4).map(|r| r.id).collect();
        let mapped = store.lookup_table_among(&hashes, &targets).unwrap();
        let resident = ram.lookup_table_among(&hashes, &targets).unwrap();
        for &hash in &hashes {
            assert_eq!(resident.entry(hash), mapped.entry(hash));
        }
        for info in store.resources().iter().step_by(5) {
            for &hash in hashes.iter().step_by(7) {
                assert_eq!(
                    ram.lookup_resource(hash, info.id).unwrap(),
                    store.lookup_resource(hash, info.id).unwrap()
                );
            }
        }
    }

    #[test]
    fn ram_lookups_equal_mapped_lookups_across_segments_and_tombstones() {
        let root = tempfile::tempdir().unwrap();
        let first: Vec<_> = (0..40)
            .map(|index| resource(&format!("a{index}"), prints(index + 1, 400)))
            .collect();
        Store::build(root.path(), first).unwrap();
        let second: Vec<_> = (0..20)
            .map(|index| resource(&format!("b{index}"), prints(index + 100, 300)))
            .collect();
        Store::append(root.path(), second, false, false).unwrap();
        // A replaced key tombstones its old postings in the first segment.
        Store::append(
            root.path(),
            vec![resource("a3", prints(999, 200))],
            true,
            false,
        )
        .unwrap();
        Store::tombstone(root.path(), &["a7".to_string()]).unwrap();

        let store = Store::open(root.path()).unwrap();
        assert!(store.segment_count() > 1, "the test needs several segments");
        let mut ram = Store::open(root.path()).unwrap();
        let stats = ram.load_ram_index().unwrap();
        assert_eq!(stats.postings, store.stats().postings);
        assert_same_lookups(&store, &ram);
    }
}
