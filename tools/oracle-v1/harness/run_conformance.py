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
  run_conformance.py --base-url http://127.0.0.1:18080 --label gate [--propose] [--scope m1]
`--propose` prints a YAML skeleton of the unaccepted failure classes instead of failing, to seed
or extend the register; it never edits the register (a human reviews every entry).

`--scope NAME` (thrust issue #58) runs only the operations of a scope named in the register's `scopes:`
section (its schema tags), because the Rust slice serves a part of the schema and a whole-schema run
would fail on every operation it does not have. A scoped run differs from the whole one in four ways,
each a way the whole one is too loose for a part:

  * the register is restricted to the scope: an entry with no operation in it is dropped, and one with
    some keeps only those (so an entry is not "stale" for an operation that is not being run);
  * staleness is counted per (entry, operation), not per entry: reproducing a quirk on one operation of
    an entry must not hide its absence on the others;
  * every operation the scope should run (its operations less `scope_exclusions`) must have been run:
    one that was not is VACUOUS, not merely counted in a floor;
  * the floors are the scope's own (`min_operations_tested`, `min_operations_with_2xx`), measured on Plane.

Only entries with `applies_to: conformance` (or none) count here; `corpus` entries belong to the corpus
comparison (`compare_corpus.py`).
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
        # a `check` is a conformance entry's; a `corpus` entry (compare_corpus.py) names a resource instead
        if e.get("applies_to", "conformance") == "conformance" and "check" not in e["match"]:
            raise SystemExit(f"{path}: entry {e['id']} match needs a `check`")
    return reg


def load_scope(reg, name):
    """The scope `name` of the register, or exit 2: a scoped run without its definition proves nothing."""
    scope = (reg.get("scopes") or {}).get(name)
    if not scope:
        raise SystemExit(f"register has no scope {name!r} (it has {sorted(reg.get('scopes') or {})})")
    for k in ("tags", "min_operations_tested", "min_operations_with_2xx"):
        if k not in scope:
            raise SystemExit(f"scope {name!r} is missing required key {k!r}")
    return scope


def scope_operations(schema_path, tags):
    """The operation labels (`METHOD /path`, as the Schemathesis report writes them) that carry one of `tags`."""
    doc = json.load(open(schema_path))
    wanted = set(tags)
    ops = set()
    for path, item in doc["paths"].items():
        for method, op in item.items():
            if isinstance(op, dict) and wanted & set(op.get("tags") or []):
                ops.add(f"{method.upper()} {path}")
    return ops


def excluded_by_scope_exclusions(ops, exclusions):
    """The operations of `ops` that the register's `scope_exclusions` keep the gate from running."""
    out = set()
    for label in ops:
        method, _, path = label.partition(" ")
        for sx in exclusions or []:
            by_method = "method" in sx and sx["method"] == method
            by_path = "path_regex" in sx and re.search(sx["path_regex"], path)
            if by_method or by_path:
                out.add(label)
    return out


def for_target(entries, target):
    """The entries of a run against `target` (`plane` or `thrust`): one that names `targets` applies only to those. An entry for
    `thrust` alone is a recorded DIFFERENCE of Thrust from the pin (an intentional deviation), so a run on the pin does not hold it
    (it would be stale there, and must not make the pin's own gate fail)."""
    return [e for e in entries if target in (e.get("targets") or [target])]


def restrict_entries(entries, scope_ops, scope=None, known_ids=None):
    """Entries restricted to a scope: those with an operation in it, each keeping only its operations in it.

    A scope that states `entry_operations` (where each entry fires in a run of that scope on Plane) replaces the entries' own
    operation lists with them: the fuzzer's cases depend on which operations a run holds, so a list measured on the whole schema
    is not valid for a part of it. An entry the scope does not list is not in it. A value is the list of operations, or a mapping with
    `operations` and `absent_on_thrust: <why>` for an entry that fires on the pin only (Thrust differs on purpose): a run on Thrust
    does not require it to fire.
    """
    given = (scope or {}).get("entry_operations")
    if given:
        by_id = {e["id"]: e for e in entries}
        out = []
        for eid, spec in given.items():
            ops = spec["operations"] if isinstance(spec, dict) else spec
            if eid not in by_id:
                if eid in (known_ids or ()):
                    continue  # an entry of the register that this target does not hold (`for_target`)
                raise SystemExit(f"scope entry_operations names {eid}, which is not a conformance entry of the register")
            outside = [o for o in ops if o not in scope_ops]
            if outside:
                raise SystemExit(f"scope entry_operations for {eid} names operations outside the scope: {outside}")
            e = by_id[eid]
            absent = isinstance(spec, dict) and spec.get("absent_on_thrust")
            out.append({**e, "match": {**e["match"], "operations": list(ops)}, **({"absent_on_thrust": absent} if absent else {})})
        return out
    out = []
    for e in entries:
        m = e["match"]
        if m.get("operations"):
            inside = [o for o in m["operations"] if o in scope_ops]
            if not inside:
                continue
            out.append({**e, "match": {**m, "operations": inside}})
        elif m.get("operation_regex"):
            if any(re.search(m["operation_regex"], o) for o in scope_ops):
                out.append(e)
        else:
            out.append(e)
    return out


def build_cmd(a, reg, token, scope=None):
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
    if scope:
        # one regex, not one flag per tag: Schemathesis ANDs repeated --include-tag, which would select nothing
        tags = "|".join(re.escape(t) for t in scope["tags"])
        cmd += ["--include-tag-regex", f"^({tags})$"]
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
    return failures, seen, with2xx, cases


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
    """Accepted and unaccepted failures, and how often each entry fired (`hits`), overall and per operation (`hits_by_op`)."""
    accepted, unaccepted = [], []
    hits = collections.Counter()
    hits_by_op = collections.Counter()
    for f in failures:
        hit = [e for e in entries if e.get("status", "active") == "active" and matches(e, f)]
        if hit and f["kind"] == "failure":
            accepted.append({**f, "entry": hit[0]["id"]})
            for e in hit:
                hits[e["id"]] += 1
                hits_by_op[(e["id"], f["label"])] += 1
        else:
            unaccepted.append(f)
    return accepted, unaccepted, hits, hits_by_op


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
    ap.add_argument("--scope", help="run only the operations of this scope of the register (`scopes:`), strictly; see the module doc")
    ap.add_argument("--target", choices=("plane", "thrust"), default="plane",
                    help="what is being run against: entries for `thrust` only (recorded deviations) are held on thrust and dropped on plane")
    a = ap.parse_args()
    # Absolute paths: Schemathesis runs from out_dir (below), so relative inputs would break.
    for attr in ("schema", "manifest", "register", "out_dir"):
        setattr(a, attr, getattr(a, attr).resolve())
    a.out_dir.mkdir(parents=True, exist_ok=True)

    reg = load_register(a.register)
    scope = load_scope(reg, a.scope) if a.scope else None
    scope_ops = scope_operations(a.schema, scope["tags"]) if scope else None
    # what a scoped run must have run: its operations, less the ones `scope_exclusions` keep the gate from
    expected_ops = (scope_ops - excluded_by_scope_exclusions(scope_ops, reg.get("scope_exclusions"))) if scope else set()
    if scope and not expected_ops:
        raise SystemExit(f"scope {a.scope!r} selects no operation of {a.schema}: its tags match nothing")
    token = json.load(open(a.manifest))["tokens"]["owner"]
    cmd = build_cmd(a, reg, token, scope)
    env = dict(os.environ, SCHEMATHESIS_HOOKS="st_hooks", ORACLE_MANIFEST=str(a.manifest),
               PYTHONPATH=str(HERE) + os.pathsep + os.environ.get("PYTHONPATH", ""))
    if scope and scope.get("json_bodies_only"):
        env["ORACLE_JSON_BODIES_ONLY"] = "1"
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

    failures, seen_ops, ops_2xx_set, cases = parse_report(nd)
    ops_tested, ops_2xx = len(seen_ops), len(ops_2xx_set)
    # only entries that are about the conformance gate; `corpus` entries belong to compare_corpus.py
    entries = [e for e in (reg.get("entries") or []) if e.get("applies_to", "conformance") == "conformance"]
    known_ids = {e["id"] for e in entries}
    entries = for_target(entries, a.target)
    if scope:
        entries = restrict_entries(entries, scope_ops, scope, known_ids)
    accepted, unaccepted, hits, hits_by_op = classify(failures, entries)

    if a.propose:
        print(propose(unaccepted))
        return 0

    # an entry a scope lists in `entry_operations` must fire there, `optional` or not: `optional` is for a whole-schema run, where
    # whether a quirk is hit depends on the fuzzer's luck, and a scope has measured where it is hit
    strict_all = bool(scope and scope.get("entry_operations"))
    active = [e for e in entries if e.get("status", "active") == "active" and (strict_all or not e.get("optional"))
              # a quirk of the pin that Thrust does not have, on purpose, is not required to fire on Thrust
              and not (a.target == "thrust" and e.get("absent_on_thrust"))]
    stale_pairs = []
    if scope:
        # per (entry, operation): an entry that fires on one of its operations says nothing about the others
        for e in active:
            ops = e["match"].get("operations")
            if ops:
                stale_pairs += [f"{e['id']} {o}" for o in ops if hits_by_op[(e["id"], o)] == 0]
            elif hits[e["id"]] == 0:
                stale_pairs.append(e["id"])
        stale = stale_pairs
    else:
        stale = [e["id"] for e in active if hits[e["id"]] == 0]
    g = {**reg["gate"], **({k: scope[k] for k in ("min_operations_tested", "min_operations_with_2xx")} if scope else {})}
    vacuous = []
    if ops_tested < g["min_operations_tested"]:
        vacuous.append(f"operations tested {ops_tested} < {g['min_operations_tested']}")
    if ops_2xx < g["min_operations_with_2xx"]:
        vacuous.append(f"operations reaching a 2xx {ops_2xx} < {g['min_operations_with_2xx']}")
    untested = sorted(expected_ops - seen_ops)
    for label in untested:
        vacuous.append(f"operation not run: {label}")

    result = {
        "label": a.label, "base_url": a.base_url, "schemathesis_exit": rc, "scope": a.scope, "target": a.target,
        "operations_tested": ops_tested, "operations_with_2xx": ops_2xx, "cases": cases,
        "failures_total": len(failures), "accepted": len(accepted),
        "unaccepted": unaccepted, "stale_entries": stale, "vacuous": vacuous,
        "accepted_by_entry": dict(hits),
        "accepted_by_entry_and_operation": {f"{k[0]} {k[1]}": v for k, v in sorted(hits_by_op.items())},
        "operations_expected": sorted(expected_ops), "operations_not_run": untested,
        "operations_with_2xx_list": sorted(ops_2xx_set),
    }
    json.dump(result, open(a.out_dir / f"{a.label}.result.json", "w"), indent=1)

    ok = not unaccepted and not vacuous and (a.no_strict_register or not stale)
    print(f"[{a.label}] ops tested={ops_tested} with-2xx={ops_2xx} cases={cases} "
          f"failures={len(failures)} accepted={len(accepted)} unaccepted={len(unaccepted)} "
          f"stale={len(stale)} vacuous={len(vacuous)}" + (f" scope={a.scope} expected={len(expected_ops)}" if scope else ""))
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
