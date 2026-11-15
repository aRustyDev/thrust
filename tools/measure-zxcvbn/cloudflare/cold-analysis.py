"""Cold-isolate comparison for a run of measure-zxcvbn-cpu.sh.

A request that lands on a fresh isolate costs cold start (the S arms: zxcvbn never runs) and, on the Z arm, the dictionaries too.
Warm requests cost 0 to 4 ms in every arm, so a request above a threshold is a cold one. The cost of the first zxcvbn call in a
fresh isolate is then median(vZ cold) - median(S cold), S being the baseline and variant S arms pooled (they are the same cold start
and are compared with each other first, because if they differ the bigger module costs cold starts something).

Usage: uv run --no-project python -I tools/measure-zxcvbn/cloudflare/cold-analysis.py <outdir>   (the outdir has client.tsv and kv.json)
"""
import json
import random
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
    if code == "400" and rec and rec.get("outcome") == "ok":
        rows.append((arm, int(rnd), kind, rec["cpuTime"]))


def q(xs, p):
    xs = sorted(xs)
    return xs[min(len(xs) - 1, int(round(p * (len(xs) - 1))))]


def cold(arm_names, t):
    return [c for (a, r, k, c) in rows if a in arm_names and c > t]


def boot_diff(a, b, n=10000, seed=1):
    rng = random.Random(seed)
    ds = []
    for _ in range(n):
        ra = [rng.choice(a) for _ in a]
        rb = [rng.choice(b) for _ in b]
        ds.append(statistics.median(ra) - statistics.median(rb))
    ds.sort()
    return ds[int(.025 * n)], ds[int(.975 * n)]


print("cold requests (cpuTime above a threshold), all kinds (seq + b1 + b2)")
print(f"{'threshold':>9} {'arm':>4} {'n cold':>7} {'of':>5} {'p25':>5} {'p50':>5} {'p75':>5} {'p90':>5} {'max':>5}")
for t in (5, 10, 15):
    for a in ("bS", "vS", "vZ"):
        xs = cold({a}, t)
        total = sum(1 for (aa, r, k, c) in rows if aa == a)
        print(f"{t:>8}ms {a:>4} {len(xs):>7} {total:>5} {q(xs, .25):>5} {q(xs, .5):>5} {q(xs, .75):>5} {q(xs, .9):>5} {max(xs):>5}")

print("\nthe two S arms against each other (does the bigger module cost a cold start anything?):")
for t in (5, 10, 15):
    b, v = cold({"bS"}, t), cold({"vS"}, t)
    lo, hi = boot_diff(v, b)
    print(f"  >{t:>2} ms: median vS {statistics.median(v)} - median bS {statistics.median(b)} = {statistics.median(v) - statistics.median(b)}   95% bootstrap interval of the difference [{lo}, {hi}]   n={len(v)}, {len(b)}")

print("\nfirst zxcvbn call on a fresh isolate = median(vZ cold) - median(S cold, bS and vS pooled):")
for t in (5, 10, 15):
    z, s = cold({"vZ"}, t), cold({"bS", "vS"}, t)
    lo, hi = boot_diff(z, s)
    print(f"  >{t:>2} ms: {statistics.median(z)} - {statistics.median(s)} = {statistics.median(z) - statistics.median(s)}   95% bootstrap interval [{lo}, {hi}]   n={len(z)}, {len(s)}")
z, s = cold({"vZ"}, 10), cold({"vS"}, 10)
lo, hi = boot_diff(z, s)
print(f"  vZ against vS alone (same build), >10 ms: {statistics.median(z)} - {statistics.median(s)} = {statistics.median(z) - statistics.median(s)}   [{lo}, {hi}]")

print("\nper round, median cpuTime of the cold requests (> 10 ms), by arm; rounds are the independent units here:")
by = defaultdict(lambda: defaultdict(list))
for (a, r, k, c) in rows:
    if c > 10:
        by[a][r].append(c)
meds = {}
for a in ("bS", "vS", "vZ"):
    meds[a] = {r: statistics.median(v) for r, v in sorted(by[a].items())}
    print(f"  {a}: " + ", ".join(f"r{r}:{m:g}(n={len(by[a][r])})" for r, m in meds[a].items()))
rounds_by_arm = {a: sorted({r for (aa, r, k, c) in rows if aa == a}) for a in ("bS", "vS", "vZ")}
for a in ("bS", "vS", "vZ"):
    missing = [r for r in rounds_by_arm[a] if r not in meds[a]]
    if missing:
        print(f"  {a}: rounds with no cold request at all: {missing}")
s_rounds = [m for a in ("bS", "vS") for m in meds[a].values()]
z_rounds = list(meds["vZ"].values())
print(f"  lowest vZ round median {min(z_rounds):g}; highest S round median {max(s_rounds):g}; "
      f"vZ rounds above every S round: {sum(1 for z in z_rounds if z > max(s_rounds))} of {len(z_rounds)}")
print("\ncold requests (> 10 ms) by kind. The second burst is only PARTLY warm: concurrent requests keep reaching fresh isolates (about 12 to 27% of b2")
print("against 35 to 64% of b1), and on the Z arm a first call blocks its isolate, so more requests find a fresh one. The first pilot assumed b2 was warm:")
for a in ("bS", "vS", "vZ"):
    for k in ("seq", "b1", "b2"):
        tot = sum(1 for (aa, r, kk, c) in rows if aa == a and kk == k)
        n_cold = sum(1 for (aa, r, kk, c) in rows if aa == a and kk == k and c > 10)
        print(f"  {a} {k:>3}: {n_cold:>3} of {tot:>3} cold")
print("\nbar 3: the first strength check in a cold isolate costs no more CPU than one argon2 hash, about 45 ms p50 deployed (research/0012)")
