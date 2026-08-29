use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use resident_core::config::seconds_to_bin;
use resident_core::{Error, Fingerprint, MatchRow, Matcher, Store, load_dump_dir, load_prints};
use serde::{Deserialize, Serialize};

pub(crate) struct Options {
    pub a_store: PathBuf,
    pub b_store: PathBuf,
    pub probes_dir: Option<PathBuf>,
    pub questions: Option<PathBuf>,
    pub k: usize,
    pub max_score_delta: usize,
    pub largest: usize,
    pub evidence: Option<String>,
}

#[derive(Clone, Debug)]
struct Question {
    name: String,
    source: QuestionSource,
}

#[derive(Clone, Debug)]
enum QuestionSource {
    Prints(PathBuf),
    Store {
        key: String,
        window: Option<[f64; 2]>,
    },
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct QuestionLine {
    name: String,
    key: String,
    #[serde(default)]
    window: Option<[f64; 2]>,
}

#[derive(Clone, Debug, Serialize)]
struct QuestionDescriptor {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    window: Option<[f64; 2]>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prints_path: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize)]
struct RowComparison {
    ref_key: String,
    present_a: bool,
    present_b: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    q_span_iou: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ref_span_iou: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    score_a: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    score_b: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    score_delta: Option<i64>,
}

#[derive(Clone, Debug, Serialize)]
struct QuestionComparison {
    #[serde(flatten)]
    question: QuestionDescriptor,
    agreed: bool,
    same_references: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_a: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_b: Option<String>,
    rows: Vec<RowComparison>,
    severity: u64,
}

#[derive(Clone, Debug, Serialize)]
struct Divergence {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    window: Option<[f64; 2]>,
    severity: u64,
    refs: Vec<String>,
}

#[derive(Clone, Debug, Serialize)]
struct EvidenceDump {
    question: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_a: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error_b: Option<String>,
    rows_a: Vec<MatchRow>,
    rows_b: Vec<MatchRow>,
}

#[derive(Clone, Debug, Serialize)]
struct Aggregate {
    questions: usize,
    agreed: usize,
    agreement_percent: f64,
    same_reference_sets: usize,
    differing_rows: usize,
}

#[derive(Clone, Debug, Serialize)]
struct StoreIdentity {
    path: PathBuf,
    generation: String,
    config_id: String,
    resources: usize,
    postings: u64,
}

#[derive(Clone, Debug, Serialize)]
struct Criteria {
    reference_sets: &'static str,
    spans: &'static str,
    max_score_delta: usize,
}

#[derive(Clone, Debug, Serialize)]
struct Report {
    store_a: StoreIdentity,
    store_b: StoreIdentity,
    criteria: Criteria,
    aggregate: Aggregate,
    questions: Vec<QuestionComparison>,
    largest_divergences: Vec<Divergence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    evidence: Option<EvidenceDump>,
}

struct SideRows {
    rows: Vec<MatchRow>,
    error: Option<String>,
}

pub(crate) fn run(options: Options) -> anyhow::Result<()> {
    if options.k == 0 {
        bail!("--k must be greater than zero");
    }
    if options.largest == 0 {
        bail!("--largest must be greater than zero");
    }
    let store_a = Store::open(&options.a_store)
        .with_context(|| format!("open A store {}", options.a_store.display()))?;
    let store_b = Store::open(&options.b_store)
        .with_context(|| format!("open B store {}", options.b_store.display()))?;
    let questions = if let Some(path) = options.probes_dir {
        questions_from_directory(&path)?
    } else if let Some(path) = options.questions {
        questions_from_manifest(&path)?
    } else {
        bail!("provide exactly one of --probes-dir or --questions");
    };
    let report = compare(
        &store_a,
        &store_b,
        &questions,
        options.k,
        options.max_score_delta,
        options.largest,
        options.evidence.as_deref(),
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

pub(crate) fn validate(fixtures: &Path) -> anyhow::Result<()> {
    let temporary = tempfile::tempdir().context("create A/B validation directory")?;
    let resources = load_dump_dir(&fixtures.join("store-dump"))?;
    let store_a = Store::build(&temporary.path().join("a"), resources.clone())?;
    let store_b = Store::build(&temporary.path().join("b"), resources)?;
    let questions = fixture_questions(fixtures)?;
    let identical = compare(&store_a, &store_a, &questions, 25, 0, questions.len(), None)?;
    if identical.aggregate.agreed != questions.len()
        || identical.aggregate.agreement_percent != 100.0
    {
        bail!("fixture store did not agree with itself");
    }

    let retired_key = "/corpus/wefunk/shows/0789/audio.m4a";
    drop(store_b);
    let (retired, _) = Store::retire(&temporary.path().join("b"), retired_key)?;
    let changed = compare(&store_a, &retired, &questions, 25, 0, questions.len(), None)?;
    let mut missing_rows = 0;
    for question in &changed.questions {
        for row in &question.rows {
            if row.present_a != row.present_b {
                if row.ref_key != retired_key || !row.present_a || row.present_b {
                    bail!(
                        "retirement changed unexpected row {:?} in {}",
                        row.ref_key,
                        question.question.name
                    );
                }
                missing_rows += 1;
            } else if row.score_delta != Some(0)
                || row.q_span_iou.is_some_and(|overlap| overlap != 1.0)
                || row.ref_span_iou.is_some_and(|overlap| overlap != 1.0)
            {
                bail!(
                    "retirement changed surviving row {:?} in {}",
                    row.ref_key,
                    question.question.name
                );
            }
        }
    }
    if missing_rows == 0 {
        bail!("retirement validation observed no missing rows");
    }
    println!(
        "ab: self={}/{} (100.0%) retired_key={retired_key} missing_rows={missing_rows} other_deltas=0",
        identical.aggregate.agreed, identical.aggregate.questions
    );
    Ok(())
}

fn compare(
    store_a: &Store,
    store_b: &Store,
    questions: &[Question],
    k: usize,
    max_score_delta: usize,
    largest: usize,
    evidence_name: Option<&str>,
) -> anyhow::Result<Report> {
    if let Some(name) = evidence_name {
        let count = questions
            .iter()
            .filter(|question| question.name == name)
            .count();
        if count != 1 {
            bail!("--evidence question {name:?} matched {count} questions");
        }
    }
    let mut comparisons = Vec::with_capacity(questions.len());
    let mut evidence = None;
    for question in questions {
        let include_evidence = evidence_name == Some(question.name.as_str());
        let (side_a, side_b) = execute_question(store_a, store_b, question, k, include_evidence)?;
        if include_evidence {
            evidence = Some(EvidenceDump {
                question: question.name.clone(),
                error_a: side_a.error.clone(),
                error_b: side_b.error.clone(),
                rows_a: side_a.rows.clone(),
                rows_b: side_b.rows.clone(),
            });
        }
        comparisons.push(compare_question(
            question,
            &side_a,
            &side_b,
            max_score_delta,
        ));
    }
    let agreed = comparisons
        .iter()
        .filter(|question| question.agreed)
        .count();
    let same_reference_sets = comparisons
        .iter()
        .filter(|question| question.same_references)
        .count();
    let differing_rows = comparisons
        .iter()
        .flat_map(|question| &question.rows)
        .filter(|row| {
            row.present_a != row.present_b
                || row.score_delta != Some(0)
                || row.q_span_iou.is_some_and(|overlap| overlap != 1.0)
                || row.ref_span_iou.is_some_and(|overlap| overlap != 1.0)
        })
        .count();
    let mut largest_divergences: Vec<_> = comparisons
        .iter()
        .filter(|question| !question.agreed)
        .map(|question| Divergence {
            name: question.question.name.clone(),
            key: question.question.key.clone(),
            window: question.question.window,
            severity: question.severity,
            refs: question
                .rows
                .iter()
                .filter(|row| {
                    row.present_a != row.present_b
                        || row
                            .score_delta
                            .is_some_and(|delta| delta.unsigned_abs() > max_score_delta as u64)
                        || row.q_span_iou == Some(0.0)
                        || row.ref_span_iou == Some(0.0)
                })
                .map(|row| row.ref_key.clone())
                .collect(),
        })
        .collect();
    largest_divergences.sort_by(|a, b| {
        b.severity
            .cmp(&a.severity)
            .then_with(|| a.name.cmp(&b.name))
    });
    largest_divergences.truncate(largest);
    let question_count = comparisons.len();
    Ok(Report {
        store_a: store_identity(store_a),
        store_b: store_identity(store_b),
        criteria: Criteria {
            reference_sets: "identical",
            spans: "positive intersection on query and reference spans",
            max_score_delta,
        },
        aggregate: Aggregate {
            questions: question_count,
            agreed,
            agreement_percent: agreed as f64 * 100.0 / question_count as f64,
            same_reference_sets,
            differing_rows,
        },
        questions: comparisons,
        largest_divergences,
        evidence,
    })
}

fn store_identity(store: &Store) -> StoreIdentity {
    let stats = store.stats();
    StoreIdentity {
        path: stats.path,
        generation: stats.generation,
        config_id: stats.config_id,
        resources: stats.resources,
        postings: stats.postings,
    }
}

fn execute_question(
    store_a: &Store,
    store_b: &Store,
    question: &Question,
    k: usize,
    evidence: bool,
) -> anyhow::Result<(SideRows, SideRows)> {
    match &question.source {
        QuestionSource::Prints(path) => {
            let prints = load_prints(path)?;
            Ok((
                run_match(store_a, Ok(prints.clone()), k, evidence)?,
                run_match(store_b, Ok(prints), k, evidence)?,
            ))
        }
        QuestionSource::Store { key, window } => Ok((
            run_match(store_a, store_prints(store_a, key, *window), k, evidence)?,
            run_match(store_b, store_prints(store_b, key, *window), k, evidence)?,
        )),
    }
}

fn run_match(
    store: &Store,
    prints: resident_core::Result<Vec<Fingerprint>>,
    k: usize,
    evidence: bool,
) -> anyhow::Result<SideRows> {
    let prints = match prints {
        Ok(prints) => prints,
        Err(Error::BadRequest(message)) => {
            return Ok(SideRows {
                rows: Vec::new(),
                error: Some(message),
            });
        }
        Err(error) => return Err(error.into()),
    };
    match Matcher::new(store).match_prints(&prints, k, evidence) {
        Ok(rows) => Ok(SideRows { rows, error: None }),
        Err(Error::BadRequest(message)) => Ok(SideRows {
            rows: Vec::new(),
            error: Some(message),
        }),
        Err(error) => Err(error.into()),
    }
}

fn store_prints(
    store: &Store,
    key: &str,
    window: Option<[f64; 2]>,
) -> resident_core::Result<Vec<Fingerprint>> {
    let bins = if let Some([start, stop]) = window {
        if !start.is_finite() || !stop.is_finite() || start < 0.0 || start >= stop {
            return Err(Error::BadRequest(
                "question window must contain finite, non-negative increasing seconds".into(),
            ));
        }
        Some((
            seconds_to_bin(start).unwrap_or(0),
            seconds_to_bin(stop).unwrap_or(u32::MAX),
        ))
    } else {
        None
    };
    store.forward(key, bins)
}

fn compare_question(
    question: &Question,
    side_a: &SideRows,
    side_b: &SideRows,
    max_score_delta: usize,
) -> QuestionComparison {
    let a: BTreeMap<_, _> = side_a
        .rows
        .iter()
        .map(|row| (row.ref_key.as_str(), row))
        .collect();
    let b: BTreeMap<_, _> = side_b
        .rows
        .iter()
        .map(|row| (row.ref_key.as_str(), row))
        .collect();
    let keys: HashSet<_> = a.keys().chain(b.keys()).copied().collect();
    let mut keys: Vec<_> = keys.into_iter().collect();
    keys.sort_unstable();
    let mut rows = Vec::new();
    let mut severity = 0_u64;
    for key in keys {
        let row_a = a.get(key).copied();
        let row_b = b.get(key).copied();
        let comparison = if let (Some(row_a), Some(row_b)) = (row_a, row_b) {
            let q_span_iou = span_iou(row_a.q_start, row_a.q_stop, row_b.q_start, row_b.q_stop);
            let ref_span_iou = span_iou(
                row_a.ref_start,
                row_a.ref_stop,
                row_b.ref_start,
                row_b.ref_stop,
            );
            let score_delta = signed_delta(row_a.score, row_b.score);
            severity = severity
                .saturating_add(score_delta.unsigned_abs())
                .saturating_add(u64::from(q_span_iou == 0.0) * 100_000)
                .saturating_add(u64::from(ref_span_iou == 0.0) * 100_000);
            RowComparison {
                ref_key: key.to_owned(),
                present_a: true,
                present_b: true,
                q_span_iou: Some(q_span_iou),
                ref_span_iou: Some(ref_span_iou),
                score_a: Some(row_a.score),
                score_b: Some(row_b.score),
                score_delta: Some(score_delta),
            }
        } else {
            severity = severity.saturating_add(1_000_000);
            RowComparison {
                ref_key: key.to_owned(),
                present_a: row_a.is_some(),
                present_b: row_b.is_some(),
                q_span_iou: None,
                ref_span_iou: None,
                score_a: row_a.map(|row| row.score),
                score_b: row_b.map(|row| row.score),
                score_delta: None,
            }
        };
        rows.push(comparison);
    }
    if side_a.error.is_some() || side_b.error.is_some() {
        severity = severity.saturating_add(10_000_000);
    }
    let same_references = a.keys().eq(b.keys());
    let agreed = side_a.error.is_none()
        && side_b.error.is_none()
        && same_references
        && rows.iter().all(|row| {
            row.q_span_iou.is_some_and(|overlap| overlap > 0.0)
                && row.ref_span_iou.is_some_and(|overlap| overlap > 0.0)
                && row
                    .score_delta
                    .is_some_and(|delta| delta.unsigned_abs() <= max_score_delta as u64)
        });
    QuestionComparison {
        question: descriptor(question),
        agreed,
        same_references,
        error_a: side_a.error.clone(),
        error_b: side_b.error.clone(),
        rows,
        severity,
    }
}

fn descriptor(question: &Question) -> QuestionDescriptor {
    match &question.source {
        QuestionSource::Prints(path) => QuestionDescriptor {
            name: question.name.clone(),
            key: None,
            window: None,
            prints_path: Some(path.clone()),
        },
        QuestionSource::Store { key, window } => QuestionDescriptor {
            name: question.name.clone(),
            key: Some(key.clone()),
            window: *window,
            prints_path: None,
        },
    }
}

fn span_iou(a_start: f64, a_stop: f64, b_start: f64, b_stop: f64) -> f64 {
    let intersection = (a_stop.min(b_stop) - a_start.max(b_start)).max(0.0);
    let union = a_stop.max(b_stop) - a_start.min(b_start);
    if union > 0.0 {
        intersection / union
    } else {
        0.0
    }
}

fn signed_delta(a: usize, b: usize) -> i64 {
    if b >= a {
        i64::try_from(b - a).unwrap_or(i64::MAX)
    } else {
        -i64::try_from(a - b).unwrap_or(i64::MAX)
    }
}

fn questions_from_directory(path: &Path) -> anyhow::Result<Vec<Question>> {
    let mut files = Vec::new();
    collect_tdb(path, &mut files)?;
    files.sort();
    if files.is_empty() {
        bail!("probe directory {} contains no .tdb files", path.display());
    }
    let mut names = HashSet::new();
    files
        .into_iter()
        .map(|file| {
            let relative = file.strip_prefix(path).expect("collected below root");
            let mut name = relative.to_string_lossy().into_owned();
            name.truncate(name.len() - ".tdb".len());
            if !names.insert(name.clone()) {
                bail!("duplicate probe question name {name:?}");
            }
            Ok(Question {
                name,
                source: QuestionSource::Prints(file),
            })
        })
        .collect()
}

fn collect_tdb(path: &Path, files: &mut Vec<PathBuf>) -> anyhow::Result<()> {
    for entry in fs::read_dir(path).with_context(|| format!("read {}", path.display()))? {
        let entry = entry.with_context(|| format!("read entry under {}", path.display()))?;
        let entry_path = entry.path();
        if entry_path.is_dir() {
            collect_tdb(&entry_path, files)?;
        } else if entry_path
            .extension()
            .is_some_and(|extension| extension == "tdb")
        {
            files.push(entry_path);
        }
    }
    Ok(())
}

fn questions_from_manifest(path: &Path) -> anyhow::Result<Vec<Question>> {
    let text = fs::read_to_string(path)
        .with_context(|| format!("read question manifest {}", path.display()))?;
    let mut names = HashSet::new();
    let mut questions = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line_number = index + 1;
        if line.trim().is_empty() {
            bail!(
                "{}:{line_number}: blank lines are not allowed",
                path.display()
            );
        }
        let parsed: QuestionLine = serde_json::from_str(line)
            .with_context(|| format!("parse {}:{line_number}", path.display()))?;
        if parsed.name.is_empty() || parsed.key.is_empty() {
            bail!(
                "{}:{line_number}: name and key must not be empty",
                path.display()
            );
        }
        if !names.insert(parsed.name.clone()) {
            bail!(
                "{}:{line_number}: duplicate name {:?}",
                path.display(),
                parsed.name
            );
        }
        questions.push(Question {
            name: parsed.name,
            source: QuestionSource::Store {
                key: parsed.key,
                window: parsed.window,
            },
        });
    }
    if questions.is_empty() {
        bail!("question manifest {} contains no questions", path.display());
    }
    Ok(questions)
}

fn fixture_questions(fixtures: &Path) -> anyhow::Result<Vec<Question>> {
    let mut query_dirs: Vec<_> = fs::read_dir(fixtures.join("queries"))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()?;
    query_dirs.retain(|path| path.is_dir());
    query_dirs.sort();
    Ok(query_dirs
        .into_iter()
        .map(|path| Question {
            name: path
                .file_name()
                .expect("fixture query directory name")
                .to_string_lossy()
                .into_owned(),
            source: QuestionSource::Prints(path.join("prints.tdb")),
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(key: &str, q: (f64, f64), reference: (f64, f64), score: usize) -> MatchRow {
        MatchRow {
            ref_key: key.into(),
            q_start: q.0,
            q_stop: q.1,
            ref_start: reference.0,
            ref_stop: reference.1,
            score,
            time_factor: 1.0,
            pitch_factor: 1.0,
            sec_with_match: 1.0,
            evidence: None,
        }
    }

    #[test]
    fn row_comparison_reports_overlap_and_signed_score_delta() {
        let question = Question {
            name: "q".into(),
            source: QuestionSource::Store {
                key: "source".into(),
                window: Some([0.0, 10.0]),
            },
        };
        let comparison = compare_question(
            &question,
            &SideRows {
                rows: vec![row("ref", (0.0, 10.0), (20.0, 30.0), 12)],
                error: None,
            },
            &SideRows {
                rows: vec![row("ref", (5.0, 15.0), (25.0, 35.0), 9)],
                error: None,
            },
            3,
        );
        assert!(comparison.agreed);
        assert_eq!(comparison.rows[0].q_span_iou, Some(1.0 / 3.0));
        assert_eq!(comparison.rows[0].ref_span_iou, Some(1.0 / 3.0));
        assert_eq!(comparison.rows[0].score_delta, Some(-3));
    }
}
