#!/bin/sh
# The ONE required check. It proves the right things ran; it never reasons from "nothing failed".
#
# Inputs (environment):
#   NEEDS_JSON     toJSON(needs) of the gate job
#   RUST_CHANGED   "true" | "false"  (output of the `changes` job)
#   PLANNED        JSON array of "target pass" strings (output of the `plan` job); "[]" if not planned
#   REPORTED_DIR   directory holding one <target>-<pass>.ok marker per cell that passed
#
# Rules
#   changes must have succeeded.
#   RUST_CHANGED != true : no needed job may be failed or cancelled; skipped is expected.
#   RUST_CHANGED == true : every needed job must be exactly `success` (a skipped job here is a
#                          vacuous pass, not a pass) AND the set of cells that reported success must
#                          equal the planned set.
set -eu

fail() {
  echo "GATE FAIL: $*" >&2
  exit 1
}

: "${NEEDS_JSON:?}" "${RUST_CHANGED:?}" "${PLANNED:?}" "${REPORTED_DIR:?}"

printf '%s' "$NEEDS_JSON" | jq -r 'to_entries[] | "\(.key) \(.value.result)"' > needs.txt
echo "needed jobs:"
sed 's/^/  /' needs.txt

changes_result="$(sed -n 's/^changes //p' needs.txt)"
[ "$changes_result" = "success" ] || fail "changes job result is '${changes_result:-missing}'"

if [ "$RUST_CHANGED" != "true" ]; then
  bad="$(grep -E ' (failure|cancelled)$' needs.txt || true)"
  [ -z "$bad" ] || fail "no rust changes, yet: $bad"
  echo "ok: no rust changes, the heavy jobs were legitimately skipped"
  exit 0
fi

not_success="$(grep -v ' success$' needs.txt || true)"
[ -z "$not_success" ] || fail "rust changed but these jobs are not 'success': $not_success"

printf '%s' "$PLANNED" | jq -r '.[]' | sort -u > planned.txt
[ -s planned.txt ] || fail "the planned cell set is empty; refusing to pass on zero cells"

# markers: "<target> <pass>" lines; no marker files at all is itself a failure
set -- "$REPORTED_DIR"/*.ok
[ -e "$1" ] || fail "no cell markers found in $REPORTED_DIR"
cat "$@" | sort -u > reported.txt

missing="$(comm -23 planned.txt reported.txt)"
[ -z "$missing" ] || fail "planned cells that never reported success: $missing"
extra="$(comm -13 planned.txt reported.txt)"
[ -z "$extra" ] || fail "markers for cells that are not in the planned set: $extra"

echo "ok: $(wc -l < planned.txt | tr -d ' ') of $(wc -l < planned.txt | tr -d ' ') planned cells reported success"
