"""Spike 0004 step 4: latency of Worker A -> Worker B over a service binding vs the same logic in-process.

Method (and why it is built this way)
  * Four request kinds against /api/lat/<kind> (a BYPASS path, so Access adds nothing to the timing):
      null     edge Worker returns a constant                      -> floor (network + edge)
      inproc   edge Worker runs compute() itself                   -> floor + compute
      binding  edge Worker calls B over a service binding (fetch)  -> floor + compute + hop
      rpc      edge Worker calls B over a service binding (RPC)    -> floor + compute + hop
  * The kinds are INTERLEAVED per iteration in a ROTATING order, so slow drift in network/edge conditions and
    position effects hit every kind equally. One keep-alive connection, so TLS/TCP setup is not in the numbers.
  * Per-kind p50/p95/p99 over N samples (default 1000, warmup discarded), and the PAIRED difference
    binding-inproc / rpc-inproc per iteration: its median with a bootstrap 95% CI. If the CI contains 0 the hop
    is below this harness's resolution (the docs claim zero overhead: same thread, same machine).
  * Date.now() in a Worker only advances on I/O, so a Worker-side stopwatch cannot time pure CPU. The in-Worker
    `loop` probe is therefore reported only for the binding kinds and is indicative (ms resolution / m calls).
  * Local (wrangler dev) numbers prove the harness and show the hop cost with no network; they say NOTHING about
    production. Run it against the real hostname for the spike's actual answer.
"""

from __future__ import annotations

import http.client
import json
import random
import ssl
import statistics
import time

from rawhttp import Target

KINDS = ["null", "inproc", "binding", "rpc"]


def pct(sorted_vals: list[float], p: float) -> float:
    if not sorted_vals:
        return float("nan")
    k = (len(sorted_vals) - 1) * p / 100
    f, c = int(k), min(int(k) + 1, len(sorted_vals) - 1)
    return sorted_vals[f] + (sorted_vals[c] - sorted_vals[f]) * (k - f)


def _conn(t: Target):
    if t.tls:
        return http.client.HTTPSConnection(t.host, t.port, context=ssl.create_default_context(), timeout=20)
    return http.client.HTTPConnection(t.host, t.port, timeout=20)


def run(t: Target, n: int = 1000, warmup: int = 100, work: int = 20000, seed: int = 4) -> dict:
    rnd = random.Random(seed)
    conn = _conn(t)
    hdr = {"Host": t.host_header, "User-Agent": "spike-0004-latency", "Accept": "application/json"}
    samples: dict[str, list[float]] = {k: [] for k in KINDS}
    errors = {k: 0 for k in KINDS}
    reconnects = 0

    def one(kind: str) -> float | None:
        nonlocal conn, reconnects
        for attempt in (0, 1):
            try:
                t0 = time.perf_counter_ns()
                conn.request("GET", f"/api/lat/{kind}?n={work}", headers=hdr)
                resp = conn.getresponse()
                body = resp.read()
                dt = (time.perf_counter_ns() - t0) / 1e6
                if resp.status != 200:
                    errors[kind] += 1
                    return None
                j = json.loads(body)
                want = {"null": "null", "inproc": "inproc", "binding": "fetch", "rpc": "rpc"}[kind]
                if j.get("via") != want:
                    errors[kind] += 1
                    return None
                return dt
            except (OSError, http.client.HTTPException, ValueError):
                reconnects += 1
                conn.close()
                conn = _conn(t)
        errors[kind] += 1
        return None

    deltas = {"binding": [], "rpc": []}
    for i in range(warmup + n):
        order = KINDS[i % len(KINDS):] + KINDS[:i % len(KINDS)]
        if i % 2:
            order = order[::-1]
        got: dict[str, float] = {}
        for k in order:
            v = one(k)
            if v is not None:
                got[k] = v
        if i < warmup:
            continue
        for k, v in got.items():
            samples[k].append(v)
        if "inproc" in got:
            for k in ("binding", "rpc"):
                if k in got:
                    deltas[k].append(got[k] - got["inproc"])

    out: dict = {"n": n, "warmup": warmup, "work_iterations": work, "target": f"{t.scheme}://{t.host_header}",
                 "errors": errors, "reconnects": reconnects, "kinds": {}, "paired_delta_ms": {}}
    for k, vals in samples.items():
        s = sorted(vals)
        out["kinds"][k] = {"count": len(s), "p50": pct(s, 50), "p95": pct(s, 95), "p99": pct(s, 99),
                           "mean": statistics.fmean(s) if s else float("nan"), "min": s[0] if s else None, "max": s[-1] if s else None}
    for k, ds in deltas.items():
        if not ds:
            continue
        boots = []
        for _ in range(2000):
            boots.append(statistics.median(rnd.choices(ds, k=len(ds))))
        boots.sort()
        out["paired_delta_ms"][f"{k}-inproc"] = {
            "median": statistics.median(ds), "ci95": [pct(boots, 2.5), pct(boots, 97.5)], "pairs": len(ds),
            "below_resolution": pct(boots, 2.5) <= 0 <= pct(boots, 97.5),
        }

    # in-Worker loops (indicative)
    loops = {}
    for mode in ("binding", "rpc"):
        per = []
        for _ in range(5):
            try:
                conn.request("GET", f"/api/lat/loop?mode={mode}&m=500&n={work}", headers=hdr)
                r = conn.getresponse()
                j = json.loads(r.read())
                per.append(j["per_call_ms"])
            except (OSError, http.client.HTTPException, ValueError, KeyError):
                conn.close()
                conn = _conn(t)
        loops[mode] = {"per_call_ms_runs": per, "median_per_call_ms": statistics.median(per) if per else None}
    out["in_worker_loop_indicative"] = loops
    conn.close()
    return out


def render(res: dict) -> str:
    lines = [f"latency target={res['target']} N={res['n']} warmup={res['warmup']} work={res['work_iterations']} "
             f"errors={res['errors']} reconnects={res['reconnects']}",
             f"{'kind':<9}{'count':>7}{'p50 ms':>10}{'p95 ms':>10}{'p99 ms':>10}{'mean':>10}"]
    for k, v in res["kinds"].items():
        lines.append(f"{k:<9}{v['count']:>7}{v['p50']:>10.3f}{v['p95']:>10.3f}{v['p99']:>10.3f}{v['mean']:>10.3f}")
    for k, d in res["paired_delta_ms"].items():
        verdict = "CI contains 0: hop is below this harness's resolution" if d["below_resolution"] else "hop overhead is measurable"
        lines.append(f"paired {k}: median {d['median']:+.3f} ms, 95% CI [{d['ci95'][0]:+.3f}, {d['ci95'][1]:+.3f}] over {d['pairs']} pairs -> {verdict}")
    for mode, d in res["in_worker_loop_indicative"].items():
        lines.append(f"in-Worker loop ({mode}, indicative): median {d['median_per_call_ms']} ms/call over runs {d['per_call_ms_runs']}")
    return "\n".join(lines)
