#!/bin/sh
# The expected test counts of the contract suite's passes, checked against the suite (issue #58).
#
#   scripts/check-contract-counts.sh              the check on the real scripts, and a control that shows it can fail
#   scripts/check-contract-counts.sh --self-test  the check on worlds it must accept and worlds it must refuse (no compile)
#
# `scripts/contract.sh` and `scripts/stage2-contract.sh` each run a pass as `cargo test ... -- <filters>` and exit 5 ("VACUITY GUARD")
# when the number of tests that passed is not the number it expects (`EXPECT_*` in contract.sh, `EXPECT=` in each arm of
# stage2-contract.sh). A count goes stale whenever a test is added, removed or renamed, and a stale one fails only when the script
# is run: contract.sh's `main`, `sweeper`, `csrfoff`, `panic`, `worker`, `cron` and `bindings` run in CI cells, but `blast`, `budget`
# and `engine` run nowhere in CI, and stage2-contract.sh needs a deployed Worker. It was 59 until 2026-11-14 while the list had been
# 61 since #151, and #169's three tests made it 64 the same day; nothing noticed, and a correct run would have exited 5.
#
# This asks the suite itself, with no server and no environment: for each arm it runs the arm's own filters through libtest,
# `cargo test -p thrust-contract-tests --test <bin> -- --list --format terse <filters>`, counts the lines that end in `: test`, and
# compares the number with the arm's. It does not copy a filter, so a filter cannot drift from its count.
#
# It fails closed. Exit 0: every arm's count is the suite's. Exit 1: a count differs. Exit 2: it cannot vouch for itself, which is any
# of: an arm it cannot read, a different number of arms read than of arms written, no arm at all, a count that is not a number, an
# `EXPECT_` variable with no value (or two), a listing that fails, or a filter that selects nothing (libtest prints nothing and exits 0
# for that, which is not a count of anything).
#
# The control (AGENTS.md rule 8, "a check that cannot fail is not evidence"): the same check on a copy of stage2-contract.sh whose
# `main` count is wrong must exit 1. `--self-test` is the cases: a world that agrees, a count one too high and one too low, a filter
# that selects nothing, a listing that fails, a script with nothing to read, an arm it cannot read, a variable with no value, the
# default arm of contract.sh read as `main`. POSIX sh: CI runs it under dash.
set -u
set -f
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
STAGE2="$ROOT/scripts/stage2-contract.sh"
CONTRACT="$ROOT/scripts/contract.sh"

# --- reading the scripts ------------------------------------------------------------------------------------------------------

# arms_of_stage2 FILE: one line per arm, `pass|expected|bin|filters`, from lines like
#   `  main)     EXPECT=64; "$RS" cargo test --manifest-path "$M" -p thrust-contract-tests --test contract -- <filters> >"$LOG.test.log" 2>&1 ;;`
arms_of_stage2() {
  sed -n 's/^ *\([a-z0-9]*\)) *EXPECT=\([0-9][0-9]*\); *"\$RS" cargo test --manifest-path "\$M" -p thrust-contract-tests --test \([a-z_]*\) -- \(.*\) >"\$LOG\.test\.log" 2>&1 ;;$/\1|\2|\3|\4/p' "$1"
}

# how many arms stage2-contract.sh has: the lines (comments left out) that run a test of the suite
written_in_stage2() {
  grep -v '^ *#' "$1" | grep -c -- '-p thrust-contract-tests --test '
}

# arms_of_contract FILE: one line per arm that has a count, `pass|variable|bin|filters`, from
#   `  <pass>)` / `    "$ROOT/scripts/rs" cargo test ... --test <bin> -- <filters> >"$LOG.test.log" 2>&1` / `    EXPECT=$EXPECT_<VARIABLE>`
# The default arm, `*)`, is the `main` pass.
arms_of_contract() {
  acn_arm=""
  acn_bin=""
  acn_args=""
  while IFS= read -r acn_line; do
    case "$acn_line" in
      "  "[a-z]*")" | "  *)")
        acn_arm="${acn_line#  }"
        acn_arm="${acn_arm%)}"
        [ "$acn_arm" = '*' ] && acn_arm=main
        acn_bin=""
        acn_args=""
        ;;
      *"-p thrust-contract-tests --test "*)
        acn_rest="${acn_line#*-p thrust-contract-tests --test }"
        acn_bin="${acn_rest%% *}"
        acn_rest="${acn_rest#* -- }"
        acn_args="${acn_rest%% >*}"
        ;;
      "    EXPECT=\$EXPECT_"*)
        printf '%s|%s|%s|%s\n' "$acn_arm" "${acn_line#    EXPECT=\$EXPECT_}" "$acn_bin" "$acn_args"
        ;;
    esac
  done < "$1"
}

written_in_contract() {
  grep -v '^ *#' "$1" | grep -c '^    EXPECT=\$EXPECT_'
}

# value_of_variable FILE NAME: the number `EXPECT_NAME=<number>` gives in contract.sh; empty if there is not exactly one
value_of_variable() {
  vov_all="$(sed -n "s/^EXPECT_$2=\([0-9][0-9]*\)\$/\1/p" "$1")"
  case "$vov_all" in
    "" | *"
"*) ;;
    *) printf '%s' "$vov_all" ;;
  esac
}

# --- asking the suite ---------------------------------------------------------------------------------------------------------

# real_lister BIN FILTERS...: libtest's own list of the tests of one binary of the suite that these filters select
real_lister() {
  rl_bin="$1"
  shift
  "$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-contract-tests --test "$rl_bin" -- --list --format terse "$@"
}

LISTER=real_lister

# selected BIN FILTERS: how many tests those filters select, or a failure of the listing (exit status 2 and a message)
selected() {
  sel_out="$($LISTER "$@" < /dev/null 2> "$W/list.err")" || {
    echo "cannot vouch: the listing failed for: $*"
    tail -n 15 "$W/list.err"
    return 2
  }
  printf '%s\n' "$sel_out" | grep -c ': test$'
  return 0
}

# --- the check ----------------------------------------------------------------------------------------------------------------

# check_kind KIND FILE [ONLY]: every arm of the script (KIND is `stage2` or `contract`), or only the pass ONLY. Prints a line per
# arm. Exit status 0, 1 (a count differs) or 2 (it cannot vouch).
check_kind() {
  ck_kind="$1"
  ck_file="$2"
  ck_only="${3:-}"
  [ -r "$ck_file" ] || { echo "cannot vouch: $ck_file is not readable"; return 2; }
  if [ "$ck_kind" = stage2 ]; then
    arms_of_stage2 "$ck_file" > "$W/arms"
    ck_written="$(written_in_stage2 "$ck_file")"
  else
    arms_of_contract "$ck_file" > "$W/arms"
    ck_written="$(written_in_contract "$ck_file")"
  fi
  ck_read="$(wc -l < "$W/arms" | tr -d ' ')"
  if [ "$ck_read" -eq 0 ]; then
    echo "cannot vouch: no arm of $ck_file could be read (did its format change?)"
    return 2
  fi
  if [ "$ck_read" -ne "$ck_written" ]; then
    echo "cannot vouch: $ck_file has $ck_written arms that run a test of the suite and $ck_read could be read"
    return 2
  fi
  ck_status=0
  ck_seen=0
  while IFS='|' read -r ck_pass ck_expect ck_bin ck_args; do
    [ -z "$ck_only" ] || [ "$ck_only" = "$ck_pass" ] || continue
    ck_seen=$((ck_seen + 1))
    if [ "$ck_kind" = contract ]; then
      ck_expect="$(value_of_variable "$ck_file" "$ck_expect")"
    fi
    case "$ck_expect" in
      "" | *[!0-9]*) echo "cannot vouch: $ck_kind $ck_pass: its count is not a number, or its variable has no single value"; return 2 ;;
    esac
    # shellcheck disable=SC2086 # the filters are words: `--skip x`, `--test-threads=1`
    ck_got="$(selected "$ck_bin" $ck_args)"
    ck_rc=$?
    if [ "$ck_rc" -ne 0 ]; then
      printf '%s\n' "$ck_got"
      return 2
    fi
    if [ "$ck_got" -eq 0 ]; then
      echo "cannot vouch: $ck_kind $ck_pass: the filters select nothing ($ck_bin -- $ck_args)"
      return 2
    fi
    if [ "$ck_got" -eq "$ck_expect" ]; then
      echo "ok   $ck_kind $ck_pass: $ck_got tests"
    else
      echo "FAIL $ck_kind $ck_pass: its filters select $ck_got tests and the script expects $ck_expect (${ck_file#"$ROOT"/})"
      ck_status=1
    fi
  done < "$W/arms"
  if [ "$ck_seen" -eq 0 ]; then
    echo "cannot vouch: $ck_kind has no pass named '$ck_only'"
    return 2
  fi
  return "$ck_status"
}

# --- the control, on the real script ------------------------------------------------------------------------------------------

# control: stage2-contract.sh with the count of `main` made wrong must be refused (exit 1, and `FAIL stage2 main`)
control() {
  sed 's/^\( *main) *EXPECT=\)[0-9][0-9]*;/\19999;/' "$STAGE2" > "$W/doctored.sh"
  if cmp -s "$STAGE2" "$W/doctored.sh"; then
    echo "cannot vouch: the control did not change the script (is the main arm still '  main) EXPECT=<n>;'?)"
    return 2
  fi
  ctl_out="$(check_kind stage2 "$W/doctored.sh" main)"
  ctl_rc=$?
  case "$ctl_rc:$ctl_out" in
    1:*"FAIL stage2 main"*) echo "ok   control: a stage2 count that is wrong is refused" ;;
    *) echo "cannot vouch: the control: a stage2 count that is wrong was not refused (exit $ctl_rc)"; printf '%s\n' "$ctl_out"; return 2 ;;
  esac
}

# --- the cases ----------------------------------------------------------------------------------------------------------------

# fake_lister BIN FILTERS...: as many lines as the world says (`$W/world`: `bin filters|count`), none if the world does not say
# it, a failure for the count `fail`
fake_lister() {
  fl_key="$*"
  fl_n="$(sed -n "s/^$(printf '%s' "$fl_key" | sed 's/[][\.*^$/]/\\&/g')|\\(.*\\)\$/\\1/p" "$W/world" | head -n 1)"
  [ "$fl_n" = fail ] && return 1
  fl_i=0
  while [ "$fl_i" -lt "${fl_n:-0}" ]; do
    fl_i=$((fl_i + 1))
    echo "t::t$fl_i: test"
  done
  echo
  echo "${fl_n:-0} tests, 0 benchmarks"
}

STAGE2_FIXTURE='case "$PASS" in
  main)     EXPECT=5; "$RS" cargo test --manifest-path "$M" -p thrust-contract-tests --test contract -- --skip a_ --skip b_ >"$LOG.test.log" 2>&1 ;;
  worker)   EXPECT=1;  "$RS" cargo test --manifest-path "$M" -p thrust-contract-tests --test worker_only -- worker_room_ --test-threads=1 --nocapture >"$LOG.test.log" 2>&1 ;;
  engine)   EXPECT=4;  "$RS" cargo test --manifest-path "$M" -p thrust-contract-tests --test engine -- --test-threads=1 --nocapture >"$LOG.test.log" 2>&1 ;;
  *) echo "unknown pass $PASS" >&2; exit 2 ;;
esac'

CONTRACT_FIXTURE='EXPECT_MAIN=5
EXPECT_SWEEPER=3
case "$PASS" in
  sweeper)
    "$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-contract-tests --test contract -- sweeper_ --test-threads=1 --nocapture >"$LOG.test.log" 2>&1
    TEST_STATUS=$?
    EXPECT=$EXPECT_SWEEPER
    ;;
  bench)
    "$ROOT/scripts/rs" cargo run --quiet --release --manifest-path "$ROOT/Cargo.toml" -p thrust-contract-tests --bin bench -- 200 1 >"$LOG.test.log" 2>&1
    exit 0
    ;;
  *)
    "$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-contract-tests --test contract -- --skip sweeper_ >"$LOG.test.log" 2>&1
    TEST_STATUS=$?
    EXPECT=$EXPECT_MAIN
    ;;
esac'

# the world in which both fixtures agree with the suite
AGREEING_WORLD='contract --skip a_ --skip b_|5
worker_only worker_room_ --test-threads=1 --nocapture|1
engine --test-threads=1 --nocapture|4
contract sweeper_ --test-threads=1 --nocapture|3
contract --skip sweeper_|5'

# case WANT LABEL KIND FIXTURE WORLD [ONLY]: check_kind on a fixture in a world, and what exit status it must give
self_case() {
  sc_want="$1"
  sc_label="$2"
  printf '%s\n' "$4" > "$W/fixture.sh"
  printf '%s\n' "$5" > "$W/world"
  sc_out="$(check_kind "$3" "$W/fixture.sh" "${6:-}")"
  sc_rc=$?
  if [ "$sc_rc" -eq "$sc_want" ]; then
    echo "ok   $sc_label (exit $sc_rc)"
    # CHECK_VERBOSE=1: what each case said, to read that it was refused for the reason its label gives
    [ -z "${CHECK_VERBOSE:-}" ] || printf '%s\n' "$sc_out" | sed 's/^/       | /'
  else
    echo "FAIL $sc_label: exit $sc_rc, wanted $sc_want"
    printf '%s\n' "$sc_out"
    SC_FAILED=$((SC_FAILED + 1))
  fi
}

self_test() {
  LISTER=fake_lister
  SC_FAILED=0
  self_case 0 "stage2: every count agrees with the suite" stage2 "$STAGE2_FIXTURE" "$AGREEING_WORLD"
  self_case 0 "contract: every count agrees, and the default arm is main" contract "$CONTRACT_FIXTURE" "$AGREEING_WORLD"
  self_case 1 "stage2: a count one too low is refused" stage2 "$STAGE2_FIXTURE" "$(printf '%s\n' "$AGREEING_WORLD" | sed 's/^contract --skip a_ --skip b_|5$/contract --skip a_ --skip b_|6/')"
  self_case 1 "stage2: a count one too high is refused" stage2 "$STAGE2_FIXTURE" "$(printf '%s\n' "$AGREEING_WORLD" | sed 's/^engine --test-threads=1 --nocapture|4$/engine --test-threads=1 --nocapture|3/')"
  self_case 1 "contract: a count that differs is refused" contract "$CONTRACT_FIXTURE" "$(printf '%s\n' "$AGREEING_WORLD" | sed 's/^contract sweeper_ --test-threads=1 --nocapture|3$/contract sweeper_ --test-threads=1 --nocapture|4/')"
  self_case 2 "stage2: a filter that selects nothing is not a count" stage2 "$STAGE2_FIXTURE" "$(printf '%s\n' "$AGREEING_WORLD" | sed 's/^worker_only worker_room_ --test-threads=1 --nocapture|1$/worker_only worker_room_ --test-threads=1 --nocapture|0/')"
  self_case 2 "stage2: a listing that fails cannot vouch" stage2 "$STAGE2_FIXTURE" "$(printf '%s\n' "$AGREEING_WORLD" | sed 's/^engine --test-threads=1 --nocapture|4$/engine --test-threads=1 --nocapture|fail/')"
  self_case 2 "stage2: a script with no arm to read cannot vouch" stage2 'echo nothing to see' "$AGREEING_WORLD"
  self_case 2 "stage2: an arm that cannot be read (its count is not a number) cannot vouch" stage2 "$(printf '%s\n' "$STAGE2_FIXTURE" | sed 's/EXPECT=5;/EXPECT=five;/')" "$AGREEING_WORLD"
  self_case 2 "stage2: an arm with no count cannot vouch" stage2 "$(printf '%s\n' "$STAGE2_FIXTURE" | sed 's/EXPECT=1;  //')" "$AGREEING_WORLD"
  self_case 2 "contract: a variable that has no value cannot vouch" contract "$(printf '%s\n' "$CONTRACT_FIXTURE" | sed '/^EXPECT_SWEEPER=3$/d')" "$AGREEING_WORLD"
  self_case 2 "contract: a variable that has two values cannot vouch" contract "$(printf '%s\n' "$CONTRACT_FIXTURE" | sed 's/^EXPECT_MAIN=5$/EXPECT_MAIN=5\
EXPECT_MAIN=6/')" "$AGREEING_WORLD"
  self_case 2 "contract: a script with no arm to read cannot vouch" contract 'echo nothing to see' "$AGREEING_WORLD"
  self_case 1 "only one pass is checked when asked: a wrong main is refused" stage2 "$STAGE2_FIXTURE" "$(printf '%s\n' "$AGREEING_WORLD" | sed 's/^contract --skip a_ --skip b_|5$/contract --skip a_ --skip b_|9/')" main
  self_case 0 "only one pass is checked when asked: a wrong main is not seen from worker" stage2 "$STAGE2_FIXTURE" "$(printf '%s\n' "$AGREEING_WORLD" | sed 's/^contract --skip a_ --skip b_|5$/contract --skip a_ --skip b_|9/')" worker
  self_case 2 "only one pass is checked when asked: a pass that is not there cannot vouch" stage2 "$STAGE2_FIXTURE" "$AGREEING_WORLD" nosuchpass
  [ "$SC_FAILED" -eq 0 ] || { echo "$SC_FAILED case(s) failed"; return 1; }
  return 0
}

# --- the real run -------------------------------------------------------------------------------------------------------------

real_run() {
  status=0
  check_kind stage2 "$STAGE2"
  rc=$?
  [ "$rc" -ne 2 ] || return 2
  [ "$rc" -eq 0 ] || status=1
  check_kind contract "$CONTRACT"
  rc=$?
  [ "$rc" -ne 2 ] || return 2
  [ "$rc" -eq 0 ] || status=1
  control || return 2
  return "$status"
}

W="$(mktemp -d /tmp/check-contract-counts.XXXXXX)" || exit 2
trap 'rm -rf "$W"' EXIT
case "${1:-}" in
  "") real_run ;;
  --self-test) self_test ;;
  *) echo "usage: scripts/check-contract-counts.sh [--self-test]" >&2; exit 2 ;;
esac
exit $?
