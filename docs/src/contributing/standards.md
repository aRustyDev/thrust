# Standards and their gates

A standard counts as enforced here when a command checks it. This page lists each one with its command, then the
configuration that selects little or nothing yet, then what is only stated. CI runs most of these commands on every
pull request that changes a Rust-relevant path, and its one required check, `gate`, fails if any of them did not run
and pass ([CI/CD](../architecture/cicd.md)). A pull request that changes only docs or agent files skips them, and the
book's own checks are never run in CI. Two Socket Security checks (`Project Report` and `Pull Request Alerts`) also
report on pull requests. No workflow in this repository produces them, and nothing in it says what they check.

The testing rules themselves are in [Rules](../testing/rules.md), and the commands with their exit codes are in
[Running the suites](../testing/running.md). This page does not restate either.

## Enforced by a command

| Standard | Gate | It fails when | More |
|---|---|---|---|
| The portable crates depend on none of 17 named runtime and platform crates, the Worker and server graphs stay apart, and no ORM crate (`sqlx`, `sea-orm`, `diesel`, `diesel-async`: ADR-0044) is anywhere in the workspace | `scripts/check-deps.sh`. CI job `deps` | A forbidden crate is in a checked graph (exit 1), or `cargo tree` cannot say whether an ORM crate is there (also exit 1) | [Architecture gates](../testing/techniques/architecture-gates.md) |
| Code is formatted by rustfmt at width 120 (`rustfmt.toml`). The one-time reformat is listed in `.git-blame-ignore-revs`; `git config blame.ignoreRevsFile .git-blame-ignore-revs` makes a local `git blame` skip it | `cargo fmt --all --check`. CI job `lint` | A file differs from rustfmt's output (exit 1) | [Running the suites](../testing/running.md) |
| The panic policy of ADR-0006: `unwrap`, `expect`, `panic!` and indexing are denied in `thrust-core`, `thrust-ports` and the five adapters (`[workspace.lints.clippy]`, opted into with `[lints] workspace = true`), and every other clippy warning is an error | `cargo clippy --workspace --all-targets -- -D warnings`, and the same for `thrust-worker` on wasm32 with `--no-default-features`. CI job `lint` | A denied lint or any warning (exit 101). `clippy.toml` exempts `#[test]` functions and `#[cfg(test)]` modules; any other site that keeps one says why in an `#[allow(.., reason = "..")]` | [Running the suites](../testing/running.md) |
| The dependency graph has no unfixed advisory, only the allowed licences, no banned crate and only crates.io sources | `cargo deny check` (`deny.toml`). CI job `deny` | Exit code 1 for an advisory (RUSTSEC-2023-0071 is the one ignore), 2 for a ban, 4 for a licence, 8 for a source: one bit per check | [CI/CD](../architecture/cicd.md) |
| Observable HTTP behaviour is the same on every target | `scripts/contract.sh <target> [pass]`, and the 16-cell matrix `scripts/contract-all.sh`. CI runs one `contract` job per cell | A test fails; the target guard trips (exit 5) because `/_info` names a different target or adapter set; the count guard trips (exit 5) because the run exited 0 with a different number of passes than expected; the target does not start (exit 4) | [Contract suite](../testing/techniques/contract-suite.md) |
| The contract suite can fail | `scripts/mutants.sh`. CI job `mutants` | A planted mutant survives. A kill needs exit 101 and at least one `FAILED` line | [Mutation](../testing/techniques/mutation.md) |
| The hermetic tests pass under the policy in `.config/nextest.toml` | `cargo nextest run`. CI job `hermetic` runs it under the `ci` profile, then the doctests | A test fails or runs past its timeout. Retries are 0, and the `prop_`, `model_` and `oracle_` tests also set `flaky-result = "fail"` | [Running the suites](../testing/running.md) |
| The Postgres adapter behaves, including after an abandoned transaction | `cargo test -p thrust-adapters-selfhosted --test pg_adapter`. CI job `pg-adapter` | A test fails. It starts its own Postgres container, so it needs Docker | [Real backends](../testing/techniques/real-backends.md) |
| The pipeline's own logic (the gate, the matrix planner, the change classifier) | `sh .github/scripts/tests/test-ci-scripts.sh` and `test-required-contexts.sh`. CI job `ci-scripts` | A helper's test fails, which fails `gate` through `needs` | [CI/CD](../architecture/cicd.md) |
| The Rust gates above, except the pipeline's own tests, in one serial run | `scripts/verify-all.sh`. CI does not call it, and runs its steps as separate jobs, without the probes | Any step fails. The steps are the hermetic tests, the contract matrix, the planted mutants, `check-deps.sh`, the Postgres adapter tests and the `send-shape` probe. The `sqlx-wasm` probe also runs but cannot fail the run, `ammonia-400` is not run, it does not run `fmt`, `clippy` or `cargo deny`, and the whole script needs Docker | [Running the suites](../testing/running.md) |
| The book builds and has no orphan page | `mdbook build docs`, a scan of the log for `^ERROR`, and the orphan check. Not run in CI | Either scan prints something. `mdbook build` itself exits 0 in both cases | `docs/AGENTS.md` |

A pass means something only if the gate can fail. The pages in the last column say how to show that for
`check-deps.sh`, the contract suite and the planted mutants, and
[Evidence and controls](../testing/evidence-and-controls.md) lists the kinds of control. `verify-all.sh` only runs the
others. The book's own pages write down no control for its checks. The `mdbook` skill's reference does: a stray page
is reported by the orphan check, and a broken `{{#include}}` gives an `ERROR` line while `mdbook build` exits 0.

The `lint` and `deny` gates were shown to fail on 2026-11-05 `[verified 7d0074f]`. A deliberate `.unwrap()` in non-test
`thrust-core` stops clippy with `used unwrap() on an Option value` (`unwrap_used`); a mis-formatted line makes
`cargo fmt --all --check` exit 1 with the diff; deleting the RUSTSEC-2023-0071 entry from `deny.toml` makes
`cargo deny check advisories` fail and name it (`rsa v0.9.10`), which also shows that the ignore is what lets `main`
pass; removing `MPL-2.0` from the licence list fails on `cssparser`; and adding a banned crate fails `bans`.

## Configured, but selecting little or nothing today

- **`cargo mutants`.** `.cargo/mutants.toml` and the nextest `mutants` profile are in place. `AGENTS.md` records that a
  crate with no tests of its own stops at the baseline with exit 4, and most crates are tested only through
  `thrust-contract-tests`, which the config excludes.
- **The `deep` and `oracle` nextest profiles.** `oracle` selects the `oracle_` tests, which need the TypeScript
  oracle or its generated output. `deep` also selects `prop_` and `model_` tests, and no test has either prefix yet.
- **The `ci` profile.** It selects what `default` selects, and it does not stop at the first failure, shows each
  failure as it happens and again at the end, and writes a JUnit report at `target/nextest/ci/junit.xml`. The `hermetic` CI job uploads the
  report as an artifact kept for 7 days, and nothing reads it.

What exists and what is missing, measured, is on [Status, gaps and drift](../testing/status.md).

## Stated, but not enforced

| Standard | Where it is stated | What checks it |
|---|---|---|
| The licence is `AGPL-3.0-only`, and no crate is published | `[workspace.package]` in the root `Cargo.toml`, which every workspace member inherits. The three probe crates are outside the workspace and declare no licence | Nothing checks the workspace crates' own licence or `publish = false`: `deny.toml` skips them (`[licenses.private]`) so that AGPL is not on the list for anyone else, and checks the licences of what they depend on. The probes are outside `cargo deny`'s graph |
| Commit subjects are Conventional Commits (`type(scope): summary`) | `.agents/plans/contrail/PROMPT.md` (the plan's handoff prompt) states it, and every non-merge commit after the initial one follows it | No hook or workflow checks it. There is no commitlint or release-please configuration |
| Secrets come from the environment, never from a file or the command line | Comments in `crates/thrust-server/src/main.rs`, `crates/thrust-worker/wrangler.toml` and the scripts' headers | Nothing scans for them |
| The testing rules | `AGENTS.md`, rules 1 to 9 | Partly the nextest config and the scripts above. No command checks that rule 3 (a failing test is a finding), rule 8 (a check that cannot fail is not evidence) or rule 9 (generative first) was followed |

## Decided, and built only in part

ADR-0100 (a build matrix per adapter set) and ADR-0101 (CI supply-chain hygiene) are accepted and partly built:
the full contract matrix runs on every Rust-relevant pull request, `gate` is the aggregate required job, and every
action is pinned to a SHA, the `lint` job runs rustfmt and clippy, and the `deny` job runs `cargo deny`. There is no
nightly run, oracle schema check, scheduled check of the required contexts (or of the advisory database), Scorecard,
attestation or trivy. ADR-0110 (the deploy pipeline) is accepted and not
built. [CI/CD](../architecture/cicd.md) sets each against the workflows. ADR-0120 to ADR-0123 (the test
model and oracle, the port contract tests, the v1 oracle and parity tests, and the test standards and tooling) are
only proposed. The ADRs are in `.agents/plans/contrail/adrs/`.
