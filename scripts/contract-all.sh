#!/bin/sh
# Run the whole matrix serially (parallel runs share crates/thrust-worker/build and a second
# wrangler rebuild reloads the first one's worker: measured). Writes target/contract-logs/summary.txt
# and exits non-zero if any cell failed. Each line: target, pass, verdict, and the test count.
#
#   scripts/contract-all.sh
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SUMMARY="$ROOT/target/contract-logs/summary.txt"
mkdir -p "$ROOT/target/contract-logs"
: >"$SUMMARY"
FAILED=0

run() {
  "$ROOT/scripts/contract.sh" "$1" "$2" >"$ROOT/target/contract-logs/$1-$2.run.log" 2>&1
  status=$?
  if [ "$status" -eq 0 ]; then verdict=PASS; else verdict="FAIL(exit $status)"; FAILED=1; fi
  counts="$(grep -o 'passed=[0-9?]* expected=[0-9]*' "$ROOT/target/contract-logs/$1-$2.run.log" | tail -1)"
  printf '%-14s %-9s %-14s %s\n' "$1" "$2" "$verdict" "$counts" >>"$SUMMARY"
}

run native-memory main
run native-memory sweeper
run native-memory panic
run native-memory csrfoff
run native-pg main
run native-pg sweeper
run native-pg panic
run worker-memory main
run worker-memory sweeper
run worker-memory panic
run worker-memory cron
run worker-memory csrfoff
run worker-cf main
run worker-cf sweeper
run worker-cf panic
run worker-cf cron
run worker-cf worker
run worker-cf bindings

cat "$SUMMARY"
exit "$FAILED"
