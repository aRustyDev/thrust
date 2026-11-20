#!/bin/sh
# The Worker's wasm32 unit tests (G-6; crates/worker/src/wasm_tests.rs): nextest's `workers` profile, one wasm-bindgen runner process per test,
# through scripts/wasm-test-runner.sh (.cargo/config.toml names it as the runner of wasm32-unknown-unknown). This is the command of the `workers` job
# of test.yml, and the one to run by hand; extra arguments go to nextest (`scripts/wasm-tests.sh -E 'test(core_config)'`).
#
#   * `-p thrust-worker`: the other workspace crates do not build their tests for wasm32, and need not.
#   * `--cargo-profile wasm-test`: no debug info (Cargo.toml). The runner processes the whole test binary once per test: 158 MB with debug info
#     took 20 s a test, 26 MB without took 1.9 s (measured 2026-11-20, three tests at once).
#   * a count guard: every `#[wasm_bindgen_test]` the source declares must have run. nextest fails when the profile selects nothing (exit 4),
#     but not when it selects fewer than were written, which is what a `cfg` or a rename that drops a test looks like.
#
#   WASM_TESTS_RUN     the command that runs the tests (default: nextest as above; for the test of this script)
#   WASM_TESTS_JUNIT   the report it writes, WASM_TESTS_SOURCE the file that declares the tests
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"
junit="${WASM_TESTS_JUNIT:-target/nextest/workers/junit.xml}"
source_file="${WASM_TESTS_SOURCE:-crates/worker/src/wasm_tests.rs}"

# a report left by an earlier run must not be counted for this one
rm -f "$junit"
# the command is a word list on purpose (it may name a script); the arguments are quoted
# shellcheck disable=SC2086
${WASM_TESTS_RUN:-scripts/rs cargo nextest run --profile workers --cargo-profile wasm-test --target wasm32-unknown-unknown -p thrust-worker --locked} "$@"

declared="$(grep -c '^#\[wasm_bindgen_test\]' "$source_file" || true)"
if [ ! -f "$junit" ]; then
  echo "scripts/wasm-tests.sh: no report at $junit" >&2
  exit 1
fi
# occurrences, not lines: a report is XML, and nothing says a test case has a line of its own
ran="$(grep -o '<testcase ' "$junit" | wc -l | tr -d ' ')"
# a run with arguments (a filter, say) runs fewer than are declared on purpose: the guard judges the run CI makes, which has none
if [ "$#" -gt 0 ]; then
  echo "wasm32 tests: $ran run, with arguments for nextest; the count guard judges only a run without them"
  exit 0
fi
if [ "$declared" -eq 0 ] || [ "$declared" -ne "$ran" ]; then
  echo "scripts/wasm-tests.sh: $source_file declares $declared tests and the report has $ran: a test was dropped (or none is declared)" >&2
  exit 1
fi
echo "wasm32 tests: $ran of $declared declared, all passed"
