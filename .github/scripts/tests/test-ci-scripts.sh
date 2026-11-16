#!/bin/sh
# Exercise plan-cells.sh and assert-gate.sh, including the cases that must FAIL. (plan-ci.py, the planner between them, is
# tested by test_plan_ci.py.)
# Every case goes through `expect`, which checks the exit status AND a message fragment, so a case cannot
# pass for the wrong reason and none is merely printed.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
S="$HERE/.github/scripts"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
pass=0; failn=0
expect() { # expect <label> <want-exit> <want-fragment> <cmd...>
  label="$1"; want="$2"; frag="$3"; shift 3
  "$@" > "$T/last.out" 2>&1
  got=$?
  if [ "$got" = "$want" ] && grep -qF -- "$frag" "$T/last.out"; then
    pass=$((pass + 1)); echo "ok   $label (exit $got; saw '$frag')"
  else
    failn=$((failn + 1)); echo "BAD  $label: wanted exit $want + '$frag', got $got"; sed 's/^/     | /' "$T/last.out"
  fi
}

# ---- plan-cells.sh -------------------------------------------------------------------------------
plan_in() { # plan_in <dir>   (runs plan-cells.sh against <dir>/scripts/contract-all.sh)
  ( cd "$1" && "$S/plan-cells.sh" )
}
mkdir -p "$T/plan/scripts" && cp "$HERE/scripts/contract-all.sh" "$T/plan/scripts/contract-all.sh"
plan_real() { # every `run <target> <pass>` line of the real script must become exactly one cell
  plan_in "$T/plan" > "$T/plan/out.txt" || return 1
  n_cells="$(wc -l < "$T/plan/cells.txt" | tr -d ' ')"
  n_runs="$(grep -cE '^run[[:space:]]' "$HERE/scripts/contract-all.sh")"
  [ "$n_cells" -ge 1 ] && [ "$n_cells" = "$n_runs" ] && grep -q '^matrix=' "$T/plan/out.txt" && grep -q '^cells=' "$T/plan/out.txt" \
    && echo "plan ok: every run line became a cell"
}
expect "plan: every run line of the real contract-all.sh becomes a cell" 0 "plan ok" plan_real
# The v1 oracle is a CI cell on each Thrust target it was accepted on (#58). The control: the same plan without those lines must fail.
plan_has_oracle() { # plan_has_oracle <cells.txt>
  for t in native-memory native-pg worker-cf; do
    grep -qx "$t oracle" "$1" || { echo "the plan lacks the $t oracle cell"; return 1; }
  done
  echo "plan ok: the oracle cells are planned"
}
expect "plan: the oracle cells are in the real plan" 0 "plan ok: the oracle cells" plan_has_oracle "$T/plan/cells.txt"
mkdir -p "$T/plan2/scripts" && grep -v ' oracle$' "$HERE/scripts/contract-all.sh" > "$T/plan2/scripts/contract-all.sh"
plan_in "$T/plan2" > /dev/null
expect "plan: a plan without the oracle cells is refused" 1 "the plan lacks the native-memory oracle cell" plan_has_oracle "$T/plan2/cells.txt"
# The first browser journey is a CI cell on the Worker with the Cloudflare set (#44). The control: the same plan without that line must fail.
plan_has_e2e() { # plan_has_e2e <cells.txt>
  grep -qx "worker-cf e2e" "$1" || { echo "the plan lacks the worker-cf e2e cell"; return 1; }
  echo "plan ok: the e2e cell is planned"
}
expect "plan: the e2e cell is in the real plan" 0 "plan ok: the e2e cell" plan_has_e2e "$T/plan/cells.txt"
mkdir -p "$T/plan3/scripts" && grep -v ' e2e$' "$HERE/scripts/contract-all.sh" > "$T/plan3/scripts/contract-all.sh"
plan_in "$T/plan3" > /dev/null
expect "plan: a plan without the e2e cell is refused" 1 "the plan lacks the worker-cf e2e cell" plan_has_e2e "$T/plan3/cells.txt"
mkdir -p "$T/plan0/scripts" && printf '#!/bin/sh\necho nothing\n' > "$T/plan0/scripts/contract-all.sh"
expect "plan: a script with no run lines fails" 1 "parsed zero cells" plan_in "$T/plan0"
mkdir -p "$T/plan1/scripts" && cp "$HERE/scripts/contract-all.sh" "$T/plan1/scripts/contract-all.sh" && printf 'run native-memory main extra\n' >> "$T/plan1/scripts/contract-all.sh"
expect "plan: a run line with a changed format fails" 1 "the format changed" plan_in "$T/plan1"

# ---- assert-gate.sh ------------------------------------------------------------------------------
JOBS="ci-scripts deps lint deny hermetic pg-adapter contract mutants web docs"
gate() { # gate <dir> <needs-json> <expected-jobs-json> <planned-json> <reported markers...>
  d="$T/gate/$1"; mkdir -p "$d/reported"; needs="$2"; expected="$3"; planned="$4"; shift 4
  for m in "$@"; do printf '%s\n' "$m" > "$d/reported/$(echo "$m" | tr ' ' '-').ok"; done
  ( cd "$d" && NEEDS_JSON="$needs" EXPECTED_JOBS="$expected" PLANNED="$planned" REPORTED_DIR="$d/reported" "$S/assert-gate.sh" )
}
# needs <changes result> <job=result>...: the gate's needs, every job of $JOBS present; a job not named is skipped
needs() {
  ch="$1"; shift
  printf '{"changes":{"result":"%s"}' "$ch"
  for j in $JOBS; do
    r=skipped
    for kv in "$@"; do [ "${kv%%=*}" = "$j" ] && r="${kv#*=}"; done
    printf ',"%s":{"result":"%s"}' "$j" "$r"
  done
  printf '}'
}
P='["native-memory main","worker-cf bindings"]'
M1="native-memory main"; M2="worker-cf bindings"

# Every job, expected or not, with every result: the gate passes exactly when an expected job succeeded or an unexpected one was
# skipped. The other jobs are in a state that passes (contract expected and green, so the cell rules hold), so the case can only
# fail because of the job under test.
for j in $JOBS; do
  for exp in yes no; do
    for r in success skipped failure cancelled; do
      if [ "$j" = contract ] && [ "$exp" = no ]; then
        n="$(needs success "contract=$r")"; e='[]'; pl='[]'; set --
      elif [ "$j" = contract ]; then
        n="$(needs success "contract=$r")"; e='["contract"]'; pl="$P"; set -- "$M1" "$M2"
      elif [ "$exp" = yes ]; then
        n="$(needs success "contract=success" "$j=$r")"; e="[\"contract\",\"$j\"]"; pl="$P"; set -- "$M1" "$M2"
      else
        n="$(needs success "contract=success" "$j=$r")"; e='["contract"]'; pl="$P"; set -- "$M1" "$M2"
      fi
      if [ "$exp" = yes ] && [ "$r" = success ]; then
        expect "gate: $j expected and $r" 0 "ok:" gate "m-$j-$exp-$r" "$n" "$e" "$pl" "$@"
      elif [ "$exp" = no ] && [ "$r" = skipped ]; then
        expect "gate: $j not expected and $r" 0 "ok:" gate "m-$j-$exp-$r" "$n" "$e" "$pl" "$@"
      elif [ "$exp" = yes ]; then
        expect "gate: $j expected but $r" 1 "$j was expected to run and its result is '$r'" gate "m-$j-$exp-$r" "$n" "$e" "$pl" "$@"
      else
        expect "gate: $j not expected but $r" 1 "$j was not expected to run and its result is '$r'" gate "m-$j-$exp-$r" "$n" "$e" "$pl" "$@"
      fi
    done
  done
done

ALL='["ci-scripts","deps","lint","deny","hermetic","pg-adapter","contract","mutants","web","docs"]'
ALLOK="$(needs success ci-scripts=success deps=success lint=success deny=success hermetic=success pg-adapter=success contract=success mutants=success web=success docs=success)"
expect "gate: everything expected, everything passed" 0 "10 expected jobs passed" gate a1 "$ALLOK" "$ALL" "$P" "$M1" "$M2"
expect "gate: nothing expected, everything skipped (README.md only)" 0 "no cells planned" gate a2 "$(needs success)" '[]' '[]'
expect "gate: the changes job failed" 1 "changes job result is 'failure'" gate a3 "$(needs failure)" '[]' '[]'
expect "gate: the changes job was skipped" 1 "changes job result is 'skipped'" gate a4 "$(needs skipped)" '[]' '[]'
# the cells
expect "gate: one planned cell never reported" 1 "planned cells that never reported success: worker-cf bindings" gate c1 "$(needs success contract=success)" '["contract"]' "$P" "$M1"
expect "gate: a marker for an unplanned cell" 1 "not in the planned set: native-pg main" gate c2 "$(needs success contract=success)" '["contract"]' "$P" "$M1" "$M2" "native-pg main"
expect "gate: contract expected but no cell planned" 1 "the planned cell set is empty" gate c3 "$(needs success contract=success)" '["contract"]' '[]' "$M1"
expect "gate: contract expected and passed, but no marker at all" 1 "no cell markers found" gate c4 "$(needs success contract=success)" '["contract"]' "$P"
expect "gate: cells planned but contract not expected" 1 "cells are planned but the contract job is not expected" gate c5 "$(needs success)" '[]' "$P"
# the planner and the gate's needs must agree on the job names
expect "gate: the planner expects a job the gate does not know" 1 "the planner expects 'pages', which is not a job the gate knows" gate n1 "$(needs success)" '["pages"]' '[]'
NOWEB='{"changes":{"result":"success"},"lint":{"result":"skipped"}}'
expect "gate: the planner expects a job that is not a need of the gate" 1 "the planner expects 'web', which is not a need of the gate" gate n2 "$NOWEB" '["web"]' '[]'
EXTRA="$(needs success | sed 's/}$/,"pages":{"result":"success"}}/')"
expect "gate: a need the planner does not know" 1 "'pages' is a need of the gate but not a job the planner knows" gate n3 "$EXTRA" '[]' '[]'
expect "gate: EXPECTED_JOBS is not JSON" 1 "EXPECTED_JOBS is not a JSON array of strings" gate n4 "$(needs success)" 'lint' '[]'
expect "gate: EXPECTED_JOBS is a JSON string" 1 "EXPECTED_JOBS is not a JSON array of strings" gate n5 "$(needs success)" '"lint"' '[]'
expect "gate: EXPECTED_JOBS is a JSON object" 1 "EXPECTED_JOBS is not a JSON array of strings" gate n6 "$(needs success)" '{}' '[]'
expect "gate: EXPECTED_JOBS holds a number" 1 "EXPECTED_JOBS is not a JSON array of strings" gate n7 "$(needs success)" '[1]' '[]'
expect "gate: PLANNED is a JSON object" 1 "PLANNED is not a JSON array of strings" gate n8 "$(needs success)" '[]' '{}'
unset_one() { # the gate run without one of its inputs: a workflow that does not pass it must fail, not read it as empty
  d="$T/gate/u-$1"; mkdir -p "$d/reported"
  # (exported first, then unset: `env -u X X=...` would set it again)
  NEEDS_JSON="$(needs success)" EXPECTED_JOBS='[]' PLANNED='[]' REPORTED_DIR="$d/reported"
  export NEEDS_JSON EXPECTED_JOBS PLANNED REPORTED_DIR
  ( cd "$d" && env -u "$1" "$S/assert-gate.sh" )
  st=$?
  unset NEEDS_JSON EXPECTED_JOBS PLANNED REPORTED_DIR
  return $st
}
for v in NEEDS_JSON EXPECTED_JOBS PLANNED REPORTED_DIR; do
  expect "gate: a run that is not given $v fails" 1 "$v is not set" unset_one "$v"
done
# The gate's list of jobs is the planner's: a job added to one and not the other is refused above, so they must be equal.
same_jobs() {
  g="$(sed -n 's/^KNOWN="\(.*\)"$/\1/p' "$S/assert-gate.sh")"
  p="$(sed -n 's/^JOBS = (\(.*\))$/\1/p' "$S/plan-ci.py" | tr -d '",')"
  if [ -n "$g" ] && [ "$g" = "$p" ] && [ "$g" = "$JOBS" ]; then echo "same jobs: $g"; else echo "gate '$g' planner '$p' test '$JOBS'"; return 1; fi
}
expect "gate: the gate, the planner and this test list the same jobs" 0 "same jobs" same_jobs

echo "passed=$pass bad=$failn"
[ "$failn" -eq 0 ]
