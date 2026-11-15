"""Summarise a run of measure-zxcvbn-cpu.sh: the platform's cpuTime of a refused password, by arm.

Usage: uv run --no-project python -I tools/measure-zxcvbn/cloudflare/summary.py <outdir>   (the outdir has client.tsv and kv.json)
Reads client.tsv (label, arm, round, kind, seqno, http status, client seconds, x-thrust-version) and kv.json (the tail consumer's
record per label: cpuTime and wallTime in ms, outcome).
Arms: bS = baseline build, request S (zxcvbn never runs); vS = variant build, request S; vZ = variant build, request Z (zxcvbn runs).
A request counts only if its response carried the tag of its own round and it was a 400 with an `ok` platform record; everything
else is counted and reported, not averaged away.
"""
import json
import statistics
import sys
from collections import defaultdict

out = sys.argv[1]
kv = json.load(open(f"{out}/kv.json"))
rows = []
for line in open(f"{out}/client.tsv"):
    f = line.rstrip("\n").split("\t")
    if len(f) != 8:
        continue
    label, arm, rnd, kind, seqno, code, secs, ver = f
    rec = kv.get(label)
    if isinstance(rec, str):
        rec = json.loads(rec)
    rows.append(dict(label=label, arm=arm, rnd=int(rnd), kind=kind, seqno=int(seqno), code=code, ver=ver, rec=rec))

print(f"requests sent: {len(rows)}; platform records found: {sum(1 for r in rows if r['rec'])}")
bad_code = [r for r in rows if r["code"] != "400"]
no_rec = [r for r in rows if not r["rec"]]
not_ok = [r for r in rows if r["rec"] and r["rec"].get("outcome") != "ok"]
# One new Worker per round, so every response of a round must come from one version; two would mean a redeploy or a mixed round.
vers_by_round = defaultdict(set)
for r in rows:
    vers_by_round[r["rnd"]].add(r["ver"])
mixed = sorted(rnd for rnd, v in vers_by_round.items() if len(v) != 1)
print(f"not 400: {len(bad_code)}   no platform record: {len(no_rec)}   outcome not ok: {len(not_ok)}   rounds with more than one version: {mixed}")
for name, lst in (("not 400", bad_code), ("no record", no_rec), ("outcome", not_ok)):
    for r in lst[:5]:
        print(f"  {name}: {r['label']} code={r['code']} ver={r['ver']} rec={r['rec']}")
good = [r for r in rows if r["code"] == "400" and r["rnd"] not in mixed and r["rec"] and r["rec"].get("outcome") == "ok"]
print(f"counted: {len(good)}")


def q(xs, p):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(p * (len(xs) - 1))))]


def stats_line(name, xs):
    if not xs:
        return f"{name:12}{0:>5}"
    return f"{name:12}{len(xs):>5}{min(xs):>7}{q(xs, .5):>7}{q(xs, .75):>7}{q(xs, .9):>7}{max(xs):>7}{statistics.mean(xs):>8.1f}"


head = f"{'':12}{'n':>5}{'min':>7}{'p50':>7}{'p75':>7}{'p90':>7}{'max':>7}{'mean':>8}"

print("\ncpuTime ms, bursts b1+b2 (32 concurrent requests per round, on whichever isolates the platform chose):")
print(head)
burst = {a: [r["rec"]["cpuTime"] for r in good if r["arm"] == a and r["kind"] in ("b1", "b2")] for a in ("bS", "vS", "vZ")}
for a in ("bS", "vS", "vZ"):
    print(stats_line(a, burst[a]))
if all(burst.values()):
    print("\n  module size, vS - bS (a cold isolate pays it whether or not zxcvbn runs):")
    for name, p in (("p50", .5), ("p75", .75), ("p90", .9)):
        print(f"    {name}: {q(burst['vS'], p)} - {q(burst['bS'], p)} = {q(burst['vS'], p) - q(burst['bS'], p)}")
    print("  zxcvbn's first call, vZ - vS:")
    for name, p in (("p50", .5), ("p75", .75), ("p90", .9)):
        print(f"    {name}: {q(burst['vZ'], p)} - {q(burst['vS'], p)} = {q(burst['vZ'], p) - q(burst['vS'], p)}")
    print(f"    max: {max(burst['vZ'])} - {max(burst['vS'])} = {max(burst['vZ']) - max(burst['vS'])}")

print("\nsame-connection sequences straight after a deploy (cpuTime ms by position; position 1 is the first request on the isolate /healthz instantiated):")
seq = defaultdict(lambda: defaultdict(dict))
for r in good:
    if r["kind"] == "seq":
        seq[r["arm"]][r["rnd"]][r["seqno"]] = r["rec"]["cpuTime"]
deltas = {}
for a in ("bS", "vS", "vZ"):
    ds = []
    for rnd in sorted(seq[a]):
        s = seq[a][rnd]
        later = [s[k] for k in sorted(s) if k > 1]
        first = s.get(1)
        row = [s[k] for k in sorted(s)]
        if first is not None and later:
            d = first - statistics.median(later)
            ds.append(d)
            print(f"  {a} round {rnd:>2}: {row}  first - median(later) = {d}")
        else:
            print(f"  {a} round {rnd:>2}: {row}  (incomplete)")
    deltas[a] = ds
print()
for a in ("bS", "vS", "vZ"):
    ds = deltas[a]
    if ds:
        print(f"first minus later, {a}: n={len(ds)} min={min(ds)} median={statistics.median(ds)} max={max(ds)}")
print("  bS and vS are the control (a first request that is not zxcvbn); vZ's excess over them is zxcvbn's first call on an instantiated isolate.")
print("\nbar 3: the first strength check costs no more CPU than one argon2 hash, about 45 ms p50 deployed (research/0012)")
