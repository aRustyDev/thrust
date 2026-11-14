#!/usr/bin/env python3
"""Map every bd command path onto the proposed Thrust agent CLI and prove the mapping is complete.

usage: uv run map_commands.py <tree.json> <out.md> [--self-test]

<tree.json> is walk_bd.py's output. Two hand-written tables are the proposal itself:

- NEW: the proposed command tree (specs/0001), each command with its audience and effect.
- MAP: what happens to each bd path. A path inherits its nearest ancestor's entry.

<out.md> is generated from them. The check exits 1 when

- a bd path resolves to no MAP entry (this is what a new bd version trips),
- a MAP key names a path that is not in the tree (a typo, or a command bd removed),
- a MAP entry repeats what its ancestor already says (noise),
- a KEEP or MERGE entry points at a command that is not in NEW,
- a NEW command has no bd source and is not listed in GAPS, or is in GAPS and has one,
- an ALIAS points at a command that is not in NEW.

--self-test deletes one MAP entry first, so the check MUST fail. Run it before trusting a pass.
`thrust` and the noun names are placeholders; the dispositions are a proposal awaiting owner review.
"""
import json
import sys
from collections import Counter, defaultdict

tree = json.load(open(sys.argv[1]))
out_path = sys.argv[2]
SELF_TEST = "--self-test" in sys.argv

# ---- the proposed tree: path -> (audience, effect, what it does) ------------------------------
# audience: human | agent | both | admin     effect: read | write | local (touches only this machine)
NEW = {
    "task create": ("both", "write", "Create a task under an issue (`--issue`), or a sub-task (`--parent`), in one request; `--key` makes a retry safe; `--from -` for many"),
    "task list": ("both", "read", "List tasks; one filter set (`--ready`, `--blocked`, `--stale`, `--parent`, `--query`, `--count`)"),
    "task update": ("both", "write", "Edit fields; `--if-status` / `--if-assignee` make it conditional"),
    "task close": ("both", "write", "Close, with the one close rule; `--duplicate-of`, `--superseded-by`"),
    "task reopen": ("both", "write", "Back to open from closed or deferred"),
    "task defer": ("both", "write", "Set aside until a date or indefinitely"),
    "task delete": ("both", "write", "Delete a task and its references"),
    "task lint": ("both", "read", "Report tasks missing required sections"),
    "issue list": ("both", "read", "List issues, epics included (`--epic`, `--in <module|project>`)"),
    "issue transition": ("both", "write", "Move an issue or epic to another state"),
    "module list": ("both", "read", "List the modules of a project"),
    "project list": ("both", "read", "List the projects the actor can see (`--in <initiative>`)"),
    "initiative list": ("both", "read", "List initiatives"),
    "dep add": ("both", "write", "Add a dependency or a see-also link (`--type`)"),
    "dep remove": ("both", "write", "Remove a dependency or link"),
    "dep list": ("both", "read", "Dependencies and dependents of an ID"),
    "dep check": ("both", "read", "Cycles and other graph faults"),
    "comment add": ("both", "write", "Comment on a task or an issue"),
    "comment list": ("both", "read", "Comments on an ID"),
    "label add": ("both", "write", "Add a label; `--children` pushes it down"),
    "label remove": ("both", "write", "Remove a label"),
    "label list": ("both", "read", "Labels on an ID, or all labels with no ID"),
    "claim take": ("agent", "write", "Claim a task atomically; `--next` claims the first ready one matching the filters"),
    "claim renew": ("agent", "write", "Extend the lease on a held claim"),
    "claim release": ("agent", "write", "Give a claim back"),
    "claim list": ("both", "read", "Who holds what, and until when"),
    "claim reap": ("admin", "write", "Return expired claims to ready"),
    "ask create": ("agent", "write", "Raise a question that blocks a task until a person answers"),
    "ask list": ("human", "read", "Open questions waiting on a person"),
    "ask answer": ("human", "write", "Answer a question and unblock its task"),
    "ask dismiss": ("human", "write", "Close a question without an answer"),
    "memory add": ("agent", "write", "Store a memory, optionally keyed and scoped"),
    "memory show": ("agent", "read", "Read one memory"),
    "memory list": ("agent", "read", "List or search memories"),
    "memory remove": ("agent", "write", "Delete a memory"),
    "activity list": ("both", "read", "What happened to an ID, or across a scope"),
    "activity tail": ("both", "read", "Follow the activity stream"),
    "activity record": ("agent", "write", "Append an agent event (decision, tool call, state)"),
    "agent prime": ("agent", "read", "Print the workflow context an agent needs at session start"),
    "agent onboard": ("agent", "read", "Print the snippet for an agent instructions file"),
    "agent setup": ("both", "local", "Install editor and agent integration files, hooks included"),
    "show": ("both", "read", "Show any ID at any level"),
    "status": ("human", "read", "Roll-up of progress for the linked project, or for any ID"),
    "tree": ("both", "read", "The hierarchy or dependency tree under any ID"),
    "init": ("both", "local", "Link this directory to a workspace and project"),
    "auth login": ("both", "local", "Store a credential"),
    "auth logout": ("both", "local", "Remove the stored credential"),
    "auth status": ("both", "read", "Server, actor, the actor's project role and linked project"),
    "config get": ("both", "local", "Read a setting"),
    "config set": ("both", "local", "Write one or more settings"),
    "config unset": ("both", "local", "Remove a setting"),
    "config list": ("both", "local", "All settings, and the server's states and types"),
    "data export": ("admin", "read", "Export tasks as JSONL"),
    "data import": ("admin", "write", "Import JSONL; `--from bd` reads a beads export"),
    "api": ("admin", "write", "Send a raw request to the Thrust API"),
    "doctor": ("both", "read", "Check the install, the link and the connection"),
    "schema": ("both", "read", "JSON Schema of the `--json` output"),
    "version": ("both", "read", "Version, and whether a newer one exists"),
    "completion": ("both", "read", "Shell completion script"),
    "help": ("both", "read", "The human set by default; `--all` adds agent and admin commands"),
}

# Top-level shortcuts. Each is one implementation: the canonical path with fixed arguments.
ALIAS = {
    "ready": ("task list", "`task list --ready`"),
    "next": ("claim take", "`claim take --next`"),
    "done": ("task close", "`task close <id>`"),
}

# NEW commands with no bd source: what bd lacks.
GAPS = {
    "auth login": "bd is local and has no credential",
    "auth logout": "as above",
    "claim list": "bd shows a claim only as an issue's status and assignee",
    "claim take": "in bd a claim is a flag (`update --claim`, `ready --claim`), not a command",
    "ask create": "in bd an agent adds the `human` label to an issue",
    "issue list": "bd has one level; an epic is an issue type",
    "module list": "bd has one level; without a list, an ID above the issue cannot be found (research 0016)",
    "project list": "as above; `init` needs it to link a directory without a prompt",
    "initiative list": "as above",
}

# ---- disposition of every bd path -------------------------------------------------------------
KEEP, MERGE, SERVER, OUT, DROP, DEFER = "keep", "merge", "server", "out", "drop", "defer"
DISP = {
    KEEP: "Kept: the same operation at a new path",
    MERGE: "Merged: becomes a flag or a case of another command",
    SERVER: "Server-side: policy or operations of the Thrust server, not a CLI command",
    OUT: "Out of scope: belongs to the tracker (plane-cli) side",
    DROP: "Dropped: no analogue",
    DEFER: "Deferred: not in v1",
}

def K(t, n=""): return (KEEP, t, n)
def M(t, n=""): return (MERGE, t, n)
def S(n=""): return (SERVER, None, n)
def O(n=""): return (OUT, None, n)
def X(n=""): return (DROP, None, n)
def D(n=""): return (DEFER, None, n)

DOLT = "Dolt-coupled; the server is the source of truth"
WF = "workflow engine, out of v1 (owner, 2026-11-14)"
TRACKER = "tracker integration"

MAP = {
    # issue commands
    "create": K("task create"),
    "q": M("task create", "`--quiet` prints only the ID"),
    "create-form": M("task create", "`--interactive`"),
    "todo": M("task list", "a task is a task"),
    "todo add": M("task create"),
    "todo done": M("task close"),
    "list": K("task list"),
    "search": M("task list", "`--text`"),
    "query": M("task list", "`--query`"),
    "count": M("task list", "`--count`"),
    "children": M("task list", "`--parent <id>`"),
    "ready": M("task list", "`--ready`; alias `ready`. `bd ready --claim` becomes `claim take --next`"),
    "blocked": M("task list", "`--blocked`"),
    "stale": M("task list", "`--stale`"),
    "show": K("show", "any level"),
    "update": K("task update", "`update --claim` becomes `claim take`"),
    "assign": M("task update", "`--assignee`"),
    "priority": M("task update", "`--priority`"),
    "note": M("task update", "`--append-notes`"),
    "edit": M("task update", "`--edit` opens `$EDITOR`"),
    "batch": M("task update", "`--from -` on each write verb, so bulk input takes the same checks"),
    "close": K("task close"),
    "duplicate": M("task close", "`--duplicate-of <id>`"),
    "supersede": M("task close", "`--superseded-by <id>`"),
    "reopen": K("task reopen"),
    "undefer": M("task reopen"),
    "defer": K("task defer"),
    "delete": K("task delete"),
    "lint": K("task lint"),
    "rename": X("the server allocates IDs"),
    "rename-prefix": X("the server allocates IDs"),
    "prune": S("retention"),
    "purge": S("retention"),
    "dep": K("dep list"),
    "dep add": K("dep add"),
    "dep remove": K("dep remove"),
    "dep relate": M("dep add", "`--type relates`"),
    "dep unrelate": M("dep remove"),
    "dep tree": M("tree"),
    "dep cycles": M("dep check"),
    "link": M("dep add"),
    "graph": M("tree"),
    "graph check": K("dep check"),
    "comments": K("comment list"),
    "comments add": K("comment add"),
    "comment": M("comment add"),
    "label": K("label list"),
    "label add": K("label add"),
    "label remove": K("label remove"),
    "label list-all": M("label list", "no ID"),
    "label propagate": M("label add", "`--children`"),
    "tag": M("label add"),
    "epic": M("status", "an epic is an issue; `status <epic-id>`"),
    "epic close-eligible": M("issue transition", "per epic; find them with `issue list --epic`"),
    "status": K("status"),
    "duplicates": D("duplicate detection"),
    "find-duplicates": D("duplicate detection"),
    "orphans": D("needs a commit scan; `--fix` wrote from a read command"),
    # claims and the agent runtime
    "heartbeat": K("claim renew"),
    "unclaim": K("claim release"),
    "reclaim": K("claim reap"),
    "human": M("help", "the default help is the human set"),
    "human list": K("ask list"),
    "human respond": K("ask answer"),
    "human dismiss": K("ask dismiss"),
    "human stats": M("ask list", "`--count`"),
    "prime": K("agent prime"),
    "onboard": K("agent onboard"),
    "audit": M("activity record"),
    "provenance": M("activity list"),
    "provenance record": M("activity record"),
    "history": M("activity list", "`activity list <id>`"),
    "events": M("activity list"),
    "events tail": K("activity tail"),
    "events prune": S("retention"),
    "state": D("label-backed state dimensions; Gas Town roles"),
    "set-state": D("label-backed state dimensions; Gas Town roles"),
    "mail": D("delegates to `gt mail`"),
    # memory
    "remember": K("memory add"),
    "recall": K("memory show"),
    "memories": K("memory list"),
    "forget": K("memory remove"),
    "kv": M("memory list"),
    "kv set": M("memory add", "`--key`"),
    "kv get": M("memory show"),
    "kv clear": M("memory remove"),
    # config, setup, meta
    "init": K("init", "links a directory; creates no database"),
    "bootstrap": M("init"),
    "init-safety": X("no local database to destroy"),
    "config": K("config list"),
    "config get": K("config get"),
    "config set": K("config set"),
    "config set-many": M("config set"),
    "config unset": K("config unset"),
    "config validate": M("doctor"),
    "config drift": M("doctor"),
    "config apply": X("nothing local to reconcile"),
    "statuses": M("config list", "server-defined, read-only"),
    "types": M("config list", "server-defined, read-only"),
    "metrics": M("config set", "a setting"),
    "context": M("auth status"),
    "where": M("auth status"),
    "info": M("auth status"),
    "doctor": K("doctor"),
    "ping": M("doctor"),
    "version": K("version"),
    "upgrade": M("version", "`--check`"),
    "completion": K("completion"),
    "quickstart": M("help"),
    "schema": K("schema"),
    "setup": K("agent setup"),
    "hooks": M("agent setup"),
    "hooks run": X("internal shim"),
    # data
    "export": K("data export"),
    "import": K("data import"),
    "sql": M("api", "the raw door is the API, not the store"),
    "events export": M("data export", "`--activity`"),
    # Dolt, sync, maintenance
    "dolt": X(DOLT), "vc": X(DOLT), "branch": X(DOLT), "diff": X(DOLT),
    "sync": X(DOLT), "federation": X(DOLT), "conflicts": X(DOLT),
    "compact": X(DOLT), "flatten": X(DOLT), "gc": X(DOLT),
    "repo": X("multi-repo hydration; one server holds every project"),
    "migrate": X("bd's own storage migrations; `data import --from bd` is the way in"),
    "migrate-personal": X("bd-specific"),
    "backup": S("backups"),
    "restore": S("compaction is not planned"),
    "recompute-blocked": S("blocked state is computed by the server"),
    "admin": S("operations"),
    "admin cleanup": S("retention"),
    "serve": X("the server is the API"),
    # tracker integrations
    "jira": O(TRACKER), "linear": O(TRACKER), "github": O(TRACKER),
    "gitlab": O(TRACKER), "ado": O(TRACKER), "notion": O(TRACKER),
    # bd contributor tooling
    "preflight": X("a checklist for contributors to beads"),
    "rules": X("audits Claude rules; not a tracker function"),
    "worktree": X("git tooling; not a tracker function"),
    # workflow engine
    "formula": D(WF), "cook": D(WF), "mol": D(WF), "gate": D(WF),
    "merge-slot": D(WF), "swarm": D(WF), "promote": D(WF), "ship": D(WF),
}

if SELF_TEST:
    MAP.pop("sql")  # the completeness check below must now fail

# ---- verification -----------------------------------------------------------------------------
nodes = sorted(k for k in tree if k != "bd")


def ancestor_entry(path, include_self=True):
    parts = path.split(" ")
    top = len(parts) if include_self else len(parts) - 1
    for i in range(top, 0, -1):
        p = " ".join(parts[:i])
        if p in MAP:
            return MAP[p]
    return None


problems = []
for n in nodes:
    if ancestor_entry(n) is None:
        problems.append(f"UNMAPPED bd path: {n}")
for k, v in MAP.items():
    if k not in tree:
        problems.append(f"DANGLING MAP key (not in tree): {k}")
    if " " in k and ancestor_entry(k, include_self=False) == v:
        problems.append(f"REDUNDANT entry (same as ancestor): {k}")
    if v[0] in (KEEP, MERGE) and v[1] not in NEW:
        problems.append(f"TARGET not in NEW: {k} -> {v[1]}")
    if v[0] not in (KEEP, MERGE) and v[1] is not None:
        problems.append(f"TARGET on a non-kept entry: {k}")

sources = defaultdict(list)
for n in nodes:
    e = ancestor_entry(n)
    if e and e[0] in (KEEP, MERGE):
        sources[e[1]].append(n)
for c in NEW:
    if c == "help":
        continue  # the walk does not visit `help`
    if not sources.get(c) and c not in GAPS:
        problems.append(f"NEW command with no bd source and not in GAPS: {c}")
    if sources.get(c) and c in GAPS:
        problems.append(f"GAPS lists a command that has a bd source: {c}")
for g in GAPS:
    if g not in NEW:
        problems.append(f"DANGLING GAPS key: {g}")
for a, (target, _) in ALIAS.items():
    if target not in NEW:
        problems.append(f"ALIAS target not in NEW: {a} -> {target}")
    if a in NEW:
        problems.append(f"ALIAS shadows a NEW command: {a}")

if problems:
    print("MAPPING CHECK FAILED:")
    for p in problems:
        print("  ", p)
    sys.exit(1)

# ---- report -----------------------------------------------------------------------------------
disp_count = Counter(ancestor_entry(n)[0] for n in nodes)
lines = []
w = lines.append
w("# bd to Thrust agent CLI: command map\n")
w("Generated by `scripts/map_commands.py` from `scripts/walk_bd.py` output. Do not edit by hand: change `NEW`,")
w("`MAP`, `ALIAS` or `GAPS` in the script and regenerate. `thrust` and the noun names are placeholders.\n")
w(f"Walked from `{tree['bd'].get('version', 'bd (version unknown)')}`: {len(nodes)} bd command paths onto "
  f"{len(NEW)} proposed commands and {len(ALIAS)} aliases.\n")
w("| Disposition | bd paths |")
w("|---|---|")
for d, label in DISP.items():
    w(f"| {label} | {disp_count.get(d, 0)} |")
w("")

w("## The proposed tree\n")
w("Audience is who the command is for. In v1 the server enforces only the actor's project role, which is coarser")
w("than audience (SPEC-0001 rule 6). Effect `local` touches only this machine.\n")
w("| Command | Audience | Effect | What it does | From bd |")
w("|---|---|---|---|---|")
for c, (aud, eff, what) in NEW.items():
    src = sources.get(c, [])
    if c in GAPS:
        frm = f"new: {GAPS[c]}"
    elif c == "help":
        frm = "`help`" + "".join(f", `{s}`" for s in src)
    else:
        frm = ", ".join(f"`{s}`" for s in src)
    w(f"| `{c}` | {aud} | {eff} | {what} | {frm} |")
w("")
w("### Aliases\n")
for a, (target, form) in ALIAS.items():
    w(f"- `{a}` is {form}")
w("")

w("## Every bd path\n")
for d, label in DISP.items():
    ns = [n for n in nodes if ancestor_entry(n)[0] == d]
    w(f"### {label}: {len(ns)}\n")
    if d in (KEEP, MERGE):
        w("| bd | Thrust | Note |")
        w("|---|---|---|")
        for n in ns:
            _, t, note = ancestor_entry(n)
            w(f"| `{n}` | `{t}` | {note} |")
    else:
        groups = defaultdict(list)
        for n in ns:
            groups[ancestor_entry(n)[2]].append(n)
        for note, members in groups.items():
            w(f"- {note or 'no note'} ({len(members)}): " + ", ".join(f"`{m}`" for m in members))
    w("")

open(out_path, "w").write("\n".join(lines) + "\n")

aud = Counter(v[0] for v in NEW.values())
eff = Counter(v[1] for v in NEW.values())
print(f"OK: {len(nodes)} bd paths mapped; {len(NEW)} proposed commands, {len(ALIAS)} aliases, {len(GAPS)} gaps.")
for d, label in DISP.items():
    print(f"  {disp_count.get(d, 0):3d}  {label.split(':')[0]}")
print("  audience:", dict(aud))
print("  effect:  ", dict(eff))
