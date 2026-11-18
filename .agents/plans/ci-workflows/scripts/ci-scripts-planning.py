"""How often is `ci-scripts` planned ONLY because a crate changed? Replays the files of recently merged pull requests through the real planner
(.github/scripts/plan-ci.py, today's crate graph), twice: with all their files, and without the files under crates/. A pull request whose first plan
has `ci-scripts` and whose second does not planned it only because of a crate change.
    python3 -I ci-scripts-planning.py <repo root> <metadata.json> <prs.json>
(prs.json: `gh pr list --state merged --json number,title,files`; metadata.json: `cargo metadata --format-version 1 --no-deps --offline`)"""
import importlib.util
import json
import os
import re
import sys

root, metafile, prsfile = sys.argv[1:4]
spec = importlib.util.spec_from_file_location("plan_ci", os.path.join(root, ".github/scripts/plan-ci.py"))
pc = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pc)
dirs, deps = pc.workspace(json.load(open(metafile)))
CELLS = ["native-memory main"]
prs = json.load(open(prsfile))

rows = {"full": [], "only crates": [], "other reasons": [], "not planned": []}
for pr in prs:
    paths = [f["path"] for f in pr["files"]]
    if not paths:
        continue
    jobs, _, notes = pc.plan(paths, dirs, deps, CELLS, None)
    if any(n.startswith("everything:") for n in notes):
        rows["full"].append(pr)
        continue
    if "ci-scripts" not in jobs:
        rows["not planned"].append(pr)
        continue
    rest = [p for p in paths if not re.match(r"^crates/", p)]
    jobs2 = pc.plan(rest, dirs, deps, CELLS, None)[0] if rest else set()
    (rows["other reasons"] if "ci-scripts" in jobs2 else rows["only crates"]).append(pr)

n = sum(len(v) for v in rows.values())
print(f"{n} merged pull requests replayed through today's planner")
for k, v in rows.items():
    print(f"  {k:14} {len(v):3}  " + ", ".join(f"#{p['number']}" for p in v[:24]) + (" ..." if len(v) > 24 else ""))
affected_only = len(rows["only crates"]) + len(rows["other reasons"]) + len(rows["not planned"])
planned = len(rows["only crates"]) + len(rows["other reasons"])
print(f"\nof the {affected_only} that were not full: ci-scripts planned in {planned}, and in {len(rows['only crates'])} of those only because a crate changed")
