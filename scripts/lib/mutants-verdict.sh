# shellcheck shell=sh
# The verdict on one row of scripts/mutants.sh: what a run against a deliberately broken build says about the tests. Sourced by
# scripts/mutants.sh; tested by .github/scripts/tests/test-mutants-verdict.sh. Each function prints one line, `VERDICT|detail`:
#
#   KILLED       the run reached its tests and at least one failed (and, when a needle is given, the log says it): the suite saw the mutant.
#   SURVIVED     the run reached its tests and every one passed: an assertion is vacuous, or the mutant does not break what it is meant to.
#   UNCLEAR      tests failed, but the log never says the needle, the text only the assertion the mutant is aimed at prints: something else
#                failed (a setup that panics in every test, say), which is not evidence the aimed assertion works.
#   DID NOT RUN  the run stopped before its tests gave a verdict (a precondition, a build, Docker, a target that never came up, a vacuity
#                guard, tests that did not build). Not evidence about any assertion.
#
# Only KILLED is a pass. Issue #93: before these were told apart, a server that never came up (AddrInUse) read like a surviving mutant,
# which is the very finding this script exists to make.

# The reason a run of scripts/contract.sh stopped, by its exit status (see that script's `exit` lines).
verdict_contract_reason() {
  case "$1" in
    2) echo "a precondition of the pass is missing (exit 2)" ;;
    3) echo "the setup failed: the server's build, a container, or a service that did not come up (exit 3)" ;;
    4) echo "the target never came up (exit 4)" ;;
    5) echo "the vacuity guard stopped it: no test ran, the wrong number passed, or the target is not the one asked for (exit 5)" ;;
    101) echo "the tests did not build (exit 101)" ;;
    *) echo "exit $1" ;;
  esac
}

# A JUnit report's `<testsuites ... NAME="N">`, or nothing when there is no report.
verdict_junit_attr() { sed -n "s/.*<testsuites[^>]* $2=\"\([0-9][0-9]*\)\".*/\1/p" "$1" 2>/dev/null | head -n 1; }

# verdict_suite <exit status> <the row's log> [needle]: a row run on nextest, which is every row: a pass of the contract suite (through
# scripts/contract.sh) or an adapter's own tests (through mutants.sh's `adapter_row`). Its JUnit report is beside the log
# (`<log>.junit.xml` for `<log>.test.log`). nextest exits 100 when a test failed; contract.sh exits 0 only when every test passed and
# as many as expected, and both it and adapter_row turn nextest's 4 (no test ran) into 5, the vacuity guard.
verdict_suite() {
  vs_status="$1"; vs_log="$2"; vs_needle="${3:-}"
  vs_junit="${vs_log%.test.log}.junit.xml"
  vs_f="$(verdict_junit_attr "$vs_junit" failures)"
  vs_e="$(verdict_junit_attr "$vs_junit" errors)"
  vs_failed=$(( ${vs_f:-0} + ${vs_e:-0} ))
  if [ "$vs_status" -eq 0 ]; then
    echo "SURVIVED|every test passed (exit 0)"
  elif [ "$vs_status" -eq 100 ] && [ "$vs_failed" -ge 1 ]; then
    if [ -z "$vs_needle" ] || grep -qF -- "$vs_needle" "$vs_log" 2>/dev/null; then
      echo "KILLED|$vs_failed test(s) failed"
    else
      echo "UNCLEAR|$vs_failed test(s) failed, but the log never says: $vs_needle"
    fi
  elif [ "$vs_status" -eq 100 ]; then
    echo "DID NOT RUN|nextest failed (exit 100) but no report counts a failed test"
  else
    echo "DID NOT RUN|$(verdict_contract_reason "$vs_status")"
  fi
}
