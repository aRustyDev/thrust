#!/bin/sh
# Derive the CI cell list from the single source of truth, scripts/contract-all.sh, instead of
# keeping a second list that can drift. Prints two outputs for $GITHUB_OUTPUT:
#   matrix=<{"include":[{"target":..,"pass":..},...]}>
#   cells=<["target pass", ...]>
#
# Vacuity guards: the number of parsed cells must equal the number of lines that look like a cell
# invocation (`run <word> <word>`), and must be at least 1.
set -eu

SRC="${1:-scripts/contract-all.sh}"
[ -f "$SRC" ] || { echo "plan: $SRC not found" >&2; exit 1; }

# lines of the form:  run native-memory main
grep -E '^run [a-z0-9-]+ [a-z0-9-]+[[:space:]]*$' "$SRC" | awk '{print $2 " " $3}' > cells.txt

# every line that starts with `run ` (the function definition is `run() {`, which has no space)
invocations="$(grep -cE '^run[[:space:]]' "$SRC" || true)"
parsed="$(wc -l < cells.txt | tr -d ' ')"

[ "$parsed" -ge 1 ] || { echo "plan: parsed zero cells from $SRC" >&2; exit 1; }
[ "$parsed" = "$invocations" ] || {
  echo "plan: parsed $parsed cells but $SRC has $invocations 'run ...' lines; the format changed" >&2
  exit 1
}
if [ "$(sort cells.txt | uniq -d | wc -l | tr -d ' ')" != "0" ]; then
  echo "plan: duplicate cells in $SRC" >&2
  exit 1
fi

matrix="$(jq -Rn -c '{include: [inputs | split(" ") | {target: .[0], pass: .[1]}]}' < cells.txt)"
cells="$(jq -Rn -c '[inputs]' < cells.txt)"
echo "matrix=$matrix"
echo "cells=$cells"
