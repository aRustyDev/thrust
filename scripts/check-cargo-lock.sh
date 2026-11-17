#!/bin/sh
# Cargo.lock is what cargo itself writes: every package where cargo sorts it, and nothing cargo would add or change.
#
#   scripts/check-cargo-lock.sh              the check on this repository's lockfile
#   scripts/check-cargo-lock.sh --self-test  the check on workspaces it must accept and refuse (no dependencies, no network)
#
# `cargo metadata --locked`, which the rest of CI runs, refuses a lockfile that would change in what it says, but it accepts one
# whose packages are in the wrong order. A cargo command without `--locked` then rewrites that file, so a clean checkout is dirty
# after its first cargo run (#279 added `thrust-preview-provisioner` one place late, and #284 moved it; nothing had failed). This
# runs cargo the way a person does, `cargo metadata` without `--locked`, and says so when the lockfile is not what cargo wrote
# back. That also catches a lockfile that has not heard of a change to a manifest. The lockfile is put back as it was found,
# whatever the result, so the check never leaves a checkout dirty.
#
# Exit status: 0 cargo left the lockfile as it was; 1 cargo rewrote it (the difference is printed); 2 it cannot vouch (no
# Cargo.toml, no lockfile, no cargo, or cargo failed, which is not an answer about the order). POSIX sh: CI runs it under dash.
#
# CHECK_CARGO=<cargo> uses that cargo and not `scripts/rs cargo` (a container whose cargo is already the right one).
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
T=""
LOCK=""
KEEP=""

# put the lockfile back as it was found (a failed or interrupted run included), and remove the scratch directory
# shellcheck disable=SC2329 # run by the trap below, which shellcheck does not follow
cleanup() {
  if [ -n "$LOCK" ] && [ -f "$KEEP" ]; then
    cp "$KEEP" "$LOCK"
  fi
  if [ -n "$T" ]; then
    rm -rf "$T"
  fi
}
trap cleanup EXIT
trap 'exit 130' INT TERM

run_cargo() {
  if [ -n "${CHECK_CARGO:-}" ]; then
    "$CHECK_CARGO" "$@"
  else
    "$ROOT/scripts/rs" cargo "$@"
  fi
}

# check_world <workspace root>: returns 0, 1 or 2 as above, and says why on stderr
check_world() {
  cw_root="$1"
  cw_lock="$cw_root/Cargo.lock"
  if [ ! -f "$cw_root/Cargo.toml" ]; then
    echo "check-cargo-lock: $cw_root has no Cargo.toml" >&2
    return 2
  fi
  if [ ! -f "$cw_lock" ]; then
    echo "check-cargo-lock: $cw_root has no Cargo.lock, so there is nothing to say about its order" >&2
    return 2
  fi
  cw_keep="$T/kept.$$.$cw_n"
  cw_n=$((cw_n + 1))
  cp "$cw_lock" "$cw_keep"
  LOCK="$cw_lock"
  KEEP="$cw_keep"
  if ! run_cargo metadata --format-version 1 --manifest-path "$cw_root/Cargo.toml" >/dev/null 2>"$T/metadata.err"; then
    cp "$cw_keep" "$cw_lock"
    LOCK=""
    echo "check-cargo-lock: cargo metadata failed, so it cannot say what cargo writes:" >&2
    tail -5 "$T/metadata.err" >&2
    return 2
  fi
  if cmp -s "$cw_keep" "$cw_lock"; then
    LOCK=""
    return 0
  fi
  diff -u -L "Cargo.lock as committed" -L "Cargo.lock as cargo writes it" "$cw_keep" "$cw_lock" | head -40 >&2
  cp "$cw_keep" "$cw_lock"
  LOCK=""
  echo "check-cargo-lock: Cargo.lock is not what cargo writes (cargo, run without --locked, rewrote it as above)." >&2
  echo "  Run \`cargo metadata > /dev/null\` and commit the file, and find out why it was not in cargo's form." >&2
  return 1
}
cw_n=0

# --- the control: workspaces it must accept and refuse --------------------------------------------------------------------------

# make_world <dir> <member>...: a workspace of path crates with no dependencies, and the lockfile cargo writes for it
make_world() {
  mw_dir="$1"
  shift
  mkdir -p "$mw_dir"
  mw_members=""
  for mw_m in "$@"; do
    mkdir -p "$mw_dir/$mw_m/src"
    printf '[package]\nname = "%s"\nversion = "0.1.0"\nedition = "2021"\n' "$mw_m" >"$mw_dir/$mw_m/Cargo.toml"
    : >"$mw_dir/$mw_m/src/lib.rs"
    mw_members="$mw_members\"$mw_m\", "
  done
  printf '[workspace]\nmembers = [%s]\nresolver = "2"\n' "${mw_members%, }" >"$mw_dir/Cargo.toml"
  run_cargo generate-lockfile --manifest-path "$mw_dir/Cargo.toml" >/dev/null 2>"$T/generate.err" || {
    echo "check-cargo-lock: self-test: cargo could not write a lockfile for its own fixture:" >&2
    tail -5 "$T/generate.err" >&2
    exit 2
  }
}

# swap_two <lockfile>: the first two packages in the other order (the header, then the second package, then the first)
swap_two() {
  awk 'BEGIN { RS = "" } { r[NR] = $0 } END { if (NR != 3) exit 3; printf "%s\n\n%s\n\n%s\n", r[1], r[3], r[2] }' "$1" >"$1.swapped" || {
    echo "check-cargo-lock: self-test: the fixture lockfile is not a header and two packages" >&2
    exit 2
  }
  mv "$1.swapped" "$1"
}

self_test() {
  # the fixtures have no dependencies, so nothing here needs a network
  CARGO_NET_OFFLINE=true
  export CARGO_NET_OFFLINE
  st_n=0
  st_bad=0
  # expect <label> <status wanted> <world dir> [text the output must have]
  expect() {
    st_n=$((st_n + 1))
    check_world "$3" >"$T/st.out" 2>&1
    st_got=$?
    if [ "$st_got" -ne "$2" ]; then
      st_bad=$((st_bad + 1))
      echo "BAD  $1: exit $st_got, wanted $2"
      sed 's/^/     | /' "$T/st.out" | head -8
    elif [ -n "${4:-}" ] && ! grep -qF -- "$4" "$T/st.out"; then
      st_bad=$((st_bad + 1))
      echo "BAD  $1: the output lacks '$4'"
      sed 's/^/     | /' "$T/st.out" | head -8
    else
      echo "ok   $1 (exit $st_got)"
    fi
  }
  # unchanged <label> <lockfile> <copy it was before>: the check put the file back as it found it
  unchanged() {
    st_n=$((st_n + 1))
    if cmp -s "$2" "$3"; then
      echo "ok   $1"
    else
      st_bad=$((st_bad + 1))
      echo "BAD  $1: the lockfile is not as it was found"
    fi
  }

  # what cargo wrote is accepted, and left alone
  make_world "$T/w-good" a b
  cp "$T/w-good/Cargo.lock" "$T/w-good.before"
  expect "the lockfile cargo wrote" 0 "$T/w-good"
  unchanged "  and it is left as it was" "$T/w-good/Cargo.lock" "$T/w-good.before"

  # the defect: two packages in the wrong order, which `cargo metadata --locked` accepts and a plain cargo run rewrites
  make_world "$T/w-swapped" a b
  swap_two "$T/w-swapped/Cargo.lock"
  cp "$T/w-swapped/Cargo.lock" "$T/w-swapped.before"
  expect "two packages in the wrong order" 1 "$T/w-swapped" "is not what cargo writes"
  unchanged "  and the disordered file is put back, not left rewritten" "$T/w-swapped/Cargo.lock" "$T/w-swapped.before"

  # a lockfile that has not heard of a member the workspace has
  make_world "$T/w-stale" a b
  mkdir -p "$T/w-stale/c/src"
  printf '[package]\nname = "c"\nversion = "0.1.0"\nedition = "2021"\n' >"$T/w-stale/c/Cargo.toml"
  : >"$T/w-stale/c/src/lib.rs"
  printf '[workspace]\nmembers = ["a", "b", "c"]\nresolver = "2"\n' >"$T/w-stale/Cargo.toml"
  cp "$T/w-stale/Cargo.lock" "$T/w-stale.before"
  expect "a member the lockfile has not heard of" 1 "$T/w-stale" "is not what cargo writes"
  unchanged "  and it is put back" "$T/w-stale/Cargo.lock" "$T/w-stale.before"

  # it cannot vouch: no lockfile, no manifest, cargo failing (and the lockfile is put back then too)
  make_world "$T/w-nolock" a b
  rm "$T/w-nolock/Cargo.lock"
  expect "no lockfile" 2 "$T/w-nolock" "no Cargo.lock"
  mkdir -p "$T/w-nomanifest"
  : >"$T/w-nomanifest/Cargo.lock"
  expect "no manifest" 2 "$T/w-nomanifest" "no Cargo.toml"
  make_world "$T/w-broken" a b
  printf '[workspace]\nmembers = ["no-such-member"]\nresolver = "2"\n' >"$T/w-broken/Cargo.toml"
  cp "$T/w-broken/Cargo.lock" "$T/w-broken.before"
  expect "cargo that fails" 2 "$T/w-broken" "cannot say what cargo writes"
  unchanged "  and the lockfile is put back" "$T/w-broken/Cargo.lock" "$T/w-broken.before"

  echo "check-cargo-lock --self-test: $((st_n - st_bad)) of $st_n as they must be"
  [ "$st_bad" -eq 0 ]
}

# --- main -----------------------------------------------------------------------------------------------------------------------

T="$(mktemp -d)" || {
  echo "check-cargo-lock: cannot make a scratch directory" >&2
  exit 2
}
case "${1:-}" in
  "")
    check_world "$ROOT"
    rc=$?
    if [ "$rc" -eq 0 ]; then
      echo "check-cargo-lock: Cargo.lock is what cargo writes ($(grep -c '^\[\[package\]\]' "$ROOT/Cargo.lock") packages)"
    fi
    exit "$rc"
    ;;
  --self-test)
    self_test
    exit $?
    ;;
  *)
    echo "usage: scripts/check-cargo-lock.sh [--self-test]" >&2
    exit 2
    ;;
esac
