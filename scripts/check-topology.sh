#!/bin/sh
# The topology check (ADR-0090 round 29, FR-05, FR-11, issue #43): the Worker is ONE Worker, in Rust, with no Workflow.
#
#   scripts/check-topology.sh [dir]     check every wrangler*.toml in dir (default: crates/thrust-worker)
#   scripts/check-topology.sh --self-test
#                                       run the check against fixtures it must accept and fixtures it must refuse
#
# For every `wrangler*.toml` it fails on
#   * a `main` other than `build/worker/shim.mjs`: the file worker-build writes. Anything else is a hand-written entry file in
#     front of it, and the Workflows shim (`entry.mjs`) was the only one (ADR-0006's non-Rust register, FR-05);
#   * a `[[workflows]]` block: the Workflows binding (FR-05 withdrew the `WorkflowEngine` port);
#   * a `[[services]]` block: a service binding, which is a second Worker to call or the Worker calling itself (FR-11: one Worker
#     until the P1 collab room, which is added as its own Worker when it lands).
# `main` is the top-level key (before the first table header); a commented-out line does not count.
#
# A check that finds no config to look at would pass vacuously, so that is an error too. `--self-test` is the control: it is run
# next to the real check in CI, and it fails if the check stops refusing what it is for.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SHIM="build/worker/shim.mjs"

# check_dir DIR: prints a verdict per config, returns non-zero on any violation.
check_dir() {
  dir="$1"
  status=0
  found=0
  for cfg in "$dir"/wrangler*.toml; do
    [ -e "$cfg" ] || continue
    found=$((found + 1))
    name="$(basename "$cfg")"
    echo "=== $name ==="
    main="$(awk '/^\[/ { exit } /^main[[:space:]]*=/ { print; exit }' "$cfg" |
      sed -e 's/^main[[:space:]]*=[[:space:]]*//' -e 's/[[:space:]]*#.*$//' -e 's/^"\(.*\)"$/\1/')"
    if [ "$main" = "$SHIM" ]; then
      echo "ok: main is $SHIM"
    else
      echo "FORBIDDEN: $name: main is '${main:-<none>}', not $SHIM"
      status=1
    fi
    for table in workflows services; do
      hits="$(grep -nE "^[[:space:]]*\[\[$table\]\]" "$cfg")"
      if [ -z "$hits" ]; then
        echo "ok: no [[$table]]"
      else
        echo "FORBIDDEN: $name has a [[$table]] block:"
        echo "$hits" | sed 's/^/    line /'
        status=1
      fi
    done
  done
  if [ "$found" -eq 0 ]; then
    echo "ERROR: no wrangler*.toml in $dir, so there is nothing to check (a check of nothing passes)"
    status=1
  fi
  return "$status"
}

self_test() {
  tmp="$(mktemp -d)" || exit 2
  trap 'rm -rf "$tmp"' EXIT
  failed=0

  # expect pass|fail LABEL GREP: run the check on $tmp/LABEL and compare. For `fail`, the output must also match GREP, so a
  # fixture refused for the wrong reason (a typo in the fixture) does not count.
  expect() {
    want="$1"
    label="$2"
    pattern="${3:-}"
    out="$(check_dir "$tmp/$label" 2>&1)"
    got=$?
    if [ "$want" = pass ] && [ "$got" -eq 0 ]; then
      echo "self-test ok: $label is accepted"
    elif [ "$want" = fail ] && [ "$got" -ne 0 ] && echo "$out" | grep -q -e "$pattern"; then
      echo "self-test ok: $label is refused ($pattern)"
    else
      echo "SELF-TEST FAILED: $label should $want${pattern:+ with '$pattern'}, got exit $got:"
      echo "$out" | sed 's/^/    /'
      failed=1
    fi
  }

  mkdir -p "$tmp/good" "$tmp/commented" "$tmp/entry" "$tmp/workflows" "$tmp/services" "$tmp/nomain" "$tmp/tablemain" "$tmp/empty" \
    "$tmp/second"
  printf 'name = "w"\nmain = "build/worker/shim.mjs"   # worker-build writes it\n\n[vars]\nA = "1"\n' >"$tmp/good/wrangler.toml"
  printf 'name = "w"\nmain = "build/worker/shim.mjs"\n# [[workflows]]\n#[[services]]\n' >"$tmp/commented/wrangler.toml"
  printf 'name = "w"\nmain = "entry.mjs"\n' >"$tmp/entry/wrangler.toml"
  printf 'name = "w"\nmain = "build/worker/shim.mjs"\n\n[[workflows]]\nname = "x"\nbinding = "WORKFLOW"\nclass_name = "C"\n' \
    >"$tmp/workflows/wrangler.toml"
  printf 'name = "w"\nmain = "build/worker/shim.mjs"\n\n[[services]]\nbinding = "SELF"\nservice = "w"\n' >"$tmp/services/wrangler.toml"
  printf 'name = "w"\ncompatibility_date = "2026-08-01"\n' >"$tmp/nomain/wrangler.toml"
  printf 'name = "w"\n\n[vars]\nmain = "build/worker/shim.mjs"\n' >"$tmp/tablemain/wrangler.toml"
  # a second config in the same directory is checked too: the first is good, so only the second can make this fail
  printf 'name = "w"\nmain = "build/worker/shim.mjs"\n' >"$tmp/second/wrangler.toml"
  printf 'name = "w2"\nmain = "entry.mjs"\n' >"$tmp/second/wrangler.stage.toml"

  expect pass good
  expect pass commented
  expect fail entry "main is 'entry.mjs'"
  expect fail workflows '\[\[workflows\]\] block'
  expect fail services '\[\[services\]\] block'
  expect fail nomain "main is '<none>'"
  expect fail tablemain "main is '<none>'"
  expect fail second "wrangler.stage.toml: main is 'entry.mjs'"
  expect fail empty "nothing to check"

  [ "$failed" -eq 0 ] && echo "self-test passed"
  return "$failed"
}

case "${1:-}" in
  --self-test)
    self_test
    exit $?
    ;;
  *)
    check_dir "${1:-$ROOT/crates/thrust-worker}"
    STATUS=$?
    [ "$STATUS" -eq 0 ] && echo "ok: one Worker, no Workflow, no service binding"
    exit "$STATUS"
    ;;
esac
