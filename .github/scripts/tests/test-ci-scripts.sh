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
    # mutate_gate.py asks for one answer, fail or not, so it stops at the first
    [ -z "${STOP_AT_FIRST_BAD:-}" ] || { echo "passed=$pass bad=$failn (stopped at the first)"; exit 1; }
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
# What ci.yml's gate needs besides `changes`: the jobs still in ci.yml and the workflows it calls, which are CALLED (ci.yml sets
# them as VERDICT_JOBS). Each migration stage moves a job's name out of this list and its workflow's name in.
JOBS="contract documentation security web lint test"
CALLED="documentation security web lint test"
gate() { # gate <dir> <needs-json> <expected-jobs-json> <planned-json> <reported markers...>
  d="$T/gate/$1"; mkdir -p "$d/reported"; needs="$2"; expected="$3"; planned="$4"; shift 4
  for m in "$@"; do printf '%s\n' "$m" > "$d/reported/$(echo "$m" | tr ' ' '-').ok"; done
  ( cd "$d" && NEEDS_JSON="$needs" EXPECTED_JOBS="$expected" PLANNED="$planned" REPORTED_DIR="$d/reported" VERDICT_JOBS="$CALLED" "$S/assert-gate.sh" )
}
# needs <changes result> <job=result>...: the gate's needs, every job of $JOBS present; a job not named is skipped. A called workflow
# reports `verdict=pass`, as its `verdict` job does when its own jobs were as planned.
needs() {
  ch="$1"; shift
  printf '{"changes":{"result":"%s"}' "$ch"
  for j in $JOBS; do
    r=skipped
    for kv in "$@"; do [ "${kv%%=*}" = "$j" ] && r="${kv#*=}"; done
    case " $CALLED " in
      *" $j "*) printf ',"%s":{"result":"%s","outputs":{"verdict":"pass"}}' "$j" "$r" ;;
      *) printf ',"%s":{"result":"%s"}' "$j" "$r" ;;
    esac
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

ALL='["contract","documentation","security","web","lint","test"]'
# every job succeeded: "contract=success documentation=success ..."
OKARGS=""
for j in $JOBS; do OKARGS="$OKARGS $j=success"; done
ALLOK="$(needs success $OKARGS)"
expect "gate: everything expected, everything passed" 0 "6 expected jobs passed" gate a1 "$ALLOK" "$ALL" "$P" "$M1" "$M2"
# a called workflow must also say pass: its result alone is not the proof (research 0018: a `verdict` job that was skipped leaves it success).
# Each called workflow is tried on its own, the others in a state that passes, so that the case can only fail because of it.
noverdict() { printf '%s' "$1" | sed "s/\"$2\":{\"result\":\"success\",\"outputs\":{\"verdict\":\"pass\"}}/\"$2\":{\"result\":\"success\"$3}/"; }
for w in $CALLED; do
  expect "gate: $w succeeded and its verdict output is missing" 1 "$w succeeded but its verdict output is 'missing'" \
    gate "a5-$w" "$(noverdict "$ALLOK" "$w" '')" "$ALL" "$P" "$M1" "$M2"
  expect "gate: $w succeeded and its verdict output is empty" 1 "$w succeeded but its verdict output is 'missing'" \
    gate "a6-$w" "$(noverdict "$ALLOK" "$w" ',"outputs":{"verdict":""}')" "$ALL" "$P" "$M1" "$M2"
  expect "gate: $w succeeded and its verdict output is 'fail'" 1 "$w succeeded but its verdict output is 'fail'" \
    gate "a7-$w" "$(noverdict "$ALLOK" "$w" ',"outputs":{"verdict":"fail"}')" "$ALL" "$P" "$M1" "$M2"
  OTHERS=""
  for j in $JOBS; do [ "$j" = "$w" ] || OTHERS="$OTHERS $j=success"; done
  expect "gate: $w is planned, and was skipped (its calling job's own if)" 1 "$w was expected to run and its result is 'skipped'" \
    gate "a8-$w" "$(needs success $OTHERS)" "$ALL" "$P" "$M1" "$M2"
done
expect "gate: no called workflow is planned, and the gate passes" 0 "ok:" \
  gate a9 "$(needs success)" '[]' '[]'
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
# ---- assert-gate.sh at another level (ADR-0102) ----------------------------------------------------------------------------------
# The one script also judges a called workflow's own jobs (its `verdict` job) and a gate that calls workflows. The scenarios of the
# gate spike (research 0018) are cases here; every case above, which judges ci.yml's own jobs as before, is unchanged.
level() { # level <dir> <needs-json> <expected-json> <known jobs> <changes job, or empty> <verdict jobs>   (no cells at this level)
  d="$T/gate/$1"; mkdir -p "$d"
  ( cd "$d" && NEEDS_JSON="$2" EXPECTED_JOBS="$3" KNOWN_JOBS="$4" CHANGES_JOB="$5" CELL_JOB="" VERDICT_JOBS="$6" "$S/assert-gate.sh" )
}
inner() { # inner <a> <b> <c>: the needs of a called workflow's verdict job, whose jobs are a, b and c
  printf '{"a":{"result":"%s"},"b":{"result":"%s"},"c":{"result":"%s"}}' "$1" "$2" "$3"
}
AB='["a","b"]'
expect "callee: green (a and b planned and passed, c not planned and skipped)" 0 "ok: 2 expected jobs passed" \
  level c-green "$(inner success success skipped)" "$AB" "a b c" "" ""
expect "callee: skip-planned (b is planned and was skipped)" 1 "b was expected to run and its result is 'skipped'" \
  level c-skip "$(inner success skipped skipped)" "$AB" "a b c" "" ""
expect "callee: fail-planned (b is planned and failed)" 1 "b was expected to run and its result is 'failure'" \
  level c-fail "$(inner success failure skipped)" "$AB" "a b c" "" ""
expect "callee: unplanned-ran (c ran and was not planned)" 1 "c was not expected to run and its result is 'success'" \
  level c-unplanned "$(inner success success success)" "$AB" "a b c" "" ""
expect "callee: malformed-plan (the plan is not JSON)" 1 "EXPECTED_JOBS is not a JSON array of strings" \
  level c-malformed "$(inner success success skipped)" 'not json' "a b c" "" ""
expect "callee: a plan that names a job the callee does not know" 1 "the planner expects 'z', which is not a job the gate knows" \
  level c-unknown "$(inner success success skipped)" '["a","z"]' "a b c" "" ""
expect "callee: a planned job the judge cannot see" 1 "the planner expects 'c', which is not a need of the gate" \
  level c-unseen '{"a":{"result":"success"},"b":{"result":"success"}}' '["a","c"]' "a b c" "" ""
expect "callee: a need the callee does not know" 1 "'d' is a need of the gate but not a job the planner knows" \
  level c-extra '{"a":{"result":"success"},"b":{"result":"success"},"c":{"result":"skipped"},"d":{"result":"success"}}' "$AB" "a b c" "" ""
expect "callee: an empty plan, every job skipped" 0 "ok: 0 expected jobs passed" level c-empty "$(inner skipped skipped skipped)" '[]' "a b c" "" ""
# Every job of the callee, planned or not, with every result: it passes exactly when a planned job succeeded or an unplanned one was
# skipped. The other two jobs are planned and passed, so the case can only fail because of the job under test.
for j in a b c; do
  for exp in yes no; do
    for r in success skipped failure cancelled; do
      ra=success; rb=success; rc=success
      case "$j" in a) ra=$r ;; b) rb=$r ;; c) rc=$r ;; esac
      if [ "$exp" = yes ]; then
        plan='["a","b","c"]'
      else
        case "$j" in a) plan='["b","c"]' ;; b) plan='["a","c"]' ;; c) plan='["a","b"]' ;; esac
      fi
      n="$(inner "$ra" "$rb" "$rc")"
      if [ "$exp" = yes ] && [ "$r" = success ]; then
        expect "callee: $j planned and $r" 0 "ok:" level "g-$j-$exp-$r" "$n" "$plan" "a b c" "" ""
      elif [ "$exp" = no ] && [ "$r" = skipped ]; then
        expect "callee: $j not planned and $r" 0 "ok:" level "g-$j-$exp-$r" "$n" "$plan" "a b c" "" ""
      elif [ "$exp" = yes ]; then
        expect "callee: $j planned but $r" 1 "$j was expected to run and its result is '$r'" level "g-$j-$exp-$r" "$n" "$plan" "a b c" "" ""
      else
        expect "callee: $j not planned but $r" 1 "$j was not expected to run and its result is '$r'" level "g-$j-$exp-$r" "$n" "$plan" "a b c" "" ""
      fi
    done
  done
done
# A verdict job that forgets one of the three inputs that say which level it judges runs with ci.yml's default for it, and fails.
forgot() { # forgot <the input to leave out: KNOWN_JOBS | CHANGES_JOB | CELL_JOB>
  d="$T/gate/forgot-$1"; mkdir -p "$d"
  KNOWN_JOBS="a b c" CHANGES_JOB="" CELL_JOB=""
  export KNOWN_JOBS CHANGES_JOB CELL_JOB
  ( cd "$d" && env -u "$1" NEEDS_JSON="$(inner success success skipped)" EXPECTED_JOBS="$AB" "$S/assert-gate.sh" )
  st=$?
  unset KNOWN_JOBS CHANGES_JOB CELL_JOB
  return $st
}
expect "callee: a verdict job that forgot KNOWN_JOBS fails" 1 "the planner expects 'a', which is not a job the gate knows" forgot KNOWN_JOBS
expect "callee: a verdict job that forgot CHANGES_JOB fails" 1 "changes job result is 'missing'" forgot CHANGES_JOB
expect "callee: a verdict job that forgot CELL_JOB fails" 1 "PLANNED is not set" forgot CELL_JOB

# A gate that calls workflows `test` and `lint`: an expected callee must have succeeded AND say its verdict was pass.
T1='["test"]'
tl() { # tl <dir> <needs-json> <expected-json> <verdict jobs>
  level "$1" "$2" "$3" "test lint" changes "$4"
}
tneeds() { # tneeds <test result> <test verdict: pass, fail, empty (the empty string), none (outputs {}) or - (no outputs at all)> <lint result>
  case "$2" in
    -) t="{\"result\":\"$1\"}" ;;
    none) t="{\"result\":\"$1\",\"outputs\":{}}" ;;
    empty) t="{\"result\":\"$1\",\"outputs\":{\"verdict\":\"\"}}" ;;
    *) t="{\"result\":\"$1\",\"outputs\":{\"verdict\":\"$2\"}}" ;;
  esac
  printf '{"changes":{"result":"success"},"test":%s,"lint":{"result":"%s"}}' "$t" "$3"
}
expect "verdict: green (test succeeded and says pass; lint not planned, skipped)" 0 "ok: 1 expected jobs passed" \
  tl v-green "$(tneeds success pass skipped)" "$T1" "test lint"
expect "verdict: no-verdict (test succeeded and says nothing)" 1 "test succeeded but its verdict output is 'missing', not 'pass'" \
  tl v-none "$(tneeds success - skipped)" "$T1" "test lint"
expect "verdict: callee-skipped (test is planned and was skipped)" 1 "test was expected to run and its result is 'skipped'" \
  tl v-skipped "$(tneeds skipped - skipped)" "$T1" "test lint"
expect "verdict: a callee that failed fails the gate whatever it says" 1 "test was expected to run and its result is 'failure'" \
  tl v-failed "$(tneeds failure pass skipped)" "$T1" "test lint"
expect "verdict: a callee that is not planned and ran" 1 "lint was not expected to run and its result is 'success'" \
  tl v-unplanned "$(tneeds success pass success)" "$T1" "test lint"
expect "verdict: a callee that is not planned needs no verdict" 0 "ok:" tl v-unplanned-ok "$(tneeds success pass skipped)" "$T1" "test lint"
expect "verdict: VERDICT_JOBS names a job the gate does not know" 1 "VERDICT_JOBS names 'zzz', which is not a job the gate knows" \
  tl v-typo "$(tneeds success pass skipped)" "$T1" "test zzz"
expect "verdict: an expected job that is not in VERDICT_JOBS needs no verdict" 0 "ok: 2 expected jobs passed" \
  tl v-inline "$(tneeds success pass success)" '["test","lint"]' "test"
# Every result of the callee with every kind of verdict: only a success that says pass passes.
for r in success skipped failure cancelled; do
  for v in pass fail empty none -; do
    case "$v" in pass) shown=pass ;; fail) shown=fail ;; *) shown=missing ;; esac
    if [ "$r" = success ] && [ "$v" = pass ]; then
      expect "verdict: test $r and says $v" 0 "ok:" tl "v-$r-$v" "$(tneeds "$r" "$v" skipped)" "$T1" "test lint"
    elif [ "$r" = success ]; then
      expect "verdict: test $r and says $v" 1 "test succeeded but its verdict output is '$shown', not 'pass'" \
        tl "v-$r-$v" "$(tneeds "$r" "$v" skipped)" "$T1" "test lint"
    else
      expect "verdict: test $r and says $v" 1 "test was expected to run and its result is '$r'" tl "v-$r-$v" "$(tneeds "$r" "$v" skipped)" "$T1" "test lint"
    fi
  done
done

# The gate's list of jobs is the planner's: a job added to one and not the other is refused above, so they must be equal. "The
# planner's" is what it expects of ci.yml's own gate when everything is planned: its `top` of a full run, the jobs still in ci.yml
# and then the workflows it calls (ADR-0102), asked of the planner itself and not worked out again here. The called workflows are
# the keys of its WORKFLOWS, which are also ci.yml's VERDICT_JOBS (test_plan_ci.py holds those two equal).
same_jobs() {
  g="$(sed -n 's/^KNOWN="${KNOWN_JOBS:-\(.*\)}"$/\1/p' "$S/assert-gate.sh")"
  printf 'native-memory main\n' > "$T/one-cell.txt"
  p="$(python3 -I "$S/plan-ci.py" --full "same jobs" --cells "$T/one-cell.txt" 2> /dev/null | sed -n 's/^top=//p' | jq -r 'join(" ")')"
  if [ -n "$g" ] && [ "$g" = "$p" ] && [ "$g" = "$JOBS" ]; then echo "same jobs: $g"; else echo "gate '$g' planner '$p' test '$JOBS'"; return 1; fi
}
expect "gate: the gate, the planner and this test list the same jobs" 0 "same jobs" same_jobs

echo "passed=$pass bad=$failn"
[ "$failn" -eq 0 ]
