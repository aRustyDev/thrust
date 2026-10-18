# nextest profiles and prefix routing

Tags: `[re-run]` reproduced on this machine; `[zip]` recorded by the source's author, not re-run; `[derived]` read
from nextest's `--help` or reasoned from the config, not run.

`examples/nextest.toml` is the template this skill owns. It is not live in any repo until a Rust workspace exists:
copy it to `.config/nextest.toml` at the workspace root (nextest's default config path `[derived]`) in the first
workspace PR, not before.

## Scope rule: where nextest applies

nextest is for new hermetic crates and for the test categories in this skill. A repo may already have a suite that
runs shared HTTP assertions against every adapter and parses libtest's `test result:` line, relying on exit 101.
Leave it on `cargo test` until someone migrates its exit-code and pass-count handling on purpose: nextest exits 100
when tests fail `[re-run]`, exits 4 when the filter selects nothing `[re-run]`, and prints no libtest `test result:`
line `[derived]`. `cargo test --doc` stays a libtest call because nextest cannot run doctests.

## Profiles

All six resolve `[re-run]`. Custom profiles inherit from `default` `[zip]`.

| Profile | Selects | Notes |
|---|---|---|
| `default` | everything except package `e2e` and `oracle_` tests | the inner loop: `cargo nextest run` |
| `ci` | same as `default` | `fail-fast = false`, `failure-output = "immediate-final"`, JUnit to `target/nextest/ci/junit.xml`; run `cargo nextest run --profile ci --workspace` then `cargo test --doc --workspace` |
| `deep` | only `prop_`, `model_`, `oracle_` | `PROPTEST_CASES=4096 cargo nextest run --profile deep`; 300 s periods; passes at 4096 cases `[re-run]`; needs Node and the legacy package for `oracle_` |
| `oracle` | only `oracle_` | `cargo nextest run --profile oracle`; needs Node and the built legacy package |
| `e2e` | package `e2e` | browser tests, owned by `rust-test-app`; needs a web build and browsers |
| `mutants` | inherits `default` | `fail-fast = { max-fail = 1, terminate = "immediate" }`, `retries = 0`; one failing test proves a mutant caught; owned by `rust-test-mutation`, keep its copy identical |

## Prefix routing

The config routes on the test-name prefix with a `\b`-anchored regex, so `sub_prop_x` does not match `prop_`. Adding
a correctly prefixed test needs no config change.

| Prefix | Runs in | Settings |
|---|---|---|
| `prop_`, `model_` | `default`, `deep` | `retries = 0`, `flaky-result = "fail"`, `slow-timeout` 60 s x 5 (`deep`: 300 s x 6) |
| `parity_` | every profile that selects it | plain; fixtures are committed, so it is hermetic |
| `oracle_` | `oracle`, `deep` only | group `ts-oracle` (2 threads), `retries = 0`, `flaky-result = "fail"`, `leak-timeout = "1s"` |
| `fuzz_replay_` | `default` | owned by `rust-test-fuzz` |
| `e2e_` | `e2e` only, package `e2e` | group `browser` (2 threads) is defined; the retry override is **commented out** in the template (see below) |

Plain tests get `retries = 0` and `slow-timeout = { period = "30s", terminate-after = 4 }`: a test is flagged slow
after a period and killed after `terminate-after` periods `[derived]`. Do not lengthen these to get green.

## How `-E` filters combine

A `-E` filterset is **intersected** with the profile's `default-filter` `[derived: --help]`. Observed, each
printing no tests and exiting 4 `[re-run]`: an `oracle_` filter under the `default` profile; a `prop_` filter under
`--profile oracle`; `package(e2e)` under `default`. If a test you expect to run does not, you are in the wrong
profile. `--ignore-default-filter` exists `[derived: --help]`; prefer naming the profile.

`-E 'not test(/\boracle_/)'` under `deep` runs the randomized tests without Node `[derived]`.

Filters are single-quoted TOML strings so `\b` reaches the regex engine unchanged; in a double-quoted TOML string it
would parse as an escape `[zip]`.

## `flaky-result = "fail"` and where retries belong

A test that fails once then passes on retry is "flaky". Observed `[re-run]` (nextest 0.9.140):

- a plain test named `plain_*` that fails once is reported FLAKY and the run exits 0;
- a `prop_*` test that fails once then passes, under `--retries 2` alone, **fails** (exit 100), because the
  `prop_|model_` override sets `flaky-result = "fail"`;
- the same test under `--retries 2 --flaky-result pass`, or `NEXTEST_RETRIES=2 NEXTEST_FLAKY_RESULT=pass`,
  **passes** (exit 0, `1 passed (1 flaky)`): the per-test `flaky-result = "fail"` is a default that the flag and the
  variable override (`--help`: `--flaky-result <RESULT>`, "default: from profile");
- a plain test under `--retries 2 --flaky-result fail` fails.

So the override fails a randomized test that was retried only when `--flaky-result` is not also passed: **the config
is not a hard guard.** A retry can turn a real counterexample green `[zip]`, and a randomized test that failed once
has found an input that breaks the code. That is why the ground rule forbids all four of `--retries`,
`--flaky-result`, `NEXTEST_RETRIES` and `NEXTEST_FLAKY_RESULT`.

Enforce it with a CI step that fails if any of those four strings appears in workflow files (for example
`.github/workflows/`) or scripts. Exclude `.config/nextest.toml` and docs: they mention the strings in comments, and
an unscoped grep matches them. Its control: add one of the four to a workflow on purpose and watch the step go red; a
step that has never failed is not evidence.

Browser tests are the one legitimate place for retries: real browsers flake for infrastructure reasons, and a
passing retry is still reported as FLAKY so it gets fixed. **Deliberate change from the zip:** its override was
`filter = "package(e2e)"` with `retries = { backoff = "fixed", count = 2, delay = "1s" }`, which retries **every**
test in package `e2e`, including the hermetic harness test, contradicting "retries only for browser tests"
`[zip]`. Browser tests are not in use here yet, so the template ships that override commented out. Keep the `e2e`
profile and the `browser` group; when browser tests exist, enable the override with a filter that selects the
browser tests only. The package-wide filter is not enough, and neither is the `e2e_` prefix: the in-process
harness test lives in that package under that prefix too.

## Test groups

`[test-groups]` sets `max-threads` for a named group; an override assigns tests to it with `test-group = "..."`.
`ts-oracle` caps how many `oracle_` tests run at once (each starts a Node process); `browser` caps browser-driving
tests.

## Adapting the template

- Replace `e2e` in each `package(e2e)` filter with your browser-test package (two live places and the commented-out
  override), or delete the `e2e` profile if the repo has none.
- Keep `nextest-version.required` at or below the oldest nextest you support; running an older one is refused, and
  an older-than-`recommended` one only prints a notice `[derived]`.
- Keep the `mutants` profile identical to the copy in `rust-test-mutation`.

## Commands

| Purpose | Command |
|---|---|
| Inner loop | `cargo nextest run` |
| One crate, or names containing a substring | `cargo nextest run -p <crate>`, `cargo nextest run <substring>` |
| One category | `cargo nextest run -E 'test(/\bprop_/)'` |
| CI | `cargo nextest run --profile ci --workspace`, then `cargo test --doc --workspace` |
| Long randomized run | `PROPTEST_CASES=4096 cargo nextest run --profile deep` |
| Live TypeScript comparison | `cargo nextest run --profile oracle` |
