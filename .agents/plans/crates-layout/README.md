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

## Status (2026-11-16)

| Item | State |
|---|---|
| ADR-0009 | merged in #208, `accepted` (owner decisions of 2026-11-15) |
| PR 1, `regression_` prefix | merged in #209: 17 pinned regressions renamed: the 15 tests behind `PLAN.md`'s list (its contract-model entry is data the model test replays, so it takes no name), the #195 ledger test that #201 added, and the `single_dot` case, labelled "found by" |
| PR 2, depth-independent paths | in review, branch `chore/depth-independent-paths`. Found while doing it: Cargo reads `.cargo/config.toml` from the directory it runs in, not from `--manifest-path`, so `THRUST_ROOT` needs cargo run from inside the repository (from outside, those tests fail to compile; they never read a wrong path) |
| PR 3, the move | not started; waits for #194 (release). #201 (migration ledger) merged on 2026-11-16 |
| PR 4, layout guard | not started; after the move |
