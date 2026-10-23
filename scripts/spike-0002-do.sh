#!/bin/sh
# Cell `worker-do` on the Woven Cloudflare account (spike 0002, issue #37): create, inspect and tear down the resources.
#
#   scripts/spike-0002-do.sh up [--var NAME:VALUE ...]   create the bucket and queues, deploy the Worker, put the admin secret
#   scripts/spike-0002-do.sh deploy [--var NAME:VALUE ...]   deploy the Worker again (a new build, or another variable)
#   scripts/spike-0002-do.sh status                      what exists under the thrust-spike-do prefix, and the ledger
#   scripts/spike-0002-do.sh url                         the workers.dev URL
#   scripts/spike-0002-do.sh down                        tear everything down, in the order research/0004 section 10.7 found
#
# Credentials: wrangler's OAuth login (workers, queues and R2 are in its scopes; the `thrust-spikes` API token could not do
# R2). The account is the Woven one; its id is an identifier, not a secret. The admin token is generated into a 0600 file
# under target/ and sent as a Worker secret over stdin: never on argv, never printed.
#
# Everything created is appended to target/spike-do/ledger.txt with a UTC time, which is the resource ledger of the run.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export CLOUDFLARE_ACCOUNT_ID=4d90c2d31b480584f8f878d2f62848ab
export WRANGLER_SEND_METRICS=false
STATE="$ROOT/target/spike-do"
LEDGER="$STATE/ledger.txt"
TOKEN_FILE="$STATE/admin-token"
CONFIG=wrangler.do.stage.toml
WORKER=thrust-spike-do
BUCKET=thrust-spike-do-blobs
QUEUE=thrust-spike-do-events
DLQ=thrust-spike-do-events-dlq
WR="$ROOT/crates/thrust-worker"
mkdir -p "$STATE"

note() { printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" | tee -a "$LEDGER"; }

deploy() {
  (cd "$WR" && wrangler deploy -c "$CONFIG" "$@") || { note "FAILED: wrangler deploy"; return 1; }
  note "deployed worker $WORKER ($*)"
}

case "${1:-}" in
  up)
    shift
    if [ ! -f "$TOKEN_FILE" ]; then
      umask 077
      openssl rand -hex 16 >"$TOKEN_FILE"
    fi
    chmod 600 "$TOKEN_FILE"
    wrangler r2 bucket create "$BUCKET" && note "created r2 bucket $BUCKET" || exit 1
    wrangler queues create "$DLQ" && note "created queue $DLQ" || exit 1
    wrangler queues create "$QUEUE" && note "created queue $QUEUE" || exit 1
    deploy "$@" || exit 1
    (cd "$WR" && wrangler secret put THRUST_ADMIN_TOKEN -c "$CONFIG" <"$TOKEN_FILE") && note "put worker secret THRUST_ADMIN_TOKEN" || exit 1
    echo "token file: $TOKEN_FILE (${#TOKEN_FILE} chars of path; $(wc -c <"$TOKEN_FILE" | tr -d ' ') bytes)"
    ;;
  deploy)
    shift
    deploy "$@"
    ;;
  url)
    (cd "$WR" && wrangler deployments list -c "$CONFIG" 2>&1 | sed -n 's/.*\(https:\/\/[a-z0-9.-]*workers\.dev\).*/\1/p' | head -1)
    echo "https://$WORKER.woven-4d9.workers.dev"
    ;;
  status)
    echo "--- ledger"; cat "$LEDGER" 2>/dev/null || echo "(none)"
    echo "--- queues"; wrangler queues list 2>&1 | grep -E "thrust-spike-do|No queues" || echo "none under thrust-spike-do"
    echo "--- r2 bucket"; wrangler r2 bucket info "$BUCKET" 2>&1 | grep -E "name:|does not exist" | head -2
    echo "--- worker"; (cd "$WR" && wrangler deployments list -c "$CONFIG" 2>&1 | grep -E "Created|not found|workers.dev|could not" | head -3)
    ;;
  down)
    # A Worker cannot be deleted while it is a queue consumer, and a queue cannot be deleted while a Worker binds it:
    # remove the consumer first, then the Worker, then the queues, then the (empty) bucket.
    status=0
    wrangler queues consumer remove "$QUEUE" "$WORKER" && note "removed queue consumer $WORKER of $QUEUE" || { note "consumer remove failed or absent"; status=1; }
    # An R2 bucket must be empty before it is deleted, and wrangler cannot bulk delete: empty it through the binding.
    if [ -f "$TOKEN_FILE" ] && [ -n "${THRUST_SPIKE_DO_URL:-}" ]; then
      TOKEN="$(cat "$TOKEN_FILE")"
      printf 'header = "x-admin-token: %s"\n' "$TOKEN" | curl -sS -m 120 -K - "$THRUST_SPIKE_DO_URL/_test/bindings/r2purge" && echo
    fi
    (cd "$WR" && wrangler delete --name "$WORKER" --force) && note "deleted worker $WORKER (with its Durable Object storage, cron and secret)" || { note "worker delete failed"; status=1; }
    wrangler queues delete "$QUEUE" && note "deleted queue $QUEUE" || { note "queue delete failed: $QUEUE"; status=1; }
    wrangler queues delete "$DLQ" && note "deleted queue $DLQ" || { note "queue delete failed: $DLQ"; status=1; }
    wrangler r2 bucket delete "$BUCKET" && note "deleted r2 bucket $BUCKET" || { note "bucket delete failed: $BUCKET"; status=1; }
    exit "$status"
    ;;
  *)
    echo "usage: $0 up|deploy|status|url|down" >&2
    exit 2
    ;;
esac
