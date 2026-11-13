#!/usr/bin/env python3
"""Place every bd command path in a group and prove the placement is complete.

usage: uv run classify.py <tree.json> <out.md> [--self-test]

<tree.json> is walk_bd.py's output. The mapping below (TOP, OVR, DOLT) is the classification itself and
the only hand-written part; <out.md> is generated from it. The check exits 1 when

- a top-level command in the tree has no entry (this is what a new bd version trips),
- a TOP, OVR or DOLT key names a path that is not in the tree (a typo, or a command bd removed),
- an override repeats its parent's category (noise).

--self-test deletes one mapping entry first, so the check MUST fail. Run it before trusting a pass.
The groups are the owner's list (RAW, issue verbs, analytic, sync, config, memory, maintenance,
integration, developer, agent-only) plus two it has no home for: WORKFLOW and META.
"""
import json
import sys
from collections import Counter, defaultdict

tree = json.load(open(sys.argv[1]))
out_path = sys.argv[2]
SELF_TEST = "--self-test" in sys.argv

CATS = {
    "RAW": "RAW (sql / Dolt passthrough)",
    "I.CREATE": "Issue / Create",
    "I.READ": "Issue / Read-only",
    "I.UPDATE": "Issue / Update",
    "I.DELETE": "Issue / Delete",
    "I.WRAP": "Issue / Wrappers",
    "ANALYTIC": "Analytic",
    "SYNC": "Sync",
    "CFG.SETUP": "Config / setup-init",
    "CFG.SETTINGS": "Config / settings",
    "MEMORY": "Memory",
    "MAINT": "Maintenance",
    "INTEG": "Integration",
    "DEV": "Developer",
    "AGENT": "Agent-only",
    # added: the owner's list has no home for these
    "WORKFLOW": "+ Workflow engine (formula/mol/gate/swarm)",
    "META": "+ Help & CLI meta",
}

# ---- top-level assignment, in the order of bd's own groups ------------------------------------
TOP = {
    # bd: Working With Issues
    "assign": "I.WRAP", "children": "I.WRAP", "close": "I.UPDATE", "comment": "I.WRAP",
    "comments": "I.READ", "create": "I.CREATE", "create-form": "I.WRAP", "delete": "I.DELETE",
    "edit": "I.WRAP", "gate": "WORKFLOW", "heartbeat": "AGENT", "label": "I.UPDATE",
    "link": "I.WRAP", "list": "I.READ", "merge-slot": "WORKFLOW", "note": "I.WRAP",
    "priority": "I.WRAP", "promote": "WORKFLOW", "provenance": "AGENT", "q": "I.WRAP",
    "query": "I.READ", "reclaim": "AGENT", "reopen": "I.WRAP", "search": "I.READ",
    "set-state": "AGENT", "show": "I.READ", "state": "AGENT", "tag": "I.WRAP",
    "todo": "I.WRAP", "unclaim": "AGENT", "update": "I.UPDATE",
    # bd: Views & Reports
    "count": "ANALYTIC", "diff": "RAW", "find-duplicates": "ANALYTIC", "history": "I.READ",
    "lint": "ANALYTIC", "stale": "ANALYTIC", "status": "ANALYTIC",
    "statuses": "CFG.SETTINGS", "types": "CFG.SETTINGS",
    # bd: Dependencies & Structure
    "dep": "I.UPDATE", "duplicate": "I.WRAP", "duplicates": "ANALYTIC", "epic": "ANALYTIC",
    "graph": "ANALYTIC", "supersede": "I.WRAP", "swarm": "WORKFLOW",
    # bd: Sync & Data
    "backup": "MAINT", "branch": "RAW", "conflicts": "SYNC", "export": "SYNC",
    "federation": "SYNC", "import": "SYNC", "restore": "MAINT", "sync": "SYNC", "vc": "RAW",
    # bd: Setup & Configuration
    "bootstrap": "CFG.SETUP", "config": "CFG.SETTINGS", "context": "CFG.SETTINGS",
    "dolt": "RAW", "forget": "MEMORY", "hooks": "INTEG", "human": "META",
    "info": "CFG.SETTINGS", "init": "CFG.SETUP", "kv": "MEMORY", "memories": "MEMORY",
    "migrate-personal": "MAINT", "onboard": "AGENT", "prime": "AGENT", "quickstart": "META",
    "recall": "MEMORY", "remember": "MEMORY", "setup": "INTEG", "where": "CFG.SETTINGS",
    # bd: Maintenance
    "batch": "I.WRAP", "compact": "MAINT", "doctor": "MAINT", "events": "INTEG",
    "flatten": "MAINT", "gc": "MAINT", "migrate": "MAINT", "ping": "MAINT",
    "preflight": "DEV", "prune": "I.DELETE", "purge": "I.DELETE", "recompute-blocked": "MAINT",
    "rename-prefix": "MAINT", "rules": "DEV", "sql": "RAW", "upgrade": "MAINT", "worktree": "DEV",
    # bd: Integrations & Advanced
    "admin": "MAINT", "jira": "INTEG", "linear": "INTEG", "repo": "SYNC",
    # bd: Additional Commands
    "ado": "INTEG", "audit": "AGENT", "blocked": "I.READ", "completion": "META",
    "cook": "WORKFLOW", "defer": "I.WRAP", "formula": "WORKFLOW", "github": "INTEG",
    "gitlab": "INTEG", "help": "META", "init-safety": "META", "mail": "AGENT",
    "metrics": "CFG.SETTINGS", "mol": "WORKFLOW", "notion": "INTEG", "orphans": "ANALYTIC",
    "ready": "I.READ", "rename": "I.UPDATE", "schema": "DEV", "serve": "INTEG",
    "ship": "WORKFLOW", "undefer": "I.WRAP", "version": "META",
}

# ---- subcommand overrides; a path inherits its nearest ancestor's category --------------------
OVR = {
    "comments add": "I.UPDATE",
    "label list": "I.READ", "label list-all": "I.READ",
    "dep list": "I.READ", "dep tree": "I.READ", "dep cycles": "ANALYTIC",
    "epic close-eligible": "I.UPDATE",
    "dolt show": "CFG.SETTINGS", "dolt set": "CFG.SETTINGS",
    "dolt start": "CFG.SETUP", "dolt stop": "CFG.SETUP",
    "dolt status": "MAINT", "dolt test": "MAINT", "dolt killall": "MAINT",
    "dolt clean-databases": "MAINT",
    "config apply": "CFG.SETUP",
    "hooks run": "DEV",
    "human dismiss": "AGENT", "human list": "AGENT", "human respond": "AGENT",
    "human stats": "AGENT",
    "events prune": "MAINT",
    "admin cleanup": "I.DELETE",
    "formula schema": "DEV",
}

# Dolt-coupled: the path or an ancestor is listed. No direct analogue on D1 or Postgres.
DOLT = {
    "diff", "history", "branch", "vc", "conflicts", "federation", "sync", "dolt", "compact",
    "flatten", "gc", "backup", "recompute-blocked", "admin compact",
    "migrate from-proxied-server-to-server", "migrate from-server-to-proxied-server",
    "migrate from-proxied-server-to-shared-server", "migrate from-shared-server-to-proxied-server",
}

if SELF_TEST:
    TOP.pop("sql")  # the completeness check below must now fail

# ---- verification -----------------------------------------------------------------------------
nodes = [k for k in tree if k != "bd"]
tops = [k for k in nodes if " " not in k]
problems = []

for t in tops:
    if t not in TOP:
        problems.append(f"UNASSIGNED top-level: {t}")
for k in TOP:
    if k not in tree and k != "help":  # `help` is the one name the walk does not visit
        problems.append(f"DANGLING TOP key (not in tree): {k}")
for k in OVR:
    if k not in tree:
        problems.append(f"DANGLING OVR key (not in tree): {k}")
for k in DOLT:
    if k not in tree:
        problems.append(f"DANGLING DOLT key (not in tree): {k}")
for k, v in OVR.items():
    if k in tree and TOP.get(k.split(" ")[0]) == v:
        problems.append(f"REDUNDANT override (same as parent): {k} -> {v}")

if problems:
    print("CLASSIFICATION CHECK FAILED:")
    for p in problems:
        print("  ", p)
    sys.exit(1)


def cat_of(path):
    parts = path.split(" ")
    for i in range(len(parts), 0, -1):
        p = " ".join(parts[:i])
        if p in OVR:
            return OVR[p]
    return TOP[parts[0]]


def dolt_bound(path):
    parts = path.split(" ")
    return any(" ".join(parts[:i]) in DOLT for i in range(1, len(parts) + 1))


# ---- report -----------------------------------------------------------------------------------
by_cat = defaultdict(list)
for n in sorted(nodes):
    by_cat[cat_of(n)].append(n)

bd_group = {t: tree[t].get("group", "?") for t in tops}
bd_group["help"] = "Additional Commands"

matrix = defaultdict(Counter)
for t in tops + ["help"]:
    matrix[bd_group[t]][TOP[t]] += 1

lines = []
w = lines.append
w("# bd command classification\n")
w("Generated by `scripts/classify.py` from `scripts/walk_bd.py` output. Do not edit by hand: change the mapping")
w("in `classify.py` and regenerate.\n")
w(f"Walked from `{tree['bd'].get('version', 'bd (version unknown)')}`: {len(nodes)} command paths, "
  f"{len(tops) + 1} top-level commands including `help`. `D` marks a Dolt-coupled path.\n")
w("## Per category\n")
for c, label in CATS.items():
    ns = by_cat.get(c, [])
    w(f"### {label}: {len(ns)}\n")
    parents = [x for x in ns if " " not in x]
    for n in parents:
        kids = [m for m in ns if m.startswith(n + " ")]
        d = " D" if dolt_bound(n) else ""
        desc = tree[n].get("desc") or tree[n]["short"]
        tail = f"  [{', '.join(k.split(' ', 1)[1] for k in kids)}]" if kids else ""
        w(f"- `{n}`{d}: {desc}{tail}  _(bd: {bd_group[n]})_")
    for n in ns:
        if " " in n and n.split(" ")[0] not in parents:
            d = " D" if dolt_bound(n) else ""
            desc = tree[n].get("desc") or tree[n]["short"]
            top = n.split(" ")[0]
            w(f"- `{n}`{d}: {desc}  _(split from `{top}`, bd: {bd_group[top]})_")
    w("")

w("## bd's own groups against this classification (top-level commands)\n")
w("| bd group | " + " | ".join(CATS) + " |")
w("|---|" + "---|" * len(CATS))
for g, cnt in matrix.items():
    w(f"| {g} ({sum(cnt.values())}) | " + " | ".join(str(cnt.get(c, "")) for c in CATS) + " |")
w("")

w("## Split commands (the parent and some subcommands land in different groups)\n")
for s in sorted({n.split(" ")[0] for n in OVR}):
    kids = {k: v for k, v in OVR.items() if k.split(" ")[0] == s}
    w(f"- `{s}` ({CATS[TOP[s]]}): " + "; ".join(f"`{k}` -> {CATS[v]}" for k, v in kids.items()))

open(out_path, "w").write("\n".join(lines) + "\n")

total = Counter(cat_of(n) for n in nodes)
print(f"OK: {len(nodes)} paths, all resolved, no dangling keys.")
for c, label in CATS.items():
    print(f"  {total.get(c, 0):3d}  {label}")
print(f"  Dolt-coupled paths: {sum(1 for n in nodes if dolt_bound(n))}")
