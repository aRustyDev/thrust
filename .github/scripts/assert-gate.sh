#!/bin/sh
# The ONE required check. It proves the right things ran; it never reasons from "nothing failed".
#
# Inputs (environment):
#   NEEDS_JSON     toJSON(needs) of the gate job
#   EXPECTED_JOBS  JSON array of the jobs that must run (the `jobs` output of the `changes` job, from plan-ci.py)
#   PLANNED        JSON array of "target pass" strings, the cells that must run (`cells` of `changes`); "[]" when none
#   REPORTED_DIR   directory holding one <target>-<pass>.ok marker per cell that passed
#
# Rules
#   changes must have succeeded.
#   Every expected job is a need of the gate (a job the gate cannot see cannot be proved), and every need is a job the planner
#   knows (plan-ci.py's JOBS), so a job added to ci.yml and not to the planner fails here instead of never running.
#   An expected job must be exactly `success`: a skipped one is a vacuous pass, not a pass.
#   A job that is not expected must be `skipped`: one that ran anyway means its `if:` and the planner disagree.
#   `contract` is expected exactly when cells are planned, and then the set of cells that reported success must equal the
#   planned set.
set -eu

fail() {
  echo "GATE FAIL: $*" >&2
  exit 1
}

# Every input must be set and non-empty. Not `: "${VAR:?}"`: the shell picks that exit status (1 in bash, which is macOS's
# `sh`, but 2 in dash, which is Ubuntu's), and a gate that refuses must refuse the same way on every runner. The names
# are literals, so the eval reads only them.
for input in NEEDS_JSON EXPECTED_JOBS PLANNED REPORTED_DIR; do
  eval "value=\${$input:-}"
  [ -n "$value" ] || fail "$input is not set"
done

# The jobs plan-ci.py can ask for. The gate's own `needs` must be these and `changes`.
KNOWN="ci-scripts deps lint deny hermetic pg-adapter contract mutants web"

printf '%s' "$NEEDS_JSON" | jq -r 'to_entries[] | "\(.key) \(.value.result)"' > needs.txt
echo "needed jobs:"
sed 's/^/  /' needs.txt

changes_result="$(sed -n 's/^changes //p' needs.txt)"
[ "$changes_result" = "success" ] || fail "changes job result is '${changes_result:-missing}'"

# Arrays of strings and nothing else: jq's `.[]` reads an object's values too, so `{}` would otherwise be "no jobs".
for input in EXPECTED_JOBS PLANNED; do
  eval "value=\$$input"
  printf '%s' "$value" | jq -e 'type == "array" and all(.[]; type == "string")' > /dev/null 2>&1 \
    || fail "$input is not a JSON array of strings: $value"
done
printf '%s' "$EXPECTED_JOBS" | jq -r '.[]' > expected.txt
echo "expected jobs: $(tr '\n' ' ' < expected.txt)"

while read -r job; do
  case " $KNOWN " in *" $job "*) ;; *) fail "the planner expects '$job', which is not a job the gate knows" ;; esac
  grep -q "^$job " needs.txt || fail "the planner expects '$job', which is not a need of the gate"
done < expected.txt

while read -r job result; do
  [ "$job" = changes ] && continue
  case " $KNOWN " in *" $job "*) ;; *) fail "'$job' is a need of the gate but not a job the planner knows" ;; esac
  if grep -qx "$job" expected.txt; then
    [ "$result" = success ] || fail "$job was expected to run and its result is '$result'"
  else
    [ "$result" = skipped ] || fail "$job was not expected to run and its result is '$result'"
  fi
done < needs.txt

printf '%s' "$PLANNED" | jq -r '.[]' | sort -u > planned.txt
if ! grep -qx contract expected.txt; then
  [ ! -s planned.txt ] || fail "cells are planned but the contract job is not expected: $(tr '\n' ',' < planned.txt)"
  echo "ok: $(wc -l < expected.txt | tr -d ' ') expected jobs passed, the rest were skipped, no cells planned"
  exit 0
fi
[ -s planned.txt ] || fail "the contract job is expected but the planned cell set is empty; refusing to pass on zero cells"

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
