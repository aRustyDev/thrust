#!/bin/sh
# Reads changed file paths on stdin (one per line) and prints `rust=true` or `rust=false`.
# Fails OPEN: an empty list (cannot tell what changed) is treated as "rust changed".
#
#   git diff --name-only "$BASE" "$HEAD" > changed.txt
#   .github/scripts/classify-changes.sh < changed.txt >> "$GITHUB_OUTPUT"
set -eu

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
cat > "$tmp"

if [ ! -s "$tmp" ]; then
  echo "rust=true"
  echo "classify: empty change list, failing open" >&2
  exit 0
fi

# Anything that can change what the Rust jobs build, test or run.
if grep -qE '^(crates/|scripts/|probes/|Cargo\.(toml|lock)$|rust-toolchain|\.cargo/|\.github/(workflows|actions|scripts)/)' "$tmp"; then
  echo "rust=true"
else
  echo "rust=false"
fi
