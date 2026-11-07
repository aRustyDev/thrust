#!/usr/bin/env bash
# Verify that the checkout under test IS the pinned Plane described by oracle/PIN.
#
#   oracle/scripts/verify-pin.sh [--require-tag] [--self-test]
#
# Checks, each reported PASS/FAIL:
#   1. apps/api at HEAD has exactly the tree sha recorded in PIN (the API source is untouched
#      since the pin; harness commits after the merge must not edit apps/api)
#   2. the pinned commit is an ancestor of HEAD
#   3. the merged upstream sha is an ancestor of the pinned commit
#   4. (--require-tag) the annotated tag exists, is a tag object, and points at the pinned commit.
#      Off by default because the tag is LOCAL ONLY (never pushed) and a CI checkout will not have it.
# --self-test is the control: it feeds a deliberately WRONG apps/api tree sha and requires check 1
# to FAIL. If it passes, this verifier cannot fail and nothing it says can be trusted.
#
# Pure git read-only plumbing; changes nothing.
set -uo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
PIN="$ROOT/oracle/PIN"
REQUIRE_TAG=0; SELF=0
for a in "$@"; do
  case "$a" in
    --require-tag) REQUIRE_TAG=1 ;;
    --self-test) SELF=1 ;;
    *) echo "unknown argument: $a" >&2; exit 64 ;;
  esac
done
get() { grep -E "^$1=" "$PIN" | head -1 | cut -d= -f2-; }

pinned="$(get pinned_commit)"; tree="$(get apps_api_tree)"; upstream="$(get upstream_sha)"
tag="$(get pin_tag)"; tag_obj="$(get pin_tag_object)"
cd "$ROOT" || exit 2

fail=0
check() { # check <label> <command...>
  local label="$1"; shift
  if "$@" > /dev/null 2>&1; then echo "PASS $label"; else echo "FAIL $label"; fail=1; fi
}

# The ONE predicate used for the real check and for the control, so the control exercises the
# exact code path that guards the pin.
tree_matches() { test "$(git rev-parse HEAD:apps/api)" = "$1"; }

if [ "$SELF" = 1 ]; then
  if tree_matches "0000000000000000000000000000000000000000"; then
    echo "SELF-TEST FAIL: tree_matches accepted a wrong sha, so this verifier cannot fail"; exit 1
  fi
  if ! tree_matches "$tree"; then
    echo "SELF-TEST FAIL: tree_matches rejects the PINNED sha (HEAD apps/api differs from the pin)"; exit 1
  fi
  echo "PASS self-test: tree_matches rejects a wrong sha and accepts the pinned one"
  exit 0
fi

check "apps/api tree at HEAD == PIN apps_api_tree ($tree)" tree_matches "$tree"
check "pinned commit is an ancestor of HEAD ($pinned)" git merge-base --is-ancestor "$pinned" HEAD
check "upstream sha is an ancestor of the pinned commit ($upstream)" git merge-base --is-ancestor "$upstream" "$pinned"
if [ "$REQUIRE_TAG" = 1 ]; then
  check "tag $tag is an annotated tag object" test "$(git cat-file -t "$tag")" = "tag"
  check "tag object == PIN pin_tag_object" test "$(git rev-parse "$tag")" = "$tag_obj"
  check "tag points at the pinned commit" test "$(git rev-parse "$tag^{commit}")" = "$pinned"
fi
exit "$fail"
