#!/bin/sh
# Cell `worker-pg` on the Woven Cloudflare account and the CNPG dev database (spike 0002, issue #37): create, inspect and tear down
# the resources, in the order research/0004 section 10.7 found.
#
#   scripts/spike-0002-pg.sh role                 create role + schema pr_<n> on thrust-pg-dev (pr-schema.sh)
#   scripts/spike-0002-pg.sh migrate              apply crates/thrust-adapters-postgres/migrations/*.sql AS that role (a real password login)
#   scripts/spike-0002-pg.sh hyperdrive           create the two Hyperdrive configs: thrust-spike-pg (caching off) and thrust-spike-pg-cached (on)
#   scripts/spike-0002-pg.sh resources            create the R2 bucket and the two queues
#   scripts/spike-0002-pg.sh deploy [--cached] [--no-placement] [--var NAME:VALUE ...]   deploy the Worker (the first deploy also puts the admin secret)
#   scripts/spike-0002-pg.sh sql                  run SQL from stdin on the primary as the postgres OS user (superuser, local socket): READ-ONLY use
#   scripts/spike-0002-pg.sh status               what exists, and the ledger
#   scripts/spike-0002-pg.sh down                 tear everything down: consumer, Worker, queues, bucket, Hyperdrive configs, role + schema
#
# Environment (nothing here is committed): CLOUDFLARE_ACCOUNT_ID; THRUST_SPIKE_PG_URL (the Worker's workers.dev URL) for `down`, to empty the bucket; HYPERDRIVE_API_TOKEN for `hyperdrive`, `deploy` and `down` (Hyperdrive Write);
# AWS_PROFILE for the cluster steps (the SSO session lasts about an hour); THRUST_DEV_VPC_SERVICE_ID for `hyperdrive`; THRUST_INFRA_ROOT (the
# infrastructure repo's `infrastructure/` directory, default ~/repos/woven/infrastructure/infrastructure); THRUST_SPIKE_PR (the role number,
# default 1101). R2, queues and the Worker go through wrangler's OAuth login (the token has no R2 or Queues permission). The database password
# is a 0600 file under target/ and travels on stdin; the admin token likewise. Every step appends to target/spike-pg/ledger.txt with a UTC time.
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
: "${CLOUDFLARE_ACCOUNT_ID:?set CLOUDFLARE_ACCOUNT_ID to the Woven Cloudflare account id (not committed: this package redacts it)}"
export CLOUDFLARE_ACCOUNT_ID WRANGLER_SEND_METRICS=false
PR="${THRUST_SPIKE_PR:-1101}"
INFRA="${THRUST_INFRA_ROOT:-$HOME/repos/woven/infrastructure/infrastructure}/products/thrust"
CONTEXT="${THRUST_KUBE_CONTEXT:-woven-o11y}"
STATE="$ROOT/target/spike-pg"
LEDGER="$STATE/ledger.txt"
PWFILE="$STATE/pr-$PR.pw"
TOKEN_FILE="$STATE/admin-token"
WORKER=thrust-spike-pg
BUCKET=thrust-spike-pg-blobs
QUEUE=thrust-spike-pg-events
DLQ=thrust-spike-pg-events-dlq
HD=thrust-spike-pg
HD_CACHED=thrust-spike-pg-cached
WR="$ROOT/crates/thrust-worker"
TEMPLATE=wrangler.pg.stage.toml
RUNCFG=.wrangler.pg.run.toml
mkdir -p "$STATE"

note() { printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" | tee -a "$LEDGER"; }
kc() { kubectl --context "$CONTEXT" -n thrust "$@"; }
primary() { kc get cluster thrust-pg-dev -o jsonpath='{.status.currentPrimary}'; }
drop_runcfg() { rm -f "${WR:?}/${RUNCFG:?}"; }

# kubectl as pr-schema.sh wants it: one executable pinned to the context (the default context here is another cluster)
WRAP="$STATE/kubectl-pinned"
printf '#!/bin/sh\nexec kubectl --context %s "$@"\n' "$CONTEXT" >"$WRAP"
chmod +x "$WRAP"

as_role() { # SQL on stdin; a real password login to the primary over TCP, the password on stdin
  pod="$(primary)" || return 1
  { cat "$PWFILE"; echo; cat; } | kc exec -i "$pod" -c postgres -- sh -c "IFS= read -r PGPASSWORD; export PGPASSWORD PGSSLMODE=require; exec psql -X -q -h 127.0.0.1 -U pr_$PR -d thrust -v ON_ERROR_STOP=1 -t -A"
}

hd_api() { # method path [json body]; the token only in curl's stdin config
  m="$1"; p="$2"
  if [ -n "${3:-}" ]; then
    printf 'header = "Authorization: Bearer %s"\n' "$HYPERDRIVE_API_TOKEN" | curl -sS -m 30 -K - -X "$m" -H 'Content-Type: application/json' --data "$3" "https://api.cloudflare.com/client/v4/accounts/$CLOUDFLARE_ACCOUNT_ID$p"
  else
    printf 'header = "Authorization: Bearer %s"\n' "$HYPERDRIVE_API_TOKEN" | curl -sS -m 30 -K - -X "$m" "https://api.cloudflare.com/client/v4/accounts/$CLOUDFLARE_ACCOUNT_ID$p"
  fi
}

hd_id() { # config name -> id (empty if absent)
  hd_api GET /hyperdrive/configs | python3 -I -c "import json,sys; n=sys.argv[1]; print(next((c['id'] for c in json.load(sys.stdin).get('result',[]) if c['name']==n), ''))" "$1"
}

case "${1:-}" in
  role)
    KUBECTL="$WRAP" "$INFRA/kube/scripts/pr-schema.sh" create "$PR" --password-file "$PWFILE" | sed 's/PASSWORD .*/PASSWORD <redacted>/' || exit 1
    note "created role and schema pr_$PR on thrust-pg-dev (pr-schema.sh create $PR)"
    ;;
  migrate)
    for f in "$ROOT"/crates/thrust-adapters-postgres/migrations/*.sql; do
      as_role <"$f" || { note "FAILED: migration $(basename "$f") as pr_$PR"; exit 1; }
      note "applied $(basename "$f") as pr_$PR (password login, role-level search_path)"
    done
    ;;
  hyperdrive)
    : "${HYPERDRIVE_API_TOKEN:?set HYPERDRIVE_API_TOKEN (Hyperdrive Write)}" "${THRUST_DEV_VPC_SERVICE_ID:?set THRUST_DEV_VPC_SERVICE_ID (the VPC service thrust-pg-dev)}"
    "$INFRA/cloudflare/scripts/hyperdrive.sh" ensure "$HD" --vpc-service thrust-pg-dev --user "pr_$PR" --password-file "$PWFILE" --vpc-service-id "$THRUST_DEV_VPC_SERVICE_ID" --origin-connection-limit 8 || exit 1
    note "created hyperdrive config $HD (caching disabled, origin connection limit 8): $(hd_id "$HD")"
    "$INFRA/cloudflare/scripts/hyperdrive.sh" ensure "$HD_CACHED" --vpc-service thrust-pg-dev --user "pr_$PR" --password-file "$PWFILE" --vpc-service-id "$THRUST_DEV_VPC_SERVICE_ID" --origin-connection-limit 8 --caching-enabled || exit 1
    note "created hyperdrive config $HD_CACHED (caching ENABLED, origin connection limit 8): $(hd_id "$HD_CACHED")"
    ;;
  resources)
    wrangler r2 bucket create "$BUCKET" && note "created r2 bucket $BUCKET" || exit 1
    wrangler queues create "$DLQ" && note "created queue $DLQ" || exit 1
    wrangler queues create "$QUEUE" && note "created queue $QUEUE" || exit 1
    ;;
  deploy)
    shift
    : "${HYPERDRIVE_API_TOKEN:?set HYPERDRIVE_API_TOKEN (to look up the Hyperdrive config id)}"
    which_hd="$HD"; placement=1; extra=""; names=""
    while [ $# -gt 0 ]; do
      case "$1" in
        --cached) which_hd="$HD_CACHED" ;;
        --no-placement) placement=0 ;;
        --var) extra="$extra --var $2"; names="$names --var ${2%%:*}"; shift ;;
        *) echo "unknown option $1" >&2; exit 2 ;;
      esac
      shift
    done
    id="$(hd_id "$which_hd")"
    [ -n "$id" ] || { echo "no hyperdrive config named $which_hd" >&2; exit 1; }
    if [ "$placement" = 1 ]; then PL='[placement]\nregion = "aws:us-east-1"'; else PL='# no placement'; fi
    sed -e "s|__HYPERDRIVE_ID__|$id|" -e "s|# __PLACEMENT__|$PL|" "$WR/$TEMPLATE" >"$WR/$RUNCFG"
    # shellcheck disable=SC2086
    (cd "$WR" && wrangler deploy -c "$RUNCFG" $extra)
    rc=$?
    if [ "$rc" = 0 ] && [ ! -f "$STATE/.secret-put" ]; then
      if [ ! -f "$TOKEN_FILE" ]; then (umask 077; openssl rand -hex 16 >"$TOKEN_FILE"); fi
      chmod 600 "$TOKEN_FILE"
      (cd "$WR" && wrangler secret put THRUST_ADMIN_TOKEN -c "$RUNCFG" <"$TOKEN_FILE") && : >"$STATE/.secret-put" && note "put worker secret THRUST_ADMIN_TOKEN"
    fi
    drop_runcfg
    [ "$rc" = 0 ] || { note "FAILED: wrangler deploy"; exit 1; }
    note "deployed worker $WORKER (hyperdrive $which_hd, placement=$placement$names; var values are not logged)"
    ;;
  sql)
    pod="$(primary)" || exit 1
    kc exec -i "$pod" -c postgres -- psql -X -q -t -A -d thrust
    ;;
  status)
    echo "--- ledger"; cat "$LEDGER" 2>/dev/null || echo "(none)"
    echo "--- queues"; wrangler queues list 2>&1 | grep -E "thrust-spike-pg|No queues" || echo "none under thrust-spike-pg"
    echo "--- r2 bucket"; wrangler r2 bucket info "$BUCKET" 2>&1 | grep -E "name:|does not exist" | head -2
    echo "--- worker"
    sed -e "s|__HYPERDRIVE_ID__|x|" "$WR/$TEMPLATE" >"$WR/$RUNCFG"
    (cd "$WR" && wrangler deployments list -c "$RUNCFG" 2>&1 | grep -E "Created|does not exist|not found|workers.dev|could not" | head -3)
    drop_runcfg
    if [ -n "${HYPERDRIVE_API_TOKEN:-}" ]; then
      echo "--- hyperdrive"
      hd_api GET /hyperdrive/configs | python3 -I -c "import json,sys; print([c['name'] for c in json.load(sys.stdin).get('result',[]) if 'thrust-spike' in c['name']] or 'none under thrust-spike')"
    fi
    echo "--- roles and schemas pr_<n> on thrust-pg-dev"
    KUBECTL="$WRAP" "$INFRA/kube/scripts/pr-schema.sh" list 2>&1 | head -5
    echo "(end)"
    ;;
  down)
    : "${HYPERDRIVE_API_TOKEN:?set HYPERDRIVE_API_TOKEN (Hyperdrive Write)}"
    status=0
    # An R2 bucket must be empty before it is deleted, and wrangler cannot bulk delete: empty it through the binding first.
    if [ -f "$TOKEN_FILE" ] && [ -n "${THRUST_SPIKE_PG_URL:-}" ]; then
      printf 'header = "x-admin-token: %s"\n' "$(cat "$TOKEN_FILE")" | curl -sS -m 120 -K - "$THRUST_SPIKE_PG_URL/_test/bindings/r2purge" && echo
    fi
    wrangler queues consumer remove "$QUEUE" "$WORKER" && note "removed queue consumer $WORKER of $QUEUE" || { note "consumer remove failed or absent"; status=1; }
    (cd "$WR" && wrangler delete --name "$WORKER" --force) && note "deleted worker $WORKER (with its Durable Object storage, cron and secret)" || { note "worker delete failed"; status=1; }
    wrangler queues delete "$QUEUE" && note "deleted queue $QUEUE" || { note "queue delete failed: $QUEUE"; status=1; }
    wrangler queues delete "$DLQ" && note "deleted queue $DLQ" || { note "queue delete failed: $DLQ"; status=1; }
    wrangler r2 bucket delete "$BUCKET" && note "deleted r2 bucket $BUCKET" || { note "bucket delete failed: $BUCKET"; status=1; }
    for n in "$HD" "$HD_CACHED"; do
      i="$(hd_id "$n")"
      if [ -n "$i" ]; then
        hd_api DELETE "/hyperdrive/configs/$i" >/dev/null && note "deleted hyperdrive config $n" || { note "hyperdrive delete failed: $n"; status=1; }
      else
        note "hyperdrive config $n already absent"
      fi
    done
    KUBECTL="$WRAP" "$INFRA/kube/scripts/pr-schema.sh" drop "$PR" && note "dropped role and schema pr_$PR (pr-schema.sh drop $PR)" || { note "pr-schema drop failed"; status=1; }
    exit "$status"
    ;;
  *)
    echo "usage: $0 role|migrate|hyperdrive|resources|deploy|sql|status|down" >&2
    exit 2
    ;;
esac
