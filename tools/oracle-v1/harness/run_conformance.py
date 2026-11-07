#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["pyyaml"]
# ///
"""Schema-conformance gate for the Plane v1 oracle (Schemathesis + exception register).

One command, one verdict. It runs Schemathesis against a base URL (the real Plane, or the
mutating proxy for controls), then classifies EVERY failure against oracle/exceptions.yaml:

  accepted    matches a register entry: a known, justified, versioned difference
  unaccepted  matches nothing: a NEW failure                                  -> gate FAILS
  error       Schemathesis itself errored on an operation (not a pass)        -> gate FAILS
  stale       a non-optional register entry that matched nothing this run     -> gate FAILS
              (a stale exception silently widens the hole; it must be removed on purpose)
  vacuous     fewer operations tested / reaching a 2xx than the register demands -> gate FAILS
              (a run that never reached the handlers proves nothing)

Exit status: 0 = PASS, 1 = gate failed, 2 = harness problem (Schemathesis crashed, bad register).

Usage:
  run_conformance.py --base-url http://127.0.0.1:18080 --label gate [--propose]
`--propose` prints a YAML skeleton of the unaccepted failure classes instead of failing, to seed
or extend the register; it never edits the register (a human reviews every entry).
"""

import argparse
import collections
import glob
import json
import os
import re
import subprocess
import sys
from pathlib import Path

import yaml

HERE = Path(__file__).resolve().parent
ORACLE = HERE.parent


def load_register(path):
    reg = yaml.safe_load(open(path))
    if reg.get("format_version") != 1:
        raise SystemExit(f"{path}: unsupported format_version {reg.get('format_version')!r}")
    for e in reg.get("entries") or []:
        for k in ("id", "kind", "match", "reason", "rust_expectation"):
            if k not in e:
                raise SystemExit(f"{path}: entry {e.get('id')!r} is missing required key {k!r}")
        if "check" not in e["match"]:
            raise SystemExit(f"{path}: entry {e['id']} match needs a `check`")
    return reg


def build_cmd(a, reg, token):
    g = reg["gate"]
    cmd = [
        "uvx", "--from", f"schemathesis=={g['schemathesis']}", "schemathesis", "run",
        str(a.schema), "--url", a.base_url,
        "-H", f"X-Api-Key: {token}",
        "--checks", ",".join(g["checks"]),
        "--phases", ",".join(g["phases"]),
        "--max-examples", str(g["max_examples"]),
        "--seed", str(g["seed"]),
        "--workers", str(g.get("workers", 4)),
        "--no-color",
        "--report", "ndjson", "--report-ndjson-path", str(a.out_dir / f"{a.label}.ndjson"),
    ]
    if g.get("deterministic"):
        # implies "no example database"; passing --generation-database as well is an error
        cmd.append("--generation-deterministic")
    else:
        cmd += ["--generation-database", "none"]
    for sx in reg.get("scope_exclusions") or []:
        if "method" in sx:
            cmd += ["--exclude-method", sx["method"]]
        if "path_regex" in sx:
            cmd += ["--exclude-path-regex", sx["path_regex"]]
    return cmd


def parse_report(ndjson):
    failures, seen, with2xx, cases = [], set(), set(), 0
    for line in open(ndjson):
        ev = json.loads(line)
        s = ev.get("ScenarioFinished")
        if not s:
            continue
        r = s["recorder"]
        label = r["label"]
        method, _, path = label.partition(" ")
        status_of = {}
        for cid, it in (r.get("interactions") or {}).items():
            resp = it.get("response") or {}
            st = resp.get("status_code")
            status_of[cid] = st
            seen.add(label)
            cases += 1
            if st is not None and 200 <= st < 300:
                with2xx.add(label)
        if s.get("status") == "error":
            failures.append({"label": label, "method": method, "path": path, "check": "runtime_error",
                             "type": "RuntimeError", "message": json.dumps(s.get("errors") or s)[:300],
                             "status": None, "kind": "error"})
        for cid, checks in (r.get("checks") or {}).items():
            for c in checks:
                if c["status"] != "failure":
                    continue
                f = c["failure_info"]["failure"]
                failures.append({"label": label, "method": method, "path": path, "check": c["name"],
                                 "type": f.get("type"), "message": f.get("message", ""),
                                 "status": status_of.get(cid), "kind": "failure"})
    return failures, len(seen), len(with2xx), cases


def matches(entry, f):
    m = entry["match"]
    if m["check"] != f["check"]:
        return False
    if "failure_type" in m and m["failure_type"] != f["type"]:
        return False
    if "methods" in m and f["method"] not in m["methods"]:
        return False
    if "operations" in m and f["label"] not in m["operations"]:
        return False
    if "operation_regex" in m and not re.search(m["operation_regex"], f["label"]):
        return False
    if "message_regex" in m and not re.search(m["message_regex"], f["message"], re.S):
        return False
    return True


def classify(failures, entries):
    accepted, unaccepted = [], []
    hits = collections.Counter()
    for f in failures:
        hit = [e for e in entries if e.get("status", "active") == "active" and matches(e, f)]
        if hit and f["kind"] == "failure":
            accepted.append({**f, "entry": hit[0]["id"]})
            for e in hit:
                hits[e["id"]] += 1
        else:
            unaccepted.append(f)
    return accepted, unaccepted, hits


def propose(unaccepted):
    groups = collections.OrderedDict()
    for f in unaccepted:
        first = f["message"].strip().split("\n")[0][:80]
        groups.setdefault((f["check"], f["type"], first), set()).add(f["label"])
    out = []
    for (check, ftype, first), ops in groups.items():
        out.append({
            "id": "EX-TODO", "kind": "TODO", "status": "active",
            "match": {"check": check, "failure_type": ftype, "message_regex": re.escape(first),
                      "operations": sorted(ops)},
            "reason": "TODO", "rust_expectation": "TODO",
        })
    return yaml.safe_dump({"entries": out}, sort_keys=False, width=100)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base-url", required=True)
    ap.add_argument("--label", default="gate")
    ap.add_argument("--schema", type=Path, default=ORACLE / "schema" / "openapi.v1.json")
    ap.add_argument("--manifest", type=Path, default=ORACLE / "corpus" / "seed-manifest.json")
    ap.add_argument("--register", type=Path, default=ORACLE / "exceptions.yaml")
    ap.add_argument("--out-dir", type=Path, default=ORACLE / ".work" / "conformance")
    ap.add_argument("--propose", action="store_true")
    ap.add_argument("--no-strict-register", action="store_true", help="do not fail on stale entries")
    a = ap.parse_args()
    # Absolute paths: Schemathesis runs from out_dir (below), so relative inputs would break.
    for attr in ("schema", "manifest", "register", "out_dir"):
        setattr(a, attr, getattr(a, attr).resolve())
    a.out_dir.mkdir(parents=True, exist_ok=True)

    reg = load_register(a.register)
    token = json.load(open(a.manifest))["tokens"]["owner"]
    cmd = build_cmd(a, reg, token)
    env = dict(os.environ, SCHEMATHESIS_HOOKS="st_hooks", ORACLE_MANIFEST=str(a.manifest),
               PYTHONPATH=str(HERE) + os.pathsep + os.environ.get("PYTHONPATH", ""))
    log = a.out_dir / f"{a.label}.log"
    nd = a.out_dir / f"{a.label}.ndjson"
    if nd.exists():
        nd.unlink()
    with open(log, "w") as lf:
        # cwd=out_dir: Schemathesis writes a `.schemathesis/` cache (with failing request data) into
        # its working directory. Keep that out of the repository root.
        rc = subprocess.run(cmd, env=env, stdout=lf, stderr=subprocess.STDOUT, cwd=a.out_dir).returncode
    if rc not in (0, 1) or not nd.exists():
        print(f"HARNESS PROBLEM: schemathesis exited {rc}; see {log}", file=sys.stderr)
        return 2

    failures, ops_tested, ops_2xx, cases = parse_report(nd)
    entries = reg.get("entries") or []
    accepted, unaccepted, hits = classify(failures, entries)

    if a.propose:
        print(propose(unaccepted))
        return 0

    stale = [e["id"] for e in entries
             if e.get("status", "active") == "active" and not e.get("optional") and hits[e["id"]] == 0]
    g = reg["gate"]
    vacuous = []
    if ops_tested < g["min_operations_tested"]:
        vacuous.append(f"operations tested {ops_tested} < {g['min_operations_tested']}")
    if ops_2xx < g["min_operations_with_2xx"]:
        vacuous.append(f"operations reaching a 2xx {ops_2xx} < {g['min_operations_with_2xx']}")

    result = {
        "label": a.label, "base_url": a.base_url, "schemathesis_exit": rc,
        "operations_tested": ops_tested, "operations_with_2xx": ops_2xx, "cases": cases,
        "failures_total": len(failures), "accepted": len(accepted),
        "unaccepted": unaccepted, "stale_entries": stale, "vacuous": vacuous,
        "accepted_by_entry": dict(hits),
    }
    json.dump(result, open(a.out_dir / f"{a.label}.result.json", "w"), indent=1)

    ok = not unaccepted and not vacuous and (a.no_strict_register or not stale)
    print(f"[{a.label}] ops tested={ops_tested} with-2xx={ops_2xx} cases={cases} "
          f"failures={len(failures)} accepted={len(accepted)} unaccepted={len(unaccepted)} "
          f"stale={len(stale)} vacuous={len(vacuous)}")
    for f in unaccepted[:15]:
        print(f"  UNACCEPTED {f['check']} {f['label']} :: {f['message'].strip().splitlines()[0][:100] if f['message'].strip() else f['type']}")
    for s in stale:
        print(f"  STALE register entry (matched nothing): {s}")
    for v in vacuous:
        print(f"  VACUOUS: {v}")
    print("VERDICT: " + ("PASS" if ok else "FAIL"))
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
