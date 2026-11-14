# Research 0015: beads (`bd`) command taxonomy

Source: `bd --help` walked recursively on `bd version 1.3.4 (Homebrew)`, 2026-11-13: 297 command paths, plus `help`
(119 top-level commands). Help text only. No beads source was read, and the only commands run were `--help`,
`bd human` and `bd version`.
Purpose: input to the open question in [`README.md`](../README.md), whether Thrust gets a `bd` equivalent. The groups
are the owner's own list, checked against what each command does. **The placement is a proposal awaiting owner
review**; the judgment calls are at the end.
Not traced: hidden commands (cobra does not list them), how any command behaves when run, flags beyond the counts
quoted, beads' storage schema. Per-path listing: [`artifacts/bd-1.3.4-command-classification.md`](../artifacts/bd-1.3.4-command-classification.md).

## Findings

1. **`bd --help`'s eight groups do not separate the owner's categories.** Three groups have a majority of one
   category: "Views & Reports" (5 of 9 analytic), "Sync & Data" (5 of 9 sync) and "Maintenance" (9 of 17). The largest,
   "Working With Issues" (31), has no category above 12. "Additional Commands" (23) spans ten.
2. **bd has one fat command per verb plus a layer of thin wrappers.** `create` has 45 flags, `update` 36 and `list`
   64 (own flags, `--help` excluded). There is one create path, 14 update paths and 4 delete paths, against 20
   wrapper paths. bd's help calls eight of those commands shorthand or an alias; the others read as compositions of
   the fat commands.
3. **50 of the 297 paths are Dolt-coupled** (by the marking in `scripts/classify.py`): version control, sync,
   federation, history squashing, Dolt server lifecycle.
4. **The owner's list has no home for 42 paths** (14%): the formula, molecule, gate and swarm engine.

## Method

`scripts/walk_bd.py` runs `bd <cmd> --help` down the tree and writes `tree.json`. `scripts/classify.py` holds the
placement (`TOP`, `OVR`, `DOLT`) and refuses to write unless every path resolves, no key names a missing path, and no
override repeats its parent. Both are read-only against `bd`.

```sh
uv run .agents/plans/bd-equivalent/scripts/walk_bd.py <outdir>                                   # [verified 2026-11-13]
uv run .agents/plans/bd-equivalent/scripts/classify.py <outdir>/tree.json <out.md>               # [verified 2026-11-13] exits 0
uv run .agents/plans/bd-equivalent/scripts/classify.py <outdir>/tree.json <out.md> --self-test   # [verified 2026-11-13] exits 1
```

The `--self-test` run deletes `sql` from the mapping and must fail with `UNASSIGNED top-level: sql`; it did. A new
bd version that adds a command trips the same check, which is the point of it.

**The first pass lost 7 commands and the cause is worth keeping.** cobra pads names to the longest one, so the longest
command in a listing has one space before its description, and a two-space pattern drops it. The lost commands were
`recompute-blocked`, `epic close-eligible` (a write: `epic` had looked read-only), `federation remove-peer`,
`dolt clean-databases`, `mol last-activity` and two `migrate from-*-shared-server*` commands. A second trap: `bd gate`
has a "Gate types:" block of indented `name  - text` lines that reads as subcommands, and an unknown subcommand falls
through to its parent's help, so the walk recursed forever. Both are in the `walk_bd.py` docstring.

## Where `bd --help` places things differently

| bd group | Holds | What is actually in it |
|---|---|---|
| Working With Issues | 31 | wrappers 12, agent 6, read 5, workflow 3, update 3, create 1, delete 1 |
| Views & Reports | 9 | analytic 5, settings 2 (`statuses`, `types`), RAW 1 (`diff`), read 1 (`history`) |
| Dependencies & Structure | 7 | analytic 3, wrappers 2, update 1 (`dep`), workflow 1 (`swarm`) |
| Sync & Data | 9 | sync 5, RAW 2 (`branch`, `vc`), maintenance 2 (`backup`, `restore`) |
| Setup & Configuration | 19 | memory 5, settings 4, setup 2, integration 2, agent 2, help 2, RAW 1 (`dolt`), maintenance 1 |
| Maintenance | 17 | maintenance 9, developer 3, delete 2, RAW 1 (`sql`), wrapper 1 (`batch`), integration 1 (`events`) |
| Integrations & Advanced | 4 | integration 2, sync 1 (`repo`), maintenance 1 (`admin`) |
| Additional Commands | 23 | integration 5, workflow 4, help 4, read 2, wrappers 2, agent 2, update 1, analytic 1, settings 1, developer 1 |

The specific misplacements, each checked against the command's own help:

- **The six tracker integrations are split.** `jira` and `linear` are under "Integrations & Advanced"; `github`,
  `gitlab`, `ado` and `notion` are under "Additional Commands". All six share pull, push, status and sync.
- **`sql` is under Maintenance**; it is the raw door to the store.
- **`dolt` is under Setup & Configuration but does two jobs.** `start`, `stop`, `show`, `set`, `status` and `test`
  run and configure the server. `commit`, `push`, `pull` and `remote` move data and overlap `vc commit` and `sync`.
- **Dolt version control is scattered**: `branch` and `vc` under Sync & Data, `diff` and `history` under Views.
- **Everyday issue commands sit under "Additional Commands".** `ready`, `blocked`, `defer` and `undefer` belong with
  `list` and `update`; `bd human`, bd's own list of essentials, includes `ready` and `blocked`.
- **Memory is filed as configuration.** `remember`, `recall`, `forget`, `memories` and `kv` are not settings.
- **Agent and help commands are filed as configuration.** `prime` and `onboard` are for agents; `human`,
  `quickstart` and `init-safety` are help.
- **Three overlapping location and identity checks** (`context`, `where`, `info`) sit apart from `ping` and `doctor`
  (Maintenance) and `version` (Additional).
- **Maintenance holds non-maintenance.** `batch` is a bulk write; `worktree`, `preflight` (a Go lint and nix-hash
  checklist for contributors to beads itself) and `rules` (audits Claude rules) are developer tooling; `events` is a
  feed for consumers.
- **`admin` is not an integration.** It is `cleanup`, `compact` and `reset`. `admin cleanup` overlaps `prune` and
  `purge`. `admin compact` and `compact` share a verb but not an operation: the first summarizes closed issues, the
  second squashes Dolt history. `bd compact`'s help points to `admin compact` for the semantic kind, but
  `admin compact`'s own examples still run `bd compact --analyze`, `--apply`, `--auto` and `--stats`, flags
  `bd compact` does not take (it takes `--days`, `--dry-run` and `--force`).
- **`repo` configures multi-repo hydration**; it is sync.
- **Close-with-a-link and duplicate-finding are split.** `duplicate` and `supersede` close an issue with a link, under
  Dependencies. `duplicates` (exact, with `--auto-merge`) is there too, while `find-duplicates` (fuzzy) is under Views.
- **Agent and workflow commands are mixed into issue CRUD**: `heartbeat`, `reclaim`, `unclaim`, `state`, `set-state`,
  `provenance`, `gate`, `merge-slot` and `promote`. `tag` duplicates `label add`, and `comments list` is described as
  "Invalid — use bd comments <issue-id> to list comments".
- **`restore` and `backup restore` are different operations.** The first undoes compaction of one issue and is filed
  under Sync & Data.
- **`epic` has a read (`status`) and a write (`close-eligible`)** under one command in Dependencies & Structure.

## Classification

`D` marks a Dolt-coupled command. Counts are paths, subcommands included.

| Group | Paths | Commands |
|---|---|---|
| RAW | 16 | `sql`; `dolt` commit, pull, push, remote `D`; `vc` `D`; `branch` `D`; `diff` `D` |
| Issue: Create | 1 | `create` |
| Issue: Read-only | 13 | `list` `show` `search` `query` `ready` `blocked` `comments` (with its `list` stub) `history` `D`; `label list`, `label list-all`, `dep list`, `dep tree` |
| Issue: Update | 14 | `update` `close` `rename`; `label` add, remove, propagate; `dep` add, remove, relate, unrelate; `comments add`; `epic close-eligible` |
| Issue: Delete | 4 | `delete` `prune` `purge` `admin cleanup` |
| Issue: Wrappers | 20 | `assign` `priority` `note` `tag` `comment` `link` `children` `todo` `q` `create-form` `edit` `reopen` `defer` `undefer` `duplicate` `supersede` `batch` |
| Analytic | 12 | `count` `status` `stale` `lint` `orphans` `find-duplicates` `duplicates` `graph` `epic status` `dep cycles` |
| Sync | 18 | `sync` `D`, `federation` `D`, `conflicts` `D`, `repo`, `export`, `import` |
| Config: setup | 5 | `init` `bootstrap` `config apply` `dolt start`, `dolt stop` |
| Config: settings | 20 | `config` get, set, set-many, unset, list, show, validate, drift; `statuses` `types` `context` `where` `info` `metrics`; `dolt show`, `dolt set` |
| Memory | 9 | `remember` `recall` `forget` `memories` `kv` |
| Maintenance | 37 | `doctor` `ping` `gc` `compact` `flatten` `backup` `restore` `migrate` `upgrade` `recompute-blocked` `rename-prefix` `migrate-personal`, `admin` reset and compact; `dolt` status, test, killall, clean-databases; `events prune` |
| Integration | 45 | `jira` `linear` `github` `gitlab` `ado` `notion`; `setup` `hooks` `serve` `events` (export, tail) |
| Developer | 12 | `preflight` `schema` `rules` `worktree` `hooks run` `formula schema` |
| Agent-only | 20 | `heartbeat` `reclaim` `unclaim` `state` `set-state` `provenance` `audit` `prime` `onboard` `mail`; `human` list, respond, dismiss, stats |
| **+ Workflow engine** | 42 | `formula` `cook` `mol` (with `wisp`) `gate` `merge-slot` `swarm` `promote` `ship` |
| **+ Help and CLI meta** | 9 | `human` `quickstart` `init-safety` `version` `completion` (`help` is not walked) |

**Wrappers.** bd's help states that eight of them are shorthand: `assign` is `update --assignee`, `priority` is
`update --priority`, `note` is `update --append-notes`, `tag` is `update --add-label`, `comment` is `comments add`,
`link` is `dep add`, `children` is `list --parent <id> --status all`, and `todo` is a wrapper for task issues. The rest
are wrappers by behavior: `q` (create, print the ID), `create-form`, `edit` (update through `$EDITOR`), `reopen`,
`defer`, `undefer`, `duplicate`, `supersede` (close plus a link) and `batch`.

**Agent-only.** `bd human` prints 20 entries it calls the essentials for people (create, list, show, update, close,
reopen, note, ready, search, status, stats, dep add, dep remove, dep tree, graph, blocked, init, doctor, quickstart,
help). Its own help says "bd has 70+ commands - many for AI agents, integrations, and advanced workflows". The Agent-only row
is the runtime set: leases (`heartbeat`, `reclaim`, `unclaim`), state, provenance and audit logs, context injection
(`prime`, `onboard`) and mail. `human list`, `respond` and `dismiss` are the human end of an agent's escalation; they
are filed as agent-only but could be argued the other way.

## Observations for a Thrust equivalent

These follow from the table. None is a decision.

- **One core operation per verb, with the wrappers as CLI sugar.** bd documents eight of its wrappers as shorthand for
  the fat commands and the rest compose them, so an equivalent would not need them as separate API surface.
- **The Dolt-coupled paths have no direct analogue on D1 or Postgres.** What replaces them (history, sync, export and
  import) is a design question this note does not answer.
- **The workflow engine looks like the Gas Town (`gt`) layer.** The evidence is in the help text: `bd mail` is
  "Delegate to mail provider (e.g., gt mail)", merge-slot beads carry a `gt:slot` label, and its help talks about
  "polecats". Whether Thrust needs it is open.
- **bd's own validation differs by entry point.** In `batch`, `update status=closed` is refused while the issue has
  open children or a live blocker, but `close <id>` in the same batch applies no such check; bd's help says so. An
  equivalent should enforce close rules in one place.
- **bd has a read/write split at the command level**: the global `--readonly` flag ("block write operations (for
  worker sandboxes)"). An equivalent would need the same distinction for sandboxed workers and API keys.

## Judgment calls

Each is a placement the owner may overrule; `classify.py` is the one place to change it.

- `backup` is filed under Maintenance (durability), where bd puts it under Sync & Data.
- `history` is Read-only but `diff` is RAW: the first is per issue, the second is between commits.
- `kv` is filed under Memory. The help does not say that `remember` is built on it.
- `events` is Integration (a feed for consumers), with `events prune` under Maintenance.
- `ship` is under Workflow: it validates and adds a `provides:<capability>` label for cross-project dependencies.
- `orphans --fix` and `duplicates --auto-merge` write, so `orphans` and `duplicates` are not strictly read-only.
- `setup` and `hooks` are Integration because bd's own help calls them integration, though `hooks run` is internal
  (Developer).
- `dolt status` and `dolt test` are Maintenance (diagnostics), not settings.
