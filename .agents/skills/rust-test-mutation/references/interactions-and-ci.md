# Interactions with nextest and the rest, and CI

Tags as in `commands-and-results.md`: `[re-run]` reproduced by running it (versions in `SOURCES.md`); `[zip]` recorded
by the source's author, not re-run; `[derived]` from the cargo-mutants book read 2026-10-18, not executed.

## What cargo-mutants does not see

| Situation | What you observe | What to do |
|---|---|---|
| **Package scope** | each mutant runs only the tests of the package being mutated `[zip]`; code guarded only by tests in another crate looks untested | keep a crate's tests inside that crate; `test_workspace = true` (`--test-workspace=true` `[derived]`) runs every crate's tests per mutant: thorough, much slower |
| **Doctests** | nextest does not run doctests, so behaviour checked only by a doctest shows as MISSED `[zip]`; cargo-mutants does not run them separately `[derived]` | add a real test; `cargo test --doc` stays a separate libtest call |
| **Macros** | macro bodies are not mutated, including Dioxus `rsx!` `[zip]`; macro calls, expansions and `macro_rules!` definitions are skipped `[derived]` | move logic out of markup into plain functions and test it there (UI layers: `rust-test-app`) |
| **`cfg`** | conditional compilation is not evaluated: `#[cfg(target_os = "...")]` code may show as missed on another OS `[zip]` | skip with a comment |
| **Flaky or external tests** | they poison the result: a randomized or environment-dependent failure is read as a catch `[zip]` | keep them out of the `mutants` profile (below); hermetic tests only |
| **Timeouts** | default is 5x the baseline test time, minimum 20 s `[zip]`; `--timeout <seconds>` sets it `[zip]` | see below |
| **Strict lints** | a workspace that denies warnings makes mutants that delete code fail on unused-variable lints, so they show as unviable `[zip]` | `cap_lints = true` `[zip]`; it also disables rustc's detection of long-running const evaluation, so a build may then need a timeout `[derived]` |

Timeouts: a hang usually means a mutated loop condition, so a timeout is often a justified skip `[zip]`.
Before deciding, check the limit is not just too low for a slow suite (raise `--timeout`). With
`--baseline=skip` the multiplier-based limit is unavailable and the fallback is 300 s, so pass `--timeout`
explicitly `[derived]`. A hang that real input could cause in production is a finding, not a skip.

## The nextest `mutants` profile

cargo-mutants runs the tests through nextest and selects this profile. The wiring is in
`examples/mutants.toml`: `test_tool = "nextest"`, `additional_cargo_test_args = ["--profile", "mutants"]`
and `profile = "mutants"` (the Cargo build profile). With those three keys, `baseline.log` and the argv in
`outcomes.json` show cargo-mutants running `cargo nextest run --no-run --cargo-profile=mutants --verbose
--package=<pkg>@<ver>` for the build and `cargo nextest run --cargo-profile=mutants --verbose
--package=<pkg>@<ver> --profile mutants` for the tests `[re-run]`. The profile itself:

```toml
# mutants: selected by cargo-mutants through .cargo/mutants.toml
# One failing test proves a mutant was caught: stop and kill stragglers.
[profile.mutants]
fail-fast = { max-fail = 1, terminate = "immediate" }
retries = 0
```

Why each line: one failing test is enough, and nextest otherwise lets straggling tests run to completion
`[derived]`; with `terminate = "immediate"` the running siblings were stopped with SIGTERM, then SIGKILL
`[re-run]`. `retries = 0` because nextest retries only after a failure: a test that fails on a mutant and
passes on retry is green, so a mutant it caught intermittently is reported MISSED, a false MISSED
`[derived]`. A per-test `flaky-result = "fail"` in the full nextest config is only a default:
`--flaky-result pass` or `NEXTEST_FLAKY_RESULT` overrides it, so a retried failure goes green `[re-run]`.
So no `--retries`, `--flaky-result`, `NEXTEST_RETRIES` or `NEXTEST_FLAKY_RESULT` in a mutation run, and
keep `retries = 0` in this profile.

Custom profiles inherit `[profile.default]`, so the `mutants` profile also inherits its `slow-timeout`
(in the zip's file 30 s x 4 = 120 s, longer for `prop_` and `model_` tests). If your cargo-mutants
`--timeout` is larger (the nightly line below uses 300), nextest can kill a hung test first, and that kill
is a failure: the mutant is reported caught instead of as a cargo-mutants timeout (exit 3)
`[derived]`. Not run. Set `slow-timeout` explicitly in the `mutants` profile, with `period` times
`terminate-after` larger than the cargo-mutants timeout, and make the same change in the full file.

**This block must match the full nextest config owned by `rust-test-logic`.** Do not keep a second
divergent copy: when the full file exists, its `[profile.mutants]` is the one that counts, and this
block is what to add if it is absent. The profile also relies on the `default-filter` of
`[profile.default]` in that file, which excludes the e2e package and `oracle_` tests `[zip]`:

```toml
default-filter = 'not (package(e2e) | test(/\boracle_/))'
```

The `mutants` profile inherits that filter: the baseline run of the reference workspace printed
`1 test skipped via profile.default.default-filter` `[re-run]`. A second check: `cargo nextest list --profile mutants` should show no `oracle_`
test and no e2e package test `[derived]`; the control that proves the check can fail is the same command
under the `oracle` profile, where the `oracle_` tests must appear.

The Cargo build profile `[profile.mutants]` (`examples/profile-mutants.toml.txt`) is separate: it speeds
up the per-mutant builds.

## Check that the config reached cargo-mutants

List twice and compare: `cargo mutants --list` with the config in place, then with the config moved
aside (or `--no-config` `[derived]`). In the reference workspace, with the config the list named none of
`crates/e2e`, `fuzz_harness`, `test_support` or `tests/common`; without it, it named
`crates/e2e/src/lib.rs` (twice) and `crates/core/src/fuzz_harness.rs` `[re-run]`. `test_support.rs` and
`tests/common/` never appeared even without the config, because code under `#[cfg(test)]` is not mutated
`[derived]`: those two globs are belt-and-braces. The first list is the check; the second is the control
that proves the check can fail. If both lists look the same, the config is not being read.

## CI

Both jobs: the tests already passed in an earlier job; install cargo-mutants by a method the user chose;
upload `mutants.out` and the log as artifacts, and print the log in a later step so the summary shows in
the job log (both steps `if: always()`, so they run after a failure).

**Read cargo-mutants' own exit code.** Redirect to a log and make the command last in the step. Never
`cmd | tee log`, and never `cmd > log 2>&1; echo "exit=$?"`: the first reports tee's status, the second
echo's, and both report 0 for a failed run.

PR job: `--in-diff` against the base branch; fail on MISSED. Full history is needed to diff
(`fetch-depth: 0` in the book's PR workflow `[derived]`).

```sh
# BASE: the PR's base branch name, set in the step's env (GitHub Actions: github.base_ref).
git diff "origin/$BASE...HEAD" > "$RUNNER_TEMP/pr.diff"
cargo mutants --in-diff "$RUNNER_TEMP/pr.diff" > mutants-pr.log 2>&1
```

Any non-zero status fails the job. To tell the causes apart, capture the status where it is produced,
keep it in the log, then exit with it:

```sh
# Same pr.diff and BASE as above.
status=0
cargo mutants --in-diff "$RUNNER_TEMP/pr.diff" > mutants-pr.log 2>&1 || status=$?
echo "cargo-mutants exit $status" >> mutants-pr.log
exit "$status"
```

2 = missed (fix), 3 = timeouts (read `timeout.txt`), 4 = the baseline is red (fix the tests; this is not
a mutation result), 5 or 6 = stale or malformed diff (regenerate).

Nightly full run, sharded, after the normal test job passed `[zip]`:

```sh
# CI ONLY: --in-place rewrites the checkout. Never copy this line to a working checkout.
# SHARD: the matrix value, 0 to 7.
PROPTEST_DISABLE_FAILURE_PERSISTENCE=1 cargo mutants --shard "$SHARD/8" --baseline=skip --timeout 300 --in-place > mutants-shard.log 2>&1
```

One matrix job per `SHARD` from 0 to 7. Every shard must use identical arguments and the same
denominator, or the split is meaningless `[derived]`. `--baseline=skip` is only sound because the tests
passed in the earlier job, and it needs the explicit `--timeout` `[derived]`. `--in-place` is acceptable
here only because a CI checkout is disposable; it is the same flag that must not touch a working checkout.

Do not use the older mutagen tool: it needs nightly and source annotations, cargo-mutants needs neither
`[zip]`.
