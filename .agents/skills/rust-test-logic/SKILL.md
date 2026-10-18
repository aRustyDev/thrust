---
name: rust-test-logic
description: "Use when writing tests for a unit of Rust logic: rstest tables for pure logic (`#[case]`), proptest properties (`prop_`, `PROPTEST_CASES`, `proptest-regressions`, `failed to find lib.rs or main.rs` in a `tests/` file), model-based sequences (proptest-state-machine, lockstep, CRUD call sequences), insta snapshots of plain values, the Rust side of a TypeScript parity harness (datatest-stable, `parity_`/`oracle_`), nextest prefix routing, a flaky `prop_` test or `--retries`, moving `cargo test` to nextest. NOT for fuzzing (rust-test-fuzz), mutation testing (rust-test-mutation), axum route, Dioxus or browser tests (rust-test-app), the TS-to-Rust port (ts-to-rust), vitest/jest or Terraform tests."
license: AGPL-3.0-only
author: aRustyDev
version: 0.1.0
---

# rust-test-logic

Pick the cheapest test that can fail for the right reason: named example cases, then properties that hold for every input, then stateful sequences against a model, then a parity or oracle check against legacy TypeScript. A counterexample not pinned as a named case will come back.

## Ground rules
- Run these tests with **cargo-nextest**, in crates written for it. A suite that parses `cargo test` output (the `test result:` line, exit 101) stays on `cargo test` until its exit-code and pass-count handling is migrated: nextest exits 100 on failures `[re-run]`. `cargo test --doc` is the one libtest call.
- Name tests by category; nextest routes on the prefix: `prop_`, `model_`, `parity_`, `oracle_`.
- Never pass `--retries`, `--flaky-result`, `NEXTEST_RETRIES` or `NEXTEST_FLAKY_RESULT`: the config's `flaky-result = "fail"` is a default, not a guard `[re-run]`. A randomized failure is a finding.
- A failing test is a finding: do not delete, weaken, `#[ignore]`, regenerate or lengthen its timeout to get green.
- Tests are hermetic and deterministic; a crate's tests live in that crate; commit regression data.
- `cargo install`, toolchains and Node builds are the user's decision: propose and wait.
- A check that cannot fail is not evidence: name the control that proves it can.

| Need | Read |
|---|---|
| rstest cases, async, matrices, fixtures, insta | `references/parameterized-and-snapshots.md` |
| proptest, config helpers, the `lib.rs or main.rs` warning | `references/property.md` |
| Stateful sequences, lockstep, proptest-state-machine, API call sequences | `references/model-based.md` |
| `parity_` / `oracle_` against legacy TypeScript | `references/parity-harness.md` |
| Profiles, prefixes, `-E` filters, `.config/nextest.toml` | `references/nextest-profiles.md` |
| Templates to copy | `references/examples/` |

## Procedure
1. State the unit's inputs and what must hold for all of them.
2. Write named example cases: boundaries, spec examples, one per fixed bug.
3. Add a `prop_` test per invariant, through the shared config helper.
4. State across calls? Add a `model_` test: lockstep first; proptest-state-machine when valid steps depend on state.
5. Must match legacy TypeScript? `parity_` fixtures first, `oracle_` for large input spaces; the port itself is `ts-to-rust`.
6. On a failure: read the shrunk input, pin it as a named case, fix the code (or say why the test was wrong).
7. Show the control: a deliberately broken implementation fails the new test.

## Output contract
Tests named by category, a named case for every counterexample, committed regression files, the exact `cargo nextest run` line with its result, and the control's result. Tag claims: `[re-run]` you ran it; `[zip]` the source author's claim, not re-run; `[derived]` from docs or source; `[unverified: <why>]` not run.

## Example
Q: "`prop_` test in `tests/roundtrip.rs` failed in CI, stderr says `failed to find lib.rs or main.rs`: is the counterexample gone?"
A: No, but the runner took the saved file. Copy the printed `cc <hash>` line into the committed `tests/roundtrip.proptest-regressions` and pin the shrunk input as an rstest `#[case::...]`. `mod common;` plus `common::proptest_config(256)` drops the warning. Fix the code, add no retries.
