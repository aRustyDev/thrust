#!/bin/sh
# Check flake-hunt.yml's inputs before any build: the target and pass must be a cell of the contract matrix, and the iteration count a
# whole number from 1 to 50.
#
#   check-flake-hunt-cell.sh <target> <pass> <iterations> [contract-all.sh]
#
# The cells are read from scripts/contract-all.sh, the list the CI matrix is planned from (plan-cells.sh), so a pass added to the
# matrix can be hunted the day it is added, and a pairing the matrix does not run (`cron` on a native target, `e2e` on native-pg) is
# refused here instead of after the build. Exit 0 when the inputs are good, 1 (with a `::error::` line) when not, 2 on a usage error.
set -u

[ "$#" -ge 3 ] && [ "$#" -le 4 ] || { echo "usage: check-flake-hunt-cell.sh <target> <pass> <iterations> [contract-all.sh]" >&2; exit 2; }
TARGET="$1"
PASS="$2"
ITER="$3"
SRC="${4:-scripts/contract-all.sh}"
[ -f "$SRC" ] || { echo "check-flake-hunt-cell: $SRC not found" >&2; exit 2; }

case "$ITER" in
  "" | *[!0-9]*) echo "::error::iterations must be a whole number, got '$ITER'"; exit 1 ;;
esac
# A leading zero is refused (so is 0 itself, which makes 1 the lower bound), and so is a third digit, before `[ -gt ]` reads the
# number: a 20-digit one would overflow it.
case "$ITER" in
  0*) echo "::error::iterations must be between 1 and 50, got '$ITER'"; exit 1 ;;
esac
if [ "${#ITER}" -gt 2 ] || [ "$ITER" -gt 50 ]; then
  echo "::error::iterations must be between 1 and 50, got '$ITER'"
  exit 1
fi

# The same line format plan-cells.sh reads:  run <target> <pass>
cells="$(grep -E '^run [a-z0-9-]+ [a-z0-9-]+[[:space:]]*$' "$SRC" | awk '{print $2 " " $3}')"
[ -n "$cells" ] || { echo "check-flake-hunt-cell: parsed zero cells from $SRC" >&2; exit 2; }

if printf '%s\n' "$cells" | grep -qxF -- "$TARGET $PASS"; then
  echo "flake-hunt: $TARGET $PASS, $ITER iterations"
  exit 0
fi
passes="$(printf '%s\n' "$cells" | awk -v t="$TARGET" '$1 == t {print $2}' | tr '\n' ' ')"
if [ -z "$passes" ]; then
  echo "::error::'$TARGET' is not a target of the contract matrix ($SRC)"
else
  echo "::error::'$TARGET $PASS' is not a cell of the contract matrix ($SRC); $TARGET runs: ${passes% }"
fi
exit 1
