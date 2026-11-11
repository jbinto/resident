//! SEGMENTS: a store that grows by appending segments instead of rewriting shards.
//!
//! A segment is one immutable set of 64 key-sharded shard files. `append` writes only the new
//! resources, as a new segment; `tombstone` (and a replacing append) marks a resource's postings in
//! its segment dead without touching any file; `compact` merges segments shard by shard, keeping
//! ids and dropping dead postings. Queries merge every segment's live hits back into one-store
//! order (store.rs `lookup_shard_range`), so a segmented store answers exactly as the store a full
//! compaction would write, which is the store in-place `ingest`/`retire` would have written.
//!
//! One writer at a time: every writer holds an exclusive lock on `<root>/LOCK` (released by the OS
//! if the process dies). Publication is the usual atomic manifest swap; files are written,
//! synced and renamed into place before the manifest that names them.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::Path;

use serde::Serialize;

use super::{
    DeadResource, Manifest, ResourceInfo, SEGMENTED_MANIFEST_VERSION, SHARD_COUNT, SegmentInfo,
    Store, publish_manifest, shard_for_key, write_shard,
};
use crate::{DumpResource, Error, Result};

/// Segments merge in tiers: a segment's tier grows by one each time its live postings grow by
/// `TIER_RATIO` past `TIER_BASE_POSTINGS`, and `MERGE_FANIN` segments in one tier merge into one.
/// An hour of audio is about 0.13 M postings, so tier 0 holds appends up to about 8 h; every
/// tier holds fewer than `MERGE_FANIN` segments after an append, so a store of total size T keeps
/// at most about 3 * log4(T / 2^20) + 1 segments.
const TIER_BASE_POSTINGS: u64 = 1 << 20;
const TIER_RATIO: u64 = 4;
const MERGE_FANIN: usize = 4;
/// A segment at least this dead is rewritten on its own (expiry reclaims space).
const DEAD_FRACTION_REWRITE: f64 = 0.5;

#[derive(Clone, Debug, Serialize)]
pub struct AppendStats {
    pub generation: String,
    pub segments: usize,
    pub resources_added: usize,
    pub resources_replaced: usize,
    pub resources_unchanged: usize,
    pub postings_added: u64,
    /// Postings written to shard files: the new segment's plus any merges it triggered.
    pub postings_written: u64,
    pub merges: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct TombstoneStats {
    pub generation: String,
    pub segments: usize,
    pub postings_retired: u64,
    pub postings_written: u64,
    pub merges: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct CompactStats {
    pub generation: String,
    pub segments_before: usize,
    pub segments: usize,
    pub merges: usize,
    pub postings_written: u64,
    pub postings_dropped: u64,
}

/// Hold the store's single-writer lock for the guard's lifetime.
pub(super) fn writer_lock(root: &Path) -> Result<File> {
    let path = root.join("LOCK");
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|source| Error::io(&path, source))?;
    match file.try_lock() {
        Ok(()) => Ok(file),
        Err(TryLockError::WouldBlock) => Err(Error::BadRequest(format!(
            "another writer holds {}",
            path.display()
        ))),
        Err(TryLockError::Error(source)) => Err(Error::io(&path, source)),
    }
}

impl Store {
    /// Add `resources` as one new segment (SEGMENTS). New keys take the next ids; a changed key
    /// (with `replace`) keeps its id, its old postings are tombstoned and its new ones go in the
    /// new segment. Then, with `merge`, due tiered merges run (a caller may defer them to a quiet
    /// hour with `compact`). Writes O(new postings) plus whatever merges fire, never the untouched
    /// shards.
    pub fn append(
        root: &Path,
        resources: Vec<DumpResource>,
        replace: bool,
        merge: bool,
    ) -> Result<(Self, AppendStats)> {
        if resources.is_empty() {
            return Err(Error::BadRequest("append has no resources".into()));
        }
        fs::create_dir_all(root).map_err(|source| Error::io(root, source))?;
        let _lock = writer_lock(root)?;
        let current = match Self::open(root) {
            Ok(store) => store,
            Err(Error::StoreMissing(_)) => {
                let resources_added = resources.len();
                let postings_added = resources.iter().map(|r| r.prints.len() as u64).sum();
                let store = Self::build(root, resources)?;
                let stats = AppendStats {
                    generation: store.manifest.generation.clone(),
                    segments: store.segments.len(),
                    resources_added,
                    resources_replaced: 0,
                    resources_unchanged: 0,
                    postings_added,
                    postings_written: postings_added,
                    merges: 0,
                };
                return Ok((store, stats));
            }
            Err(error) => return Err(error),
        };
        let plan = current.plan_ingest(resources, replace)?;
        if plan.changes.is_empty() {
            let stats = AppendStats {
                generation: current.manifest.generation.clone(),
                segments: current.segments.len(),
                resources_added: 0,
                resources_replaced: 0,
                resources_unchanged: plan.resources_unchanged,
                postings_added: 0,
                postings_written: 0,
                merges: 0,
            };
            return Ok((current, stats));
        }

        let mut manifest = current.manifest.clone();
        let mut segments = manifest.segment_list();
        for key in plan.changes.keys() {
            if let Ok(existing) = current.resource(key) {
                segments[existing.segment as usize].dead.push(DeadResource {
                    id: existing.id,
                    shard: existing.shard,
                    postings: existing.postings,
                });
            }
        }
        manifest
            .resources
            .retain(|resource| !plan.changes.contains_key(&resource.key));
        let mut groups = BTreeMap::<u32, Vec<(u32, DumpResource)>>::new();
        for (id, resource) in plan.changes.into_values() {
            groups
                .entry(shard_for_key(&resource.meta.key))
                .or_default()
                .push((id, resource));
        }
        let index = segments.len() as u32;
        let (segment, infos) = write_segment(root, index, groups)?;
        manifest.resources.extend(infos);
        segments.push(segment);
        manifest.set_segments(segments);
        publish_manifest(root, &manifest)?;
        drop(current);

        let store = Self::open(root)?;
        let (store, compacted) = if merge {
            store.compact_locked(root, false)?
        } else {
            let stats = CompactStats {
                generation: store.manifest.generation.clone(),
                segments_before: store.segments.len(),
                segments: store.segments.len(),
                merges: 0,
                postings_written: 0,
                postings_dropped: 0,
            };
            (store, stats)
        };
        let stats = AppendStats {
            generation: store.manifest.generation.clone(),
            segments: store.segments.len(),
            resources_added: plan.resources_added,
            resources_replaced: plan.resources_replaced,
            resources_unchanged: plan.resources_unchanged,
            postings_added: plan.postings_added,
            postings_written: plan.postings_added + compacted.postings_written,
            merges: compacted.merges,
        };
        Ok((store, stats))
    }

    /// Retire `keys` without rewriting anything: their postings are marked dead in their segment
    /// and stop counting at once. Space comes back when a merge rewrites that segment.
    pub fn tombstone(root: &Path, keys: &[String]) -> Result<(Self, TombstoneStats)> {
        let _lock = writer_lock(root)?;
        let current = Self::open(root)?;
        let unique: BTreeSet<&str> = keys.iter().map(String::as_str).collect();
        if unique.is_empty() {
            return Err(Error::BadRequest("tombstone has no keys".into()));
        }
        if unique.len() >= current.manifest.resources.len() {
            return Err(Error::BadRequest(
                "retiring every resource would create an empty store".into(),
            ));
        }
        let mut manifest = current.manifest.clone();
        let mut segments = manifest.segment_list();
        let mut postings_retired = 0;
        for key in &unique {
            let resource = current.resource(key)?;
            segments[resource.segment as usize].dead.push(DeadResource {
                id: resource.id,
                shard: resource.shard,
                postings: resource.postings,
            });
            postings_retired += resource.postings;
        }
        manifest
            .resources
            .retain(|resource| !unique.contains(resource.key.as_str()));
        manifest.set_segments(segments);
        publish_manifest(root, &manifest)?;
        drop(current);
        let (store, compacted) = Self::open(root)?.compact_locked(root, false)?;
        let stats = TombstoneStats {
            generation: store.manifest.generation.clone(),
            segments: store.segments.len(),
            postings_retired,
            postings_written: compacted.postings_written,
            merges: compacted.merges,
        };
        Ok((store, stats))
    }

    /// Run due tiered merges, or with `all` merge every segment into one: the result is then a
    /// plain version-1 store, identical to the one in-place ingest/retire would have written.
    pub fn compact(root: &Path, all: bool) -> Result<(Self, CompactStats)> {
        let _lock = writer_lock(root)?;
        Self::open(root)?.compact_locked(root, all)
    }

    fn compact_locked(self, root: &Path, all: bool) -> Result<(Self, CompactStats)> {
        let segments_before = self.segments.len();
        let mut store = self;
        let mut merges = 0;
        let mut postings_written = 0;
        let mut postings_dropped = 0;
        while let Some(chosen) = merge_plan(&store.manifest, all) {
            let (manifest, written, dropped) = store.merge_segments(root, &chosen)?;
            publish_manifest(root, &manifest)?;
            drop(store);
            store = Self::open(root)?;
            merges += 1;
            postings_written += written;
            postings_dropped += dropped;
        }
        let stats = CompactStats {
            generation: store.manifest.generation.clone(),
            segments_before,
            segments: store.segments.len(),
            merges,
            postings_written,
            postings_dropped,
        };
        Ok((store, stats))
    }

    /// Write the live resources of segments `chosen` as one segment, shard by shard (peak memory
    /// about one shard of theirs), in place of the first chosen one. Ids are kept.
    fn merge_segments(&self, root: &Path, chosen: &[usize]) -> Result<(Manifest, u64, u64)> {
        let chosen_set: HashSet<usize> = chosen.iter().copied().collect();
        let mut segments = self.manifest.segment_list();
        let dropped: u64 = chosen
            .iter()
            .flat_map(|&index| &segments[index].dead)
            .map(|dead| dead.postings)
            .sum();
        let mut groups = BTreeMap::<u32, Vec<&ResourceInfo>>::new();
        for resource in &self.manifest.resources {
            if chosen_set.contains(&(resource.segment as usize)) {
                groups.entry(resource.shard).or_default().push(resource);
            }
        }
        let target = *chosen.iter().min().expect("a merge has segments");
        let mut written = 0;
        let mut shards = Vec::with_capacity(SHARD_COUNT as usize);
        let mut infos = Vec::new();
        for number in 0..SHARD_COUNT {
            let group = groups
                .remove(&number)
                .unwrap_or_default()
                .into_iter()
                .map(|resource| Ok((resource.id, self.dump_resource(resource)?)))
                .collect::<Result<Vec<_>>>()?;
            let (shard, mut shard_infos) = write_shard(&self.root, number, group)?;
            written += shard.postings;
            shards.push(shard);
            infos.append(&mut shard_infos);
        }
        debug_assert_eq!(root, self.root.as_path());

        // Renumber: the merged segment takes `target`'s place, the other chosen ones go.
        let mut renumber = vec![None; segments.len()];
        let mut kept = Vec::new();
        // A merge that leaves nothing alive (every resource retired) drops the segment, unless it
        // is the store's only one.
        let mut merged =
            (!infos.is_empty() || chosen.len() == segments.len()).then(|| SegmentInfo {
                shards,
                dead: Vec::new(),
            });
        for (index, segment) in segments.drain(..).enumerate() {
            if index == target {
                if let Some(merged) = merged.take() {
                    renumber[index] = Some(kept.len() as u32);
                    kept.push(merged);
                }
            } else if !chosen_set.contains(&index) {
                renumber[index] = Some(kept.len() as u32);
                kept.push(segment);
            }
        }
        let merged_index = renumber[target].unwrap_or(0);
        let mut manifest = self.manifest.clone();
        manifest
            .resources
            .retain(|resource| !chosen_set.contains(&(resource.segment as usize)));
        for resource in &mut manifest.resources {
            resource.segment = renumber[resource.segment as usize].expect("unchosen kept");
        }
        for mut info in infos {
            info.segment = merged_index;
            manifest.resources.push(info);
        }
        manifest.set_segments(kept);
        Ok((manifest, written, dropped))
    }
}

/// Write one new segment from `groups` (shard number to `(id, resource)`), every shard present.
fn write_segment(
    root: &Path,
    index: u32,
    mut groups: BTreeMap<u32, Vec<(u32, DumpResource)>>,
) -> Result<(SegmentInfo, Vec<ResourceInfo>)> {
    let mut shards = Vec::with_capacity(SHARD_COUNT as usize);
    let mut infos = Vec::new();
    for number in 0..SHARD_COUNT {
        let group = groups.remove(&number).unwrap_or_default();
        let (shard, mut shard_infos) = write_shard(root, number, group)?;
        shards.push(shard);
        for info in &mut shard_infos {
            info.segment = index;
        }
        infos.append(&mut shard_infos);
    }
    Ok((
        SegmentInfo {
            shards,
            dead: Vec::new(),
        },
        infos,
    ))
}

fn tier(live_postings: u64) -> u32 {
    let mut tier = 0;
    let mut ceiling = TIER_BASE_POSTINGS;
    while live_postings > ceiling {
        tier += 1;
        ceiling = ceiling.saturating_mul(TIER_RATIO);
    }
    tier
}

/// The next merge to run, if any: with `all`, every segment while there is more than one or any
/// tombstone; else a segment at least half dead, then the lowest tier holding `MERGE_FANIN`.
fn merge_plan(manifest: &Manifest, all: bool) -> Option<Vec<usize>> {
    if manifest.version != SEGMENTED_MANIFEST_VERSION {
        return None;
    }
    let segments = manifest.segment_list();
    if all {
        return Some((0..segments.len()).collect());
    }
    let mut live = Vec::with_capacity(segments.len());
    for (index, segment) in segments.iter().enumerate() {
        let total: u64 = segment.shards.iter().map(|shard| shard.postings).sum();
        let dead: u64 = segment.dead.iter().map(|dead| dead.postings).sum();
        if total > 0 && dead as f64 >= DEAD_FRACTION_REWRITE * total as f64 {
            return Some(vec![index]);
        }
        live.push(total.saturating_sub(dead));
    }
    let mut by_tier = BTreeMap::<u32, Vec<usize>>::new();
    for (index, postings) in live.iter().enumerate() {
        by_tier.entry(tier(*postings)).or_default().push(index);
    }
    by_tier
        .into_values()
        .find(|members| members.len() >= MERGE_FANIN)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiers_grow_by_the_ratio() {
        assert_eq!(tier(0), 0);
        assert_eq!(tier(TIER_BASE_POSTINGS), 0);
        assert_eq!(tier(TIER_BASE_POSTINGS + 1), 1);
        assert_eq!(tier(TIER_BASE_POSTINGS * TIER_RATIO), 1);
        assert_eq!(tier(TIER_BASE_POSTINGS * TIER_RATIO + 1), 2);
    }
}
