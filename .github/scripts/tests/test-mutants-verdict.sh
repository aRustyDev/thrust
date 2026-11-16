#!/bin/sh
# scripts/lib/mutants-verdict.sh: a row of scripts/mutants.sh is KILLED only when its tests ran and one failed (saying the needle, when
# there is one); SURVIVED when its tests ran and all passed; UNCLEAR when tests failed without the needle; DID NOT RUN when it stopped
# before its tests gave a verdict. Issue #93: a server that never came up used to read "SURVIVED or did not run", like a real survivor.
# Generated: every exit status contract.sh or cargo can give (and one neither does) x every shape of report (none, no failure, failures,
# errors; no test lines, `... FAILED` lines, a --nocapture `test result: FAILED` line) x no needle, a needle the log says, one it never
# says, against the table below. Then the reasons a run stopped, and mutants.sh's use of the library.
#
#   test-mutants-verdict.sh [path to a copy of mutants-verdict.sh]   (a path points it at a deliberately broken copy)
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
LIB="${1:-$HERE/scripts/lib/mutants-verdict.sh}"
MUTANTS="$HERE/scripts/mutants.sh"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
pass=0; failn=0; n=0
ok() { pass=$((pass + 1)); }
bad() { failn=$((failn + 1)); echo "BAD  $*"; }
NEEDLE="the aimed assertion fired"

# check <label> <want verdict> <want text in the detail or ''> <function> <args...>
check() {
  label="$1"; want="$2"; text="$3"; shift 3
  n=$((n + 1))
  got="$(sh -c '. "$1"; shift; "$@"' _ "$LIB" "$@" 2>&1)"
  v="${got%%|*}"
  if [ "$v" != "$want" ]; then bad "$label: '$got', wanted $want"; return; fi
  if [ -n "$text" ] && ! printf '%s' "$got" | grep -qF -- "$text"; then bad "$label: '$got' lacks '$text'"; return; fi
  ok
}

# The table, written as the cases it decides (not as the library's own branches):
#   tests ran, all passed           -> exit 0                                          -> SURVIVED
#   tests ran, some failed          -> suite: exit 100 and the report counts >= 1     -> KILLED, or UNCLEAR when the needle is missing
#                                      cargo: exit 101 and a failed test in the log
#   anything else                                                                     -> DID NOT RUN
want_for() {
  # $1 exit status, $2 failed tests the run's own report counts, $3 the code that means "a test failed", $4 needle mode
  if [ "$1" = 0 ]; then echo SURVIVED
  elif [ "$1" = "$3" ] && [ "$2" -ge 1 ]; then if [ "$4" = absent ]; then echo UNCLEAR; else echo KILLED; fi
  else echo "DID NOT RUN"; fi
}

# 1. rows of the contract suite (nextest; a JUnit report beside the log)
for status in 0 1 2 3 4 5 100 101 137; do
  for report in none f0e0 f2e0 f0e1 f1e1; do
    for nm in none present absent; do
      d="$T/suite-$status-$report-$nm"; mkdir -p "$d"; log="$d/native-memory-main.test.log"
      printf '        FAIL [   0.1s] thrust-contract-tests::contract some_test\n' >"$log"
      [ "$nm" = present ] && echo "panicked: $NEEDLE" >>"$log"
      failed=0
      case "$report" in
        none) ;;
        f*) f="${report#f}"; f="${f%e*}"; e="${report#*e}"; failed=$((f + e))
            printf '<?xml version="1.0"?>\n<testsuites name="contract" tests="70" failures="%s" errors="%s" timestamp="x">\n</testsuites>\n' "$f" "$e" \
              >"$d/native-memory-main.junit.xml" ;;
      esac
      needle=""; [ "$nm" != none ] && needle="$NEEDLE"
      want="$(want_for "$status" "$failed" 100 "$nm")"
      text=""; [ "$want" = KILLED ] && text="$failed test(s) failed"
      if [ -n "$needle" ]; then check "suite exit $status report $report needle $nm" "$want" "$text" verdict_suite "$status" "$log" "$needle"
      else check "suite exit $status report $report" "$want" "$text" verdict_suite "$status" "$log"; fi
    done
  done
done

# 2. rows of an adapter's own tests (cargo test; the log only)
for status in 0 1 101 137; do
  for shape in none failed-lines nocapture ok-line; do
    for nm in none present absent; do
      log="$T/cargo-$status-$shape-$nm.log"
      case "$shape" in
        none) printf 'error[E0425]: cannot find value `x` in this scope\n' >"$log"; failed=0 ;;
        failed-lines) printf 'test a ... ok\ntest b ... FAILED\ntest c ... FAILED\n' >"$log"; failed=2 ;;
        nocapture) printf 'test a ... \nthread panicked\nFAILED\ntest result: FAILED. 3 passed; 1 failed; 0 ignored\n' >"$log"; failed=1 ;;
        ok-line) printf 'test a ... ok\ntest result: ok. 1 passed; 0 failed\n' >"$log"; failed=0 ;;
      esac
      [ "$nm" = present ] && echo "assertion: $NEEDLE" >>"$log"
      needle=""; [ "$nm" != none ] && needle="$NEEDLE"
      want="$(want_for "$status" "$failed" 101 "$nm")"
      text=""; [ "$want" = KILLED ] && text="$failed test(s) failed"
      if [ -n "$needle" ]; then check "cargo exit $status $shape needle $nm" "$want" "$text" verdict_cargo "$status" "$log" "$needle"
      else check "cargo exit $status $shape" "$want" "$text" verdict_cargo "$status" "$log"; fi
    done
  done
done

# 3. a run that stopped says why, by contract.sh's exit status
log="$T/reason.test.log"; : >"$log"
check "exit 2" "DID NOT RUN" "precondition" verdict_suite 2 "$log"
check "exit 3" "DID NOT RUN" "setup failed" verdict_suite 3 "$log"
check "exit 4" "DID NOT RUN" "never came up (exit 4)" verdict_suite 4 "$log"
check "exit 5" "DID NOT RUN" "vacuity guard" verdict_suite 5 "$log"
check "exit 101" "DID NOT RUN" "did not build" verdict_suite 101 "$log"
check "exit 100, no report" "DID NOT RUN" "no report counts" verdict_suite 100 "$log"
check "exit 137" "DID NOT RUN" "exit 137" verdict_suite 137 "$log"
check "missing log" "DID NOT RUN" "never came up" verdict_suite 4 "$T/no-such.test.log"
check "cargo build failed" "DID NOT RUN" "did not build" verdict_cargo 101 "$T/cargo-101-none-none.log"

# 4. mutants.sh: every row goes through the library, the old merged wording is gone, a non-kill fails the script, and the summary ends in counts
n=$((n + 1))
if grep -qF '. "$ROOT/scripts/lib/mutants-verdict.sh"' "$MUTANTS"; then ok; else bad "mutants.sh must source scripts/lib/mutants-verdict.sh"; fi
n=$((n + 1))
if grep -q 'SURVIVED or did not run' "$MUTANTS"; then bad "mutants.sh still says 'SURVIVED or did not run'"; else ok; fi
for fn in kill_check kill_check_msg kill_check_suite kill_check_suite_msg; do
  n=$((n + 1))
  body="$(sed -n "s/^$fn() { \(.*\) }\$/\1/p" "$MUTANTS")"
  case "$fn" in kill_check_suite*) lib=verdict_suite ;; *) lib=verdict_cargo ;; esac
  case "$body" in
    "record "*"\$($lib "*) ok ;;
    *) bad "$fn must hand its row to record with $lib (found: '$body')" ;;
  esac
done
n=$((n + 1))
if grep -qx 'exit "$NOT_KILLED"' "$MUTANTS" && grep -q 'NOT_KILLED=1' "$MUTANTS"; then ok; else bad "mutants.sh must exit \$NOT_KILLED, set when a row is not killed"; fi
n=$((n + 1))
if grep -qF 'killed=%s survived=%s unclear=%s did-not-run=%s' "$MUTANTS"; then ok; else bad "the summary must end in the counts of each verdict"; fi

echo "passed=$pass bad=$failn (of $n)"
[ "$failn" -eq 0 ] && [ "$pass" = "$n" ]
