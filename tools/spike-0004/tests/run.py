# /// script
# requires-python = ">=3.11"
# dependencies = []
# ///
"""Spike 0004 harness entry point. Standard library only; run with `uv run run.py ...`.

  uv run run.py checks   --base https://spike-0004.<zone> [--expect-gate yes] [--report out.json]
                         [--ws-soak 120] [--workers-dev-host spike-0004-edge.<sub>.workers.dev] [--quick]
  uv run run.py latency  --base https://spike-0004.<zone> [-n 1000] [--report out.json]
  uv run run.py selftest --upstream http://127.0.0.1:8787     # needs ./dev.sh running; touches no Cloudflare

Credentials (service token for the gated paths) are read ONLY from the environment, never argv:
  SPIKE_ACCESS_CLIENT_ID, SPIKE_ACCESS_CLIENT_SECRET

Exit codes: 0 ok · 1 any FAIL · 3 any VOID (a control that must fail passed) · 4 only WEAK results.
"""

from __future__ import annotations

import argparse
import json
import os
import sys

import checks
import latency
from mockgate import MOCK_CLIENT_ID, MOCK_CLIENT_SECRET, MOCK_HOST, MOCK_SPACE_HOST, MockGate
from rawhttp import Target

ORDER = {"FAIL": 0, "VOID": 1, "WEAK": 2, "PASS": 3, "INFO": 4, "SKIP": 5}


def render(results: list[checks.Result]) -> str:
    out = []
    for r in results:
        tag = f"[{r.status}]"
        plat = " (platform-only)" if r.platform_only else ""
        out.append(f"{tag:<7}{r.id:<5}{r.claim}{plat}")
        if r.detail:
            out.append(f"         {r.detail}")
    counts: dict[str, int] = {}
    for r in results:
        counts[r.status] = counts.get(r.status, 0) + 1
    out.append("summary: " + "  ".join(f"{k}={counts[k]}" for k in sorted(counts, key=lambda k: ORDER[k])))
    return "\n".join(out)


def exit_code(results: list[checks.Result]) -> int:
    st = {r.status for r in results}
    if "FAIL" in st:
        return 1
    if "VOID" in st:
        return 3
    if "WEAK" in st:
        return 4
    return 0


def do_checks(a: argparse.Namespace) -> int:
    t = Target.parse(a.base, a.host_header)
    cid, csec = os.environ.get("SPIKE_ACCESS_CLIENT_ID"), os.environ.get("SPIKE_ACCESS_CLIENT_SECRET")
    space_t = Target.parse(f"{t.scheme}://{a.space_host}") if a.space_host else None
    ctx = checks.Ctx(t, a.expect_gate == "yes", cid, csec, a.ws_soak, a.workers_dev_host, a.quick, a.overlap, space_t)
    res = checks.run_all(ctx)
    print(render(res))
    if a.report:
        json.dump([r.__dict__ for r in res], open(a.report, "w"), indent=1, default=str)
    return exit_code(res)


def do_latency(a: argparse.Namespace) -> int:
    t = Target.parse(a.base, a.host_header)
    res = latency.run(t, n=a.n, warmup=a.warmup, work=a.work)
    print(latency.render(res))
    if a.report:
        json.dump(res, open(a.report, "w"), indent=1)
    return 0 if all(v == 0 for v in res["errors"].values()) else 1


# ------------------------------------------------------------------------------------------------------
# selftest: prove the harness discriminates (known-good and known-bad targets)
# ------------------------------------------------------------------------------------------------------
def selftest(a: argparse.Namespace) -> int:
    up = Target.parse(a.upstream)
    probe = checks.request(up, "/api/ping")
    if probe.status != 200:
        print(f"upstream {a.upstream} not serving the spike Worker (got {probe.status} {probe.error}); start ./dev.sh first")
        return 2

    space_up = Target.parse(a.space_upstream)
    sp = checks.request(space_up, "/")
    have_space = sp.status == 200 and "spike-0004-space" in sp.text
    if not have_space:
        print(f"note: space Worker not reachable at {a.space_upstream} (start `ROLE=space ./dev.sh`); SP1/SP2 scenarios will be skipped")

    OK = {"PASS", "INFO", "SKIP"}
    scenarios = [
        # name, mode, expect_gate, expectations {id: allowed statuses}; "*" applies to every id not listed
        ("local-direct, ungated wrangler dev, --expect-gate no (functional run)", None, False, {"*": OK}),
        ("local-direct, ungated, but --expect-gate yes: gate controls MUST void the run", None, True,
         {"G0": {"VOID"}, "B1": {"VOID"}, "W1": {"VOID"}, "T1": {"VOID"}, "H1": {"VOID"}, "M1": {"VOID"}}),
        ("mock gate: GOOD model, consistent parsing", "good", True, {"*": OK}),
        ("mock gate: LEAKY (raw-path match, normalised forward) -> traversal must FAIL", "leaky", True, {"T1": {"FAIL"}}),
        ("mock gate: DOTLEAK (trailing-dot Host matches no app) -> host probe must FAIL", "dotleak", True, {"H1": {"FAIL"}}),
        ("mock gate: SHALLOW (`/x/*` covers one segment) -> deep bypass paths must FAIL", "shallow", True,
         {"B3": {"FAIL"}, "B7": {"FAIL"}, "T1": OK | {"WEAK"}}),
        ("mock gate: OPEN (gates nothing) -> gate-liveness control must VOID", "open", True,
         {"G0": {"VOID"}, "T1": {"VOID"}, "H1": {"VOID"}, "B1": {"VOID"}, "O1": {"VOID"}, **({"SP1": {"VOID"}, "SP2": {"VOID"}} if have_space else {})}),
    ]
    if have_space:
        scenarios.append(("mock gate: SPACELEAK (public space hostname wired to the PRODUCT Worker) -> SP1 must FAIL",
                          "spaceleak", True, {"SP1": {"FAIL"}}))
    failures = 0
    for name, mode, expect_gate, expect in scenarios:
        gate = None
        space_t = None
        if mode is None:
            t = up
            cid = csec = None
        else:
            gate = MockGate(mode=mode, upstream=(up.host, up.port), space_upstream=(space_up.host, space_up.port) if have_space else None)
            port = gate.start()
            t = Target.parse(f"http://127.0.0.1:{port}", MOCK_HOST)
            space_t = Target.parse(f"http://127.0.0.1:{port}", MOCK_SPACE_HOST) if have_space else None
            cid, csec = MOCK_CLIENT_ID, MOCK_CLIENT_SECRET
        ctx = checks.Ctx(t, expect_gate, cid, csec, 0, None, quick=False, overlap=mode is not None, space_target=space_t)
        res = checks.run_all(ctx)
        if gate:
            gate.stop()
        by_id = {r.id: r for r in res}
        bad = []
        for rid, r in by_id.items():
            allowed = expect.get(rid, expect.get("*"))
            if allowed is not None and r.status not in allowed:
                bad.append(f"{rid}={r.status} (expected {sorted(allowed)})")
        for rid in expect:
            if rid != "*" and rid not in by_id:
                bad.append(f"{rid} missing")
        verdict = "OK " if not bad else "BAD"
        counts: dict[str, int] = {}
        for r in res:
            counts[r.status] = counts.get(r.status, 0) + 1
        print(f"[{verdict}] {name}\n        actual: " + " ".join(f"{k}={v}" for k, v in sorted(counts.items(), key=lambda kv: ORDER[kv[0]])))
        if a.verbose or bad:
            for r in res:
                if a.verbose or r.status in ("FAIL", "VOID", "WEAK"):
                    print(f"        {r.status:<5}{r.id:<4}{r.detail[:230]}")
        for b in bad:
            print(f"        EXPECTATION MISSED: {b}")
        failures += bool(bad)
    print("\nselftest: " + ("ALL SCENARIOS BEHAVED AS REQUIRED" if not failures else f"{failures} scenario(s) did NOT behave as required"))
    print("(this validates the harness, not Cloudflare: the mock is a model of the documented rules)")
    return 1 if failures else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = ap.add_subparsers(dest="cmd", required=True)

    c = sub.add_parser("checks")
    c.add_argument("--base", required=True)
    c.add_argument("--host-header")
    c.add_argument("--expect-gate", choices=["yes", "no"], default="yes")
    c.add_argument("--report")
    c.add_argument("--ws-soak", type=int, default=0)
    c.add_argument("--workers-dev-host")
    c.add_argument("--quick", action="store_true")
    c.add_argument("--overlap", action="store_true", help="also run O1 (needs tofu enable_overlap_experiment=true)")
    c.add_argument("--space-host", help="public space hostname (spike step 6), e.g. spike-0004-space.<zone>")
    c.set_defaults(fn=do_checks)

    l = sub.add_parser("latency")
    l.add_argument("--base", required=True)
    l.add_argument("--host-header")
    l.add_argument("-n", type=int, default=1000)
    l.add_argument("--warmup", type=int, default=100)
    l.add_argument("--work", type=int, default=20000)
    l.add_argument("--report")
    l.set_defaults(fn=do_latency)

    s = sub.add_parser("selftest")
    s.add_argument("--upstream", default="http://127.0.0.1:8787")
    s.add_argument("--space-upstream", default="http://127.0.0.1:8788")
    s.add_argument("-v", "--verbose", action="store_true")
    s.set_defaults(fn=selftest)

    a = ap.parse_args()
    return a.fn(a)


if __name__ == "__main__":
    sys.exit(main())
