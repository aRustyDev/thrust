#!/bin/sh
# SPIKE for stage 0.2 of the CI split (.agents/plans/ci-workflows/phases/0001-foundation.md). Not for merging.
# The rules of assert-gate.sh that matter to the question, over one called workflow's jobs: a planned job must be exactly
# `success`, an unplanned job must be `skipped`, a job the judge cannot see is a failure, and a plan that is not a JSON array of
# strings is a failure.
#
# Environment: NEEDS_JSON (toJSON(needs)), PLAN (JSON array of the jobs that must run), KNOWN (space-separated job names)
set -eu

fail() {
  echo "JUDGE FAIL: $*" >&2
  exit 1
}

for input in NEEDS_JSON PLAN KNOWN; do
  eval "value=\${$input:-}"
  [ -n "$value" ] || fail "$input is not set"
done

printf '%s' "$PLAN" | jq -e 'type == "array" and all(.[]; type == "string")' > /dev/null 2>&1 \
  || fail "PLAN is not a JSON array of strings: $PLAN"

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
printf '%s' "$NEEDS_JSON" | jq -r 'to_entries[] | "\(.key) \(.value.result)"' > "$T/needs.txt"

for job in $KNOWN; do
  result="$(sed -n "s/^$job //p" "$T/needs.txt")"
  [ -n "$result" ] || fail "$job is not a need of the judge"
  if printf '%s' "$PLAN" | jq -e --arg j "$job" 'index($j)' > /dev/null 2>&1; then
    [ "$result" = success ] || fail "$job was planned and its result is '$result'"
  else
    [ "$result" = skipped ] || fail "$job was not planned and its result is '$result'"
  fi
done
echo "judge ok: $(tr '\n' ' ' < "$T/needs.txt")"
