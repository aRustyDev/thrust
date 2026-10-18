---
name: rust-testing
description: >-
  Use when asked "does this contract test actually fail?" or when writing, running or extending tests
  in this Rust workspace: the HTTP contract suite (scripts/contract.sh, scripts/contract-all.sh),
  "X is rejected" assertions, in-memory adapters vs mocks, testcontainers Postgres tests, mutation
  controls (scripts/mutants.sh), scripts/flake-hunt.sh, scripts/check-deps.sh. Fires on "vacuous
  assertion", "mutant survived", "VACUITY GUARD", "passed count != expected", "flaky contract test".
  Also holds digests (not yet used here) for doc-tests, proptest-state-machine, mockall/wiremock,
  nextest, cargo-mutants. NOT for Terraform, JS/TS or Dioxus UI tests, or CI workflow design.
allowed-tools:
  - Bash(scripts/contract.sh native-memory:*)
  - Bash(scripts/contract.sh native-pg:*)
  - Bash(scripts/contract.sh worker-memory:*)
  - Bash(scripts/contract.sh worker-cf:*)
  - Bash(scripts/contract-all.sh)
  - Bash(scripts/mutants.sh)
  - Bash(scripts/flake-hunt.sh:*)
  - Bash(scripts/check-deps.sh)
author: aRustyDev
version: 0.1.0
---

# rust-testing

**A verification that cannot fail is not evidence.** Every new "X is rejected" assertion ships with a
control that proves it can fail.

The spine here (not the generic proptest/mockall stack): one HTTP contract suite
(`crates/thrust-contract-tests`) run per target and pass by `scripts/contract.sh <target> [pass]`,
in-memory adapters instead of mocks, real Postgres for adapter properties, and must-fail controls.
Dependency graph: `scripts/check-deps.sh` (ADR-0002, ADR-0007).

## Two traps

- `scripts/flake-hunt.sh` always exits 0. Read its `failures: F / N` line.
- Read **which tests** are listed under KILLED in `target/mutants/summary.txt`, not just that something
  failed. `scripts/mutants.sh`: "A mutant that survives means some assertion is vacuous."

## Route

| Need | Read |
|---|---|
| Targets, passes, guards, expected counts | `references/contract-suite.md` |
| Writing a control; wiring a mutant; guard positive controls | `references/must-fail-controls.md` |
| Fakes vs real adapters; testcontainers; flake loop | `references/adapters-and-flakes.md` |
| Doc-tests, model-based, mocks and wiremock, nextest and cargo-mutants (not yet used here) | `references/doc-tests.md`, `model-based-testing.md`, `mock-based-testing.md`, `nextest-and-mutants.md` |

## Adding an assertion: the one procedure

1. Write it in `crates/thrust-contract-tests/tests/contract.rs` with its accepted neighbour in the same
   test. For async effects, assert the precondition first ("not delivered yet").
2. Add a mutant that breaks exactly that rule (wiring: `references/must-fail-controls.md`), run
   `scripts/mutants.sh`, and confirm your test is listed under KILLED.
3. If a pass's test count changed, update its `EXPECT_*` at the top of `scripts/contract.sh`.
4. Run the touched cell(s), serially.

## Output contract

Report per cell: target, pass, exit status, `passed=N expected=M`; per mutant: KILLED or SURVIVED
with test names; and what was not run.

Tags: `[re-run]` reproduced locally (offline scratch copy or crate); `[spike]` recorded by the spike,
not re-run; `[derived]` reasoned from this repo's code, not tried; `[unverified]` or
`[unverified: <tool> not installed]` not confirmed from a source read, or tool missing.
