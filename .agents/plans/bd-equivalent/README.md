# bd-equivalent plan

A Thrust counterpart to beads (`bd`, the Dolt-backed issue tracker for agent workflows). The owner's stated intent
(2026-11-14) is a CLI that is the agent layer's interface to Thrust: agents claim and track work with it, and people
use it to check progress. There is a proposed command structure and no ADR, milestone or code yet.

| Document | What it holds |
|---|---|
| [`research/0015-bd-command-taxonomy.md`](research/0015-bd-command-taxonomy.md) | Every `bd` command (297 paths, bd 1.3.4) placed in the owner's groups, with where `bd --help` places them differently and why it matters for a Thrust equivalent. |
| [`research/0016-plane-clis-and-the-agent-cli.md`](research/0016-plane-clis-and-the-agent-cli.md) | SPEC-0001 checked against Plane's Prime CLI, two community Plane CLIs and what the Thrust server has decided and built; what changed in the spec as a result. |
| [`specs/0001-cli-command-structure.md`](specs/0001-cli-command-structure.md) | `bd-equivalent/SPEC-0001`, a proposal: the levels and the write boundary (owner's decisions), the naming rules, the command tree, what bd lacks, and what is still open. |
| [`artifacts/bd-1.3.4-command-classification.md`](artifacts/bd-1.3.4-command-classification.md) | The per-path listing behind research 0015. Generated: do not edit it by hand. |
| [`artifacts/bd-1.3.4-to-thrust-command-map.md`](artifacts/bd-1.3.4-to-thrust-command-map.md) | The proposed tree with audience and effect per command, and the fate of each of the 297 bd paths. Generated: do not edit it by hand. |
| [`scripts/`](scripts/) | `walk_bd.py` reads `bd --help` recursively; `classify.py` holds the placement of research 0015; `map_commands.py` holds the proposed tree and the mapping. Each checks its own completeness and has a `--self-test` that must fail. All are read-only against `bd`. |

## Open

- The open items of SPEC-0001: the ID format, leases, `ask`, the activity stream, memory scope, names, the
  exit-code table, what a task is on the server, `--dry-run`, MCP, and how `data import --from bd` relates to
  Contrail ADR-0085.
- How the CLI is built: the crate, and which API it calls. Nothing here addresses it.
- Enforcement: v1 enforces by the bot user's project role (owner, 2026-11-14), which cannot express the limited
  writes on issues; that part is a CLI convention until API keys have scopes.

## Numbering

Research numbers are repo-wide, so this plan's notes are 0015 and 0016: `contrail/research/` holds 0001 to 0014. The gap
under this plan is expected. Spec numbers are per plan, so always write `bd-equivalent/SPEC-0001`.
