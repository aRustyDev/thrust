#!/bin/sh
# The schema diff of issue #58 (ADR-0032 Confirmation, ADR-0122): the OpenAPI document that `thrust-api-v1` writes from the types its
# routes answer with, against the pinned Plane schema, with `oasdiff`.
#
#   scripts/check-openapi.sh              the check, and the controls that show it can fail
#   scripts/check-openapi.sh --update     write tools/oracle-v1/schema/openapi.thrust.json from the code, then check
#   scripts/check-openapi.sh --self-test  the helper's cases and the agreement of the oasdiff pin (no compile, no Docker)
#
# What it does, in order (each step names what it would fail on):
#   1. builds the document: `cargo run -p thrust-api-v1 --features openapi --example openapi`;
#   2. compares it with the committed copy (tools/oracle-v1/schema/openapi.thrust.json): a copy that is not what the code writes is
#      STALE, and `--update` is the way to change it;
#   3. checks it describes exactly the operations of tools/oracle-v1/schema/described.operations;
#   4. cuts the pin to those operations and diffs the two with oasdiff. The pin is OpenAPI 3.0.3 and utoipa writes 3.1, and oasdiff
#      v1.33.0 does NOT read the two spellings of "nullable" as one (measured: the pin against its own 3.2 upgrade is 384 false
#      breaking changes), so BOTH sides go through `oasdiff upgrade` first;
#   5. every finding must be an entry of tools/oracle-v1/schema/diff-accepted.json (the decided differences: EX-006, D-030 (5), ...)
#      and every entry must still be found: an entry that is not is STALE.
#   Then the controls (AGENTS.md rule 8, "a check that cannot fail is not evidence"): the document broken three ways (a key dropped,
#   a type changed, a request key made required) must each be refused, and the register with an entry that no longer matches must be.
#
# Exit codes: 0 clean, 1 a violation, 2 it cannot vouch for itself (no Docker, no document, a control that did not fail).
# oasdiff is pinned by image digest, the one tools/oracle-v1/scripts/schema_diff.sh names. NEVER pass --open: it uploads the specs to
# oasdiff.com. Needs Docker and python3 (standard library only; no uv).
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SCHEMA_DIR="$ROOT/tools/oracle-v1/schema"
PIN="$SCHEMA_DIR/openapi.v1.json"
COMMITTED="$SCHEMA_DIR/openapi.thrust.json"
OPS="$SCHEMA_DIR/described.operations"
ACCEPTED="$SCHEMA_DIR/diff-accepted.json"
HELPER="$ROOT/tools/oracle-v1/scripts/openapi_diff.py"
IMAGE='tufin/oasdiff:v1.33.0@sha256:6263a96dd2ef0726c54e21fea9b8e1607eac4841add0079324b424c1f52b819c'

helper() { python3 -I "$HELPER" "$@"; }

# oasdiff on files in $W (mounted read-only at /specs)
oasdiff() { docker run --rm -v "$W:/specs:ro" "$IMAGE" "$@"; }

# cut_and_upgrade: the pin cut to the described operations, and both sides upgraded to one OpenAPI version, in $W
# (pin.json, pin.up.json, and gen.up.json from $1)
canonical() {
  helper scope "$PIN" "$OPS" "$W/pin.json" || return 2
  oasdiff upgrade /specs/pin.json --format json --allow-external-refs=false > "$W/pin.up.json" 2> "$W/upgrade.err" || return 2
  cp "$1" "$W/gen.json" || return 2
  oasdiff upgrade /specs/gen.json --format json --allow-external-refs=false > "$W/gen.up.json" 2>> "$W/upgrade.err" || return 2
}

# diff_to FILE: the findings of oasdiff between the upgraded pin and the upgraded document at $W/gen.up.json. Exit 1 from oasdiff means
# it found breaking changes, which is not a failure to run; anything else is.
diff_to() {
  oasdiff breaking /specs/pin.up.json /specs/gen.up.json --fail-on ERR --format json > "$1" 2> "$W/breaking.err"
  rc=$?
  [ "$rc" -le 1 ] || { echo "oasdiff exited $rc:"; cat "$W/breaking.err"; return 2; }
}

generate() {
  "$ROOT/scripts/rs" cargo run -q --manifest-path "$ROOT/Cargo.toml" -p thrust-api-v1 --features openapi --example openapi > "$1" 2> "$W/generate.err" \
    || { echo "the document does not build:"; cat "$W/generate.err"; return 2; }
}

self_test() {
  helper self-test || return 1
  # one oasdiff for the whole repo: the digest of this script is the digest of the schema-diff job that proves oasdiff sees breaks
  other="$(sed -n "s/^IMAGE='\(.*\)'\$/\1/p" "$ROOT/tools/oracle-v1/scripts/schema_diff.sh")"
  if [ "$other" = "$IMAGE" ]; then
    echo "ok   the oasdiff image is the one schema_diff.sh names"
  else
    echo "FAIL the oasdiff image differs from schema_diff.sh's: '$other'"
    return 1
  fi
}

# control NAME KIND: break the document KIND way; the check must refuse it (an UNACCEPTED finding), or the check is void
control() {
  helper mutate "$W/gen.json" "$2" "$W/m.json" || { echo "FAIL control $1: nothing to break"; return 2; }
  canonical "$W/m.json" || return 2
  diff_to "$W/m.diff.json" || return 2
  out="$(helper classify "$W/m.diff.json" "$ACCEPTED" "$OPS")"
  case "$out" in
    *UNACCEPTED*) echo "ok   control $1: refused" ;;
    *) echo "FAIL control $1: the check did not refuse a document that was broken on purpose"; echo "$out"; return 2 ;;
  esac
}

main() {
  W="$(mktemp -d /tmp/check-openapi.XXXXXX)" || exit 2
  trap 'rm -rf "$W"' EXIT
  # mktemp makes it 0700; the oasdiff container reads it through a bind mount, and must be able to on a rootless Docker too
  chmod 755 "$W" || exit 2
  command -v docker > /dev/null 2>&1 || { echo "cannot vouch: no docker (oasdiff runs in its pinned image)"; exit 2; }
  command -v python3 > /dev/null 2>&1 || { echo "cannot vouch: no python3"; exit 2; }

  generate "$W/built.json" || exit 2
  if [ "$UPDATE" = 1 ]; then
    cp "$W/built.json" "$COMMITTED" || exit 2
    echo "wrote ${COMMITTED#"$ROOT"/}"
  fi
  if ! cmp -s "$W/built.json" "$COMMITTED"; then
    echo "STALE: ${COMMITTED#"$ROOT"/} is not what the code writes; run scripts/check-openapi.sh --update and commit it"
    exit 1
  fi
  helper same-operations "$COMMITTED" "$OPS" || exit $?

  status=0
  canonical "$COMMITTED" || exit 2
  diff_to "$W/real.diff.json" || exit 2
  helper classify "$W/real.diff.json" "$ACCEPTED" "$OPS"
  rc=$?
  [ "$rc" -eq 0 ] || status=$rc

  echo "--- controls"
  control "a key the routes send is dropped from the document" drop-property || exit 2
  control "a key changes type" retype || exit 2
  control "a request key becomes required" require || exit 2
  # the register with one entry that matches nothing now: the sort_order difference is uncovered, and its entry is stale
  sed 's/"text_contains": "sort_order`"/"text_contains": "no_such_property`"/' "$ACCEPTED" > "$W/doctored.json"
  if cmp -s "$ACCEPTED" "$W/doctored.json"; then echo "FAIL control: the register was not doctored"; exit 2; fi
  out="$(helper classify "$W/real.diff.json" "$W/doctored.json" "$OPS")"
  case "$out" in
    *UNACCEPTED*STALE*|*STALE*UNACCEPTED*) echo "ok   control a register entry that stops matching: refused" ;;
    *) echo "FAIL control: a register entry that stops matching was not refused"; echo "$out"; exit 2 ;;
  esac
  exit "$status"
}

UPDATE=0
case "${1:-}" in
  "") ;;
  --update) UPDATE=1 ;;
  --self-test) self_test; exit $? ;;
  *) echo "usage: scripts/check-openapi.sh [--update | --self-test]" >&2; exit 2 ;;
esac
main
