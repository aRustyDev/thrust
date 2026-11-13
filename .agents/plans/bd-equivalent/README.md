# bd-equivalent plan

Exploratory. The owner is considering a Thrust counterpart to beads (`bd`, the Dolt-backed issue tracker for agent
workflows), or at least a way to enable what `bd` does inside Thrust. Nothing is decided: there is no ADR, spec or
milestone yet, and this directory holds research only.

| Document | What it holds |
|---|---|
| [`research/0015-bd-command-taxonomy.md`](research/0015-bd-command-taxonomy.md) | Every `bd` command (297 paths, bd 1.3.4) placed in the owner's groups, with where `bd --help` places them differently and why it matters for a Thrust equivalent. |
| [`artifacts/bd-1.3.4-command-classification.md`](artifacts/bd-1.3.4-command-classification.md) | The per-path listing behind it. Generated: do not edit it by hand. |
| [`scripts/`](scripts/) | `walk_bd.py` reads `bd --help` recursively; `classify.py` holds the placement and checks it is complete. Both are read-only against `bd`. |

## Open

- Whether to build an equivalent, to integrate with `bd` itself, or to expose the functionality another way. Not
  decided, and research 0015 does not argue for one.
- Which of the groups in research 0015 are in scope for a first version. The note records what each group contains,
  not whether Thrust needs it.

## Numbering

Research numbers are repo-wide, so this plan's first note is 0015: `contrail/research/` holds 0001 to 0014. The gap
under this plan is expected.
