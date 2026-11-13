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
print(f"{'config':<10} {'n':>4} {'200':>4} {'cpu p50':>8} {'cpu p99':>8} {'cpu max':>8} {'first':>7} {'p50 miss':>8} {'wall p50':>9} {'client p50':>11} outcomes")
labels = sorted({k[0] for k in client}, key=lambda s: -int(re.match(r"m(\d+)", s).group(1)))


def p50_or_dash(xs):
    return f"{pct(xs, 50):>8.1f}" if xs else f"{'-':>8}"


for label in labels:
    for burst in (False, True):
        rows = client.get((label, burst))
        if not rows:
            continue
        # Only a sign-in that found its user hashed (Plane's route answers an unknown email with a 400 before it hashes
        # anything), so the platform's CPU and wall time and the client's time are counted over the 200s; the CPU of the
        # rest is the `p50 miss` column, which is a control: it should be a millisecond or two, not a hash.
        found = [(by_run[(label, burst, n)], ms) for n, st, ms in rows if st == "200" and (label, burst, n) in by_run]
        missed = [by_run[(label, burst, n)]["cpuTime"] for n, st, _ in rows if st != "200" and (label, burst, n) in by_run]
        cpu = [r["cpuTime"] for r, _ in found]
        wall = [r["wallTime"] for r, _ in found]
        oks = sum(1 for _, st, _ in rows if st == "200")
        first_status = next((st for n, st, _ in rows if n == 1), None)
        first = by_run.get((label, burst, 1), {}).get("cpuTime") if first_status == "200" else None
        outcomes = dict(Counter(by_run[(label, burst, n)]["outcome"] for n, _, _ in rows if (label, burst, n) in by_run))
        tag = f"{label}{' burst' if burst else ''}"
        first_cell = "-" if first is None else f"{first:.1f}"
        if cpu:
            print(
                f"{tag:<10} {len(rows):>4} {oks:>4} {pct(cpu, 50):>8.1f} {pct(cpu, 99):>8.1f} {max(cpu):>8.1f} "
                f"{first_cell:>7} {p50_or_dash(missed)} "
                f"{pct(wall, 50):>9.1f} {pct([ms for _, ms in found], 50):>11.1f} {outcomes}"
            )
        elif oks:
            print(f"{tag:<10} {len(rows):>4} {oks:>4} (no platform records for the 200s) p50 miss {p50_or_dash(missed).strip()}")
        else:
            print(f"{tag:<10} {len(rows):>4} {oks:>4} (no sign-in found the user) p50 miss {p50_or_dash(missed).strip()}")
print()
print("cpu and wall are the platform's, client is curl's, all in milliseconds. `first` is the CPU of request 1 of the")
print("configuration (a fresh deploy: cold start included; '-' when that sign-in did not find its user). `200` is how many")
print("sign-ins found their user: the memory adapter's state is per isolate and a request does not always land on the isolate")
print("that was seeded. Plane's sign-in answers an unknown email with a 400 before it hashes, so only the 200s are an argon2")
print("measurement: cpu, wall and client are over them, and `p50 miss` is the CPU of the others (a control: about a")
print("millisecond, not a hash). The slice's route hashed on a miss as well, and the table recorded in research/0012 §4 was")
print("taken against it.")
