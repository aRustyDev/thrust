---
name: rust-test-mutation
description: "Use for mutation testing of Rust code with cargo-mutants: 'are these tests good enough', 'does coverage mean anything', 'mutant survived', MISSED in `mutants.out/missed.txt`, cargo-mutants exit code 2/3/4, `--in-diff` on a PR, `--iterate`, `#[mutants::skip]`, `exclude_re`, `[profile.mutants]`, doctest-only code shown as missed, or choosing which tests to add from a cargo-mutants report. NOT for writing the property/model tests that kill mutants (rust-test-logic), fuzzing (rust-test-fuzz), axum/Dioxus/browser tests (rust-test-app), line coverage (llvm-cov, tarpaulin), Stryker or other JS/TS mutation testing, or Terraform tests."
license: AGPL-3.0-only
author: aRustyDev
version: 0.1.0
---

# Mutation testing with cargo-mutants

Coverage says a line ran; mutation says whether a test would fail if the line were wrong. cargo-mutants injects one bug at a time, runs the tests through nextest and lists every bug nothing caught.

## Ground rules
- Run tests with **cargo-nextest**. Some suites parse plain `cargo test` output (`test result:`, exit 101): do not convert them without migrating exit-code and pass-count handling (nextest exits 100 on failure `[re-run]`). `cargo test --doc` is the one libtest call.
- Tests are named by category (`prop_`, `model_`, `parity_`, `oracle_`, `fuzz_replay_`, `e2e_`); nextest routes on the prefix.
- No `--retries`, `--flaky-result`, `NEXTEST_RETRIES` or `NEXTEST_FLAKY_RESULT`. A retry follows a failure, so a test that catches a mutant, then passes on retry, is green and the mutant is reported MISSED `[derived]`; `--flaky-result pass` (or the env var) overrides a per-test `flaky-result = "fail"` `[re-run]`. Keep `retries = 0` in the `mutants` profile. A failing test is a finding: never delete, weaken, `#[ignore]`, regenerate or lengthen it to get green.
- Tests are hermetic, deterministic and live in the crate they test. Never commit `mutants.out*`.
- `cargo install cargo-mutants` and CI installers are the user's decision: propose and wait.
- Never `--in-place` on a working checkout: it rewrites your source.
- A run that tested zero mutants, or a fix you never saw fail first, is not evidence: read the counts.

| Need | Read |
|---|---|
| Commands, outcomes, exit codes, `mutants.out/`, the `--in-place` trap | `references/commands-and-results.md` |
| A mutant is MISSED: fix, skip, `exclude_re`, equivalent mutants | `references/fixing-missed.md` |
| Doctests, `rsx!`, `cfg`, timeouts, nextest `mutants` profile, CI | `references/interactions-and-ci.md` |
| Config templates | `references/examples/` |

## Procedure
1. Scope: one file or package while iterating, `--in-diff` for a PR, full runs in CI. `--list` first.
2. Run in the default copy mode, output redirected to a log; read cargo-mutants' own exit code.
3. For each line in `missed.txt`, ask what observable behaviour would break.
4. Fix through the public API; strengthen a property or model invariant to cover the class. Never write a test aimed at the mutant.
5. `--iterate` until clean, then one run without it.
6. Skip only an equivalent or untestable mutant, with a written reason; prefer `exclude_re` for a class.

## Output contract
Counts per outcome (caught, missed, timeout, unviable); each MISSED mutant with its fix or justified skip; the exact command and its exit code; cargo-mutants and nextest versions; what was not run (shards, unchanged code, workspace tests). Tags: `[re-run]` reproduced by running it (versions in `references/SOURCES.md`); `[zip]` recorded by the source's author, not re-run; `[derived]` from documentation; `[unverified: <reason>]` not run.

## Example
Reference workspace, names shortened. With the `is_empty` assertion removed from both model tests, `missed.txt` holds `src/store.rs:37:9: replace Store::is_empty -> bool with true` and `... with false`; exit 2. Restoring either `assert_eq!(sut.is_empty(), model.is_empty())` alone gives 18 of 18 caught `[re-run]`: one invariant covers the class. No test named after the mutant, no skip.
