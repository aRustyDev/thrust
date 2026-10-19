#!/bin/sh
# Run one Worker (Rust build or the esbuild-bundled JS editor code) under local `wrangler dev` and benchmark it.
#   tools/spike-0006/bench/run-bench.sh <label> <dir-with-wrangler.toml> <config-file> [plan-json]
# Local only: --local, no login, nothing deployed. wrangler is started under `env -i`.
set -u
LABEL="${1:?label}"
DIR="${2:?dir}"
CFG="${3:?config}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
OUT="$ROOT/target/spike-0006/bench"
mkdir -p "$OUT"
PORT="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"
STATE="$OUT/state-$LABEL"
rm -rf "$STATE"
mkdir -p "$STATE"
cd "$DIR" || exit 99
env -i HOME="$HOME" PATH="$PATH" TMPDIR="${TMPDIR:-/tmp}" WRANGLER_SEND_METRICS=false NO_COLOR=1 \
  wrangler dev -c "$CFG" --local --ip 127.0.0.1 --port "$PORT" --persist-to "$STATE" --show-interactive-dev-session=false --log-level log \
  > "$OUT/$LABEL.wrangler.log" 2>&1 &
WPID=$!
# wait until the dev server answers (the Rust build runs inside wrangler's [build] step first)
i=0
READY=0
while [ "$i" -lt 600 ]; do
  if curl -s -o /dev/null --max-time 5 -w "first-successful-health-request-ms=%{time_total}\n" "http://127.0.0.1:$PORT/health" > "$OUT/$LABEL.cold.txt" 2>/dev/null; then READY=1; break; fi
  kill -0 "$WPID" 2>/dev/null || break   # wrangler exited (build or startup failure): do not wait it out
  i=$((i + 1))
  sleep 1
done
if [ "$READY" != 1 ]; then
  echo "wrangler dev did not become ready; log tail:"
  tail -n 30 "$OUT/$LABEL.wrangler.log"
  kill "$WPID" 2>/dev/null
  exit 98
fi
# cold-ish first request time after the dev server is up (module instantiation happens on the first request)
curl -s -o /dev/null -w "first-health-ms=%{time_total}\n" "http://127.0.0.1:$PORT/health" > "$OUT/$LABEL.first.txt"
curl -s -o /dev/null -w "second-health-ms=%{time_total}\n" "http://127.0.0.1:$PORT/health" >> "$OUT/$LABEL.first.txt"
curl -s "http://127.0.0.1:$PORT/env" > "$OUT/$LABEL.env.json" 2>/dev/null || true
if [ -n "${4:-}" ]; then export BENCH_PLAN="$4"; fi
rc=0
node "$HERE/bench-worker.mjs" "http://127.0.0.1:$PORT" "$LABEL" "$ROOT/crates/thrust-editor-doc/tests/corpus/corpus.json" "$ROOT/target/spike-0006/corpus-large.json" > "$OUT/$LABEL.json" 2> "$OUT/$LABEL.err" || rc=$?
kill "$WPID" 2>/dev/null
wait "$WPID" 2>/dev/null
echo "bench rc=$rc  results: $OUT/$LABEL.json"
cat "$OUT/$LABEL.first.txt"
exit $rc
