#!/bin/sh
# Timing on local workerd (research/0015 section 4.1): N cold starts of the Worker; per start, the wall time (curl, seconds, loopback) of
#   short        refused by the old rule before zxcvbn runs (a request that does not touch zxcvbn)
#   first_zxcvbn the first request that reaches zxcvbn in that isolate (the dictionaries are built here)
#   warm x3      later requests that reach zxcvbn
# It is WALL clock, not CPU, and local workerd is not the platform: tools/measure-zxcvbn/cloudflare is. The variant is the current source (apply
# patches/variant-called.patch first); on the unmodified tree zxcvbn never runs and the first request is no slower than the others.
#
#   tools/measure-zxcvbn/local-workerd/cold-runs.sh <runs> <port> [outdir]      (THRUST_OPERATOR_TOKEN is made here)
# Only processes this script started are signalled (tree-kill by PID, as scripts/contract.sh does). Never pkill. After each kill it checks that
# nothing still listens on the port; `lsof` saw the listener while the server was up, so "nothing left" means something.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
cd "$ROOT" || exit 2
runs="${1:?usage: cold-runs.sh <runs> <port> [outdir]}"
port="${2:?usage: cold-runs.sh <runs> <port> [outdir]}"
logdir="${3:-$ROOT/target/measure-zxcvbn-local}"
mkdir -p "$logdir"
base="http://127.0.0.1:$port"
THRUST_OPERATOR_TOKEN="$(openssl rand -hex 16)"
export THRUST_OPERATOR_TOKEN

kill_tree() {
  for child in $(pgrep -P "$1" 2>/dev/null); do
    kill_tree "$child"
  done
  kill -TERM "$1" 2>/dev/null || true
}

post_time() { # email password -> seconds
  curl -s -o /dev/null -w '%{time_total}' --max-time 30 -X POST "$base/internal/operator/users" \
    -H 'content-type: application/json' --variable %THRUST_OPERATOR_TOKEN --expand-header 'x-operator-token: {{THRUST_OPERATOR_TOKEN}}' \
    --data "{\"email\":\"$1\",\"password\":\"$2\"}"
}
post_code() { # email password -> http status
  curl -s -o /dev/null -w '%{http_code}' --max-time 30 -X POST "$base/internal/operator/users" \
    -H 'content-type: application/json' --variable %THRUST_OPERATOR_TOKEN --expand-header 'x-operator-token: {{THRUST_OPERATOR_TOKEN}}' \
    --data "{\"email\":\"$1\",\"password\":\"$2\"}"
}

run=0
while [ "$run" -lt "$runs" ]; do
  run=$((run + 1))
  THRUST_PORT="$port" THRUST_ADAPTERS=memory THRUST_OPERATOR_ROUTES=1 scripts/worker-dev.sh >"$logdir/run$run.log" 2>&1 &
  pid=$!
  n=0
  code=000
  while [ "$n" -lt 240 ]; do
    code="$(curl -s -o /dev/null -w '%{http_code}' --max-time 2 "$base/healthz")"
    [ "$code" = "200" ] && break
    n=$((n + 1))
    sleep 1
  done
  listening="$(lsof -nP -iTCP:"$port" -sTCP:LISTEN | wc -l | tr -d ' ')"
  if [ "$code" != "200" ]; then
    echo "run $run: healthz never answered 200 (last $code); skipped"
  else
    a="$(post_time s1@example.test short)"
    b="$(post_time a1@example.test abcdefghijkl)"
    c="$(post_time a2@example.test qazwsxedcrfv)"
    d="$(post_time a3@example.test abcdefghijkl)"
    e="$(post_time a5@example.test qazwsxedcrfv)"
    s="$(post_code a6@example.test abcdefghijkl)"
    echo "run $run: short=$a first_zxcvbn=$b warm=$c $d $e (the last request answered $s; 400 with zxcvbn; listeners seen while up: $listening)"
  fi
  kill_tree "$pid"
  sleep 3
  kill -KILL "$pid" 2>/dev/null || true
  m=0
  left=1
  while [ "$m" -lt 20 ]; do
    left="$(lsof -nP -iTCP:"$port" -sTCP:LISTEN | wc -l | tr -d ' ')"
    [ "$left" = "0" ] && break
    m=$((m + 1))
    sleep 1
  done
  echo "run $run: listeners left on :$port after the kill: $left"
done
