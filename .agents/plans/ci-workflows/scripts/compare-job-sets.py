"""Two full ci runs, before and after the migration: the same jobs, apart from the five `verdict` jobs the migration adds?
A job's name loses the `<workflow> / ` the migration puts in front of it (`test / hermetic` is `hermetic`); a contract cell is
`contract (t, p) / t p` before and `test / contract (t, p) / t p` after.  Needs `gh`.
    python3 -I compare-job-sets.py <repo> <run before> <run after>"""
import json
import re
import subprocess
import sys

repo, before, after = sys.argv[1:4]
WORKFLOWS = ("documentation", "security", "web", "lint", "test")


def names(run):
    out = subprocess.run(["gh", "run", "view", run, "--repo", repo, "--json", "jobs,headSha,event,conclusion"], capture_output=True, text=True, check=True).stdout
    doc = json.loads(out)
    info = f"run {run}: {doc['event']} on {doc['headSha'][:7]}, {doc['conclusion']}, {len(doc['jobs'])} jobs"
    return info, [j["name"] for j in doc["jobs"]]


def plain(name):
    for w in WORKFLOWS:
        if name.startswith(w + " / "):
            return name[len(w) + 3:]
    return name


(info_b, nb), (info_a, na) = names(before), names(after)
print(info_b)
print(info_a)
sb, sa = sorted(plain(n) for n in nb), sorted(plain(n) for n in na)
missing = sorted(set(sb) - set(sa))
added = sorted(set(sa) - set(sb))
print(f"before: {len(sb)} names, {len(set(sb))} distinct; after: {len(sa)} names, {len(set(sa))} distinct")
print("in the run before and not after (what the migration lost):", missing or "nothing")
print("in the run after and not before (what it added):", added or "nothing")
cells_b = [n for n in sb if re.match(r"contract \(", n)]
cells_a = [n for n in sa if re.match(r"contract \(", n)]
print(f"contract cells: {len(cells_b)} before, {len(cells_a)} after, same set: {sorted(cells_b) == sorted(cells_a)}")
sys.exit(1 if missing else 0)
