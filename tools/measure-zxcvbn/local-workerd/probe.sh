#!/bin/sh
# Does the Worker run on local workerd, and does zxcvbn decide? (research/0015 section 4.1: the controls of the local run.)
#
#   tools/measure-zxcvbn/local-workerd/probe.sh [port]        exit 0 = every status was as expected; 1 = at least one was not; 2 = no server
#
# The build under test is the CURRENT SOURCE: scripts/worker-dev.sh builds it (the memory adapter set) and serves it with the operator's
# routes on and a throwaway token made here. For the variant, apply patches/variant-called.patch first. On the UNMODIFIED tree this probe MUST
# FAIL, at exactly the two requests that only zxcvbn refuses (the old rule accepts them, so they answer 204): that is the control that the
# probe can fail. Both outcomes are in results/2026-11-14/local-workerd.txt.
#
# Six requests to POST /internal/operator/users, in an order that matters (the 409 needs the 204 before it):
#   short             refused by the old rule's length bound; zxcvbn never runs                  400
#   abcdefghijkl      old rule accepts it, zxcvbn scores Zero                                    400 with zxcvbn, 204 without
#   qazwsxedcrfv      old rule accepts it, zxcvbn scores One                                     400 with zxcvbn, 204 without
#   the passphrase    passes both                                                                204
#   the same email    the route is live and reached the store                                    409
#   no token          the route's own gate                                                       401
# Only processes this script started are signalled (tree-kill by PID, as scripts/contract.sh does). Never pkill.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
cd "$ROOT" || exit 2
port="${1:-8791}"
base="http://127.0.0.1:$port"
THRUST_OPERATOR_TOKEN="$(openssl rand -hex 16)"
export THRUST_OPERATOR_TOKEN
logfile="${TMPDIR:-/tmp}/measure-zxcvbn-probe-$port.log"

kill_tree() {
  for child in $(pgrep -P "$1" 2>/dev/null); do
    kill_tree "$child"
  done
  kill -TERM "$1" 2>/dev/null || true
}

THRUST_PORT="$port" THRUST_ADAPTERS=memory THRUST_OPERATOR_ROUTES=1 scripts/worker-dev.sh >"$logfile" 2>&1 &
pid=$!
trap 'kill_tree "$pid"; sleep 2; kill -KILL "$pid" 2>/dev/null || true' EXIT INT TERM

n=0
code=000
while [ "$n" -lt 240 ]; do
  code="$(curl -s -o /dev/null -w '%{http_code}' --max-time 2 "$base/healthz")"
  [ "$code" = "200" ] && break
  n=$((n + 1))
  sleep 1
done
if [ "$code" != "200" ]; then echo "the Worker never answered /healthz with 200 (last $code); see $logfile" >&2; exit 2; fi
echo "healthz 200 after about ${n}s"

fail=0
expect() { # name want email password [token: yes|no]
  if [ "${5:-yes}" = "yes" ]; then
    got="$(curl -s -o /dev/null -w '%{http_code}' --max-time 30 -X POST "$base/internal/operator/users" -H 'content-type: application/json' \
      --variable %THRUST_OPERATOR_TOKEN --expand-header 'x-operator-token: {{THRUST_OPERATOR_TOKEN}}' --data "{\"email\":\"$3\",\"password\":\"$4\"}")"
  else
    got="$(curl -s -o /dev/null -w '%{http_code}' --max-time 30 -X POST "$base/internal/operator/users" -H 'content-type: application/json' \
      --data "{\"email\":\"$3\",\"password\":\"$4\"}")"
  fi
  if [ "$got" = "$2" ]; then echo "ok    $1: $got"; else echo "FAIL  $1: wanted $2, got $got"; fail=$((fail + 1)); fi
}
expect "short (old rule refuses)" 400 s1@example.test short
expect "abcdefghijkl (only zxcvbn refuses)" 400 a1@example.test abcdefghijkl
expect "qazwsxedcrfv (only zxcvbn refuses)" 400 a2@example.test qazwsxedcrfv
expect "the passphrase (passes both)" 204 a3@example.test 'tr0ub4dor-&-3-horses-lantern'
expect "the same email again" 409 a3@example.test 'tr0ub4dor-&-3-horses-lantern'
expect "no token" 401 a4@example.test abcdefghijkl no
echo "$fail of 6 requests answered unexpectedly"
[ "$fail" -eq 0 ]
