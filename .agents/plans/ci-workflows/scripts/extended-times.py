"""What the jobs of `build-extended.yml` cost, from GitHub's own records: every run of the workflow on a real branch (a `probe/` branch is left out), each job's successful
durations (count, min, median, max, by event), and the failed jobs listed. They are not in `ci`, so ci-baseline.py does not see them. Research 0021 uses it.

    python3 -I .agents/plans/ci-workflows/scripts/extended-times.py

Needs `gh`, authenticated. A job that was cancelled (a newer push replaced the run) is in no row."""
import json
import statistics
import subprocess
import sys
from collections import defaultdict
from datetime import datetime

repo = "aRustyDev/thrust"


def t(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()


def gh(*args):
    return subprocess.run(["gh", "api", *args], capture_output=True, text=True, check=True).stdout


runs = []
for line in gh("--paginate", f"repos/{repo}/actions/workflows/build-extended.yml/runs?per_page=100", "--jq", ".workflow_runs[] | [.id, .event, .head_branch, .conclusion, .created_at] | @tsv").splitlines():
    rid, event, branch, conclusion, created = line.split("\t")
    runs.append((int(rid), event, branch, conclusion, created))

times = defaultdict(list)  # job -> [(seconds, event, run id)]
failed = []
skipped_probe = 0
for rid, event, branch, conclusion, created in sorted(runs):
    if branch.startswith("probe/"):
        skipped_probe += 1
        continue
    out = gh("--paginate", f"repos/{repo}/actions/runs/{rid}/jobs?filter=latest&per_page=100", "--jq", ".jobs[]")
    for j in (json.loads(l) for l in out.splitlines() if l.strip()):
        if not (j.get("started_at") and j.get("completed_at")):
            continue
        secs = t(j["completed_at"]) - t(j["started_at"])
        if j["conclusion"] == "success":
            times[j["name"]].append((secs, event, rid))
        elif j["conclusion"] == "failure":
            failed.append((rid, event, branch, j["name"], round(secs)))

print(f"{len(runs)} runs of build-extended.yml, {skipped_probe} on probe/ branches left out\n")
print("| job | runs | min (s) | median (s) | max (s) | by event |")
print("|---|---|---|---|---|---|")
for name in sorted(times):
    v = sorted(s for s, _, _ in times[name])
    ev = defaultdict(int)
    for _, e, _ in times[name]:
        ev[e] += 1
    print(f"| {name} | {len(v)} | {v[0]:.0f} | {statistics.median(v):.0f} | {v[-1]:.0f} | {', '.join(f'{k} {n}' for k, n in sorted(ev.items()))} |")
print("\nfailed jobs (not probes):")
for rid, event, branch, name, secs in failed:
    print(f"  run {rid} ({event}, {branch}): {name}, {secs} s")
if not failed:
    print("  none")
