#!/bin/sh
# The 500 guard (issue #51): no handler of a Plane-shaped surface writes a 500 itself.
#
#   scripts/check-plane-500.sh [dir...]   scan dirs (default: the src of thrust-api-v1 and thrust-compat-plane)
#   scripts/check-plane-500.sh --self-test
#                                         run the check against trees it must accept and trees it must refuse
#
# The pinned Plane has three 500 texts and the text is part of the contract (research/0007 PQ-409). Every reproduced 500 is
# answered by `thrust_plane_types::plane_500`, which sets the status, writes the exact body and attaches the cause for the log.
# A handler that builds its own 500 (`StatusCode::INTERNAL_SERVER_ERROR`, `from_u16(500)`, `try_from(500)`, `.status(500)`)
# writes a body of its own and is the thing this crate exists to prevent. The helper itself lives in thrust-plane-types, which
# is not scanned, so the answer for the two surfaces is "no match at all".
#
# Exit: 0 clean; 1 a hand-written 500 was found; 2 a directory that cannot vouch for itself (missing, or holding no .rs file):
# a scan that looked at nothing would pass vacuously, which is not evidence. `--self-test` is the control, run next to the real
# check in CI: it fails if the check stops refusing what it is for.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PATTERN='INTERNAL_SERVER_ERROR|from_u16\([[:space:]]*500[[:space:]]*\)|try_from\([[:space:]]*500[[:space:]]*\)|\.status\([[:space:]]*500[[:space:]]*\)'

# check_dirs DIR...: prints a verdict per directory and returns the worst status (see the header).
check_dirs() {
  status=0
  for dir in "$@"; do
    if [ ! -d "$dir" ]; then
      echo "ERROR: $dir is not a directory, so there is nothing to vouch for"
      status=2
      continue
    fi
    files="$(find "$dir" -name '*.rs' | wc -l | tr -d ' ')"
    if [ "$files" -eq 0 ]; then
      echo "ERROR: no .rs file under $dir: a scan of nothing passes vacuously"
      status=2
      continue
    fi
    hits="$(grep -rnE "$PATTERN" --include='*.rs' "$dir")"
    if [ -z "$hits" ]; then
      echo "ok: $dir ($files .rs files, no hand-written 500)"
    else
      echo "FORBIDDEN: $dir writes a 500 itself; use thrust_plane_types::plane_500:"
      echo "$hits" | sed 's/^/    /'
      [ "$status" -lt 1 ] && status=1
    fi
  done
  return "$status"
}

self_test() {
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  failed=0

  # expect WANT LABEL DIR...: the check must return exactly WANT
  expect() {
    want="$1"
    label="$2"
    shift 2
    check_dirs "$@" >/dev/null 2>&1
    got=$?
    if [ "$got" -eq "$want" ]; then
      echo "ok: $label (exit $got)"
    else
      echo "SELF-TEST FAILED: $label: wanted exit $want, got $got"
      failed=1
    fi
  }

  mkdir -p "$tmp/clean" "$tmp/hit-const" "$tmp/hit-from-u16" "$tmp/hit-spaced" "$tmp/hit-try-from" "$tmp/hit-status" \
    "$tmp/hit-nested/a/b" "$tmp/empty" "$tmp/only-text"
  printf 'fn f() -> StatusCode { StatusCode::BAD_REQUEST }\nfn g() { let n = 5000; let _ = StatusCode::from_u16(5000); }\n' >"$tmp/clean/a.rs"
  printf 'fn f() -> StatusCode { StatusCode::INTERNAL_SERVER_ERROR }\n' >"$tmp/hit-const/a.rs"
  printf 'fn f() { let _ = StatusCode::from_u16(500); }\n' >"$tmp/hit-from-u16/a.rs"
  printf 'fn f() { let _ = StatusCode::from_u16( 500 ); }\n' >"$tmp/hit-spaced/a.rs"
  printf 'fn f() { let _ = StatusCode::try_from(500); }\n' >"$tmp/hit-try-from/a.rs"
  printf 'fn f() { let _ = Response::builder().status(500); }\n' >"$tmp/hit-status/a.rs"
  printf 'fn f() -> StatusCode { StatusCode::INTERNAL_SERVER_ERROR }\n' >"$tmp/hit-nested/a/b/a.rs"
  printf 'INTERNAL_SERVER_ERROR in a file the scan does not read\n' >"$tmp/only-text/notes.txt"

  expect 0 "a tree with no 500 is accepted" "$tmp/clean"
  expect 1 "StatusCode::INTERNAL_SERVER_ERROR is refused" "$tmp/hit-const"
  expect 1 "from_u16(500) is refused" "$tmp/hit-from-u16"
  expect 1 "from_u16( 500 ) with spaces is refused" "$tmp/hit-spaced"
  expect 1 "try_from(500) is refused" "$tmp/hit-try-from"
  expect 1 ".status(500) is refused" "$tmp/hit-status"
  expect 1 "a hit in a nested directory is refused" "$tmp/hit-nested"
  expect 1 "one clean tree beside a tree with a hit is refused" "$tmp/clean" "$tmp/hit-const"
  expect 2 "an empty directory is not a pass" "$tmp/empty"
  expect 2 "a directory with no .rs file is not a pass" "$tmp/only-text"
  expect 2 "a missing directory is not a pass" "$tmp/does-not-exist"
  expect 2 "a hit does not hide an empty directory beside it" "$tmp/hit-const" "$tmp/empty"

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
    if [ "$#" -eq 0 ]; then
      set -- "$ROOT/crates/thrust-api-v1/src" "$ROOT/crates/thrust-compat-plane/src"
    fi
    check_dirs "$@"
    exit $?
    ;;
esac
