use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{
    Error, EvidenceHit, Result, Segment, Store,
    span::{
        RegionGeometry, crosscheck_between_multiline_geometry, span_between_multiline_geometry,
    },
};

pub const PASSAGE_PROFILE: &str = "passage-v2";

// Production identify artifacts used 12-second windows every 8 seconds. Passage mode replays that
// geometry over stored prints; compatibility span/crosscheck keep their original 30-second regions.
const PASSAGE_GEOMETRY: RegionGeometry = RegionGeometry {
    window_seconds: 12.0,
    hop_seconds: 8.0,
    anchor_at_zero: true,
};

// A passage may retain a short unsupported hole (for example, a station drop over a corpus song)
// only as an explicit gap between support spans. This is the maximum separation at which two
// region-local lines may still be members of one geometric alignment family; it is never reported
// as matched audio.
const MAX_ALIGNMENT_GAP_SECONDS: f64 = 20.0;
const MAX_OFFSET_JUMP_SECONDS: f64 = 2.0;
const MAX_TIME_FACTOR_JUMP: f64 = 0.03;
const MAX_PITCH_FACTOR_JUMP: f64 = 0.03;

// A support run is evidence-dense only while both clocks continue to receive filtered hits. Longer
// holes are surfaced rather than filled by the first/last hit envelope.
const MAX_SUPPORT_HIT_GAP_SECONDS: f64 = 1.5;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PassageSnapshot {
    pub profile: String,
    /// Passage observations are directional facts: A was probed against B. A missing reverse
    /// observation is not a negative match and must not erase this result during graph fusion.
    pub direction: String,
    pub config_id: String,
    pub a_generation: String,
    pub b_generation: String,
    pub a_key: String,
    pub a_content_hash: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SupportSpan {
    pub a_start: f64,
    pub a_stop: f64,
    pub b_start: f64,
    pub b_stop: f64,
    pub hits: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PassageQuality {
    pub score_peak: usize,
    pub matched_hits: usize,
    pub supported_seconds: f64,
    pub query_coverage: f64,
    pub largest_gap: f64,
    pub segment_count: usize,
    pub support_count: usize,
    pub time_factor_min: f64,
    pub time_factor_max: f64,
    pub pitch_factor_min: f64,
    pub pitch_factor_max: f64,
    pub sec_with_match_min: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Passage {
    pub passage_id: String,
    /// The first-to-last support envelope. It is not a claim that holes inside it matched.
    pub a_envelope: [f64; 2],
    /// The corresponding first-to-last envelope on the reference clock.
    pub b_envelope: [f64; 2],
    /// Dense match support. Gaps between entries are explicitly unsupported by this alignment.
    pub support: Vec<SupportSpan>,
    pub quality: PassageQuality,
    /// Raw accepted region lines, only when requested. Passage identity never depends on whether
    /// this diagnostic payload was requested.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub segments: Option<Vec<Segment>>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PairPassages {
    pub snapshot: PassageSnapshot,
    pub b_key: String,
    pub b_content_hash: String,
    pub passages: Vec<Passage>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PassageMatch {
    pub ref_key: String,
    pub ref_content_hash: String,
    pub passages: Vec<Passage>,
    pub matched_hits: usize,
    pub supported_seconds: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PassageDiscovery {
    pub snapshot: PassageSnapshot,
    pub matches: Vec<PassageMatch>,
}

pub fn passages_between(
    a_store: &Store,
    b_store: &Store,
    a_key: &str,
    a_window: Option<(f64, f64)>,
    b_key: &str,
    include_segments: bool,
) -> Result<PairPassages> {
    // Evidence is an internal requirement even when the caller does not request the diagnostic
    // lines: honest support comes from filtered hits, not a segment's first/last-hit envelope.
    let segments = span_between_multiline_geometry(
        a_store,
        b_store,
        a_key,
        a_window,
        b_key,
        true,
        PASSAGE_GEOMETRY,
    )?;
    let a_content_hash = a_store.fingerprint_content_hash(a_key)?;
    let b_content_hash = b_store.fingerprint_content_hash(b_key)?;
    let snapshot = snapshot(a_store, b_store, a_key)?;
    let passages = passages_from_segments(
        a_key,
        &a_content_hash,
        b_key,
        &b_content_hash,
        a_store.config_id(),
        segments,
        include_segments,
    )?;
    Ok(PairPassages {
        snapshot,
        b_key: b_key.to_owned(),
        b_content_hash,
        passages,
    })
}

pub fn discover_passages_between(
    a_store: &Store,
    b_store: &Store,
    a_key: &str,
    a_window: Option<(f64, f64)>,
    targets: Option<&[String]>,
    exclude_keys: Option<&[String]>,
    include_segments: bool,
) -> Result<PassageDiscovery> {
    // Discovery is exhaustive. A ranking limit cannot define absence in a persistent evidence
    // graph, so the existing crosscheck limit is set to the complete target population here.
    let raw = crosscheck_between_multiline_geometry(
        a_store,
        b_store,
        a_key,
        a_window,
        targets,
        true,
        PASSAGE_GEOMETRY,
    )?;
    let a_content_hash = a_store.fingerprint_content_hash(a_key)?;
    let snapshot = snapshot(a_store, b_store, a_key)?;
    let excluded: HashSet<&str> = exclude_keys
        .unwrap_or_default()
        .iter()
        .map(String::as_str)
        .collect();
    let mut matches = Vec::new();
    for found in raw {
        // A resource is not evidence of its own recurrence. Exact-key exclusion is owned here so
        // every consumer gets the same discovery semantics.
        if found.ref_key == a_key || excluded.contains(found.ref_key.as_str()) {
            continue;
        }
        let b_content_hash = b_store.fingerprint_content_hash(&found.ref_key)?;
        let passages = passages_from_segments(
            a_key,
            &a_content_hash,
            &found.ref_key,
            &b_content_hash,
            a_store.config_id(),
            found.segments,
            include_segments,
        )?;
        if passages.is_empty() {
            continue;
        }
        matches.push(PassageMatch {
            ref_key: found.ref_key,
            ref_content_hash: b_content_hash,
            matched_hits: passages
                .iter()
                .map(|passage| passage.quality.matched_hits)
                .sum(),
            supported_seconds: passages
                .iter()
                .map(|passage| passage.quality.supported_seconds)
                .sum(),
            passages,
        });
    }
    matches.sort_by(|a, b| {
        b.supported_seconds
            .total_cmp(&a.supported_seconds)
            .then_with(|| b.matched_hits.cmp(&a.matched_hits))
            .then_with(|| a.ref_key.cmp(&b.ref_key))
    });
    Ok(PassageDiscovery { snapshot, matches })
}

fn snapshot(a_store: &Store, b_store: &Store, a_key: &str) -> Result<PassageSnapshot> {
    let a_stats = a_store.stats();
    let b_stats = b_store.stats();
    Ok(PassageSnapshot {
        profile: PASSAGE_PROFILE.to_owned(),
        direction: "a_to_b".to_owned(),
        config_id: a_store.config_id().to_owned(),
        a_generation: a_stats.generation,
        b_generation: b_stats.generation,
        a_key: a_key.to_owned(),
        a_content_hash: a_store.fingerprint_content_hash(a_key)?,
    })
}

fn passages_from_segments(
    a_key: &str,
    a_content_hash: &str,
    b_key: &str,
    b_content_hash: &str,
    config_id: &str,
    mut segments: Vec<Segment>,
    include_segments: bool,
) -> Result<Vec<Passage>> {
    segments.sort_by(|a, b| {
        a.a_start
            .total_cmp(&b.a_start)
            .then_with(|| a.b_start.total_cmp(&b.b_start))
            .then_with(|| b.score.cmp(&a.score))
    });

    let mut tracks = Vec::<Vec<Segment>>::new();
    for segment in segments {
        if segment.evidence.is_none() {
            return Err(Error::Internal(
                "passage construction requires internal match evidence".into(),
            ));
        }
        let best = tracks
            .iter()
            .enumerate()
            .filter_map(|(index, track)| {
                let last = track.last()?;
                compatible(last, &segment).then_some((index, offset_jump(last, &segment)))
            })
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(index, _)| index);
        if let Some(index) = best {
            tracks[index].push(segment);
        } else {
            tracks.push(vec![segment]);
        }
    }

    let mut passages = tracks
        .into_iter()
        .map(|track| {
            passage_from_track(
                a_key,
                a_content_hash,
                b_key,
                b_content_hash,
                config_id,
                track,
                include_segments,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    passages.sort_by(|a, b| {
        a.a_envelope[0]
            .total_cmp(&b.a_envelope[0])
            .then_with(|| a.b_envelope[0].total_cmp(&b.b_envelope[0]))
            .then_with(|| a.passage_id.cmp(&b.passage_id))
    });
    Ok(passages)
}

fn compatible(previous: &Segment, next: &Segment) -> bool {
    if next.a_start + MAX_OFFSET_JUMP_SECONDS < previous.a_start
        || next.b_start + MAX_OFFSET_JUMP_SECONDS < previous.b_start
    {
        return false;
    }
    let a_gap = (next.a_start - previous.a_stop).max(0.0);
    let b_gap = (next.b_start - previous.b_stop).max(0.0);
    a_gap <= MAX_ALIGNMENT_GAP_SECONDS
        && b_gap <= MAX_ALIGNMENT_GAP_SECONDS
        && offset_jump(previous, next) <= MAX_OFFSET_JUMP_SECONDS
        && (next.time_factor - previous.time_factor).abs() <= MAX_TIME_FACTOR_JUMP
        && (next.pitch_factor - previous.pitch_factor).abs() <= MAX_PITCH_FACTOR_JUMP
}

fn offset_jump(previous: &Segment, next: &Segment) -> f64 {
    let previous_end_offset = previous.b_stop - previous.a_stop;
    let next_start_offset = next.b_start - next.a_start;
    (next_start_offset - previous_end_offset).abs()
}

fn passage_from_track(
    a_key: &str,
    a_content_hash: &str,
    b_key: &str,
    b_content_hash: &str,
    config_id: &str,
    segments: Vec<Segment>,
    include_segments: bool,
) -> Result<Passage> {
    let mut hits = Vec::new();
    for segment in &segments {
        let evidence = segment.evidence.as_ref().ok_or_else(|| {
            Error::Internal("passage construction requires internal match evidence".into())
        })?;
        hits.extend(evidence.hits.iter().cloned());
    }
    hits.sort_by(|a, b| {
        (a.q_t, a.ref_t, a.original_hash, a.matched_hash).cmp(&(
            b.q_t,
            b.ref_t,
            b.original_hash,
            b.matched_hash,
        ))
    });
    hits.dedup_by(|a, b| {
        (a.q_t, a.ref_t, a.original_hash, a.matched_hash)
            == (b.q_t, b.ref_t, b.original_hash, b.matched_hash)
    });
    let support = support_from_hits(&hits);
    let first = support.first().ok_or_else(|| {
        Error::Internal("accepted match line carried no filtered-hit evidence".into())
    })?;
    let last = support.last().expect("support was checked nonempty");
    let a_envelope = [first.a_start, last.a_stop];
    let b_envelope = [
        support
            .iter()
            .map(|span| span.b_start)
            .min_by(f64::total_cmp)
            .expect("support is nonempty"),
        support
            .iter()
            .map(|span| span.b_stop)
            .max_by(f64::total_cmp)
            .expect("support is nonempty"),
    ];
    let supported_seconds: f64 = support
        .iter()
        .map(|span| (span.a_stop - span.a_start).max(0.0))
        .sum();
    let envelope_seconds = (a_envelope[1] - a_envelope[0]).max(0.0);
    let largest_gap = support
        .windows(2)
        .map(|pair| (pair[1].a_start - pair[0].a_stop).max(0.0))
        .max_by(f64::total_cmp)
        .unwrap_or(0.0);
    let quality = PassageQuality {
        score_peak: segments
            .iter()
            .map(|segment| segment.score)
            .max()
            .unwrap_or(0),
        matched_hits: hits.len(),
        supported_seconds,
        query_coverage: if envelope_seconds > 0.0 {
            (supported_seconds / envelope_seconds).clamp(0.0, 1.0)
        } else {
            0.0
        },
        largest_gap,
        segment_count: segments.len(),
        support_count: support.len(),
        time_factor_min: segments
            .iter()
            .map(|segment| segment.time_factor)
            .min_by(f64::total_cmp)
            .expect("track is nonempty"),
        time_factor_max: segments
            .iter()
            .map(|segment| segment.time_factor)
            .max_by(f64::total_cmp)
            .expect("track is nonempty"),
        pitch_factor_min: segments
            .iter()
            .map(|segment| segment.pitch_factor)
            .min_by(f64::total_cmp)
            .expect("track is nonempty"),
        pitch_factor_max: segments
            .iter()
            .map(|segment| segment.pitch_factor)
            .max_by(f64::total_cmp)
            .expect("track is nonempty"),
        sec_with_match_min: segments
            .iter()
            .map(|segment| segment.sec_with_match)
            .min_by(f64::total_cmp)
            .expect("track is nonempty"),
    };
    let passage_id = passage_id(
        a_key,
        a_content_hash,
        b_key,
        b_content_hash,
        config_id,
        &support,
    );
    Ok(Passage {
        passage_id,
        a_envelope,
        b_envelope,
        support,
        quality,
        segments: include_segments.then_some(segments),
    })
}

fn support_from_hits(hits: &[EvidenceHit]) -> Vec<SupportSpan> {
    let mut hits = hits.to_vec();
    hits.sort_by(|a, b| {
        a.q_seconds
            .total_cmp(&b.q_seconds)
            .then_with(|| a.ref_seconds.total_cmp(&b.ref_seconds))
    });
    let mut out = Vec::new();
    let mut open: Option<SupportSpan> = None;
    for hit in hits {
        if let Some(span) = open.as_mut()
            && hit.q_seconds - span.a_stop <= MAX_SUPPORT_HIT_GAP_SECONDS
            && hit.ref_seconds - span.b_stop <= MAX_SUPPORT_HIT_GAP_SECONDS
            && hit.ref_seconds + MAX_OFFSET_JUMP_SECONDS >= span.b_stop
        {
            span.a_stop = hit.q_seconds;
            span.b_start = span.b_start.min(hit.ref_seconds);
            span.b_stop = span.b_stop.max(hit.ref_seconds);
            span.hits += 1;
            continue;
        }
        if let Some(span) = open.take() {
            out.push(span);
        }
        open = Some(SupportSpan {
            a_start: hit.q_seconds,
            a_stop: hit.q_seconds,
            b_start: hit.ref_seconds,
            b_stop: hit.ref_seconds,
            hits: 1,
        });
    }
    if let Some(span) = open {
        out.push(span);
    }
    out
}

fn passage_id(
    a_key: &str,
    a_content_hash: &str,
    b_key: &str,
    b_content_hash: &str,
    config_id: &str,
    support: &[SupportSpan],
) -> String {
    let mut digest = Sha256::new();
    for value in [
        PASSAGE_PROFILE,
        config_id,
        a_key,
        a_content_hash,
        b_key,
        b_content_hash,
    ] {
        digest.update(value.as_bytes());
        digest.update([0]);
    }
    for span in support {
        for value in [span.a_start, span.a_stop, span.b_start, span.b_stop] {
            digest.update(value.to_bits().to_le_bytes());
        }
        digest.update((span.hits as u64).to_le_bytes());
    }
    format!("psg_{}", hex::encode(digest.finalize()))
}

#[cfg(test)]
mod tests {
    use crate::{DensityBin, Evidence, EvidenceHit, HistogramBin};

    use super::*;

    fn segment(a_start: f64, b_start: f64, hit_offsets: &[f64]) -> Segment {
        let hits: Vec<_> = hit_offsets
            .iter()
            .enumerate()
            .map(|(index, offset)| EvidenceHit {
                q_t: ((a_start + offset) / crate::config::TIME_BIN_SECONDS) as u32,
                ref_t: ((b_start + offset) / crate::config::TIME_BIN_SECONDS) as u32,
                q_seconds: a_start + offset,
                ref_seconds: b_start + offset,
                original_hash: index as u64,
                matched_hash: index as u64,
            })
            .collect();
        Segment {
            a_start: hits.first().expect("test hits").q_seconds,
            a_stop: hits.last().expect("test hits").q_seconds,
            b_start: hits.first().expect("test hits").ref_seconds,
            b_stop: hits.last().expect("test hits").ref_seconds,
            score: hits.len(),
            time_factor: 1.0,
            pitch_factor: 1.0,
            sec_with_match: 1.0,
            evidence: Some(Evidence {
                hits,
                offset_top: vec![HistogramBin { bin: 0, count: 1 }],
                per_second: vec![DensityBin {
                    second: 0,
                    count: 1,
                }],
            }),
        }
    }

    fn passages(segments: Vec<Segment>) -> Vec<Passage> {
        passages_from_segments("a", "ah", "b", "bh", "config", segments, false).expect("passages")
    }

    #[test]
    fn stitches_compatible_regions_but_keeps_the_unsupported_hole() {
        let found = passages(vec![
            segment(0.0, 100.0, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]),
            segment(10.0, 110.0, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]),
        ]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].support.len(), 2);
        assert_eq!(found[0].a_envelope, [0.0, 15.0]);
        assert_eq!(found[0].quality.largest_gap, 5.0);
        assert!((found[0].quality.query_coverage - 2.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn offset_jump_remains_a_separate_concurrent_passage() {
        let found = passages(vec![
            segment(0.0, 100.0, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]),
            segment(6.0, 250.0, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]),
        ]);
        assert_eq!(found.len(), 2);
        assert_ne!(found[0].passage_id, found[1].passage_id);
    }

    #[test]
    fn passage_identity_does_not_depend_on_diagnostic_segments() {
        let segments = vec![segment(0.0, 100.0, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0])];
        let compact =
            passages_from_segments("a", "ah", "b", "bh", "config", segments.clone(), false)
                .expect("compact");
        let explained = passages_from_segments("a", "ah", "b", "bh", "config", segments, true)
            .expect("explained");
        assert_eq!(compact[0].passage_id, explained[0].passage_id);
        assert!(compact[0].segments.is_none());
        assert!(explained[0].segments.is_some());
    }

    #[test]
    fn overlapping_query_regions_do_not_count_the_same_hits_twice() {
        let repeated = segment(0.0, 100.0, &[0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        let found = passages(vec![repeated.clone(), repeated]);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].quality.segment_count, 2);
        assert_eq!(found[0].quality.matched_hits, 6);
        assert_eq!(
            found[0].support.iter().map(|span| span.hits).sum::<usize>(),
            6
        );
    }
}
