#!/bin/sh
# Re-run one contract cell N times, keeping every iteration's logs, to characterise flakiness.
#   scripts/flake-hunt.sh <target> <pass> <iterations>
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TARGET="${1:?target}"
PASS="${2:?pass}"
N="${3:-5}"
OUT="$ROOT/target/flake-hunt/$TARGET-$PASS"
mkdir -p "$OUT"
FAILS=0
i=1
while [ "$i" -le "$N" ]; do
  THRUST_LOGDIR="$OUT/iter$i" "$ROOT/scripts/contract.sh" "$TARGET" "$PASS" >"$OUT/iter$i.out" 2>&1
  status=$?
  echo "iter $i: exit=$status"
  if [ "$status" -ne 0 ]; then FAILS=$((FAILS + 1)); fi
  i=$((i + 1))
done
echo "failures: $FAILS / $N"
