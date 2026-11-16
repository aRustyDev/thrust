#!/bin/sh
# The ONE required check. It proves the right things ran; it never reasons from "nothing failed".
#
# One script judges at every level (ADR-0102): the `gate` of ci.yml, and the `verdict` job at the end of each called workflow,
# which judges that workflow's own jobs. Run alone with the first four inputs it judges ci.yml's own jobs, as it always has.
#
# Inputs (environment):
#   NEEDS_JSON     toJSON(needs) of the judging job
#   EXPECTED_JOBS  JSON array of the jobs that must run (the `jobs` output of the `changes` job, from plan-ci.py; in a called
#                  workflow, its `plan` input)
#   PLANNED        JSON array of "target pass" strings, the cells that must run (`cells` of `changes`); "[]" when none
#   REPORTED_DIR   directory holding one <target>-<pass>.ok marker per cell that passed
#                  (PLANNED and REPORTED_DIR are needed while CELL_JOB is set)
# Optional, to judge another level. Unset, each is what ci.yml needs today, so a level that forgets one fails here and does not
# quietly pass:
#   KNOWN_JOBS     the jobs this level knows, separated by spaces (default: the jobs of ci.yml)
#   CHANGES_JOB    the planner job that must have succeeded (default `changes`; set and empty: there is none, as in a called workflow)
#   CELL_JOB       the job whose cells the markers prove (default `contract`; set and empty: there is none)
#   VERDICT_JOBS   the jobs that call a workflow, separated by spaces (default: none); see the rules
#
# Rules
#   The planner job must have succeeded.
#   Every expected job is a need of the judging job (a job it cannot see cannot be proved), and every need is a job this level
#   knows (plan-ci.py's JOBS for ci.yml), so a job added to ci.yml and not to the planner fails here instead of never running.
#   An expected job must be exactly `success`: a skipped one is a vacuous pass, not a pass.
#   An expected job that calls a workflow (VERDICT_JOBS) must also carry the output `verdict` equal to `pass`: a called workflow
#   whose own `verdict` job was skipped is `success` and proves nothing.
#   A job that is not expected must be `skipped`: one that ran anyway means its `if:` and the planner disagree.
#   The cell job is expected exactly when cells are planned, and then the set of cells that reported success must equal the
#   planned set.
set -eu

fail() {
  echo "GATE FAIL: $*" >&2
  exit 1
}

# The jobs plan-ci.py can ask for: the default of KNOWN_JOBS. ci.yml's own `needs` must be these and the planner job.
KNOWN="${KNOWN_JOBS:-ci-scripts deps lint deny hermetic pg-adapter contract mutants web docs}"
# `-` and not `:-` for these two: set and empty means "there is none", and only unset means what ci.yml needs.
CHANGES="${CHANGES_JOB-changes}"
CELLS="${CELL_JOB-contract}"
VERDICTS="${VERDICT_JOBS:-}"

# Every input must be set and non-empty. Not `: "${VAR:?}"`: the shell picks that exit status (1 in bash, which is macOS's
# `sh`, but 2 in dash, which is Ubuntu's), and a gate that refuses must refuse the same way on every runner. The names
# are literals, so the eval reads only them.
inputs="NEEDS_JSON EXPECTED_JOBS"
arrays="EXPECTED_JOBS"
if [ -n "$CELLS" ]; then
  inputs="$inputs PLANNED REPORTED_DIR"
  arrays="$arrays PLANNED"
fi
for input in $inputs; do
  eval "value=\${$input:-}"
  [ -n "$value" ] || fail "$input is not set"
done

printf '%s' "$NEEDS_JSON" | jq -r 'to_entries[] | "\(.key) \(.value.result)"' > needs.txt
printf '%s' "$NEEDS_JSON" | jq -r 'to_entries[] | "\(.key) \(.value.outputs.verdict // "")"' > verdicts.txt
echo "needed jobs:"
sed 's/^/  /' needs.txt

if [ -n "$CHANGES" ]; then
  changes_result="$(sed -n "s/^$CHANGES //p" needs.txt)"
  [ "$changes_result" = "success" ] || fail "$CHANGES job result is '${changes_result:-missing}'"
fi

# Arrays of strings and nothing else: jq's `.[]` reads an object's values too, so `{}` would otherwise be "no jobs".
for input in $arrays; do
  eval "value=\$$input"
  printf '%s' "$value" | jq -e 'type == "array" and all(.[]; type == "string")' > /dev/null 2>&1 \
    || fail "$input is not a JSON array of strings: $value"
done
printf '%s' "$EXPECTED_JOBS" | jq -r '.[]' > expected.txt
echo "expected jobs: $(tr '\n' ' ' < expected.txt)"

# A name in VERDICT_JOBS that is no known job is a typo, and a typo would switch the rule off for the job it meant.
for job in $VERDICTS; do
  case " $KNOWN " in *" $job "*) ;; *) fail "VERDICT_JOBS names '$job', which is not a job the gate knows" ;; esac
done

while read -r job; do
  case " $KNOWN " in *" $job "*) ;; *) fail "the planner expects '$job', which is not a job the gate knows" ;; esac
  grep -q "^$job " needs.txt || fail "the planner expects '$job', which is not a need of the gate"
done < expected.txt

while read -r job result; do
  [ -n "$CHANGES" ] && [ "$job" = "$CHANGES" ] && continue
  case " $KNOWN " in *" $job "*) ;; *) fail "'$job' is a need of the gate but not a job the planner knows" ;; esac
  if grep -qx "$job" expected.txt; then
    [ "$result" = success ] || fail "$job was expected to run and its result is '$result'"
    case " $VERDICTS " in
      *" $job "*)
        verdict="$(sed -n "s/^$job //p" verdicts.txt)"
        [ "$verdict" = pass ] || fail "$job succeeded but its verdict output is '${verdict:-missing}', not 'pass': it calls a workflow, and one whose verdict job did not run proves nothing"
        ;;
    esac
  else
    [ "$result" = skipped ] || fail "$job was not expected to run and its result is '$result'"
  fi
done < needs.txt

if [ -z "$CELLS" ]; then
  echo "ok: $(wc -l < expected.txt | tr -d ' ') expected jobs passed, the rest were skipped"
  exit 0
fi

printf '%s' "$PLANNED" | jq -r '.[]' | sort -u > planned.txt
if ! grep -qx "$CELLS" expected.txt; then
  [ ! -s planned.txt ] || fail "cells are planned but the $CELLS job is not expected: $(tr '\n' ',' < planned.txt)"
  echo "ok: $(wc -l < expected.txt | tr -d ' ') expected jobs passed, the rest were skipped, no cells planned"
  exit 0
fi
[ -s planned.txt ] || fail "the $CELLS job is expected but the planned cell set is empty; refusing to pass on zero cells"

# markers: "<target> <pass>" lines; no marker files at all is itself a failure
set -- "$REPORTED_DIR"/*.ok
[ -e "$1" ] || fail "no cell markers found in $REPORTED_DIR"
cat "$@" | sort -u > reported.txt

missing="$(comm -23 planned.txt reported.txt)"
[ -z "$missing" ] || fail "planned cells that never reported success: $missing"
extra="$(comm -13 planned.txt reported.txt)"
[ -z "$extra" ] || fail "markers for cells that are not in the planned set: $extra"

echo "ok: $(wc -l < expected.txt | tr -d ' ') expected jobs passed, the rest were skipped;" \
  "$(wc -l < planned.txt | tr -d ' ') of $(wc -l < planned.txt | tr -d ' ') planned cells reported success"
