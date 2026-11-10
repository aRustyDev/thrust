#!/usr/bin/env -S uv run --script
# /// script
# requires-python = ">=3.11"
# dependencies = ["pyyaml"]
# ///
"""Must-fail controls for the conformance gate (spike 0008 step 4).

For each control, a mutating proxy is placed between the SAME gate (run_conformance.py, same
register, same seed) and an otherwise unmodified Plane, injecting one known defect:

  C0 passthrough     no mutation   gate must PASS   (the proxy itself is transparent)
  C1 type            State.name -> integer           gate must FAIL, on the right operation/field
  C2 value           State.group -> non-enum value   gate must FAIL, on the right operation/field
  C3 drop-required   State.name removed (required)   gate must FAIL, on the right operation/field
  C4 blind-spot      State.id removed (NOT required in the schema)
                                   gate is expected to PASS: schema conformance cannot see it.
                                   Recorded as a documented DETECTION GAP, not a success.

A control only counts if the proxy REALLY mutated a response (stats.mutated >= 1). A control that
mutated nothing and "failed to fail" is VOID and voids the whole harness.
"""

import argparse
import json
import socket
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
ORACLE = HERE.parent
TARGET_OP = "GET /api/v1/workspaces/{slug}/projects/{project_id}/states/"

CONTROLS = [
    {"id": "C0-passthrough", "mode": "passthrough", "field": "name", "expect": "PASS"},
    {"id": "C1-type", "mode": "type", "field": "name", "expect": "FAIL",
     "check": "response_schema_conformance", "message_regex": r"12345|is not of type"},
    {"id": "C2-value", "mode": "value", "field": "group", "expect": "FAIL",
     "check": "response_schema_conformance", "message_regex": r"not-a-valid-enum-member|is not one of|enum"},
    {"id": "C3-drop-required", "mode": "drop", "field": "name", "expect": "FAIL",
     "check": "response_schema_conformance", "message_regex": r'"name" is a required property'},
    {"id": "C4-blind-spot-drop-id", "mode": "drop", "field": "id", "expect": "BLIND-SPOT"},
]


def wait_port(port, timeout=15):
    t0 = time.time()
    while time.time() - t0 < timeout:
        with socket.socket() as s:
            s.settimeout(0.5)
            if s.connect_ex(("127.0.0.1", port)) == 0:
                return True
        time.sleep(0.2)
    return False


def reset_state(a, work, label):
    """Restore the database snapshot so this run starts from the SAME state as the baseline gate."""
    if not a.reset_script:
        return None
    r = subprocess.run([str(a.reset_script), "restore"], capture_output=True, text=True)
    (work / f"{label}.reset.log").write_text(r.stdout + r.stderr)
    return r.returncode


def run_control(c, port, a, work):
    import re
    rc = reset_state(a, work, c["id"])
    if rc not in (None, 0):
        return {"id": c["id"], "verdict": "VOID", "why": f"database reset failed (exit {rc}); see {c['id']}.reset.log"}
    stats_file = work / f"{c['id']}.proxy-stats.json"
    proxy = subprocess.Popen(
        ["uv", "run", str(HERE / "mutating_proxy.py"), "--listen-port", str(port), "--upstream", a.upstream,
         "--mode", c["mode"], "--field", c["field"], "--stats-file", str(stats_file)],
        stdout=open(work / f"{c['id']}.proxy.log", "w"), stderr=subprocess.STDOUT)
    try:
        if not wait_port(port):
            return {"id": c["id"], "verdict": "VOID", "why": "proxy did not start"}
        gate = subprocess.run(
            ["uv", "run", str(HERE / "run_conformance.py"), "--base-url", f"http://127.0.0.1:{port}",
             "--label", c["id"], "--schema", str(a.schema), "--manifest", str(a.manifest),
             "--out-dir", str(work), "--no-strict-register",
             # the same scope and target as the gate whose controls these are
             *(["--scope", a.scope, "--target", a.target] if a.scope else [])],
            capture_output=True, text=True)
    finally:
        proxy.terminate()
        proxy.wait(timeout=10)

    stats = json.load(open(stats_file)) if stats_file.exists() else {"mutated": 0}
    res_path = work / f"{c['id']}.result.json"
    if gate.returncode == 2 or not res_path.exists():
        return {"id": c["id"], "verdict": "VOID", "why": f"gate crashed (exit {gate.returncode})",
                "gate_output": gate.stdout[-600:] + gate.stderr[-600:]}
    res = json.load(open(res_path))
    gate_failed = gate.returncode != 0
    mutated = stats["mutated"]
    out = {"id": c["id"], "mode": c["mode"], "field": c["field"], "proxy_mutated_responses": mutated,
           "last_mutation": stats.get("last_mutation"), "gate_exit": gate.returncode,
           "unaccepted_failures": len(res["unaccepted"]), "expect": c["expect"]}

    if c["mode"] != "passthrough" and mutated < 1:
        return {**out, "verdict": "VOID", "why": "the proxy mutated NOTHING, so this control proves nothing"}

    if c["expect"] == "PASS":
        out["verdict"] = "OK" if not gate_failed else "BAD"
        out["why"] = "gate passed through the unmodified proxy" if not gate_failed else \
            "gate FAILED with no mutation: the proxy (or the gate) is not transparent"
    elif c["expect"] == "FAIL":
        hit = [f for f in res["unaccepted"]
               if f["check"] == c["check"] and f["label"] == TARGET_OP
               and re.search(c["message_regex"], f["message"], re.S)]
        out["failed_on_target"] = bool(hit)
        out["verdict"] = "OK" if gate_failed and hit else "BAD"
        out["why"] = ("gate FAILED on the mutated operation for the expected reason"
                      if out["verdict"] == "OK" else
                      "gate did not fail on the target operation/field: it cannot see this class of defect")
        if hit:
            out["evidence"] = hit[0]["message"].strip().splitlines()[0][:160]
    else:  # BLIND-SPOT
        out["verdict"] = "GAP-CONFIRMED" if not gate_failed else "NO-GAP"
        out["why"] = ("gate passed although a response lost `id`: schema conformance is blind to it "
                      "(State.required is only [color, name]); the golden corpus must catch this"
                      if not gate_failed else "gate unexpectedly failed; the blind spot did not reproduce")
    return out


def run_corpus_control(port, a, work):
    """C4 again, through the corpus comparison. The gate cannot see a State that has lost its `id` (the schema does not require it:
    C4 above), so the corpus comparison has to. The same proxy drops `id` from every State, the target's corpus is recorded THROUGH
    it, and that recording is compared with the committed one: the comparison must FAIL, on an `id`, in `states`. A comparison that
    passes here, or a proxy that mutated nothing, makes the control VOID."""
    import yaml
    scope = yaml.safe_load(open(ORACLE / "exceptions.yaml"))["scopes"][a.scope]
    resources = ",".join(scope["corpus"]["resources"])
    cid = "C4-corpus-comparison"
    rc = reset_state(a, work, cid)
    if rc not in (None, 0):
        return {"id": cid, "verdict": "VOID", "why": f"reset failed (exit {rc}); see {cid}.reset.log"}
    stats_file = work / f"{cid}.proxy-stats.json"
    recorded = work / f"{cid}.recorded"
    proxy = subprocess.Popen(
        ["uv", "run", str(HERE / "mutating_proxy.py"), "--listen-port", str(port), "--upstream", a.upstream,
         "--mode", "drop", "--field", "id", "--stats-file", str(stats_file)],
        stdout=open(work / f"{cid}.proxy.log", "w"), stderr=subprocess.STDOUT)
    try:
        if not wait_port(port):
            return {"id": cid, "verdict": "VOID", "why": "proxy did not start"}
        rec = subprocess.run(
            ["uv", "run", "--script", str(HERE / "record_corpus.py"), "--base-url", f"http://127.0.0.1:{port}",
             "--manifest", str(a.manifest), "--only", resources, "--out-dir", str(recorded),
             "--raw-dir", str(work / f"{cid}.raw")], capture_output=True, text=True)
    finally:
        proxy.terminate()
        proxy.wait(timeout=10)
    stats = json.load(open(stats_file)) if stats_file.exists() else {"mutated": 0}
    if rec.returncode != 0:
        return {"id": cid, "verdict": "VOID", "why": f"the recording failed (exit {rec.returncode})",
                "output": rec.stdout[-400:] + rec.stderr[-400:]}
    if stats["mutated"] < 1:
        return {"id": cid, "verdict": "VOID", "proxy_mutated_responses": 0,
                "why": "the proxy mutated NOTHING, so this control proves nothing"}
    result_path = work / f"{cid}.result.json"
    cmp = subprocess.run(
        ["uv", "run", "--script", str(HERE / "compare_corpus.py"), "--recorded", str(recorded), "--scope", a.scope,
         "--target", a.target, "--out", str(result_path)], capture_output=True, text=True)
    if not result_path.exists():
        return {"id": cid, "verdict": "VOID", "why": f"the comparison crashed (exit {cmp.returncode})",
                "output": cmp.stdout[-400:] + cmp.stderr[-400:]}
    found = [d for d in json.load(open(result_path))["unaccepted"]
             if d["resource"] == "states" and d["path"].endswith(".id")]
    out = {"id": cid, "mode": "drop", "field": "id", "proxy_mutated_responses": stats["mutated"],
           "comparison_exit": cmp.returncode, "differences_on_states_id": len(found), "expect": "FAIL"}
    out["verdict"] = "OK" if cmp.returncode != 0 and found else "BAD"
    out["why"] = ("the corpus comparison FAILED on the dropped `id` of a State, which the gate cannot see"
                  if out["verdict"] == "OK" else
                  "the corpus comparison did not fail on a dropped `id`: nothing sees this class of defect")
    if found:
        out["evidence"] = f"{found[0]['exchange']} {found[0]['path']}: committed={found[0]['committed']!r} recorded={found[0]['recorded']!r}"[:200]
    return out


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--upstream", required=True)
    ap.add_argument("--schema", type=Path, default=ORACLE / "schema" / "openapi.v1.json")
    ap.add_argument("--manifest", type=Path, default=ORACLE / "corpus" / "seed-manifest.json")
    ap.add_argument("--out", type=Path, default=ORACLE / "results" / "controls.json")
    ap.add_argument("--scope", help="a scope of the register (`scopes.<name>`): run the controls' gate over just that part of the schema")
    ap.add_argument("--target", choices=("plane", "thrust"), default="plane",
                    help="what --upstream is; selects the register entries that apply to it (see run_conformance.py)")
    ap.add_argument("--base-port", type=int, default=18090)
    ap.add_argument("--reset-script", type=Path,
                    help="script run as `<script> restore` before EVERY control; without it, runs share a "
                         "database the previous run already fuzzed and the comparison is not valid")
    a = ap.parse_args()
    work = ORACLE / ".work" / "controls"
    work.mkdir(parents=True, exist_ok=True)
    a.out.parent.mkdir(parents=True, exist_ok=True)

    results = []
    for i, c in enumerate(CONTROLS):
        r = run_control(c, a.base_port + i, a, work)
        results.append(r)
        print(f"{r['id']:24} verdict={r['verdict']:14} mutated={r.get('proxy_mutated_responses', '-')} "
              f"gate_exit={r.get('gate_exit', '-')} :: {r.get('why', '')}")
        if r.get("evidence"):
            print(f"{'':24} evidence: {r['evidence']}")

    if a.scope:  # what the gate cannot see, the corpus comparison must: only a scope has a corpus comparison
        r = run_corpus_control(a.base_port + len(CONTROLS), a, work)
        results.append(r)
        print(f"{r['id']:24} verdict={r['verdict']:14} mutated={r.get('proxy_mutated_responses', '-')} "
              f"comparison_exit={r.get('comparison_exit', '-')} :: {r.get('why', '')}")
        if r.get("evidence"):
            print(f"{'':24} evidence: {r['evidence']}")

    must = [r for r in results if r["id"] != "C4-blind-spot-drop-id"]
    harness_ok = all(r["verdict"] == "OK" for r in must)
    doc = {
        "scope": a.scope,
        "target": a.target,
        "harness_valid": harness_ok,
        "controls": results,
        "summary": {
            "baseline_passes_through_proxy": next(r["verdict"] == "OK" for r in results if r["id"] == "C0-passthrough"),
            "mutation_controls_failed_as_required": [r["id"] for r in must[1:] if r["verdict"] == "OK"],
            "void_or_bad": [r["id"] for r in must if r["verdict"] != "OK"],
            "documented_detection_gap": [r for r in results if r["id"] == "C4-blind-spot-drop-id"][0]["verdict"],
        },
    }
    json.dump(doc, open(a.out, "w"), indent=1, sort_keys=True)
    print("HARNESS " + (f"VALID: passes on the {a.target}, fails on every must-fail control" if harness_ok
                        else "VOID OR BROKEN: a control did not behave; do not trust a green gate"))
    return 0 if harness_ok else 1


if __name__ == "__main__":
    sys.exit(main())
