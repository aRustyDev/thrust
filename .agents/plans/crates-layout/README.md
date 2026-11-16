# crates-layout

Moves the 18 workspace crates from the flat `crates/thrust-<name>/` layout into the grouped layout that
[ADR-0009](../contrail/adrs/architecture/0009-crate-directory-layout.md) decides, and adds the `regression_` test-name
prefix decided with it. Done looks like this: every crate is at its ADR-0009 path with its package name unchanged, no
file outside `.agents/plans/` and `CHANGELOG.md` names an old path, and a CI guard keeps the layout's rules.

The decision, its rules and the alternatives that were not taken are in the ADR; this plan holds only the sequence of
work.

## Map

| Path | Contents |
|---|---|
| `PLAN.md` | the four pull requests, in order, with what each changes and how it is verified; the status table; what is deliberately left for later |

## Status (2026-11-15)

| Item | State |
|---|---|
| ADR-0009 | written, `accepted` (owner decisions of 2026-11-15) |
| PR 1, `regression_` prefix | not started; independent of the move |
| PR 2, depth-independent paths | not started; lands before the move |
| PR 3, the move | not started; waits for #194 (release) and #201 (migration ledger), which touch `crates/` |
| PR 4, layout guard | not started; after the move |
