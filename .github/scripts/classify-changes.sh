#!/bin/sh
# Reads changed file paths on stdin (one per line) and prints `rust=true|false` and `web=true|false`.
# Fails OPEN: an empty list (cannot tell what changed) is treated as "rust and web changed".
#
#   git diff --name-only "$BASE" "$HEAD" > changed.txt
#   .github/scripts/classify-changes.sh < changed.txt >> "$GITHUB_OUTPUT"
set -eu

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
cat > "$tmp"

if [ ! -s "$tmp" ]; then
  echo "rust=true"
  echo "web=true"
  echo "classify: empty change list, failing open" >&2
  exit 0
fi

# Anything that can change what the Rust jobs build, test or run (.config/ holds nextest.toml, which decides
# what the test jobs select; deny.toml, clippy.toml and rustfmt.toml are read by the `deny` and `lint` jobs, so a change
# to one of them alone must run those jobs, not be skipped as docs). tools/oracle-v1/ is the v1 oracle's harness, register, corpus
# and results: the `oracle` cells of the contract matrix run it, so a change to it alone must run them, not be skipped.
if grep -qE '^(crates/|scripts/|probes/|tools/oracle-v1/|Cargo\.(toml|lock)$|rust-toolchain|\.cargo/|\.config/|\.github/(workflows|actions|scripts)/|deny\.toml$|\.?(clippy|rustfmt)\.toml$)' "$tmp"; then
  echo "rust=true"
else
  echo "rust=false"
fi

# The stock React app (web/, issue #44) has its own job: the Q-R2 gate and a build. A change to the CI files or to the gate
# script runs it too, because they are what it runs. It is independent of `rust`: a web-only change is rust=false.
if grep -qE '^(web/|scripts/check-web-diff\.sh$|\.github/(workflows|actions|scripts)/)' "$tmp"; then
  echo "web=true"
else
  echo "web=false"
fi
