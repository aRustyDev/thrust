#!/bin/sh
# Spike 0003 K4 (issue #38): argon2id CPU time per sign-in on a DEPLOYED Worker, at each of OWASP's five equivalent
# configurations. Creates a KV namespace and deploys two throwaway Workers (the memory-adapter Worker, and a tail consumer
# that stores each invocation's cpuTime in the namespace), signs in N times per configuration, reads the times back, and
# deletes all three. Everything is named thrust-spike-*.
#
#   CLOUDFLARE_ACCOUNT_ID=<id> scripts/measure-argon2.sh [runs] [outdir]
#
# Credentials: wrangler's OAuth login (workers, workers_tail, workers_kv). The Worker's admin token is made here, goes to
# the Worker through stdin (`wrangler secret put`) and to curl through a 0600 config file that is deleted after the
# batch, so it is never on an argv. Output: outdir (default target/measure-argon2) holds the client's timings
# (client.tsv), the platform's (kv.json) and summary.txt. The resources are deleted by a trap even if a step fails; confirm
# with `wrangler deployments list --name thrust-spike-auth` and `wrangler kv namespace list`.
#
# One curl process per configuration sends the seed and every sign-in over one connection, which keeps them on one
# isolate (the memory adapter's state is per isolate); the user is seeded again every 20 sign-ins in case the isolate
# changed. A sign-in that finds no user is a 401 and costs the same one hash (that is the point of ADR-0050's
# timing rule), so the CPU numbers hold either way; the `ok` column of the summary says how many found the user.
set -u
RUNS="${1:-100}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${2:-$ROOT/target/measure-argon2}"
: "${CLOUDFLARE_ACCOUNT_ID:?set CLOUDFLARE_ACCOUNT_ID}"
export CLOUDFLARE_ACCOUNT_ID WRANGLER_SEND_METRICS=false NO_COLOR=1
WORKER_DIR="$ROOT/crates/thrust-worker"
MAIN=thrust-spike-auth
TAIL=thrust-spike-auth-tail
KV_TITLE=thrust-spike-auth-cpu
# OWASP Password Storage Cheat Sheet: the minimum and its four equivalents, as m(KiB):t, p = 1
CONFIGS="47104:1 19456:2 12288:3 9216:4 7168:5"
BURST=16
RESEED_EVERY=20
mkdir -p "$OUT"
: >"$OUT/client.tsv"

KV_ID=""
cleanup() {
  cd "$WORKER_DIR" || return
  wrangler delete --name "$MAIN" --force >"$OUT/delete-main.log" 2>&1
  wrangler delete --name "$TAIL" --force >"$OUT/delete-tail.log" 2>&1
  [ -n "$KV_ID" ] && wrangler kv namespace delete --namespace-id "$KV_ID" --skip-confirmation >"$OUT/delete-kv.log" 2>&1
  echo "teardown: see $OUT/delete-*.log; verify with: wrangler deployments list --name $MAIN ; wrangler kv namespace list"
}
trap cleanup EXIT INT TERM

esc() { printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }

ADMIN_TOKEN="$(openssl rand -hex 16)"
cd "$WORKER_DIR" || exit 2

echo "[measure] create the KV namespace and deploy the tail consumer"
wrangler kv namespace create "$KV_TITLE" >"$OUT/kv-create.log" 2>&1 || { echo "kv create failed, see $OUT/kv-create.log" >&2; exit 3; }
KV_ID="$(grep -o '[0-9a-f]\{32\}' "$OUT/kv-create.log" | head -1)"
[ -n "$KV_ID" ] || { echo "no namespace id in $OUT/kv-create.log" >&2; exit 3; }
cat >"$OUT/wrangler.tail.toml" <<TOML
name = "$TAIL"
main = "$WORKER_DIR/measure/tail.mjs"
compatibility_date = "2026-08-01"
workers_dev = false

[[kv_namespaces]]
binding = "CPU"
id = "$KV_ID"
TOML
wrangler deploy -c "$OUT/wrangler.tail.toml" >"$OUT/deploy-tail.log" 2>&1 || { echo "tail deploy failed, see $OUT/deploy-tail.log" >&2; exit 3; }

first=1
for cfg in $CONFIGS; do
  m="${cfg%%:*}"; t="${cfg##*:}"
  label="m${m}t${t}"
  echo "[measure] deploy $label"
  wrangler deploy -c wrangler.measure.toml --var "THRUST_ARGON2_M_KIB:$m" --var "THRUST_ARGON2_T:$t" --var "THRUST_ARGON2_P:1" \
    >"$OUT/deploy-$label.log" 2>&1 || { echo "deploy failed, see $OUT/deploy-$label.log" >&2; exit 3; }
  if [ "$first" -eq 1 ]; then
    first=0
    printf '%s' "$ADMIN_TOKEN" | wrangler secret put THRUST_ADMIN_TOKEN -c wrangler.measure.toml >"$OUT/secret.log" 2>&1 \
      || { echo "secret put failed, see $OUT/secret.log" >&2; exit 3; }
    URL="$(grep -o 'https://[a-z0-9.-]*workers.dev' "$OUT/deploy-$label.log" | head -1)"
    [ -n "$URL" ] || { echo "no workers.dev URL in the deploy log" >&2; exit 3; }
  fi
  i=0
  until curl -fsS --max-time 20 "$URL/healthz" >/dev/null 2>&1; do
    i=$((i + 1)); [ "$i" -gt 60 ] && { echo "the Worker did not come up" >&2; exit 4; }
    sleep 2
  done
  email="measure-$label@example.test"
  password="measure-password-$label"
  user_id="00000000-0000-4000-8000-0000000000$(printf %02d "$t")"
  seed="{\"workspaces\":[],\"users\":[],\"projects\":[],\"memberships\":[],\"sessions\":[],\"logins\":[{\"user_id\":\"$user_id\",\"email\":\"$email\",\"password\":\"$password\"}]}"
  signin="{\"email\":\"$email\",\"password\":\"$password\"}"
  req="$OUT/requests-$label.curl"
  ( umask 077; : >"$req" )
  block_seed() {
    [ -s "$req" ] && echo next >>"$req"
    printf 'url = "%s/internal/seed?run=%s-seed%s"\nrequest = "POST"\nheader = "x-admin-token: %s"\nheader = "content-type: application/json"\ndata = "%s"\noutput = "/dev/null"\nwrite-out = "%s-seed%s\\t%%{http_code}\\t%%{time_total}\\n"\n' \
      "$URL" "$label" "$1" "$ADMIN_TOKEN" "$(esc "$seed")" "$label" "$1" >>"$req"
  }
  block_signin() {
    [ -s "$req" ] && echo next >>"$req"
    printf 'url = "%s/api/_slice/auth/sign-in/email?run=%s-%s"\nrequest = "POST"\nheader = "content-type: application/json"\ndata = "%s"\noutput = "/dev/null"\nwrite-out = "%s-%s\\t%%{http_code}\\t%%{time_total}\\n"\n' \
      "$URL" "$label" "$1" "$(esc "$signin")" "$label" "$1" >>"$req"
  }
  block_seed 0
  n=0
  while [ "$n" -lt "$RUNS" ]; do
    n=$((n + 1))
    [ $((n % RESEED_EVERY)) -eq 0 ] && block_seed "$n"
    block_signin "$n"
  done
  curl -sS --max-time 600 --config "$req" >>"$OUT/client.tsv" 2>>"$OUT/curl.err"
  rm -f "$req"
  if [ "$cfg" = "19456:2" ]; then
    # the minimum, $BURST at once on separate connections: hashing is synchronous, so one isolate serialises them (memory
    # does not stack) and the platform may start more isolates
    echo "[measure] burst of $BURST at $label"
    b=0
    pids=""
    while [ "$b" -lt "$BURST" ]; do
      b=$((b + 1))
      curl -sS --max-time 120 -o /dev/null -w "$label-burst$b\t%{http_code}\t%{time_total}\n" -X POST -H 'content-type: application/json' \
        --data "$signin" "$URL/api/_slice/auth/sign-in/email?run=$label-burst$b" >>"$OUT/client.tsv" &
      pids="$pids $!"
    done
    wait $pids
  fi
done
echo "[measure] waiting for the tail consumer"
sleep 30
wrangler kv key list --namespace-id "$KV_ID" --remote >"$OUT/kv-keys.json" 2>"$OUT/kv-list.err"
# `kv bulk get` takes at most 100 keys a call
jq -c '[.[].name] as $k | range(0; $k | length; 100) | $k[.:.+100]' "$OUT/kv-keys.json" >"$OUT/key-chunks.jsonl" 2>>"$OUT/kv-list.err"
: >"$OUT/kv-parts.jsonl"
n=0
while IFS= read -r chunk; do
  n=$((n + 1))
  printf '%s' "$chunk" >"$OUT/keys-$n.json"
  wrangler kv bulk get "$OUT/keys-$n.json" --namespace-id "$KV_ID" --remote 2>>"$OUT/kv-list.err" | sed -n '/^{/,$p' >>"$OUT/kv-parts.jsonl"
done <"$OUT/key-chunks.jsonl"
jq -s 'add // {}' "$OUT/kv-parts.jsonl" >"$OUT/kv.json" 2>>"$OUT/kv-list.err"
echo "[measure] read back $(jq length "$OUT/kv.json") records from $(jq length "$OUT/kv-keys.json") keys"
uv run --no-project "$ROOT/scripts/measure-argon2-summary.py" "$OUT" >"$OUT/summary.txt" 2>&1
cat "$OUT/summary.txt"
