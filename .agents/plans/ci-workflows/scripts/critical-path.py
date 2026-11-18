"""Where does a full run's wall clock go? For each run given: the time in `changes`, the wait from `changes` to the first other job, the job
that finished last before the end (not a verdict, not the gate), the wait from it to the first verdict (or the gate), the verdicts and the gate.
    python3 -I critical-path.py <repo> <run id>..."""
import json
import subprocess
import sys
from datetime import datetime

repo = sys.argv[1]


def t(s):
    return datetime.fromisoformat(s.replace("Z", "+00:00")).timestamp()


for run in sys.argv[2:]:
    doc = json.loads(subprocess.run(["gh", "run", "view", run, "--repo", repo, "--json", "jobs,event,headSha,createdAt,startedAt,updatedAt"], capture_output=True, text=True, check=True).stdout)
    jobs = [j for j in doc["jobs"] if j.get("conclusion") == "success" and j["startedAt"] and j["completedAt"]]
    by = {j["name"]: j for j in jobs}
    ch, gate = by["changes"], by["gate"]
    others = [j for j in jobs if j["name"] not in ("changes", "gate") and not j["name"].endswith("verdict")]
    verdicts = [j for j in jobs if j["name"].endswith("verdict")]
    first = min(others, key=lambda j: t(j["startedAt"]))
    last = max(others, key=lambda j: t(j["completedAt"]))
    t0, t1 = t(ch["startedAt"]), t(gate["completedAt"])
    print(f"== run {run} ({doc['event']} on {doc['headSha'][:7]}): {t1 - t0:.0f} s from changes' start to the gate's end")
    print(f"   changes ran {t(ch['completedAt']) - t(ch['startedAt']):.0f} s; the first other job ({first['name']}) started {t(first['startedAt']) - t(ch['completedAt']):.0f} s after it")
    print(f"   the last job to finish: {last['name']} ({t(last['completedAt']) - t(last['startedAt']):.0f} s, ended {t(last['completedAt']) - t0:.0f} s in)")
    if verdicts:
        v = min(verdicts, key=lambda j: t(j["startedAt"]))
        vend = max(t(j["completedAt"]) for j in verdicts)
        print(f"   first verdict started {t(v['startedAt']) - t(last['completedAt']):.0f} s after it; the last verdict ended {vend - t(last['completedAt']):.0f} s after it; the gate started {t(gate['startedAt']) - vend:.0f} s after that and ran {t(gate['completedAt']) - t(gate['startedAt']):.0f} s")
    else:
        print(f"   the gate started {t(gate['startedAt']) - t(last['completedAt']):.0f} s after it and ran {t(gate['completedAt']) - t(gate['startedAt']):.0f} s")
