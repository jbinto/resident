use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};

use resident_core::config::{MAX_FREQUENCY_BIN, MAX_HASH, bins_to_seconds};
use resident_core::{
    DumpResource, Error, Fingerprint, Matcher, ResourceMeta, Store, candidates, candidates_among,
    crosscheck_between, crosscheck_between_multiline, discover_passages_between, extract_audio,
    load_dump_dir, load_prints, passages_between, probe_window, span_between,
    span_between_multiline,
};
use serde::Deserialize;
use serde_json::{Value, json};

struct State {
    root: PathBuf,
    ram_index: bool,
    store: RwLock<Option<Arc<Store>>>,
    writer: Mutex<()>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "verb", rename_all = "lowercase")]
enum Request {
    Ping,
    Candidates {
        #[serde(default)]
        prints: Option<Vec<[u64; 3]>>,
        #[serde(default)]
        prints_path: Option<PathBuf>,
        #[serde(default)]
        audio_path: Option<PathBuf>,
        #[serde(default)]
        window: Option<[f64; 2]>,
        #[serde(default = "default_candidates_k")]
        k: usize,
        #[serde(default)]
        targets: Option<Vec<String>>,
    },
    Match {
        #[serde(default)]
        prints: Option<Vec<[u64; 3]>>,
        #[serde(default)]
        prints_path: Option<PathBuf>,
        #[serde(default = "default_match_k")]
        k: usize,
        #[serde(default)]
        evidence: bool,
        #[serde(default)]
        multi_line: bool,
    },
    Span {
        a_key: String,
        #[serde(default)]
        a_window: Option<[f64; 2]>,
        b_key: String,
        #[serde(default)]
        b_store: Option<PathBuf>,
        #[serde(default)]
        evidence: bool,
        #[serde(default)]
        multi_line: bool,
    },
    Crosscheck {
        a_key: String,
        #[serde(default)]
        a_window: Option<[f64; 2]>,
        targets: Targets,
        #[serde(default)]
        b_store: Option<PathBuf>,
        #[serde(default = "default_crosscheck_k")]
        k: usize,
        #[serde(default)]
        evidence: bool,
        #[serde(default)]
        multi_line: bool,
        #[serde(default)]
        skip_self: bool,
    },
    Passages {
        a_key: String,
        #[serde(default)]
        a_window: Option<[f64; 2]>,
        b_key: String,
        #[serde(default)]
        b_store: Option<PathBuf>,
        #[serde(default)]
        evidence: bool,
    },
    Discover {
        a_key: String,
        #[serde(default)]
        a_window: Option<[f64; 2]>,
        targets: Targets,
        #[serde(default)]
        exclude_keys: Vec<String>,
        #[serde(default)]
        b_store: Option<PathBuf>,
        #[serde(default)]
        evidence: bool,
    },
    Ingest {
        #[serde(default)]
        dump_dir: Option<PathBuf>,
        #[serde(default)]
        resources: Option<Vec<WireResource>>,
        #[serde(default)]
        replace: bool,
    },
    Retire {
        key: String,
    },
    Stats,
    Extract {
        audio_path: PathBuf,
    },
    Enroll {
        audio_path: PathBuf,
        key: String,
        #[serde(default)]
        replace: bool,
    },
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum Targets {
    All(String),
    Keys(Vec<String>),
}

#[derive(Debug, Deserialize)]
struct WireResource {
    key: String,
    prints_path: PathBuf,
    duration: f64,
}

fn default_match_k() -> usize {
    10
}

fn default_candidates_k() -> usize {
    5
}

fn default_crosscheck_k() -> usize {
    25
}

/// Serve requests with `workers` request threads (0 = one per Rayon thread).
///
/// Requests run on plain threads, never as Rayon tasks: a Rayon worker that blocks inside one
/// request's parallel loop steals other queued Rayon tasks, so per-line Rayon tasks nested whole
/// requests on one stack until a flood overflowed it. Here each request thread hands its parallel
/// work to the global Rayon pool and waits, and at most `workers` requests are in flight. A blocked
/// Rayon worker can still steal another in-flight request's work, so nesting is bounded by
/// `workers`, not by the backlog. The stdin reader blocks once `workers` more lines are queued, so a
/// flood is backpressure, not memory.
pub(crate) fn run(root: PathBuf, workers: usize, ram_index: bool) -> anyhow::Result<()> {
    let store = match Store::open(&root) {
        Ok(store) => Some(Arc::new(serving(store, ram_index)?)),
        Err(Error::StoreMissing(_)) => None,
        Err(error) => return Err(error.into()),
    };
    let state = Arc::new(State {
        root,
        ram_index,
        store: RwLock::new(store),
        writer: Mutex::new(()),
    });
    let workers = if workers == 0 {
        rayon::current_num_threads()
    } else {
        workers
    };
    let output = Arc::new(Mutex::new(BufWriter::new(std::io::stdout())));
    let (sender, receiver) = std::sync::mpsc::sync_channel::<std::io::Result<String>>(workers);
    let receiver = Arc::new(Mutex::new(receiver));
    let threads = (0..workers)
        .map(|index| {
            let state = Arc::clone(&state);
            let output = Arc::clone(&output);
            let receiver = Arc::clone(&receiver);
            std::thread::Builder::new()
                .name(format!("request-{index}"))
                .spawn(move || {
                    loop {
                        // The guard is a temporary of this statement: released before the request
                        // runs, so the other request threads keep taking lines meanwhile.
                        let line = match receiver.lock() {
                            Ok(receiver) => receiver.recv(),
                            Err(_) => return,
                        };
                        let Ok(line) = line else { return };
                        let response = match line {
                            Ok(line) => process_line(&state, &line),
                            Err(error) => error_response(
                                Value::Null,
                                &Error::BadRequest(format!("read stdin: {error}")),
                            ),
                        };
                        if let Ok(mut output) = output.lock() {
                            let _ = serde_json::to_writer(&mut *output, &response);
                            let _ = output.write_all(b"\n");
                            let _ = output.flush();
                        }
                    }
                })
        })
        .collect::<std::io::Result<Vec<_>>>()?;
    for line in BufReader::new(std::io::stdin()).lines() {
        if sender.send(line).is_err() {
            break;
        }
    }
    drop(sender);
    for thread in threads {
        if thread.join().is_err() {
            anyhow::bail!("a request thread panicked");
        }
    }
    Ok(())
}

fn process_line(state: &State, line: &str) -> Value {
    let raw: Value = match serde_json::from_str(line) {
        Ok(value) => value,
        Err(error) => {
            return error_response(
                Value::Null,
                &Error::BadRequest(format!("malformed JSON: {error}")),
            );
        }
    };
    let id = raw
        .get("id")
        .filter(|id| id.is_string())
        .cloned()
        .unwrap_or(Value::Null);
    if id.is_null() {
        return error_response(
            Value::Null,
            &Error::BadRequest("request id must be a string".into()),
        );
    }
    let Some(verb) = raw.get("verb").and_then(Value::as_str) else {
        return error_response(id, &Error::BadRequest("missing string verb".into()));
    };
    if !matches!(
        verb,
        "ping"
            | "match"
            | "span"
            | "crosscheck"
            | "passages"
            | "discover"
            | "ingest"
            | "retire"
            | "stats"
            | "extract"
            | "enroll"
            | "candidates"
    ) {
        return error_response(id, &Error::Unsupported(format!("verb {verb:?}")));
    }
    let request: Request = match serde_json::from_value(raw) {
        Ok(request) => request,
        Err(error) => {
            return error_response(id, &Error::BadRequest(error.to_string()));
        }
    };
    match handle(state, request) {
        Ok(result) => success_response(id, result),
        Err(error) => error_response(id, &error),
    }
}

fn handle(state: &State, request: Request) -> resident_core::Result<Value> {
    match request {
        Request::Ping => {
            let store = snapshot(state)?;
            let store = store.as_ref().map(|store| {
                let stats = store.stats();
                json!({
                    "path": stats.path,
                    "generation": stats.generation,
                    "resources": stats.resources,
                    "postings": stats.postings,
                    "config_id": stats.config_id,
                })
            });
            Ok(json!({
                "engine": "resident",
                "version": env!("CARGO_PKG_VERSION"),
                "store": store,
            }))
        }
        Request::Candidates {
            prints,
            prints_path,
            audio_path,
            window,
            k,
            targets,
        } => {
            let (prints, duration) = match audio_path {
                Some(audio_path) => {
                    if prints.is_some() || prints_path.is_some() {
                        return Err(Error::BadRequest(
                            "provide exactly one of prints, prints_path or audio_path".into(),
                        ));
                    }
                    let extraction = extract_audio(&audio_path)?;
                    (extraction.prints, Some(extraction.duration))
                }
                None => (wire_prints(prints, prints_path)?, None),
            };
            let (prints, duration) = probe_window(
                prints,
                window.map(|window| (window[0], window[1])),
                duration,
            )?;
            let store = required_store(state)?;
            let found = match targets {
                Some(targets) => candidates_among(&store, &prints, duration, &targets, k)?,
                None => candidates(&store, &prints, duration, k)?,
            };
            Ok(json!({ "candidates": found }))
        }
        Request::Match {
            prints,
            prints_path,
            k,
            evidence,
            multi_line,
        } => {
            let prints = wire_prints(prints, prints_path)?;
            let store = required_store(state)?;
            let matcher = Matcher::new(&store);
            let rows = if multi_line {
                matcher.match_prints_multiline(&prints, k, evidence)?
            } else {
                matcher.match_prints(&prints, k, evidence)?
            };
            Ok(json!({ "rows": rows }))
        }
        Request::Span {
            a_key,
            a_window,
            b_key,
            b_store,
            evidence,
            multi_line,
        } => {
            let store = required_store(state)?;
            let target_store = b_store.as_deref().map(Store::open).transpose()?;
            let target_store = target_store.as_ref().unwrap_or(store.as_ref());
            let window = a_window.map(|window| (window[0], window[1]));
            let segments = if multi_line {
                span_between_multiline(&store, target_store, &a_key, window, &b_key, evidence)?
            } else {
                span_between(&store, target_store, &a_key, window, &b_key, evidence)?
            };
            Ok(json!({ "segments": segments }))
        }
        Request::Crosscheck {
            a_key,
            a_window,
            targets,
            b_store,
            k,
            evidence,
            multi_line,
            skip_self,
        } => {
            let keys = match targets {
                Targets::All(value) if value == "all" => None,
                Targets::All(value) => {
                    return Err(Error::BadRequest(format!(
                        "targets string must be \"all\", got {value:?}"
                    )));
                }
                Targets::Keys(keys) => Some(keys),
            };
            let store = required_store(state)?;
            let target_store = b_store.as_deref().map(Store::open).transpose()?;
            let target_store = target_store.as_ref().unwrap_or(store.as_ref());
            let window = a_window.map(|window| (window[0], window[1]));
            let matches = if multi_line {
                crosscheck_between_multiline(
                    &store,
                    target_store,
                    &a_key,
                    window,
                    keys.as_deref(),
                    k,
                    evidence,
                    skip_self,
                )?
            } else {
                crosscheck_between(
                    &store,
                    target_store,
                    &a_key,
                    window,
                    keys.as_deref(),
                    k,
                    evidence,
                    skip_self,
                )?
            };
            Ok(json!({ "matches": matches }))
        }
        Request::Passages {
            a_key,
            a_window,
            b_key,
            b_store,
            evidence,
        } => {
            let store = required_store(state)?;
            let target_store = b_store.as_deref().map(Store::open).transpose()?;
            let target_store = target_store.as_ref().unwrap_or(store.as_ref());
            let window = a_window.map(|window| (window[0], window[1]));
            Ok(json!(passages_between(
                &store,
                target_store,
                &a_key,
                window,
                &b_key,
                evidence,
            )?))
        }
        Request::Discover {
            a_key,
            a_window,
            targets,
            exclude_keys,
            b_store,
            evidence,
        } => {
            let keys = match targets {
                Targets::All(value) if value == "all" => None,
                Targets::All(value) => {
                    return Err(Error::BadRequest(format!(
                        "targets string must be \"all\", got {value:?}"
                    )));
                }
                Targets::Keys(keys) => Some(keys),
            };
            let store = required_store(state)?;
            let target_store = b_store.as_deref().map(Store::open).transpose()?;
            let target_store = target_store.as_ref().unwrap_or(store.as_ref());
            let window = a_window.map(|window| (window[0], window[1]));
            Ok(json!(discover_passages_between(
                &store,
                target_store,
                &a_key,
                window,
                keys.as_deref(),
                (!exclude_keys.is_empty()).then_some(exclude_keys.as_slice()),
                evidence,
            )?))
        }
        Request::Ingest {
            dump_dir,
            resources,
            replace,
        } => {
            let resources = ingest_resources(dump_dir, resources)?;
            let _writer = state
                .writer
                .lock()
                .map_err(|_| Error::Internal("writer lock poisoned".into()))?;
            let (store, stats) = Store::ingest(&state.root, resources, replace)?;
            *state
                .store
                .write()
                .map_err(|_| Error::Internal("store lock poisoned".into()))? =
                Some(Arc::new(serving(store, state.ram_index)?));
            serde_json::to_value(stats)
                .map_err(|error| Error::Internal(format!("serialize ingest result: {error}")))
        }
        Request::Retire { key } => {
            let _writer = state
                .writer
                .lock()
                .map_err(|_| Error::Internal("writer lock poisoned".into()))?;
            let (store, stats) = Store::retire(&state.root, &key)?;
            *state
                .store
                .write()
                .map_err(|_| Error::Internal("store lock poisoned".into()))? =
                Some(Arc::new(serving(store, state.ram_index)?));
            serde_json::to_value(stats)
                .map_err(|error| Error::Internal(format!("serialize retire result: {error}")))
        }
        Request::Stats => {
            let store = required_store(state)?;
            let resources: Vec<_> = store
                .resources()
                .iter()
                .map(|resource| {
                    json!({
                        "key": resource.key,
                        "duration": resource.duration,
                        "postings": resource.postings,
                        "t_min": bins_to_seconds(resource.t_min),
                        "t_max": bins_to_seconds(resource.t_max),
                    })
                })
                .collect();
            Ok(json!({
                "store": store.stats(),
                "resources": resources,
            }))
        }
        Request::Extract { audio_path } => {
            let extraction = extract_audio(&audio_path)?;
            Ok(json!({
                "prints": prints_to_wire(&extraction.prints),
                "duration": extraction.duration,
            }))
        }
        Request::Enroll {
            audio_path,
            key,
            replace,
        } => {
            let extraction = extract_audio(&audio_path)?;
            let resource = DumpResource {
                meta: ResourceMeta {
                    source_id: "native".into(),
                    key,
                    duration: extraction.duration,
                    declared_prints: extraction.prints.len() as u64,
                },
                prints: extraction.prints,
                prints_path: audio_path,
            };
            let _writer = state
                .writer
                .lock()
                .map_err(|_| Error::Internal("writer lock poisoned".into()))?;
            let (store, stats) = Store::ingest(&state.root, vec![resource], replace)?;
            *state
                .store
                .write()
                .map_err(|_| Error::Internal("store lock poisoned".into()))? =
                Some(Arc::new(serving(store, state.ram_index)?));
            Ok(json!({
                "generation": stats.generation,
                "postings_added": stats.postings_added,
                "duration": extraction.duration,
            }))
        }
    }
}

fn prints_to_wire(prints: &[Fingerprint]) -> Vec<[u64; 3]> {
    prints
        .iter()
        .map(|print| [print.hash, u64::from(print.t), u64::from(print.f)])
        .collect()
}

/// The store as this daemon serves it: with `--ram-index`, its inverted index loaded into memory.
/// A failed load is an error, never a silent fallback to the mapped path.
fn serving(mut store: Store, ram_index: bool) -> resident_core::Result<Store> {
    if ram_index {
        let stats = store.load_ram_index()?;
        tracing::info!(
            postings = stats.postings,
            bytes = stats.bytes,
            build_seconds = stats.build_seconds,
            "ram index loaded"
        );
    }
    Ok(store)
}

fn snapshot(state: &State) -> resident_core::Result<Option<Arc<Store>>> {
    state
        .store
        .read()
        .map(|store| store.clone())
        .map_err(|_| Error::Internal("store lock poisoned".into()))
}

fn required_store(state: &State) -> resident_core::Result<Arc<Store>> {
    snapshot(state)?.ok_or_else(|| Error::StoreMissing(state.root.clone()))
}

fn wire_prints(
    inline: Option<Vec<[u64; 3]>>,
    path: Option<PathBuf>,
) -> resident_core::Result<Vec<Fingerprint>> {
    match (inline, path) {
        (Some(_), Some(_)) | (None, None) => Err(Error::BadRequest(
            "provide exactly one of prints or prints_path".into(),
        )),
        (None, Some(path)) => load_prints(&path),
        (Some(values), None) => values
            .into_iter()
            .enumerate()
            .map(|(index, [hash, t, f])| {
                if hash > MAX_HASH || t > u64::from(u32::MAX) || f > u64::from(MAX_FREQUENCY_BIN) {
                    return Err(Error::BadRequest(format!(
                        "prints[{index}] is outside hash/time/frequency range"
                    )));
                }
                Ok(Fingerprint::new(hash, t as u32, f as u16))
            })
            .collect(),
    }
}

fn ingest_resources(
    dump_dir: Option<PathBuf>,
    resources: Option<Vec<WireResource>>,
) -> resident_core::Result<Vec<DumpResource>> {
    match (dump_dir, resources) {
        (Some(_), Some(_)) | (None, None) => Err(Error::BadRequest(
            "provide exactly one of dump_dir or resources".into(),
        )),
        (Some(path), None) => load_dump_dir(&path),
        (None, Some(resources)) => resources
            .into_iter()
            .map(|resource| {
                if !resource.duration.is_finite() || resource.duration < 0.0 {
                    return Err(Error::BadRequest(format!(
                        "resource {:?} duration must be finite and non-negative",
                        resource.key
                    )));
                }
                let prints = load_prints(&resource.prints_path)?;
                Ok(DumpResource {
                    meta: ResourceMeta {
                        source_id: "wire".into(),
                        key: resource.key,
                        duration: resource.duration,
                        declared_prints: prints.len() as u64,
                    },
                    prints,
                    prints_path: resource.prints_path,
                })
            })
            .collect(),
    }
}

fn success_response(id: Value, result: Value) -> Value {
    let mut response = serde_json::Map::new();
    response.insert("id".into(), id);
    response.insert("ok".into(), Value::Bool(true));
    if let Value::Object(result) = result {
        response.extend(result);
    }
    Value::Object(response)
}

fn error_response(id: Value, error: &Error) -> Value {
    json!({
        "id": id,
        "ok": false,
        "error": {
            "kind": error.wire_kind(),
            "message": error.to_string(),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn malformed_line_has_null_id_and_typed_error() {
        let state = State {
            root: PathBuf::from("absent"),
            ram_index: false,
            store: RwLock::new(None),
            writer: Mutex::new(()),
        };
        let response = process_line(&state, "not json");
        assert_eq!(response["id"], Value::Null);
        assert_eq!(response["error"]["kind"], "bad_request");
    }

    #[test]
    fn unknown_verb_is_unsupported() {
        let state = State {
            root: PathBuf::from("absent"),
            ram_index: false,
            store: RwLock::new(None),
            writer: Mutex::new(()),
        };
        let response = process_line(&state, r#"{"id":"x","verb":"olaf"}"#);
        assert_eq!(response["id"], "x");
        assert_eq!(response["error"]["kind"], "unsupported");
    }

    #[test]
    fn multiline_match_flag_defaults_off() {
        let request: Request =
            serde_json::from_str(r#"{"verb":"match","prints":[[1,2,3]],"k":10,"evidence":false}"#)
                .expect("valid match request");
        let Request::Match { multi_line, .. } = request else {
            panic!("expected match request");
        };
        assert!(!multi_line);

        let request: Request =
            serde_json::from_str(r#"{"verb":"match","prints":[[1,2,3]],"multi_line":true}"#)
                .expect("valid multiline match request");
        let Request::Match { multi_line, .. } = request else {
            panic!("expected match request");
        };
        assert!(multi_line);
    }

    #[test]
    fn cross_store_path_defaults_to_attached_store() {
        let request: Request =
            serde_json::from_str(r#"{"verb":"span","a_key":"a","b_key":"b","evidence":false}"#)
                .expect("valid span request");
        let Request::Span {
            b_store,
            multi_line,
            ..
        } = request
        else {
            panic!("expected span request");
        };
        assert!(b_store.is_none());
        assert!(!multi_line);

        let request: Request =
            serde_json::from_str(r#"{"verb":"crosscheck","a_key":"a","targets":"all"}"#)
                .expect("valid crosscheck request");
        let Request::Crosscheck { multi_line, .. } = request else {
            panic!("expected crosscheck request");
        };
        assert!(!multi_line);

        let request: Request = serde_json::from_str(
            r#"{"verb":"crosscheck","a_key":"a","targets":"all","multi_line":true}"#,
        )
        .expect("valid multiline crosscheck request");
        let Request::Crosscheck { multi_line, .. } = request else {
            panic!("expected crosscheck request");
        };
        assert!(multi_line);
    }

    #[test]
    fn passage_verbs_are_additive_and_evidence_defaults_off() {
        let request: Request =
            serde_json::from_str(r#"{"verb":"passages","a_key":"a","b_key":"b"}"#)
                .expect("valid passages request");
        let Request::Passages {
            b_store, evidence, ..
        } = request
        else {
            panic!("expected passages request");
        };
        assert!(b_store.is_none());
        assert!(!evidence);

        let request: Request =
            serde_json::from_str(r#"{"verb":"discover","a_key":"a","targets":"all"}"#)
                .expect("valid discover request");
        let Request::Discover {
            b_store, evidence, ..
        } = request
        else {
            panic!("expected discover request");
        };
        assert!(b_store.is_none());
        assert!(!evidence);
    }

    #[test]
    fn passage_verbs_reach_dispatch() {
        let state = State {
            root: PathBuf::from("absent"),
            ram_index: false,
            store: RwLock::new(None),
            writer: Mutex::new(()),
        };
        for request in [
            r#"{"id":"p","verb":"passages","a_key":"a","b_key":"b"}"#,
            r#"{"id":"d","verb":"discover","a_key":"a","targets":"all"}"#,
        ] {
            let response = process_line(&state, request);
            assert_eq!(response["error"]["kind"], "store_missing");
        }
    }

    #[test]
    fn ram_index_daemon_answers_like_the_mapped_daemon() {
        let dump = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../fixtures/store-dump");
        let first = resident_core::load_dump_dir(&dump).unwrap().remove(0);
        let wire: Vec<[u64; 3]> = prints_to_wire(&first.prints[200..2_200]);
        let answers = |ram_index: bool| {
            let root = tempfile::tempdir().unwrap();
            let state = State {
                root: root.path().to_path_buf(),
                ram_index,
                store: RwLock::new(None),
                writer: Mutex::new(()),
            };
            let ingest = json!({"id": "i", "verb": "ingest", "dump_dir": dump});
            assert_eq!(process_line(&state, &ingest.to_string())["ok"], true);
            assert_eq!(
                required_store(&state).unwrap().ram_index_loaded(),
                ram_index,
                "the swapped-in store must serve from the RAM index when asked"
            );
            let ask = |verb: &str| {
                let request = json!({"id": verb, "verb": verb, "prints": wire, "k": 5,
                    "evidence": true, "multi_line": true});
                process_line(&state, &request.to_string())
            };
            (ask("match"), ask("candidates"))
        };
        let (mapped, ram) = (answers(false), answers(true));
        assert!(!mapped.0["rows"].as_array().unwrap().is_empty());
        assert!(!mapped.1["candidates"].as_array().unwrap().is_empty());
        assert_eq!(mapped, ram);
    }
}
