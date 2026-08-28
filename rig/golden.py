#!/usr/bin/env python3
"""Golden-fixture generator. Runs ON the corpus rig (not in this repo's CI-less world).

Builds a mini Panako LMDB store from ~18 real resources (via the jar, using its print cache —
no re-extraction), slices real query windows, captures the exact prints the jar extracts for
them, queries the mini store, and packages dumps + probes + golden answers + manifest.

Everything fails loudly. Honors the production query lock (best-effort, same file the prod
runner uses). Output: /scratch/resident-fixtures/fixtures.tar.zst
"""
import json, os, re, shutil, subprocess, sys, time, hashlib, glob

JAVA = "/scratch/jdk-17/bin/java"
JAR = "/scratch/Panako/build/libs/panako-2.1-all.jar"
OPENS = ["--add-opens", "java.base/java.nio=ALL-UNNAMED",
         "--add-opens", "java.base/sun.nio.ch=ALL-UNNAMED",
         "--add-opens", "java.base/java.lang=ALL-UNNAMED"]
CACHE = os.path.expanduser("~/.panako/panako_cache")
PAIRS = "/scratch/mixmd-runner/dup-pairs.jsonl"
OUT = "/scratch/resident-fixtures"
LOCK = "/tmp/panako-query.lock"
N_PAIRS, N_DISTRACT, WIN = 6, 4, 12.0
COMMON = ["STRATEGY=PANAKO", "CHECK_DUPLICATE_FILE_NAMES=FALSE"]

def run(cmd, **kw):
    p = subprocess.run(cmd, capture_output=True, text=True, stdin=subprocess.DEVNULL, **kw)
    if p.returncode != 0 or "Exception" in p.stderr:
        sys.exit(f"FATAL: {' '.join(cmd[:6])}... rc={p.returncode}\nstderr: {p.stderr[-1500:]}")
    return p.stdout

def jar(args):
    return run(["nice", "-n", "15", JAVA] + OPENS + ["-jar", JAR] + args)

def take_lock():
    for _ in range(60):
        try:
            st = os.stat(LOCK)
            if time.time() - st.st_mtime > 4 * 3600:
                break  # stale, steal
            print("query lock held; waiting 10s...", flush=True)
            time.sleep(10)
        except FileNotFoundError:
            break
    open(LOCK, "w").write(f"resident-fixtures {os.getpid()}\n")

def load_meta():
    """path -> (resourceID, duration, numPrints) from every cache meta file."""
    meta = {}
    for mf in glob.glob(f"{CACHE}/*_meta_data.txt"):
        lines = open(mf).read().splitlines()
        if len(lines) >= 4:
            rid, dur, n, path = lines[0], float(lines[1]), int(lines[2]), lines[3]
            # full-length resources with readable dump files only
            if dur > 600 and os.access(mf, os.R_OK) and os.access(f"{CACHE}/{rid}.tdb", os.R_OK):
                meta[path] = (rid, dur, n)
    return meta

def select(meta):
    pairs, used = [], set()
    for line in open(PAIRS):
        r = json.loads(line)
        s = r["self"]
        if s not in meta or s in used or not os.access(s, os.R_OK):
            continue
        for e in r.get("edges", []):
            if (e["ref"] in meta and not e.get("sameSha") and e.get("spanSec", 0) >= 150
                    and os.access(e["ref"], os.R_OK)):
                pairs.append({"self": s, "ref": e["ref"], "spanSec": e["spanSec"],
                              "windows": e["windows"], "longestRun": e["longestRun"]})
                used.update([s, e["ref"]])
                break
        if len(pairs) >= N_PAIRS:
            break
    if len(pairs) < N_PAIRS:
        sys.exit(f"FATAL: only {len(pairs)} usable pairs found")
    distract = [p for p in meta if p not in used and os.access(p, os.R_OK)][:N_DISTRACT]
    return pairs, distract

def main():
    if os.path.exists(OUT):
        shutil.rmtree(OUT)
    dump_dir, qdir = f"{OUT}/fixtures/store-dump", f"{OUT}/fixtures/queries"
    mini, qcache, wavs = f"{OUT}/mini_db", f"{OUT}/qcache", f"{OUT}/wavs"
    for d in (dump_dir, qdir, mini, qcache, wavs):
        os.makedirs(d)

    meta = load_meta()
    print(f"{len(meta)} full-length cached resources", flush=True)
    pairs, distract = select(meta)
    resources = sorted({p["self"] for p in pairs} | {p["ref"] for p in pairs} | set(distract))
    print(f"{len(pairs)} pairs + {len(distract)} distractors = {len(resources)} resources", flush=True)

    take_lock()
    try:
        # 1. copy dumps FIRST — the jar's store step writes metadata back into whatever cache
        # folder it is given (even on cached reads), so it must NEVER be pointed at the prod
        # cache. It gets our writable dump copy instead.
        for p in resources:
            rid = meta[p][0]
            for suf in (f"{rid}.tdb", f"{rid}_meta_data.txt"):
                shutil.copy(f"{CACHE}/{suf}", f"{dump_dir}/{suf}")
        # 2. mini store from the copied prints (no extraction, no prod-cache mutation)
        jar(["store"] + COMMON + [f"PANAKO_LMDB_FOLDER={mini}",
             f"PANAKO_CACHE_FOLDER={dump_dir}", "PANAKO_USE_CACHED_PRINTS=TRUE"] + resources)
        # 3. slice query windows: pairs' selfs at 25/50/75% + one window per distractor
        specs = []  # (name, src, t0)
        for i, pr in enumerate(pairs):
            dur = meta[pr["self"]][1]
            for frac in (0.25, 0.50, 0.75):
                specs.append((f"pair{i}_{int(frac*100)}", pr["self"], round(dur * frac, 1)))
        for i, d in enumerate(distract):
            specs.append((f"lone{i}_50", d, round(meta[d][1] * 0.5, 1)))
        wav_of = {}
        for name, src, t0 in specs:
            w = f"{wavs}/{name}.wav"
            run(["nice", "-n", "15", "ffmpeg", "-nostdin", "-v", "error", "-ss", str(t0),
                 "-t", str(WIN), "-i", src, "-ac", "1", "-ar", "44100", w, "-y"])
            wav_of[w] = (name, src, t0)
        # 4. capture the jar's own prints for each window (STORAGE=MEM leaves LMDB untouched)
        jar(["store"] + COMMON + ["PANAKO_STORAGE=MEM", "PANAKO_CACHE_TO_FILE=TRUE",
             "PANAKO_USE_CACHED_PRINTS=FALSE", f"PANAKO_CACHE_FOLDER={qcache}"] + sorted(wav_of))
        # 5. golden answers: query mini store, reusing the exact cached prints from step 4
        stdout = jar(["query"] + COMMON + [f"PANAKO_LMDB_FOLDER={mini}",
                      f"PANAKO_CACHE_FOLDER={qcache}", "PANAKO_USE_CACHED_PRINTS=TRUE",
                      "AVAILABLE_PROCESSORS=4", "NUMBER_OF_QUERY_RESULTS=25"] + sorted(wav_of))
    finally:
        os.path.exists(LOCK) and os.remove(LOCK)

    # 6. parse rows per window (keep raw lines verbatim)
    rows_by_wav = {}
    for line in stdout.splitlines():
        parts = [x.strip() for x in line.strip().split(";")]
        if len(parts) < 12 or "query path" in line.lower():
            continue
        if parts[5] in ("", "null", "-1") or parts[6] in ("", "null", "-1"):
            continue
        rows_by_wav.setdefault(parts[2], []).append({
            "q_start": float(parts[3]), "q_stop": float(parts[4]),
            "ref_path": parts[5], "ref_id": parts[6],
            "ref_start": float(parts[7]), "ref_stop": float(parts[8]),
            "score": float(parts[9]),
            "time_factor": float(parts[10].replace("%", "").strip()),
            "pitch_factor": float(parts[11].replace("%", "").strip()),
            "raw": line.strip()})
    # 7. per-query fixture dirs: the probe prints + the golden rows
    qmeta_by_path = {}
    for mf in glob.glob(f"{qcache}/*_meta_data.txt"):
        ls = open(mf).read().splitlines()
        qmeta_by_path[ls[3]] = ls[0]
    n_rows = 0
    for w, (name, src, t0) in sorted(wav_of.items()):
        qid = qmeta_by_path.get(w) or sys.exit(f"FATAL: no cached prints for {w}")
        d = f"{qdir}/{name}"
        os.makedirs(d)
        shutil.copy(f"{qcache}/{qid}.tdb", f"{d}/prints.tdb")
        shutil.copy(w, f"{d}/window.wav")
        rows = rows_by_wav.get(w, [])
        n_rows += len(rows)
        json.dump({"query": name, "source_key": src, "window": [t0, t0 + WIN],
                   "rows": rows}, open(f"{d}/golden.json", "w"), indent=1)
    # 8. manifest
    jar_sha = hashlib.sha256(open(JAR, "rb").read()).hexdigest()
    manifest = {
        "generated": run(["date", "-u", "+%FT%TZ"]).strip(),
        "jar": {"path": JAR, "sha256": jar_sha, "upstream_commit": "e4b0e1d"},
        "config_snapshot": open(os.path.dirname(JAR) + "/config.properties").read(),
        "resources": [{"key": p, "resource_id": meta[p][0], "duration": meta[p][1],
                       "num_prints": meta[p][2]} for p in resources],
        "known_pairs": pairs,
        "note": "known_pairs spans come from full-corpus production runs; on the mini store "
                "expect at least those overlaps between pair members.",
    }
    json.dump(manifest, open(f"{OUT}/fixtures/manifest.json", "w"), indent=1)
    print(f"windows={len(wav_of)} golden_rows={n_rows}", flush=True)
    run(["tar", "-C", OUT, "--zstd", "-cf", f"{OUT}/fixtures.tar.zst", "fixtures"])
    print("DONE " + f"{OUT}/fixtures.tar.zst", flush=True)

main()
