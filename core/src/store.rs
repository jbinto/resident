use std::collections::{BTreeMap, HashMap};
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{MAX_HASH, config_id};
use crate::mmap_view::MappedFile;
use crate::{DumpResource, Error, Fingerprint, Result};

const STORE_VERSION: u32 = 1;
const SHARD_COUNT: u32 = 64;
const MAGIC: &[u8; 8] = b"RESIDNT1";
const HEADER_SIZE: u64 = 128;
const FORWARD_SIZE: u64 = 20;
const HASH_INDEX_SIZE: u64 = 24;
const HIT_SIZE: u64 = 12;
const FINGERPRINT_IDENTITY_PROFILE: &str = "prints-v1";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceInfo {
    pub id: u32,
    pub key: String,
    pub duration: f64,
    pub postings: u64,
    pub t_min: u32,
    pub t_max: u32,
    pub content_hash: String,
    pub shard: u32,
    pub forward_start: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ShardInfo {
    number: u32,
    file: String,
    postings: u64,
    hashes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Manifest {
    version: u32,
    generation: String,
    config_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    fingerprint_identity_profile: Option<String>,
    shard_count: u32,
    resources: Vec<ResourceInfo>,
    shards: Vec<ShardInfo>,
}

#[derive(Clone, Debug, Serialize)]
pub struct StoreStats {
    pub path: PathBuf,
    pub generation: String,
    pub config_id: String,
    pub fingerprint_identity_profile: Option<String>,
    pub resources: usize,
    pub postings: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct IdentityRehashStats {
    pub previous_generation: String,
    pub generation: String,
    pub resources_changed: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct IngestStats {
    pub generation: String,
    pub resources_added: usize,
    pub resources_replaced: usize,
    pub resources_unchanged: usize,
    pub postings_added: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct RetireStats {
    pub generation: String,
    pub postings_removed: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoredHit {
    pub resource_id: u32,
    pub t: u32,
    pub f: u16,
    pub matched_hash: u64,
}

struct Shard {
    map: MappedFile,
    forward_offset: u64,
    forward_count: u64,
    hash_index_offset: u64,
    hash_count: u64,
    hits_offset: u64,
    hit_count: u64,
}

pub struct Store {
    root: PathBuf,
    manifest: Manifest,
    resources_by_key: HashMap<String, usize>,
    resources_by_id: HashMap<u32, usize>,
    shards: Vec<Shard>,
}

#[derive(Clone, Copy)]
struct FullPosting {
    hash: u64,
    resource_id: u32,
    t: u32,
    f: u16,
}

impl Store {
    pub fn build(root: &Path, resources: Vec<DumpResource>) -> Result<Self> {
        if resources.is_empty() {
            return Err(Error::BadRequest("cannot build an empty store".into()));
        }
        let mut resources = resources;
        resources.sort_by(|a, b| a.meta.key.cmp(&b.meta.key));
        for pair in resources.windows(2) {
            if pair[0].meta.key == pair[1].meta.key {
                return Err(Error::BadRequest(format!(
                    "duplicate resource key {:?}",
                    pair[0].meta.key
                )));
            }
        }

        fs::create_dir_all(root.join("generations"))
            .map_err(|source| Error::io(root.join("generations"), source))?;
        fs::create_dir_all(root.join("shards"))
            .map_err(|source| Error::io(root.join("shards"), source))?;

        let mut grouped: BTreeMap<u32, Vec<(u32, DumpResource)>> = BTreeMap::new();
        for (index, resource) in resources.into_iter().enumerate() {
            let id = u32::try_from(index + 1)
                .map_err(|_| Error::BadRequest("too many resources".into()))?;
            grouped
                .entry(shard_for_key(&resource.meta.key))
                .or_default()
                .push((id, resource));
        }

        let mut resource_infos = Vec::new();
        let mut shard_infos = Vec::new();
        for number in 0..SHARD_COUNT {
            let group = grouped.remove(&number).unwrap_or_default();
            let (shard_info, mut infos) = write_shard(root, number, group)?;
            shard_infos.push(shard_info);
            resource_infos.append(&mut infos);
        }
        resource_infos.sort_by_key(|resource| resource.id);
        let identity_profile = Some(FINGERPRINT_IDENTITY_PROFILE.to_owned());
        let generation = generation_id_from_infos(&resource_infos, identity_profile.as_deref());
        let manifest = Manifest {
            version: STORE_VERSION,
            generation: generation.clone(),
            config_id: config_id(),
            fingerprint_identity_profile: identity_profile,
            shard_count: SHARD_COUNT,
            resources: resource_infos,
            shards: shard_infos,
        };
        publish_manifest(root, &manifest)?;
        Self::open(root)
    }

    pub fn ingest(
        root: &Path,
        resources: Vec<DumpResource>,
        replace: bool,
    ) -> Result<(Self, IngestStats)> {
        if resources.is_empty() {
            return Err(Error::BadRequest("ingest has no resources".into()));
        }
        let current = match Self::open(root) {
            Ok(store) => store,
            Err(Error::StoreMissing(_)) => {
                let resources_added = resources.len();
                let postings_added = resources
                    .iter()
                    .map(|resource| resource.prints.len() as u64)
                    .sum();
                let store = Self::build(root, resources)?;
                let stats = IngestStats {
                    generation: store.manifest.generation.clone(),
                    resources_added,
                    resources_replaced: 0,
                    resources_unchanged: 0,
                    postings_added,
                };
                return Ok((store, stats));
            }
            Err(error) => return Err(error),
        };

        let mut incoming = resources;
        incoming.sort_by(|a, b| a.meta.key.cmp(&b.meta.key));
        for pair in incoming.windows(2) {
            if pair[0].meta.key == pair[1].meta.key {
                return Err(Error::BadRequest(format!(
                    "duplicate resource key {:?}",
                    pair[0].meta.key
                )));
            }
        }
        let mut next_id = current
            .manifest
            .resources
            .iter()
            .map(|resource| resource.id)
            .max()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or_else(|| Error::BadRequest("resource id space exhausted".into()))?;
        let mut changes = HashMap::<String, (u32, DumpResource)>::new();
        let mut resources_added = 0;
        let mut resources_replaced = 0;
        let mut resources_unchanged = 0;
        let mut postings_added = 0;
        for mut resource in incoming {
            resource
                .prints
                .sort_by_key(|print| (print.t, print.hash, print.f));
            let content_hash = fingerprint_content_hash(&resource.prints);
            if let Some(existing) = current
                .resources_by_key
                .get(&resource.meta.key)
                .map(|index| &current.manifest.resources[*index])
            {
                if existing.content_hash == content_hash {
                    resources_unchanged += 1;
                    continue;
                }
                if !replace {
                    return Err(Error::BadRequest(format!(
                        "resource {:?} exists with different content; set replace=true",
                        resource.meta.key
                    )));
                }
                resources_replaced += 1;
                postings_added += resource.prints.len() as u64;
                changes.insert(resource.meta.key.clone(), (existing.id, resource));
            } else {
                let id = next_id;
                next_id = next_id
                    .checked_add(1)
                    .ok_or_else(|| Error::BadRequest("resource id space exhausted".into()))?;
                resources_added += 1;
                postings_added += resource.prints.len() as u64;
                changes.insert(resource.meta.key.clone(), (id, resource));
            }
        }
        if changes.is_empty() {
            let generation = current.manifest.generation.clone();
            return Ok((
                current,
                IngestStats {
                    generation,
                    resources_added: 0,
                    resources_replaced: 0,
                    resources_unchanged,
                    postings_added: 0,
                },
            ));
        }

        let mut affected = std::collections::BTreeSet::new();
        for (key, (_, resource)) in &changes {
            affected.insert(shard_for_key(&resource.meta.key));
            if let Some(existing) = current
                .resources_by_key
                .get(key)
                .map(|index| &current.manifest.resources[*index])
            {
                affected.insert(existing.shard);
            }
        }
        let manifest = current.rebuild_changed_shards(&affected, &changes, None)?;
        publish_manifest(root, &manifest)?;
        let store = Self::open(root)?;
        Ok((
            store,
            IngestStats {
                generation: manifest.generation,
                resources_added,
                resources_replaced,
                resources_unchanged,
                postings_added,
            },
        ))
    }

    pub fn retire(root: &Path, key: &str) -> Result<(Self, RetireStats)> {
        let current = Self::open(root)?;
        let retired = current.resource(key)?.clone();
        if current.manifest.resources.len() == 1 {
            return Err(Error::BadRequest(
                "retiring the final resource would create an empty store".into(),
            ));
        }
        let affected = std::collections::BTreeSet::from([retired.shard]);
        let manifest = current.rebuild_changed_shards(&affected, &HashMap::new(), Some(key))?;
        publish_manifest(root, &manifest)?;
        let store = Self::open(root)?;
        Ok((
            store,
            RetireStats {
                generation: manifest.generation,
                postings_removed: retired.postings,
            },
        ))
    }

    /// Publish a manifest whose endpoint identities cover only canonical `(hash,t,f)` postings.
    /// Shard bytes are reused; this operation neither extracts nor rewrites fingerprints.
    pub fn rehash_identities(root: &Path) -> Result<(Self, IdentityRehashStats)> {
        let current = Self::open(root)?;
        let previous_generation = current.manifest.generation.clone();
        let mut manifest = current.manifest.clone();
        let mut resources_changed = 0;
        for resource in &mut manifest.resources {
            let corrected = fingerprint_content_hash(&current.forward(&resource.key, None)?);
            if resource.content_hash != corrected {
                resource.content_hash = corrected;
                resources_changed += 1;
            }
        }
        let profile_changed =
            manifest.fingerprint_identity_profile.as_deref() != Some(FINGERPRINT_IDENTITY_PROFILE);
        manifest.fingerprint_identity_profile = Some(FINGERPRINT_IDENTITY_PROFILE.to_owned());
        manifest.generation = generation_id_from_infos(
            &manifest.resources,
            manifest.fingerprint_identity_profile.as_deref(),
        );
        let published = resources_changed != 0 || profile_changed;
        if published {
            publish_manifest(root, &manifest)?;
        }
        let store = if published {
            // Each generation maps all 64 shards; release the previous view before opening the
            // replacement so a low file-descriptor ceiling cannot make publication look failed.
            drop(current);
            Self::open(root)?
        } else {
            current
        };
        let generation = store.manifest.generation.clone();
        Ok((
            store,
            IdentityRehashStats {
                previous_generation,
                generation,
                resources_changed,
            },
        ))
    }

    pub fn open(root: &Path) -> Result<Self> {
        let current_path = root.join("CURRENT");
        let current = fs::read_to_string(&current_path).map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                Error::StoreMissing(root.to_path_buf())
            } else {
                Error::io(&current_path, source)
            }
        })?;
        let manifest_name = current.trim();
        if manifest_name.is_empty() || Path::new(manifest_name).file_name().is_none() {
            return Err(Error::InvalidStore("invalid CURRENT pointer".into()));
        }
        let manifest_path = root.join("generations").join(manifest_name);
        let bytes = fs::read(&manifest_path).map_err(|source| Error::io(&manifest_path, source))?;
        let manifest: Manifest = serde_json::from_slice(&bytes)
            .map_err(|error| Error::InvalidStore(format!("manifest JSON: {error}")))?;
        if manifest.version != STORE_VERSION {
            return Err(Error::StoreVersionMismatch {
                expected: STORE_VERSION,
                found: manifest.version,
            });
        }
        let expected_config = config_id();
        if manifest.config_id != expected_config {
            return Err(Error::ConfigMismatch {
                expected: expected_config,
                found: manifest.config_id,
            });
        }
        if let Some(profile) = &manifest.fingerprint_identity_profile
            && profile != FINGERPRINT_IDENTITY_PROFILE
        {
            return Err(Error::InvalidStore(format!(
                "unknown fingerprint identity profile {profile:?}"
            )));
        }
        if manifest.shard_count != SHARD_COUNT || manifest.shards.len() != SHARD_COUNT as usize {
            return Err(Error::InvalidStore(format!(
                "expected {SHARD_COUNT} shards, manifest has {}",
                manifest.shards.len()
            )));
        }

        let mut shards = Vec::with_capacity(SHARD_COUNT as usize);
        for (expected_number, info) in manifest.shards.iter().enumerate() {
            if info.number != expected_number as u32 {
                return Err(Error::InvalidStore(
                    "shards are not in numeric order".into(),
                ));
            }
            shards.push(Shard::open(&root.join("shards").join(&info.file), info)?);
        }
        let resources_by_key = manifest
            .resources
            .iter()
            .enumerate()
            .map(|(index, resource)| (resource.key.clone(), index))
            .collect();
        let resources_by_id = manifest
            .resources
            .iter()
            .enumerate()
            .map(|(index, resource)| (resource.id, index))
            .collect();
        Ok(Self {
            root: root.to_path_buf(),
            manifest,
            resources_by_key,
            resources_by_id,
            shards,
        })
    }

    pub fn stats(&self) -> StoreStats {
        StoreStats {
            path: self.root.clone(),
            generation: self.manifest.generation.clone(),
            config_id: self.manifest.config_id.clone(),
            fingerprint_identity_profile: self.manifest.fingerprint_identity_profile.clone(),
            resources: self.manifest.resources.len(),
            postings: self
                .manifest
                .resources
                .iter()
                .map(|resource| resource.postings)
                .sum(),
        }
    }

    pub fn resources(&self) -> &[ResourceInfo] {
        &self.manifest.resources
    }

    pub fn config_id(&self) -> &str {
        &self.manifest.config_id
    }

    pub fn resource(&self, key: &str) -> Result<&ResourceInfo> {
        self.resources_by_key
            .get(key)
            .map(|index| &self.manifest.resources[*index])
            .ok_or_else(|| Error::BadRequest(format!("unknown resource key {key:?}")))
    }

    pub fn resource_by_id(&self, id: u32) -> Result<&ResourceInfo> {
        self.resources_by_id
            .get(&id)
            .map(|index| &self.manifest.resources[*index])
            .ok_or_else(|| Error::InvalidStore(format!("posting references unknown resource {id}")))
    }

    /// Identity of the stored fingerprint vector only. Duration is mutable metadata and Panako's
    /// cached-store path is known to corrupt it; passage identity must survive a metadata repair.
    pub fn fingerprint_content_hash(&self, key: &str) -> Result<String> {
        if self.manifest.fingerprint_identity_profile.as_deref()
            == Some(FINGERPRINT_IDENTITY_PROFILE)
        {
            return Ok(self.resource(key)?.content_hash.clone());
        }
        Ok(fingerprint_content_hash(&self.forward(key, None)?))
    }

    pub fn forward(&self, key: &str, window: Option<(u32, u32)>) -> Result<Vec<Fingerprint>> {
        let resource = self.resource(key)?;
        let shard = &self.shards[resource.shard as usize];
        shard.forward(resource, window)
    }

    pub fn lookup(&self, hash: u64) -> Result<Vec<StoredHit>> {
        if hash > MAX_HASH {
            return Err(Error::BadRequest(format!("hash {hash} exceeds 34 bits")));
        }
        let start = hash.saturating_sub(crate::config::QUERY_RANGE);
        let stop = hash
            .saturating_add(crate::config::QUERY_RANGE)
            .min(MAX_HASH);
        let mut hits = Vec::new();
        for shard in &self.shards {
            shard.lookup_range(start, stop, &mut hits)?;
        }
        Ok(hits)
    }

    /// Look up one hash only in the shard that owns a known target resource. Every posting for a
    /// resource lives in its key-selected shard, so pair queries need not scan the other 63 shards.
    pub fn lookup_resource(&self, hash: u64, resource_id: u32) -> Result<Vec<StoredHit>> {
        if hash > MAX_HASH {
            return Err(Error::BadRequest(format!("hash {hash} exceeds 34 bits")));
        }
        let resource = self.resource_by_id(resource_id)?;
        let start = hash.saturating_sub(crate::config::QUERY_RANGE);
        let stop = hash
            .saturating_add(crate::config::QUERY_RANGE)
            .min(MAX_HASH);
        let mut hits = Vec::new();
        self.shards[resource.shard as usize].lookup_range(start, stop, &mut hits)?;
        hits.retain(|hit| hit.resource_id == resource_id);
        Ok(hits)
    }

    fn rebuild_changed_shards(
        &self,
        affected: &std::collections::BTreeSet<u32>,
        changes: &HashMap<String, (u32, DumpResource)>,
        retired_key: Option<&str>,
    ) -> Result<Manifest> {
        let mut manifest = self.manifest.clone();
        manifest.resources.retain(|resource| {
            Some(resource.key.as_str()) != retired_key && !changes.contains_key(&resource.key)
        });
        for &number in affected {
            let mut group = Vec::new();
            for resource in &self.manifest.resources {
                if resource.shard == number
                    && Some(resource.key.as_str()) != retired_key
                    && !changes.contains_key(&resource.key)
                {
                    group.push((resource.id, self.dump_resource(resource)?));
                }
            }
            for (id, resource) in changes.values() {
                if shard_for_key(&resource.meta.key) == number {
                    group.push((*id, resource.clone()));
                }
            }
            group.sort_by(|a, b| a.1.meta.key.cmp(&b.1.meta.key));
            let (shard, mut infos) = write_shard(&self.root, number, group)?;
            manifest.shards[number as usize] = shard;
            manifest.resources.append(&mut infos);
        }
        manifest.resources.sort_by_key(|resource| resource.id);
        manifest.generation = generation_id_from_infos(
            &manifest.resources,
            manifest.fingerprint_identity_profile.as_deref(),
        );
        Ok(manifest)
    }

    fn dump_resource(&self, resource: &ResourceInfo) -> Result<DumpResource> {
        let prints = self.forward(&resource.key, None)?;
        Ok(DumpResource {
            meta: crate::ResourceMeta {
                source_id: resource.id.to_string(),
                key: resource.key.clone(),
                duration: resource.duration,
                declared_prints: resource.postings,
            },
            prints,
            prints_path: PathBuf::new(),
        })
    }
}

impl Shard {
    fn open(path: &Path, info: &ShardInfo) -> Result<Self> {
        let map = MappedFile::open(path)?;
        if map.len() < HEADER_SIZE as usize || &map[0..8] != MAGIC {
            return Err(Error::InvalidStore(format!(
                "{} has an invalid shard header",
                path.display()
            )));
        }
        let version = read_u32(&map, 8)?;
        if version != STORE_VERSION {
            return Err(Error::StoreVersionMismatch {
                expected: STORE_VERSION,
                found: version,
            });
        }
        if read_u32(&map, 12)? != info.number {
            return Err(Error::InvalidStore(format!(
                "{} shard number disagrees with manifest",
                path.display()
            )));
        }
        let shard = Self {
            forward_offset: read_u64(&map, 16)?,
            forward_count: read_u64(&map, 24)?,
            hash_index_offset: read_u64(&map, 32)?,
            hash_count: read_u64(&map, 40)?,
            hits_offset: read_u64(&map, 48)?,
            hit_count: read_u64(&map, 56)?,
            map,
        };
        shard.validate(path, info)?;
        Ok(shard)
    }

    fn validate(&self, path: &Path, info: &ShardInfo) -> Result<()> {
        let forward_end = checked_region(self.forward_offset, self.forward_count, FORWARD_SIZE)?;
        let index_end = checked_region(self.hash_index_offset, self.hash_count, HASH_INDEX_SIZE)?;
        let hits_end = checked_region(self.hits_offset, self.hit_count, HIT_SIZE)?;
        if self.forward_offset != HEADER_SIZE
            || self.hash_index_offset != forward_end
            || self.hits_offset != index_end
            || hits_end != self.map.len() as u64
            || self.forward_count != info.postings
            || self.hit_count != info.postings
            || self.hash_count != info.hashes
        {
            return Err(Error::InvalidStore(format!(
                "{} has inconsistent shard regions",
                path.display()
            )));
        }
        Ok(())
    }

    fn forward(
        &self,
        resource: &ResourceInfo,
        window: Option<(u32, u32)>,
    ) -> Result<Vec<Fingerprint>> {
        let start = resource.forward_start;
        let end = start
            .checked_add(resource.postings)
            .ok_or_else(|| Error::InvalidStore("forward range overflow".into()))?;
        if end > self.forward_count {
            return Err(Error::InvalidStore(format!(
                "resource {:?} forward range is outside shard",
                resource.key
            )));
        }
        let (t0, t1) = window.unwrap_or((0, u32::MAX));
        if t0 >= t1 {
            return Err(Error::BadRequest("window start must be before stop".into()));
        }
        let mut prints = Vec::new();
        for index in start..end {
            let offset = self.forward_offset + index * FORWARD_SIZE;
            let id = read_u32(&self.map, offset as usize)?;
            if id != resource.id {
                return Err(Error::InvalidStore(format!(
                    "resource {:?} forward range contains id {id}",
                    resource.key
                )));
            }
            let t = read_u32(&self.map, offset as usize + 4)?;
            if t >= t0 && t < t1 {
                prints.push(Fingerprint {
                    f: read_u16(&self.map, offset as usize + 8)?,
                    hash: read_u64(&self.map, offset as usize + 12)?,
                    t,
                });
            }
        }
        Ok(prints)
    }

    fn lookup_range(&self, start: u64, stop: u64, out: &mut Vec<StoredHit>) -> Result<()> {
        let mut low = 0;
        let mut high = self.hash_count;
        while low < high {
            let middle = low + (high - low) / 2;
            if self.index_hash(middle)? < start {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let mut index = low;
        while index < self.hash_count {
            let base = self.hash_index_offset + index * HASH_INDEX_SIZE;
            let hash = read_u64(&self.map, base as usize)?;
            if hash > stop {
                break;
            }
            let hit_start = read_u64(&self.map, base as usize + 8)?;
            let hit_len = read_u64(&self.map, base as usize + 16)?;
            let hit_end = hit_start
                .checked_add(hit_len)
                .ok_or_else(|| Error::InvalidStore("hit range overflow".into()))?;
            if hit_end > self.hit_count {
                return Err(Error::InvalidStore("hash index points outside hits".into()));
            }
            for hit_index in hit_start..hit_end {
                let offset = self.hits_offset + hit_index * HIT_SIZE;
                out.push(StoredHit {
                    resource_id: read_u32(&self.map, offset as usize)?,
                    t: read_u32(&self.map, offset as usize + 4)?,
                    f: read_u16(&self.map, offset as usize + 8)?,
                    matched_hash: hash,
                });
            }
            index += 1;
        }
        Ok(())
    }

    fn index_hash(&self, index: u64) -> Result<u64> {
        read_u64(
            &self.map,
            (self.hash_index_offset + index * HASH_INDEX_SIZE) as usize,
        )
    }
}

fn write_shard(
    root: &Path,
    number: u32,
    resources: Vec<(u32, DumpResource)>,
) -> Result<(ShardInfo, Vec<ResourceInfo>)> {
    let mut postings = Vec::new();
    let mut infos = Vec::new();
    for (id, resource) in resources {
        let mut canonical = resource.prints;
        canonical.sort_by_key(|print| (print.t, print.hash, print.f));
        let content_hash = fingerprint_content_hash(&canonical);
        let t_min = canonical.first().map_or(0, |print| print.t);
        let t_max = canonical.last().map_or(0, |print| print.t);
        let forward_start = postings.len() as u64;
        postings.extend(canonical.iter().map(|print| FullPosting {
            hash: print.hash,
            resource_id: id,
            t: print.t,
            f: print.f,
        }));
        infos.push(ResourceInfo {
            id,
            key: resource.meta.key,
            duration: resource.meta.duration,
            postings: canonical.len() as u64,
            t_min,
            t_max,
            content_hash,
            shard: number,
            forward_start,
        });
    }
    postings.sort_by_key(|posting| (posting.resource_id, posting.t, posting.hash, posting.f));
    let mut inverted = postings.clone();
    inverted.sort_by_key(|posting| (posting.hash, posting.resource_id, posting.t, posting.f));
    let hash_count = inverted
        .iter()
        .enumerate()
        .filter(|(index, posting)| *index == 0 || inverted[index - 1].hash != posting.hash)
        .count() as u64;
    let shard_digest = shard_content_hash(&infos);
    let file_name = format!("shard-{number:02}-{}.bin", &shard_digest[..24]);
    let path = root.join("shards").join(&file_name);
    if !path.exists() {
        write_shard_file(&path, number, &postings, &inverted, hash_count)?;
    }
    Ok((
        ShardInfo {
            number,
            file: file_name,
            postings: postings.len() as u64,
            hashes: hash_count,
        },
        infos,
    ))
}

fn write_shard_file(
    path: &Path,
    number: u32,
    forward: &[FullPosting],
    inverted: &[FullPosting],
    hash_count: u64,
) -> Result<()> {
    let temp = path.with_extension("bin.tmp");
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)
        .map_err(|source| Error::io(&temp, source))?;
    let mut writer = BufWriter::new(file);
    writer
        .write_all(&vec![0_u8; HEADER_SIZE as usize])
        .map_err(|source| Error::io(&temp, source))?;
    for posting in forward {
        write_forward(&mut writer, *posting).map_err(|source| Error::io(&temp, source))?;
    }
    let hash_index_offset = HEADER_SIZE + forward.len() as u64 * FORWARD_SIZE;
    let hits_offset = hash_index_offset + hash_count * HASH_INDEX_SIZE;
    let mut hit_start = 0_u64;
    let mut cursor = 0;
    while cursor < inverted.len() {
        let hash = inverted[cursor].hash;
        let start = cursor;
        while cursor < inverted.len() && inverted[cursor].hash == hash {
            cursor += 1;
        }
        write_u64_to(&mut writer, hash).map_err(|source| Error::io(&temp, source))?;
        write_u64_to(&mut writer, hit_start).map_err(|source| Error::io(&temp, source))?;
        write_u64_to(&mut writer, (cursor - start) as u64)
            .map_err(|source| Error::io(&temp, source))?;
        hit_start += (cursor - start) as u64;
    }
    for posting in inverted {
        writer
            .write_all(&posting.resource_id.to_le_bytes())
            .and_then(|()| writer.write_all(&posting.t.to_le_bytes()))
            .and_then(|()| writer.write_all(&posting.f.to_le_bytes()))
            .and_then(|()| writer.write_all(&[0, 0]))
            .map_err(|source| Error::io(&temp, source))?;
    }
    writer
        .seek(SeekFrom::Start(0))
        .and_then(|_| writer.write_all(MAGIC))
        .and_then(|()| writer.write_all(&STORE_VERSION.to_le_bytes()))
        .and_then(|()| writer.write_all(&number.to_le_bytes()))
        .and_then(|()| write_u64_to(&mut writer, HEADER_SIZE))
        .and_then(|()| write_u64_to(&mut writer, forward.len() as u64))
        .and_then(|()| write_u64_to(&mut writer, hash_index_offset))
        .and_then(|()| write_u64_to(&mut writer, hash_count))
        .and_then(|()| write_u64_to(&mut writer, hits_offset))
        .and_then(|()| write_u64_to(&mut writer, inverted.len() as u64))
        .map_err(|source| Error::io(&temp, source))?;
    let file = writer
        .into_inner()
        .map_err(|error| Error::io(&temp, error.into_error()))?;
    file.sync_all().map_err(|source| Error::io(&temp, source))?;
    fs::rename(&temp, path).map_err(|source| Error::io(path, source))?;
    Ok(())
}

fn publish_manifest(root: &Path, manifest: &Manifest) -> Result<()> {
    let previous = fs::read_to_string(root.join("CURRENT"))
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    let name = format!("{}.json", manifest.generation);
    let path = root.join("generations").join(&name);
    let temp_manifest = root
        .join("generations")
        .join(format!("{}.tmp", manifest.generation));
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|error| Error::Internal(format!("serialize manifest: {error}")))?;
    write_synced(&temp_manifest, &bytes)?;
    fs::rename(&temp_manifest, &path).map_err(|source| Error::io(&path, source))?;

    let current_temp = root.join(format!("CURRENT.{}.tmp", manifest.generation));
    write_synced(&current_temp, format!("{name}\n").as_bytes())?;
    fs::rename(&current_temp, root.join("CURRENT"))
        .map_err(|source| Error::io(root.join("CURRENT"), source))?;
    sync_directory(root)?;
    garbage_collect(root, &name, previous.as_deref());
    Ok(())
}

fn garbage_collect(root: &Path, current: &str, previous: Option<&str>) {
    let retained: std::collections::HashSet<_> =
        [Some(current), previous].into_iter().flatten().collect();
    let mut referenced_shards = std::collections::HashSet::new();
    for manifest_name in &retained {
        let path = root.join("generations").join(manifest_name);
        if let Ok(bytes) = fs::read(&path)
            && let Ok(manifest) = serde_json::from_slice::<Manifest>(&bytes)
        {
            referenced_shards.extend(manifest.shards.into_iter().map(|shard| shard.file));
        }
    }
    if let Ok(entries) = fs::read_dir(root.join("generations")) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            if !retained.contains(name.to_string_lossy().as_ref()) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    if let Ok(entries) = fs::read_dir(root.join("shards")) {
        for entry in entries.flatten() {
            if !referenced_shards.contains(entry.file_name().to_string_lossy().as_ref()) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
}

fn write_synced(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = File::create(path).map_err(|source| Error::io(path, source))?;
    file.write_all(bytes)
        .map_err(|source| Error::io(path, source))?;
    file.sync_all().map_err(|source| Error::io(path, source))
}

fn sync_directory(path: &Path) -> Result<()> {
    let directory = File::open(path).map_err(|source| Error::io(path, source))?;
    directory
        .sync_all()
        .map_err(|source| Error::io(path, source))
}

fn generation_id_from_infos(resources: &[ResourceInfo], identity_profile: Option<&str>) -> String {
    let mut digest = Sha256::new();
    digest.update(config_id());
    digest.update(identity_profile.unwrap_or("legacy-duration-v0"));
    for resource in resources {
        digest.update(resource.id.to_le_bytes());
        digest.update(resource.key.as_bytes());
        digest.update(resource.duration.to_le_bytes());
        digest.update(resource.content_hash.as_bytes());
    }
    hex::encode(digest.finalize())[..24].to_owned()
}

fn fingerprint_content_hash(prints: &[Fingerprint]) -> String {
    let mut digest = Sha256::new();
    for print in prints {
        digest.update(print.hash.to_le_bytes());
        digest.update(print.t.to_le_bytes());
        digest.update(print.f.to_le_bytes());
    }
    hex::encode(digest.finalize())
}

fn shard_content_hash(resources: &[ResourceInfo]) -> String {
    let mut digest = Sha256::new();
    digest.update(config_id());
    for resource in resources {
        digest.update(resource.id.to_le_bytes());
        digest.update(resource.key.as_bytes());
        digest.update(resource.content_hash.as_bytes());
    }
    hex::encode(digest.finalize())
}

fn shard_for_key(key: &str) -> u32 {
    let hash = Sha256::digest(key.as_bytes());
    u32::from_le_bytes(hash[..4].try_into().expect("digest prefix")) % SHARD_COUNT
}

fn write_forward(writer: &mut impl Write, posting: FullPosting) -> std::io::Result<()> {
    writer.write_all(&posting.resource_id.to_le_bytes())?;
    writer.write_all(&posting.t.to_le_bytes())?;
    writer.write_all(&posting.f.to_le_bytes())?;
    writer.write_all(&[0, 0])?;
    writer.write_all(&posting.hash.to_le_bytes())
}

fn write_u64_to(writer: &mut impl Write, value: u64) -> std::io::Result<()> {
    writer.write_all(&value.to_le_bytes())
}

fn checked_region(offset: u64, count: u64, size: u64) -> Result<u64> {
    count
        .checked_mul(size)
        .and_then(|length| offset.checked_add(length))
        .ok_or_else(|| Error::InvalidStore("store region overflow".into()))
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16> {
    let array = bytes
        .get(offset..offset + 2)
        .ok_or_else(|| Error::InvalidStore("unexpected end of shard".into()))?
        .try_into()
        .expect("two-byte slice");
    Ok(u16::from_le_bytes(array))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32> {
    let array = bytes
        .get(offset..offset + 4)
        .ok_or_else(|| Error::InvalidStore("unexpected end of shard".into()))?
        .try_into()
        .expect("four-byte slice");
    Ok(u32::from_le_bytes(array))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64> {
    let array = bytes
        .get(offset..offset + 8)
        .ok_or_else(|| Error::InvalidStore("unexpected end of shard".into()))?
        .try_into()
        .expect("eight-byte slice");
    Ok(u64::from_le_bytes(array))
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::ResourceMeta;

    static STORE_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn resource(key: &str, prints: &[(u64, u32, u16)]) -> DumpResource {
        DumpResource {
            meta: ResourceMeta {
                source_id: "1".into(),
                key: key.into(),
                duration: 10.0,
                declared_prints: prints.len() as u64,
            },
            prints: prints
                .iter()
                .map(|&(hash, t, f)| Fingerprint { hash, t, f })
                .collect(),
            prints_path: PathBuf::new(),
        }
    }

    #[test]
    fn round_trips_both_access_orders() {
        let _guard = STORE_TEST_LOCK.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let store = Store::build(
            root.path(),
            vec![
                resource("b", &[(99, 7, 8), (101, 9, 10)]),
                resource("a", &[(100, 2, 3), (99, 1, 4)]),
            ],
        )
        .unwrap();
        assert_eq!(
            store.forward("a", Some((2, 3))).unwrap(),
            vec![Fingerprint::new(100, 2, 3)]
        );
        let hits = store.lookup(100).unwrap();
        assert_eq!(hits.len(), 4);
        let a_id = store.resource("a").unwrap().id;
        assert_eq!(
            store.lookup_resource(100, a_id).unwrap(),
            hits.into_iter()
                .filter(|hit| hit.resource_id == a_id)
                .collect::<Vec<_>>()
        );
        assert_eq!(store.stats().postings, 4);
    }

    #[test]
    fn absent_store_is_not_an_empty_store() {
        let root = tempfile::tempdir().unwrap();
        assert!(matches!(
            Store::open(&root.path().join("absent")),
            Err(Error::StoreMissing(_))
        ));
    }

    #[test]
    fn ingest_is_idempotent_and_retire_removes_real_postings() {
        let _guard = STORE_TEST_LOCK.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        Store::build(
            root.path(),
            vec![resource("a", &[(10, 1, 2)]), resource("b", &[(20, 3, 4)])],
        )
        .unwrap();
        let (store, unchanged) =
            Store::ingest(root.path(), vec![resource("a", &[(10, 1, 2)])], false).unwrap();
        assert_eq!(unchanged.resources_unchanged, 1);
        assert_eq!(unchanged.postings_added, 0);
        assert_eq!(store.stats().postings, 2);

        let (store, retired) = Store::retire(root.path(), "a").unwrap();
        assert_eq!(retired.postings_removed, 1);
        assert_eq!(store.stats().postings, 1);
        assert!(matches!(store.resource("a"), Err(Error::BadRequest(_))));
        assert!(store.lookup(10).unwrap().is_empty());
    }

    #[test]
    fn identity_rehash_reuses_shards_and_is_idempotent() {
        let _guard = STORE_TEST_LOCK.lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        Store::build(
            root.path(),
            vec![resource("a", &[(10, 1, 2)]), resource("b", &[(20, 3, 4)])],
        )
        .unwrap();
        let current = fs::read_to_string(root.path().join("CURRENT")).unwrap();
        let current_path = root.path().join("generations").join(current.trim());
        let mut legacy: Manifest =
            serde_json::from_slice(&fs::read(&current_path).unwrap()).unwrap();
        let shard_files: Vec<_> = legacy
            .shards
            .iter()
            .map(|shard| shard.file.clone())
            .collect();
        legacy.generation = "legacy-duration-hash".into();
        legacy.fingerprint_identity_profile = None;
        for resource in &mut legacy.resources {
            resource.content_hash = format!("duration-coupled-{}", resource.key);
        }
        fs::write(
            root.path()
                .join("generations")
                .join("legacy-duration-hash.json"),
            serde_json::to_vec_pretty(&legacy).unwrap(),
        )
        .unwrap();
        fs::write(root.path().join("CURRENT"), "legacy-duration-hash.json\n").unwrap();

        let (store, changed) = Store::rehash_identities(root.path()).unwrap();
        assert_eq!(changed.previous_generation, "legacy-duration-hash");
        assert_eq!(changed.resources_changed, 2);
        assert_ne!(changed.generation, changed.previous_generation);
        assert_eq!(store.resource("a").unwrap().duration, 10.0);
        assert_eq!(
            store.stats().fingerprint_identity_profile.as_deref(),
            Some(FINGERPRINT_IDENTITY_PROFILE)
        );
        assert_eq!(
            store
                .manifest
                .shards
                .iter()
                .map(|shard| shard.file.clone())
                .collect::<Vec<_>>(),
            shard_files
        );

        let (_, unchanged) = Store::rehash_identities(root.path()).unwrap();
        assert_eq!(unchanged.resources_changed, 0);
        assert_eq!(unchanged.previous_generation, unchanged.generation);
    }
}
