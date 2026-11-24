"""The full runs one by one: for each full run that ci-baseline.py listed, the event, the attempt, the wall clock of its final attempt, the e2e cell's duration and the
longest wait of a job for a runner. Research 0021 uses it to say what lifts a median of 18 runs: the clean runs and the pull-request runs are different populations.

    python3 -I .agents/plans/ci-workflows/scripts/full-runs.py <the output of ci-baseline.py, saved to a file>

Needs `gh`, authenticated. It reads the `- full:` line of that output for the run ids.

A job with `needs` is created when its run starts, so its created_at-to-started_at gap includes waiting for its dependencies. The wait here is therefore taken only for the jobs
that wait for `changes` alone (the verdicts and the gate are left out), and from the moment `changes` finished."""
import json
import re
import subprocess
import sys
from datetime import datetime

baseline = open(sys.argv[1], encoding="utf-8").read()
line = next(l for l in baseline.splitlines() if l.startswith("- full:"))
ids = re.findall(r"(\d{11}) \((\w+)\)", line)
repo = "aRustyDev/thrust"


def t(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()


def gh(*args):
    return subprocess.run(["gh", "api", *args], capture_output=True, text=True, check=True).stdout


rows = []
for rid, event in ids:
    run = json.loads(gh(f"repos/{repo}/actions/runs/{rid}"))
    out = gh("--paginate", f"repos/{repo}/actions/runs/{rid}/jobs?filter=latest&per_page=100", "--jq", ".jobs[]")
    jobs = [json.loads(l) for l in out.splitlines() if l.strip()]
    done = [j for j in jobs if j.get("started_at") and j.get("completed_at") and j.get("conclusion") == "success"]
    e2e = next((j for j in done if j["name"].endswith("worker-cf e2e")), None)
    # A job with `needs` is created when the run starts, so its wait includes its dependencies: only the jobs that wait for `changes` alone say anything about runners,
    # and they wait from the moment `changes` has finished.
    changes_done = t(next(j for j in done if j["name"] == "changes")["completed_at"])
    waits = [(t(j["started_at"]) - max(t(j["created_at"]), changes_done), j["name"]) for j in done
             if j.get("created_at") and j["name"] != "changes" and j["name"] != "gate" and not j["name"].endswith("verdict")]
    wall = t(run["updated_at"]) - t(run["run_started_at"])
    rows.append((wall, rid, event, run["run_attempt"], (t(e2e["completed_at"]) - t(e2e["started_at"])) if e2e else None, max(waits)))

print("run          event          att  wall(s)  e2e(s)  longest queue wait (s), the job")
for wall, rid, event, att, e2e, (w, name) in sorted(rows):
    print(f"{rid}  {event:14} {att:3}  {wall:7.0f}  {'-' if e2e is None else round(e2e):>6}  {w:6.0f}  {name}")
walls = sorted(r[0] for r in rows)
print(f"median wall {walls[len(walls) // 2]:.0f} s over {len(rows)} runs")
