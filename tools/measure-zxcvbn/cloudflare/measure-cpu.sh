#!/bin/sh
# Stage B of research/0015: the CPU time of zxcvbn on a Cloudflare isolate, read from the platform (cpuTime).
#
# Throwaway Workers `thrust-spike-zxcvbn-rNN`, one NEW Worker per round, each deployed from one of two builds of the memory adapter
# set (no bindings): the BASELINE, and the VARIANT where `password_is_strong` also calls zxcvbn. A tail consumer and a KV namespace
# store each invocation's cpuTime (the #146 harness: `wrangler tail` does not print cpuTime, and CPU is measured only on Cloudflare's
# network). A new name per round, not a redeploy: the pilot showed that after a redeploy new connections still reached the previous
# version's warm isolates for seconds, which both hides cold starts and mixes builds; a Worker that did not exist has no such traffic.
#
# Two request types, both refused with 400 BEFORE any hashing, so cpuTime is the request plus the strength check and no argon2:
#   S  password "short"         refused by the length rule, zxcvbn never runs
#   Z  password "abcdefghijkl"  accepted by the old rule, refused by zxcvbn (score Zero): the dictionaries are built on first use
# Three arms, rotated so that no arm is always first: bS = baseline build + S, vS = variant build + S, vZ = variant build + Z.
#   vS - bS  is what the bigger module costs a cold isolate whether or not zxcvbn runs;  vZ - vS  is the first zxcvbn call.
# Each round: deploy a new Worker with the token (--secrets-file), probe the operator route with an S request until it answers 400
# five times running (this absorbs the first-request overhead of the route, which is not zxcvbn: the pilot saw 22 ms of it), then
#   seq    SEQ requests on ONE connection. The design said: on the vZ arm the first is the first zxcvbn call on an isolate that has already
#          served the route, the later ones are warm, and on the S arms the same sequence is the CONTROL, first minus later about 0.
#          THE CONTROL FAILED (0 to 45 ms on the S arms): a new connection sometimes reaches a FRESH isolate, which pays a cold start that is
#          not zxcvbn. So the sequences are recorded but not used for the figure (research/0015, section 4.2); cold-analysis.py is.
#   b1,b2  two bursts of BURST concurrent requests on separate connections. The platform spreads them over isolates, so the second burst is
#          only PARTLY warm (12 to 27% of its requests were still cold in the run, against 35 to 64% of the first), and they are not
#          labelled warm and cold.
#
#   BUILD_BASE=<dir> BUILD_VAR=<dir> NEEDLE=<dictionary entries> CLOUDFLARE_ACCOUNT_ID=<id> \
#     tools/measure-zxcvbn/cloudflare/measure-cpu.sh [cycles] [outdir]
#   <dir> is a worker-build `build/` directory of the memory adapter set (`worker-build --release .` in crates/worker), the baseline from the
#   unmodified tree and the variant with patches/variant-called.patch applied. The NEEDLE is a run of dictionary entries (bundle/make-needle.py).
#   A cycle is one round per arm, so 6 cycles are 18 rounds and 18 Workers. The default outdir is target/measure-zxcvbn.
#   It writes client.tsv and kv.json, which summary.py and cold-analysis.py read.
#
# It deletes only what it created and refuses to start if any name or the KV title is in use, because `wrangler deploy` on an existing
# name overwrites that Worker. The operator token is made here, goes to wrangler in a 0600 file that the teardown deletes and to curl in
# 0600 config files deleted after use, never an argv. Confirm the teardown afterwards with verify-teardown.sh (every name must
# answer code 10007, and no thrust-spike KV namespace may remain).
set -u
CYCLES="${1:-6}"
ROOT="$(cd "$(dirname "$0")/../../.." && pwd)"
OUT="${2:-$ROOT/target/measure-zxcvbn}"
: "${CLOUDFLARE_ACCOUNT_ID:?set CLOUDFLARE_ACCOUNT_ID}"
: "${BUILD_BASE:?set BUILD_BASE}"
: "${BUILD_VAR:?set BUILD_VAR}"
: "${NEEDLE:?set NEEDLE: a run of dictionary entries that is in the variant's wasm and not in the baseline's}"
export CLOUDFLARE_ACCOUNT_ID WRANGLER_SEND_METRICS=false NO_COLOR=1
WORKER_DIR="$ROOT/crates/worker"
PREFIX=thrust-spike-zxcvbn-r
TAIL=thrust-spike-zxcvbn-tail
KV_TITLE=thrust-spike-zxcvbn-cpu
BURST=16
SEQ=5
BODY_S='{"email":"m@example.test","password":"short"}'
BODY_Z='{"email":"m@example.test","password":"abcdefghijkl"}'
TOTAL=$((CYCLES * 3))
mkdir -p "$OUT"
: >"$OUT/client.tsv"
{ wrangler --version; curl --version | head -1; uname -sm; wc -c "$BUILD_BASE/index_bg.wasm" "$BUILD_VAR/index_bg.wasm"; } >"$OUT/env.txt" 2>&1

KV_ID=""
TAIL_CREATED=0
CREATED=""
cleanup() {
  cd "$WORKER_DIR" || return
  rm -f "$OUT/secrets.env"
  for w in $CREATED; do wrangler delete --name "$w" --force >"$OUT/delete-$w.log" 2>&1; done
  [ "$TAIL_CREATED" -eq 1 ] && wrangler delete --name "$TAIL" --force >"$OUT/delete-tail.log" 2>&1
  [ -n "$KV_ID" ] && wrangler kv namespace delete --namespace-id "$KV_ID" --skip-confirmation >"$OUT/delete-kv.log" 2>&1
  echo "teardown: deleted$CREATED; tail=$TAIL_CREATED; kv=${KV_ID:-none}; logs $OUT/delete-*.log"
}
trap cleanup EXIT INT TERM

esc() { printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g'; }

# --- the two builds are what they are said to be ---
grep -a -q -F "$NEEDLE" "$BUILD_VAR/index_bg.wasm" || { echo "BUILD_VAR does not contain the dictionary: it is not the variant" >&2; exit 2; }
if grep -a -q -F "$NEEDLE" "$BUILD_BASE/index_bg.wasm"; then echo "BUILD_BASE contains the dictionary: it is not the baseline" >&2; exit 2; fi
for d in "$BUILD_BASE" "$BUILD_VAR"; do
  for f in worker/shim.mjs index.js index_bg.wasm; do [ -s "$d/$f" ] || { echo "$d/$f is missing" >&2; exit 2; }; done
done

# --- refuse to run over something that exists ---
n=0
for w in "$TAIL" $(i=0; while [ "$i" -lt "$TOTAL" ]; do i=$((i + 1)); printf '%s%02d ' "$PREFIX" "$i"; done); do
  n=$((n + 1))
  if wrangler deployments list --name "$w" >"$OUT/preflight-$w.log" 2>&1; then
    echo "a Worker named $w already exists; refusing to overwrite it" >&2; exit 2
  fi
  grep -q 'code: 10007' "$OUT/preflight-$w.log" || { echo "preflight for $w did not say 'does not exist'; see $OUT/preflight-$w.log" >&2; exit 2; }
done
echo "[measure] $n names checked: none exists"
wrangler kv namespace list >"$OUT/preflight-kv.json" 2>&1
grep -q '"title"' "$OUT/preflight-kv.json" || { echo "the KV listing is empty or unreadable; see $OUT/preflight-kv.json" >&2; exit 2; }
if grep -q "\"$KV_TITLE\"" "$OUT/preflight-kv.json"; then echo "KV namespace $KV_TITLE already exists" >&2; exit 2; fi

THRUST_OPERATOR_TOKEN="$(openssl rand -hex 16)"
export THRUST_OPERATOR_TOKEN
( umask 077; printf 'THRUST_OPERATOR_TOKEN=%s\n' "$THRUST_OPERATOR_TOKEN" >"$OUT/secrets.env" )

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
TAIL_CREATED=1
wrangler deploy -c "$OUT/wrangler.tail.toml" >"$OUT/deploy-tail.log" 2>&1 || { echo "tail deploy failed, see $OUT/deploy-tail.log" >&2; exit 3; }

# seq_run <label-prefix>: SEQ requests of the round's type over ONE connection (one curl process, one --config)
seq_run() {
  req="$OUT/seq-$1.curl"
  ( umask 077; : >"$req" )
  k=0
  while [ "$k" -lt "$SEQ" ]; do
    k=$((k + 1))
    [ -s "$req" ] && echo next >>"$req"
    printf 'url = "%s/internal/operator/users?run=%s-%s"\nrequest = "POST"\nheader = "content-type: application/json"\nheader = "x-operator-token: %s"\ndata = "%s"\noutput = "/dev/null"\nwrite-out = "%s-%s\\t%s\\t%s\\tseq\\t%s\\t%%{http_code}\\t%%{time_total}\\t%%header{x-thrust-version}\\n"\n' \
      "$URL" "$1" "$k" "$THRUST_OPERATOR_TOKEN" "$(esc "$body")" "$1" "$k" "$arm" "$r" "$k" >>"$req"
  done
  curl -sS --max-time 120 --config "$req" >>"$OUT/client.tsv" 2>>"$OUT/curl.err"
  rm -f "$req"
}

# burst <label-prefix> <kind>: BURST concurrent requests of the round's type, one connection each
burst() {
  b=0
  pids=""
  while [ "$b" -lt "$BURST" ]; do
    b=$((b + 1))
    curl -sS --max-time 60 -o /dev/null \
      -w "$1-$b\t$arm\t$r\t$2\t0\t%{http_code}\t%{time_total}\t%header{x-thrust-version}\n" \
      -X POST -H 'content-type: application/json' \
      --variable %THRUST_OPERATOR_TOKEN --expand-header 'x-operator-token: {{THRUST_OPERATOR_TOKEN}}' \
      --data "$body" "$URL/internal/operator/users?run=$1-$b" >>"$OUT/client.tsv" 2>>"$OUT/curl.err" &
    pids="$pids $!"
  done
  wait $pids
}

r=0
while [ "$r" -lt "$TOTAL" ]; do
  r=$((r + 1))
  p=$(((r - 1) % 3))
  c=$(((r - 1) / 3))
  case $(((p + c) % 3)) in
    0) arm=bS; dir="$BUILD_BASE"; body="$BODY_S" ;;
    1) arm=vS; dir="$BUILD_VAR"; body="$BODY_S" ;;
    *) arm=vZ; dir="$BUILD_VAR"; body="$BODY_Z" ;;
  esac
  rr="$(printf %02d "$r")"
  name="$PREFIX$rr"
  cat >"$OUT/wrangler.$rr.toml" <<TOML
name = "$name"
main = "$dir/worker/shim.mjs"
compatibility_date = "2026-08-01"
workers_dev = true
tail_consumers = [{ service = "$TAIL" }]

[version_metadata]
binding = "CF_VERSION_METADATA"

[vars]
THRUST_ADAPTERS = "memory"
THRUST_OPERATOR_ROUTES = "1"
TOML
  echo "[measure] round $r of $TOTAL: new Worker $name, arm $arm"
  CREATED="$CREATED $name"
  wrangler deploy -c "$OUT/wrangler.$rr.toml" --secrets-file "$OUT/secrets.env" --tag "round-$r" --message "measure round $r arm $arm" >"$OUT/deploy-$rr.log" 2>&1 \
    || { echo "deploy failed, see $OUT/deploy-$rr.log" >&2; exit 3; }
  URL="$(grep -o 'https://[a-z0-9.-]*workers.dev' "$OUT/deploy-$rr.log" | head -1)"
  [ -n "$URL" ] || { echo "no workers.dev URL in $OUT/deploy-$rr.log" >&2; exit 3; }
  # readiness: the operator route answers 400 to an S request five times running (a refusal; the route exists and has its token)
  i=0
  ok=0
  while [ "$ok" -lt 5 ]; do
    code="$(curl -s -o /dev/null -w '%{http_code}' --max-time 15 -X POST -H 'content-type: application/json' \
      --variable %THRUST_OPERATOR_TOKEN --expand-header 'x-operator-token: {{THRUST_OPERATOR_TOKEN}}' --data "$BODY_S" "$URL/internal/operator/users")"
    if [ "$code" = "400" ]; then ok=$((ok + 1)); else ok=0; i=$((i + 1)); sleep 2; fi
    [ "$i" -gt 60 ] && { echo "round $r: $name never answered 400 five times running (last $code)" >&2; exit 4; }
  done
  seq_run "r$rr-$arm-seq"
  burst "r$rr-$arm-b1" b1
  burst "r$rr-$arm-b2" b2
done

echo "[measure] waiting for the tail consumer"
sleep 40
wrangler kv key list --namespace-id "$KV_ID" --remote >"$OUT/kv-keys.json" 2>"$OUT/kv-list.err"
jq -c '[.[].name] as $k | range(0; $k | length; 100) | $k[.:.+100]' "$OUT/kv-keys.json" >"$OUT/key-chunks.jsonl" 2>>"$OUT/kv-list.err"
: >"$OUT/kv-parts.jsonl"
n=0
while IFS= read -r chunk; do
  n=$((n + 1))
  printf '%s' "$chunk" >"$OUT/keys-$n.json"
  wrangler kv bulk get "$OUT/keys-$n.json" --namespace-id "$KV_ID" --remote 2>>"$OUT/kv-list.err" | sed -n '/^{/,$p' >>"$OUT/kv-parts.jsonl"
done <"$OUT/key-chunks.jsonl"
jq -s 'add // {}' "$OUT/kv-parts.jsonl" >"$OUT/kv.json" 2>>"$OUT/kv-list.err"
echo "[measure] read back $(jq length "$OUT/kv.json") records from $(jq length "$OUT/kv-keys.json") keys; client sent $(wc -l <"$OUT/client.tsv" | tr -d ' ') requests"
