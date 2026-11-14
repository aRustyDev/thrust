# Research 0016: what Plane's CLIs say about the agent CLI

Date: 2026-11-14. Purpose: check [`bd-equivalent/SPEC-0001`](../specs/0001-cli-command-structure.md) against the CLIs
that exist for Plane, and against what the Thrust server has decided and built.

Sources:

- Plane's Prime CLI page, <https://developers.plane.so/self-hosting/manage/prime-cli>, and the CLI errors page it
  links. Read through a fetch agent, not directly.
- `mggarofalo/plane-cli` at `a5039bf` (2026-10-29): `README.md`, `AGENTS.md`, `cmd/root.go`, `cmd/skills.go`,
  `internal/cmdgen/batch.go`, `internal/mcpserver/annotations.go`, the skill's gotchas template.
- `cpatrickalves/plane-cli` at `dbcf44b` (2026-11-11, v0.6.0, alpha): `README.md`, `CONTEXT.md`, `skills/SKILL.md`,
  the head of its command reference, ADRs 0002, 0003, 0005, 0006, 0007 and 0008.
- This repository at `origin/main`: `crates/thrust-api-v1/src/`, and Contrail's `FEATURES.md`, `TRACEABILITY.md`,
  ADR-0052 and ADR-0085.

Not done: neither CLI was installed or run, and most of their code was not read. The claims about them below come
from their own documents. Thrust's role matrix was not read; only its feature row (F-A08) was.

## Findings

1. **None of the three is a beads equivalent.** The Prime CLI is an operations tool for a self-hosted Commercial
   Edition install (`start`, `stop`, `restart`, `monitor`, `healthcheck`, `repair`, `configure`, `upgrade`,
   `update-cli`, `uninstall`); its page documents no flags, credential, JSON output or exit codes. The two community
   CLIs are clients for Plane's tracker objects: projects, states, labels, modules, cycles, intake, comments, work
   items. Neither has a claim, a lease, a dependency graph, a memory or a question for a person. They cannot confirm
   that half of SPEC-0001. They can test what any CLI over a Plane-shaped API shares: reading the hierarchy, IDs,
   output, errors, bulk writes and the agent surface.

2. **SPEC-0001's rule 6 relied on something Thrust decided not to have.** The rule enforced audience through the
   credential's scope and replaced bd's `--readonly` with a read-only key. Contrail's ADR-0052 is accepted with "no
   key scopes in v1" (Q-0052.2, D-008); scopes can come later through the `Authorizer`. **Owner's decision,
   2026-11-14: v1 enforces by the bot user's project role.** A project role (F-A08: 5, 15, 20) cannot say "issues:
   comment and change state only", so that part of the write boundary is held by the CLI not offering the command,
   not by the server.

3. **The levels depend on server features that are not all planned for the same time.**

   | Level | Server feature | State |
   |---|---|---|
   | Initiative | F-E14 | P2; `TRACEABILITY.md` marks it a gap, no ADR |
   | Project | F-B02 | P0; routes exist in `thrust-api-v1` |
   | Module | F-B11, titled "Modules (epics)" | P0; no `/api/v1` route yet. The title treats a module and an epic as one thing; the owner's levels do not |
   | Epic | F-E08, work item types | P1, schema-affecting. `work_items.rs` answers `type` and `type_id`, always null |
   | Issue | F-B03 | P0; routes exist |
   | Task | F-B03's parent and sub-items | A work item with a `parent`. No task resource exists |

   `thrust-api-v1` serves projects, states, labels and work items (list, create, read, update, delete, search, by
   identifier). It has no route for modules, comments or relations.

4. **The tree could not find an ID above the issue.** `show`, `status` and `tree` take any ID, and `issue list --in`
   takes a module or a project, but nothing listed them. Both community CLIs have `project list` and `module list`.

5. **Resolving names on a write path fails in three ways.** cpatrickalves matches names fuzzily at a 60% score.
   mggarofalo matches exactly and, when nothing matches, sends the literal string to the API unless `--strict` is
   given (its `README.md` says without a warning, its `AGENTS.md` says with one). Both resolve on the client, which
   costs list calls per command: cpatrickalves's skill reports HTTP 429 after about twenty creates in a row. Both
   skills also warn that the resolution cache is stale just after a create.

6. **A 200 is not proof of a write.** Plane's intake `PATCH` answers 200 with the record unchanged when the caller
   is not a project admin; cpatrickalves printed "Accepted" and exited 0 until its ADR-0007 made the command compare
   the answer with the request.

7. **The two CLIs agree on streams and disagree on exit codes.** Both keep data on stdout and everything else on
   stderr. mggarofalo: 2 authentication, 3 not found, 4 validation, 5 rate limited. cpatrickalves: 2 authentication,
   3 not found, 4 API error, 5 validation. mggarofalo prints the error as JSON on stderr in JSON mode. There is no
   Plane convention to inherit.

8. **Bulk and retry.** mggarofalo's `--batch` reads JSONL, writes one result line per input line, never stops at a
   failure and exits 1 when any line failed. Neither CLI can make a retried create safe: cpatrickalves's skill says
   a retry of a create that succeeded duplicates the item. In both, attaching a module is a second call, so an item
   can exist outside its module while the command reports an error. Thrust already has what the retry needs: a
   work item carries an integration key (`external_id` with its source), and the list route looks one up by it.

9. **Prompts.** cpatrickalves's ADR-0008 keeps every delete unprompted: a prompt hangs a caller with no terminal,
   and a prompt on some deletes teaches that an unprompted one is safe. The obligation it takes on instead is to
   state a delete's reach in the first two lines of its help. mggarofalo has a global `--dry-run` that prints the
   request without sending it.

10. **Agent surface.** Both ship an agent skill; mggarofalo generates its skill from its command specs and ships an
    MCP server whose read-only, destructive and idempotent hints come from the HTTP method. Each skill's list of
    gotchas is where the failures of findings 5, 6 and 8 are written down.

11. **Generating commands from documentation is fragile.** mggarofalo builds its commands by parsing Plane's docs
    site with regular expressions; its `AGENTS.md` warns that a change to the site can silently produce commands
    with no parameters.

12. **Beads already has a planned route into Thrust.** ADR-0085 accepts GitHub and Beads sync at P1 (F-E25),
    through `/api/v1` with an API key; its importer question (Q-0085.2) is open. SPEC-0001's `data import --from bd`
    did not mention it.

## What changed in SPEC-0001

| Finding | Change |
|---|---|
| 2 | Decision 6 added; rule 6 rewritten; rule 3 no longer cites a read-only credential |
| 3 | The level table gains a "Depends on" column; task identity added to Open |
| 4 | `project list`, `module list` and `initiative list` added (60 commands, 9 gaps) |
| 5 | Rule 9: a write takes IDs, and a name that does not resolve is an error |
| 6 | Rule 10: exit 0 means the write was applied |
| 7 | Rule 8 extended with the streams; the exit-code table added to Open |
| 8 | Rule 5 extended with the bulk result contract; rule 11: one command is one request, and a create can be retried |
| 9 | Rule 12: no prompts; the reach of a delete is stated in its help. `--dry-run` added to Open |
| 10, 11 | Added to Open, not to the rules: MCP is not decided, and nothing is built |
| 12 | Linked from Open |

Not adopted: fuzzy matching (finding 5), bundling secondary reads into `show` with a null on failure
(cpatrickalves's ADR-0006: three states for one field), and noun and verb aliases (cpatrickalves has `wi`, `issues`
and `issue`, `ls`, `read` and `new`; SPEC-0001's rule 2 stands).
