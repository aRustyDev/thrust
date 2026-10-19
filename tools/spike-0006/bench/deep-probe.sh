#!/bin/sh
# Nesting-depth probe of the Option A Worker in local workerd: at which depth does the Rust build stop answering?
#   tools/spike-0006/bench/deep-probe.sh
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
OUT="$ROOT/target/spike-0006/bench"
mkdir -p "$OUT"
PORT="$(python3 -c 'import socket;s=socket.socket();s.bind(("127.0.0.1",0));print(s.getsockname()[1]);s.close()')"
STATE="$OUT/state-deep"
rm -rf "$STATE"
mkdir -p "$STATE"
cd "$ROOT/crates/thrust-editor-worker" || exit 99
env -i HOME="$HOME" PATH="$PATH" TMPDIR="${TMPDIR:-/tmp}" WRANGLER_SEND_METRICS=false NO_COLOR=1 \
  wrangler dev -c wrangler.toml --local --ip 127.0.0.1 --port "$PORT" --persist-to "$STATE" --show-interactive-dev-session=false --log-level log \
  > "$OUT/deep.wrangler.log" 2>&1 &
WPID=$!
i=0
READY=0
while [ "$i" -lt 600 ]; do
  if curl -s -o /dev/null --max-time 5 "http://127.0.0.1:$PORT/health"; then READY=1; break; fi
  kill -0 "$WPID" 2>/dev/null || break
  i=$((i + 1))
  sleep 1
done
[ "$READY" = 1 ] || { echo "not ready"; tail -n 20 "$OUT/deep.wrangler.log"; kill "$WPID" 2>/dev/null; exit 98; }
: > "$OUT/deep-probe.txt"
for shape in blockquote list div inline; do
  for depth in 10 100 500 1000 2000 3000 5000 8000 20000; do
    code="$(curl -s -o "$OUT/deep.body" -w '%{http_code}' --max-time 60 -X POST "http://127.0.0.1:$PORT/deep?shape=$shape&depth=$depth" 2>/dev/null)"
    echo "$shape $depth http=$code $(head -c 90 "$OUT/deep.body" | tr '\n' ' ')" >> "$OUT/deep-probe.txt"
  done
done
kill "$WPID" 2>/dev/null
wait "$WPID" 2>/dev/null
cat "$OUT/deep-probe.txt"
