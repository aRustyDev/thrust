#!/bin/sh
# The acceptance-list check (issue #51, F-X13 as revised in round 29).
#
#   scripts/check-acceptance-list.sh
#   scripts/check-acceptance-list.sh --self-test    run the check against trees it must accept and trees it must refuse
#
# The compat plane's replay and oracle obligation is the acceptance list: the 48-route slice (S01 to S48), the three rows that open
# the created work item (X01 to X03) and X04. The list is a file (crates/thrust-compat-plane/tests/parity/acceptance.csv) written by
# tools/compat-record/gen-acceptance.py from research/0003 section 8.1. This holds the rest to it, each as `comm -3` of two sorted
# lists that must print nothing:
#   * the rows the list says are `served`  and  the rows that have a fixture (a fixture is a row's recording from the pin);
#   * the rows the list says are `served`  and  the rows tools/compat-record/record.py records (the oracle's route set);
#   * the fixtures' names  and  the recorder's step names (a variant such as S04-existing is a step of its own);
#   * and no `pending` row has a fixture (a row that is not served is not recorded).
# The replay itself is held to the fixtures by `parity_every_fixture_has_a_test` in tests/parity.rs. It also runs
# `gen-acceptance.py --check`: the file is what the generator writes, and every route in it is a route of the pinned Plane.
#
# Exit: 0 clean; 1 a list differs; 2 a list that cannot vouch for itself (missing or empty): comparing nothing with nothing
# passes vacuously, which is not evidence. `--self-test` is the control, run next to the real check in CI.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PARITY="$ROOT/crates/thrust-compat-plane/tests/parity"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# compare LABEL A B: `comm -3` of two sorted files; returns 1 and prints the lines only one side has, if there are any.
compare() {
  # every variable here is prefixed: a POSIX shell function shares its variables with its caller, and an unprefixed `label` here
  # overwrote the self-test's, so every case was reported under the last comparison's name
  cmp_label="$1"
  cmp_differ="$(comm -3 "$2" "$3")"
  if [ -z "$cmp_differ" ]; then
    echo "ok: $cmp_label"
    return 0
  fi
  echo "FAIL: $cmp_label differ (a line with no indent is only on the left, one with an indent only on the right):"
  echo "$cmp_differ" | sed 's/^/    /'
  return 1
}

# check_lists ACCEPT FIXTURES_DIR ROWS_FILE: ROWS_FILE holds the recorder's step names, one per line.
check_lists() {
  accept="$1"
  fixtures="$2"
  rows="$3"
  for f in "$accept" "$rows"; do
    if [ ! -s "$f" ]; then
      echo "ERROR: $f is missing or empty: a comparison with nothing passes vacuously"
      return 2
    fi
  done
  if [ ! -d "$fixtures" ]; then
    echo "ERROR: $fixtures is not a directory"
    return 2
  fi
  work="$TMP/work"
  rm -rf "$work"
  mkdir -p "$work"
  awk -F, 'NR > 1 && $4 == "served" { print $1 }' "$accept" | LC_ALL=C sort -u >"$work/served"
  awk -F, 'NR > 1 && $4 != "served" { print $1 }' "$accept" | LC_ALL=C sort -u >"$work/pending"
  find "$fixtures" -maxdepth 1 -name '*.json' | sed 's|.*/||; s|\.json$||' | LC_ALL=C sort -u >"$work/fixture-steps"
  LC_ALL=C sort -u "$rows" >"$work/recorder-steps"
  sed 's/-.*//' "$work/fixture-steps" | LC_ALL=C sort -u >"$work/fixture-rows"
  sed 's/-.*//' "$work/recorder-steps" | LC_ALL=C sort -u >"$work/recorder-rows"
  for f in served fixture-steps recorder-steps; do
    if [ ! -s "$work/$f" ]; then
      echo "ERROR: the list of $f is empty: a comparison with nothing passes vacuously"
      return 2
    fi
  done
  status=0
  compare "the served rows and the fixtures' rows" "$work/served" "$work/fixture-rows" || status=1
  compare "the served rows and the recorder's rows" "$work/served" "$work/recorder-rows" || status=1
  compare "the fixtures' names and the recorder's step names" "$work/fixture-steps" "$work/recorder-steps" || status=1
  shared="$(comm -12 "$work/pending" "$work/fixture-rows")"
  if [ -n "$shared" ]; then
    echo "FAIL: a pending row has a fixture, so it is served: $shared"
    status=1
  else
    echo "ok: no pending row has a fixture ($(wc -l <"$work/pending" | tr -d ' ') pending)"
  fi
  return "$status"
}

self_test() {
  failed=0
  # expect WANT LABEL: the lists written below must give exactly WANT
  expect() {
    t_want="$1"
    t_label="$2"
    check_lists "$T/accept.csv" "$T/fixtures" "$T/rows" >/dev/null 2>&1
    t_got=$?
    if [ "$t_got" -eq "$t_want" ]; then
      echo "ok: $t_label (exit $t_got)"
    else
      echo "SELF-TEST FAILED: $t_label: wanted exit $t_want, got $t_got"
      failed=1
    fi
  }
  # fresh: a consistent world, to be broken one way at a time
  fresh() {
    T="$TMP/self"
    rm -rf "$T"
    mkdir -p "$T/fixtures"
    printf 'row,method,path,state\nS01,GET,/a/,served\nS02,GET,/b/,served\nS03,GET,/c/,pending-x\n' >"$T/accept.csv"
    : >"$T/fixtures/S01.json"
    : >"$T/fixtures/S02.json"
    : >"$T/fixtures/S02-after.json"
    printf 'S01\nS02\nS02-after\n' >"$T/rows"
  }

  fresh
  expect 0 "a consistent world is accepted"
  fresh
  : >"$T/fixtures/S04.json"
  expect 1 "a fixture for a row that is not on the list is refused"
  fresh
  rm "$T/fixtures/S02.json" "$T/fixtures/S02-after.json"
  expect 1 "a served row without a fixture is refused"
  fresh
  printf 'S01\nS02\n' >"$T/rows"
  expect 1 "a fixture the recorder does not record is refused"
  fresh
  printf 'S01\nS02\nS02-after\nS09\n' >"$T/rows"
  expect 1 "a step the recorder takes for a row that is not on the list is refused"
  fresh
  : >"$T/fixtures/S03.json"
  expect 1 "a pending row that has a fixture is refused"
  fresh
  printf 'row,method,path,state\nS01,GET,/a/,served\nS02,GET,/b/,served\nS05,GET,/e/,served\n' >"$T/accept.csv"
  expect 1 "a served row that nothing records is refused"
  fresh
  printf 'row,method,path,state\n' >"$T/accept.csv"
  expect 2 "a list with no rows is not a pass"
  fresh
  rm "$T/fixtures/"*.json
  expect 2 "a directory with no fixtures is not a pass"
  fresh
  : >"$T/rows"
  expect 2 "a recorder that lists nothing is not a pass"
  fresh
  rm -rf "$T/fixtures"
  expect 2 "a missing fixtures directory is not a pass"

  python3 -I "$ROOT/tools/compat-record/gen-acceptance.py" --self-test || failed=1
  if [ "$failed" -eq 0 ]; then
    echo "self-test passed"
  fi
  return "$failed"
}

case "${1:-}" in
  --self-test)
    self_test
    exit $?
    ;;
  *)
    python3 -I "$ROOT/tools/compat-record/gen-acceptance.py" --check || exit 1
    python3 -I "$ROOT/tools/compat-record/record.py" --list-rows >"$TMP/recorder-rows" || {
      echo "ERROR: record.py --list-rows failed"
      exit 2
    }
    check_lists "$PARITY/acceptance.csv" "$PARITY/fixtures" "$TMP/recorder-rows"
    exit $?
    ;;
esac
