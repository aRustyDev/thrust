"""cicd.md's "What exists" table against the tree: every workflow and action on disk has a row, and every row names something that exists.
    python3 -I cicd-table-vs-tree.py <repo root>      (exit 1 on a difference)"""
import os
import re
import sys

root = sys.argv[1]
text = open(os.path.join(root, "docs/src/architecture/cicd.md"), encoding="utf-8").read()
start = text.index("## What exists")
end = text.index("\n## ", start + 5)
table = text[start:end]
named = set()
for row in table.splitlines():
    if row.startswith("| `"):
        first = row.split("|")[1]
        named.update(re.findall(r"`(\.github/(?:workflows/[\w.-]+\.yml|actions/[\w-]+))`", first))
on_disk = set()
for f in os.listdir(os.path.join(root, ".github/workflows")):
    if f.endswith(".yml"):
        on_disk.add(f".github/workflows/{f}")
for d in os.listdir(os.path.join(root, ".github/actions")):
    if os.path.isdir(os.path.join(root, ".github/actions", d)):
        on_disk.add(f".github/actions/{d}")
no_row = sorted(on_disk - named)
no_file = sorted(named - on_disk)
print(f"on disk: {len(on_disk)} workflows and actions; named in the table: {len(named)}")
print("on disk and not in the table:", no_row or "nothing")
print("in the table and not on disk:", no_file or "nothing")
sys.exit(1 if (no_row or no_file) else 0)
