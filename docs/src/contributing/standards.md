# Standards and their gates

A standard counts as enforced here when a command checks it. This page lists each one with its command, then the
configuration that selects nothing yet, then what is only stated. **No CI workflow runs any of these commands.** Every
gate runs when someone runs it. The only automated checks on a pull request are the two Socket Security checks
(`Project Report` and `Pull Request Alerts`), and neither builds or tests the code.

The testing rules themselves are in [Rules](../testing/rules.md), and the commands with their exit codes are in
[Running the suites](../testing/running.md). This page does not restate either.

## Enforced by a command

| Standard | Gate | It fails when | More |
|---|---|---|---|
| The portable crates depend on none of 17 named runtime and platform crates, and the Worker and server graphs stay apart | `scripts/check-deps.sh` | A forbidden crate is in a checked graph (exit 1) | [Architecture gates](../testing/techniques/architecture-gates.md) |
| Observable HTTP behaviour is the same on every target | `scripts/contract.sh <target> [pass]`, and the 16-cell matrix `scripts/contract-all.sh` | A test fails; the target guard trips (exit 5) because `/_info` names a different target or adapter set; the count guard trips (exit 5) because the run exited 0 with a different number of passes than expected; the target does not start (exit 4) | [Contract suite](../testing/techniques/contract-suite.md) |
| The contract suite can fail | `scripts/mutants.sh` | A planted mutant survives. A kill needs exit 101 and at least one `FAILED` line | [Mutation](../testing/techniques/mutation.md) |
| The hermetic tests pass under the policy in `.config/nextest.toml` | `cargo nextest run` | A test fails or runs past its timeout. Retries are 0, and the `prop_`, `model_` and `oracle_` tests also set `flaky-result = "fail"` | [Running the suites](../testing/running.md) |
| All of the above in one serial run | `scripts/verify-all.sh` | Any step fails. The steps are the hermetic tests, the contract matrix, the planted mutants, `check-deps.sh`, the Postgres adapter tests and the `send-shape` probe. The `sqlx-wasm` probe also runs but cannot fail the run, `ammonia-400` is not run, and the whole script needs Docker | [Running the suites](../testing/running.md) |
| The book builds and has no orphan page | `mdbook build docs`, a scan of the log for `^ERROR`, and the orphan check | Either scan prints something. `mdbook build` itself exits 0 in both cases | `docs/AGENTS.md` |

A pass means something only if the gate can fail. The pages in the last column say how to show that for
`check-deps.sh`, the contract suite and the planted mutants, and
[Evidence and controls](../testing/evidence-and-controls.md) lists the kinds of control. `verify-all.sh` only runs the
others, and no control is written down for the book's checks: a page left out of `SUMMARY.md` and a broken
`{{#include}}` are the cases to try.

## Configured, but selecting little or nothing today

- **`cargo mutants`.** `.cargo/mutants.toml` and the nextest `mutants` profile are in place. `AGENTS.md` records that a
  crate with no tests of its own stops at the baseline with exit 4, and most crates are tested only through
  `thrust-contract-tests`, which the config excludes.
- **The `deep` and `oracle` nextest profiles.** `oracle` selects the `oracle_` tests, which need the TypeScript
  oracle or its generated output. `deep` also selects `prop_` and `model_` tests, and no test has either prefix yet.
- **The `ci` profile.** It writes JUnit output to `target/nextest/ci/junit.xml`, and nothing consumes it.

What exists and what is missing, measured, is on [Status, gaps and drift](../testing/status.md).

## Stated, but not enforced

| Standard | Where it is stated | What checks it |
|---|---|---|
| The licence is `AGPL-3.0-only`, and no crate is published | `[workspace.package]` in the root `Cargo.toml`, which every workspace member inherits. The three probe crates are outside the workspace and declare no licence | Nothing checks the licences of dependencies. There is no `deny.toml` |
| Commit subjects are Conventional Commits (`type(scope): summary`) | `.agents/plans/contrail/PROMPT.md` (the plan's handoff prompt) states it, and every non-merge commit after the initial one follows it | No hook or workflow checks it. There is no commitlint or release-please configuration |
| Secrets come from the environment, never from a file or the command line | Comments in `crates/thrust-server/src/main.rs`, `crates/thrust-worker/wrangler.toml` and the scripts' headers | Nothing scans for them |
| Format and lint | ADR-0123 (proposed) says `clippy` and `rustfmt` are required in CI, and `research/0004` lists the `clippy` panic lints as a TODO for P0 | There is no `rustfmt.toml`, `clippy.toml` or `rust-toolchain` file at the repository root, and nothing runs `cargo fmt` or `cargo clippy` |
| The testing rules | `AGENTS.md`, rules 1 to 8 | Partly the nextest config and the scripts above. No command checks that rule 3 (a failing test is a finding) or rule 8 (a check that cannot fail is not evidence) was followed |

## Decided, not built

ADR-0100 (a build matrix per adapter set), ADR-0101 (CI supply-chain hygiene) and ADR-0110 (the deploy pipeline) are
accepted, and no workflow implements them. ADR-0120 to ADR-0123 (the test model and oracle, the port contract tests,
the v1 oracle and parity tests, and the test standards and tooling) are only proposed. The ADRs are in
`.agents/plans/contrail/adrs/`.
