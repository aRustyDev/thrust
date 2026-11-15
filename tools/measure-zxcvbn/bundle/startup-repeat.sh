#!/bin/sh
# Repeat scripts/startup-check.sh and print one line per run: which bundle was profiled, and the profile window.
#
#   tools/measure-zxcvbn/bundle/startup-repeat.sh <label> <runs>
#
# The bundle size is printed on every line because `wrangler check startup` (4.123.0) RE-RUNS the `[build]` command: it profiles the current
# source, not whatever is in `build/`. Spike 0001 said it reused the build output; here the first "baseline" repeat profiled the variant until
# this line's bundle size said so (6953.40 KiB, not the baseline's 4895.81 KiB). Read the label against the size.
set -u
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
cd "$ROOT" || exit 2
label="$1"
runs="$2"
i=0
while [ "$i" -lt "$runs" ]; do
  i=$((i + 1))
  out="$(scripts/startup-check.sh wrangler.toml 2>&1)"
  bundle="$(printf '%s\n' "$out" | grep 'Bundle:' | sed 's/^[^B]*//')"
  window="$(printf '%s\n' "$out" | grep 'Profile window' | sed 's/^[^P]*//')"
  active="$(printf '%s\n' "$out" | grep 'Active:' | sed 's/^[^A]*//')"
  echo "$label run $i | $bundle | $window | $active"
done
