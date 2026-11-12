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
# The compat acceptance list (issue #51) is held to its sources by `scripts/check-acceptance-list.sh`, which the `deps` job runs: the
# recorder (tools/compat-record/), the table the list is generated from, and the pinned route inventory it is joined with. A change to
# one of them alone must run that guard: a PR that only drops a step from record.py is the one the guard is for.
# e2e/ is the browser journey (issue #44) that the `e2e` cell of the contract matrix runs, so a change to it alone must run that cell.
# (A change to web/ alone does not: it runs the `web` job below, which builds the app and does not drive it. That is a known gap, G-30.)
if grep -qE '^(crates/|scripts/|e2e/|probes/|tools/oracle-v1/|tools/compat-record/|tools/compat-inventory/out-pinned/endpoint-inventory\.csv$|\.agents/plans/contrail/research/0003-compat-plane-inventory\.md$|Cargo\.(toml|lock)$|rust-toolchain|\.cargo/|\.config/|\.github/(workflows|actions|scripts)/|deny\.toml$|\.?(clippy|rustfmt)\.toml$)' "$tmp"; then
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
