"""Summarise scripts/measure-argon2.sh: the platform's CPU time per sign-in (from the tail consumer) and the client's
wall time, per OWASP configuration. Run with `uv run --no-project scripts/measure-argon2-summary.py <outdir>`.
"""
import json
import pathlib
import re
import sys
from collections import Counter, defaultdict

out = pathlib.Path(sys.argv[1])


def pct(xs, p):
    """Nearest-rank percentile of a non-empty list."""
    s = sorted(xs)
    return s[max(1, -(-len(s) * p // 100)) - 1] if p < 100 else s[-1]


# the platform's records: the tail consumer stored one JSON value per invocation under its `?run=` label
kv = json.loads((out / "kv.json").read_text() or "{}")
by_run = {}
platform = []
for key, value in kv.items():
    m = re.fullmatch(r"(m\d+t\d+)-(burst)?(\d+)", key)
    if not m:
        continue
    rec = json.loads(value)
    platform.append(rec)
    by_run[(m.group(1), bool(m.group(2)), int(m.group(3)))] = rec

client = defaultdict(list)
for line in (out / "client.tsv").read_text().splitlines():
    name, status, secs = line.split("\t")
    m = re.match(r"(m\d+t\d+)-(burst)?(\d+)$", name)
    if not m:
        continue  # a seed request
    client[(m.group(1), bool(m.group(2)))].append((int(m.group(3)), status, float(secs) * 1000))

print(f"platform records: {len(platform)}; sign-in invocations matched: {len(by_run)}; client requests: {sum(len(v) for v in client.values())}")
print()
print(f"{'config':<10} {'n':>4} {'200':>4} {'cpu p50':>8} {'cpu p99':>8} {'cpu max':>8} {'first':>7} {'p50 200':>8} {'p50 401':>8} {'wall p50':>9} {'client p50':>11} outcomes")
labels = sorted({k[0] for k in client}, key=lambda s: -int(re.match(r"m(\d+)", s).group(1)))


def p50_or_dash(xs):
    return f"{pct(xs, 50):>8.1f}" if xs else f"{'-':>8}"


for label in labels:
    for burst in (False, True):
        rows = client.get((label, burst))
        if not rows:
            continue
        recs = [(by_run[(label, burst, n)], status) for n, status, _ in rows if (label, burst, n) in by_run]
        cpu = [r["cpuTime"] for r, _ in recs]
        wall = [r["wallTime"] for r, _ in recs]
        cpu200 = [r["cpuTime"] for r, st in recs if st == "200"]
        cpu401 = [r["cpuTime"] for r, st in recs if st == "401"]
        oks = sum(1 for _, st, _ in rows if st == "200")
        first = by_run.get((label, burst, 1), {}).get("cpuTime")
        outcomes = dict(Counter(r["outcome"] for r, _ in recs))
        tag = f"{label}{' burst' if burst else ''}"
        if cpu:
            print(
                f"{tag:<10} {len(rows):>4} {oks:>4} {pct(cpu, 50):>8.1f} {pct(cpu, 99):>8.1f} {max(cpu):>8.1f} "
                f"{(first if first is not None else float('nan')):>7.1f} {p50_or_dash(cpu200)} {p50_or_dash(cpu401)} "
                f"{pct(wall, 50):>9.1f} {pct([c for _, _, c in rows], 50):>11.1f} {outcomes}"
            )
        else:
            print(f"{tag:<10} {len(rows):>4} {oks:>4} (no platform records) client p50 {pct([c for _, _, c in rows], 50):.1f} ms")
print()
print("cpu and wall are the platform's, client is curl's, all in milliseconds. `first` is the CPU of request 1 of the")
print("configuration (a fresh deploy: cold start included). `200` is how many sign-ins found their user: the memory adapter's")
print("state is per isolate and a request does not always land on the isolate that was seeded, and a 401 costs the same single")
print("hash by design, so p50 200 and p50 401 are shown apart.")
