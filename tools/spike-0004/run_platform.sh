#!/usr/bin/env bash
# Spike 0004: the REAL-PLATFORM runner. Every mutating step creates or deletes real Cloudflare resources named
# spike-0004*; run it only with the owner's go-ahead and the inputs listed in research/0006 "Runbook".
#
# Usage:  ./run_platform.sh <step>      steps, in order:
#   preflight       local only: required env present, tools present, tofu validate, verifier selftests
#   token-verify    GET /accounts/<id>/tokens/verify (account-owned tokens; /user/tokens/verify returns 401 for them)
#   snapshot LABEL  read-only inventory of names (Workers, custom domains, Access apps/tokens/policies, zone records) -> $outdir
#   compare LABEL   read-only: current inventory vs the snapshot taken as LABEL, and vs the named baseline if provided
#   deploy          wrangler deploy: Worker B first (binding target must exist), then space, then the edge Worker
#   apply           tofu plan (gate: exactly 10 adds, nothing else) then apply
#   wait-ready      poll the public hostname until it serves, logging how long certificates/Access took to propagate
#   set-aud         redeploy the edge Worker with TEAM_DOMAIN + the UI app AUD so /internal/whoami can verify the JWT
#   verify-present  CONTROL: the inventory script must SEE what we created (else its later "zero" is meaningless)
#   checks          the assertion suite (gated/bypass/WebSocket/traversal/host/method/assets/overlap/space/workers.dev)
#   latency         binding vs in-process latency, N=1000 per kind
#   teardown        tofu destroy, wrangler delete the three Workers, delete any leftover spike-0004 edge certificate pack
#   verify-absent   inventory must show ZERO spike-0004* resources and every required endpoint must have answered
#   scrub-state     delete the local tofu state (holds the service-token secret) once verify-absent passed
#
# Required env (none of these is a secret):
#   SPIKE_ACCOUNT_ID  SPIKE_ZONE_ID  SPIKE_ZONE_NAME  SPIKE_TEST_EMAIL  SPIKE_TEAM_NAME (Zero Trust team, <team>.cloudflareaccess.com)
#   SPIKE_OP_TOKEN_REF   an op:// reference to the API token item field (the VALUE is never printed or passed on argv)
# Optional:
#   SPIKE_ENV_FILE            a file of `export NAME=value` lines (non-secret) sourced first
#   SPIKE_STATE_DIR           tofu state + data dir + plan file; keep OUTSIDE the git worktree (default ~/.local/state/spike-0004)
#   SPIKE_OUT_DIR             logs and reports (default tests/out, gitignored)
#   SPIKE_WORKERS_DEV_HOST    spike-0004-edge.<account-subdomain>.workers.dev  (enables check X2)
#   SPIKE_WS_SOAK             seconds for the WebSocket soak (default 120)
#   SPIKE_WRANGLER_AUTH=oauth use the machine's wrangler OAuth login for deploy/delete instead of the API token
#   SPIKE_ALLOW_UNVERIFIABLE  comma list of inventory endpoints allowed to be unverifiable (e.g. "dns,certificate,routes")
#   SPIKE_BASELINE_FILE       JSON of the pre-existing resource names that must be unchanged (see tests/inventory.py)
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
if [ -n "${SPIKE_ENV_FILE:-}" ]; then
  # shellcheck disable=SC1090
  . "$SPIKE_ENV_FILE"
fi
outdir="${SPIKE_OUT_DIR:-$here/tests/out}"
mkdir -p "$outdir"

need() {
  local v
  for v in "$@"; do
    [ -n "${!v:-}" ] || { echo "missing required env: $v" >&2; exit 2; }
  done
}

load_token() {
  need SPIKE_OP_TOKEN_REF SPIKE_ACCOUNT_ID
  # op read, not `op whoami`: whoami reports session state and lies on this machine (rules/1password-op-cli.md)
  CLOUDFLARE_API_TOKEN="$(op read "$SPIKE_OP_TOKEN_REF")"
  export CLOUDFLARE_API_TOKEN
  export CLOUDFLARE_ACCOUNT_ID="$SPIKE_ACCOUNT_ID"
  echo "token loaded (length ${#CLOUDFLARE_API_TOKEN}; value not shown)" >&2
  [ "${#CLOUDFLARE_API_TOKEN}" -gt 20 ] || { echo "token looks empty or short: unlock 1Password / check the reference" >&2; exit 2; }
}

tf_env() {
  # SPIKE_TRACE=1 traces everything AFTER the token was read (xtrace before that point would print it)
  if [ -n "${SPIKE_TRACE:-}" ]; then set -x; fi
  need SPIKE_ACCOUNT_ID SPIKE_ZONE_ID SPIKE_ZONE_NAME SPIKE_TEST_EMAIL
  export TF_VAR_account_id="$SPIKE_ACCOUNT_ID" TF_VAR_zone_id="$SPIKE_ZONE_ID"
  export TF_VAR_zone_name="$SPIKE_ZONE_NAME" TF_VAR_test_identity_email="$SPIKE_TEST_EMAIL"
}

edge_host() { echo "spike-0004.${SPIKE_ZONE_NAME}"; }

# State, provider data dir and the plan file live OUTSIDE the git worktree (a pruned worktree must not strand live
# resources, and the state holds the service-token secret): private directory, never committed.
state_dir="${SPIKE_STATE_DIR:-$HOME/.local/state/spike-0004}"
state_file="$state_dir/terraform.tfstate"
plan_file="$state_dir/spike.plan"

tf() { TF_DATA_DIR="$state_dir/tfdata" tofu -chdir="$here/tofu" "$@"; }

tf_init() {
  ( umask 077; mkdir -p "$state_dir" )
  chmod 700 "$state_dir"
  tf init -reconfigure -input=false -backend-config="path=$state_file" > "$outdir/init.log" 2>&1 \
    || { cat "$outdir/init.log" >&2; echo "tofu init failed (log above)" >&2; exit 1; }
}

# wrangler with the API token, or (SPIKE_WRANGLER_AUTH=oauth) with the OAuth login only
wr() {
  if [ "${SPIKE_WRANGLER_AUTH:-token}" = "oauth" ]; then
    ( unset CLOUDFLARE_API_TOKEN; export CLOUDFLARE_ACCOUNT_ID="$SPIKE_ACCOUNT_ID"; wrangler "$@" )
  else
    wrangler "$@"
  fi
}

inv() { ( cd "$here/tests" && uv run inventory.py "$@" ); }

step="${1:-}"
case "$step" in
  preflight)
    need SPIKE_ACCOUNT_ID SPIKE_ZONE_ID SPIKE_ZONE_NAME SPIKE_TEST_EMAIL SPIKE_TEAM_NAME SPIKE_OP_TOKEN_REF
    for tool in op wrangler tofu uv; do
      command -v "$tool" > /dev/null || { echo "missing tool: $tool" >&2; exit 2; }
    done
    ( umask 077; mkdir -p "$state_dir" )
    tf init -backend=false -input=false > /dev/null
    tf validate
    ( cd "$here/tests" && uv run verify_teardown.py --selftest > /dev/null ) && echo "verifier selftest ok"
    ( cd "$here/tests" && uv run delete_cert_packs.py --selftest > /dev/null ) && echo "cert-delete selftest ok"
    echo "preflight ok"
    ;;

  token-verify)
    load_token
    inv verify-token
    ;;

  workers-subdomain)
    load_token
    inv workers-subdomain
    ;;

  snapshot)
    load_token
    need SPIKE_ZONE_ID
    inv snapshot --out "$outdir/snapshot-${2:?label required}.json"
    ;;

  compare)
    load_token
    need SPIKE_ZONE_ID
    extra=()
    if [ -n "${SPIKE_BASELINE_FILE:-}" ]; then extra=(--baseline "$SPIKE_BASELINE_FILE"); fi
    inv compare --against "$outdir/snapshot-${2:?label required}.json" ${extra[@]+"${extra[@]}"}
    ;;

  deploy)
    load_token
    wr deploy -c "$here/worker/b/wrangler.jsonc"
    wr deploy -c "$here/worker/space/wrangler.jsonc"
    wr deploy -c "$here/worker/edge/wrangler.jsonc"
    ;;

  apply)
    load_token
    tf_env
    tf_init
    tf plan -input=false -no-color -out="$plan_file" > "$outdir/plan.txt" 2>&1
    grep -E "^Plan:" "$outdir/plan.txt"
    # 2 custom domains + service token + 3 policies + 4 apps (ui, bypass, space-bypass, overlap) = 10 adds, nothing else
    grep -q "^Plan: 10 to add, 0 to change, 0 to destroy\." "$outdir/plan.txt" \
      || { echo "plan is not exactly 10 adds / 0 change / 0 destroy: STOP and read $outdir/plan.txt" >&2; exit 1; }
    # every planned change must be a create carrying a spike-0004 name (checked on the machine-readable plan)
    tf show -json "$plan_file" > "$state_dir/plan.json"
    inv check-plan "$state_dir/plan.json"
    tf apply -input=false "$plan_file"
    ;;

  wait-ready)
    need SPIKE_ZONE_NAME
    ng=()
    if [ "${SPIKE_EXPECT_GATE:-yes}" = "no" ]; then ng=(--no-gate); fi
    ( cd "$here/tests" && uv run inventory.py wait-ready ${ng[@]+"${ng[@]}"} "https://$(edge_host)" "https://spike-0004-space.${SPIKE_ZONE_NAME}" )
    ;;

  set-aud)
    load_token
    need SPIKE_TEAM_NAME
    tf_init
    aud="$(tf output -raw ui_app_aud)"
    wr deploy -c "$here/worker/edge/wrangler.jsonc" \
      --var "POLICY_AUD:${aud}" --var "TEAM_DOMAIN:https://${SPIKE_TEAM_NAME}.cloudflareaccess.com"
    ;;

  verify-present)
    load_token
    need SPIKE_ZONE_ID
    ( cd "$here/tests" && uv run verify_teardown.py --expect present --overlap --allow-unverifiable "${SPIKE_ALLOW_UNVERIFIABLE:-}" )
    ;;

  checks)
    need SPIKE_ZONE_NAME
    gate="${SPIKE_EXPECT_GATE:-yes}"   # "no" = Access apps were NOT created (e.g. token lacks Access write): functional run only
    ov=()
    if [ "$gate" = "yes" ]; then
      tf_init
      SPIKE_ACCESS_CLIENT_ID="$(tf output -raw service_token_client_id)"
      SPIKE_ACCESS_CLIENT_SECRET="$(tf output -raw service_token_client_secret)"
      export SPIKE_ACCESS_CLIENT_ID SPIKE_ACCESS_CLIENT_SECRET
      ov=(--overlap)
    fi
    wd=()
    if [ -n "${SPIKE_WORKERS_DEV_HOST:-}" ]; then wd=(--workers-dev-host "$SPIKE_WORKERS_DEV_HOST"); fi
    rc=0
    ( cd "$here/tests" && uv run run.py checks --base "https://$(edge_host)" --expect-gate "$gate" ${ov[@]+"${ov[@]}"} \
        --space-host "spike-0004-space.${SPIKE_ZONE_NAME}" \
        --ws-soak "${SPIKE_WS_SOAK:-120}" ${wd[@]+"${wd[@]}"} --report "$outdir/checks-gate-$gate.report.json" ) > "$outdir/checks-gate-$gate.log" 2>&1 || rc=$?
    echo "harness exit status: $rc (0 ok, 1 FAIL, 3 VOID = a control passed, 4 WEAK)" >> "$outdir/checks-gate-$gate.log"
    cat "$outdir/checks-gate-$gate.log"
    exit "$rc"
    ;;

  latency)
    need SPIKE_ZONE_NAME
    rc=0
    ( cd "$here/tests" && uv run run.py latency --base "https://$(edge_host)" -n 1000 --report "$outdir/latency.report.json" ) \
      > "$outdir/latency.log" 2>&1 || rc=$?
    echo "harness exit status: $rc" >> "$outdir/latency.log"
    cat "$outdir/latency.log"
    exit "$rc"
    ;;

  teardown)
    load_token
    tf_env
    need SPIKE_ZONE_ID
    # Every sub-step runs even if an earlier one fails or its target is already gone; the exit status says if any failed.
    # Deletes are hard-limited to the three spike-0004-* Worker names (no variable, no glob).
    rc=0
    ( tf_init && tf destroy -input=false -auto-approve ) || { echo "TEARDOWN: tofu destroy FAILED (continuing)" >&2; rc=1; }
    for w in spike-0004-edge spike-0004-space spike-0004-b; do
      wr delete "$w" --force || { echo "TEARDOWN: wrangler delete $w failed or target already absent (continuing)" >&2; rc=1; }
    done
    # Deleting a Custom Domain does NOT delete the edge certificate it caused to be issued (Workers docs, Custom Domains > Certificates)
    ( cd "$here/tests" && uv run delete_cert_packs.py --yes ) || { echo "TEARDOWN: certificate-pack cleanup failed/unverifiable (continuing)" >&2; rc=1; }
    echo "TEARDOWN finished with status $rc (non-zero = at least one sub-step failed; run verify-absent and compare baseline)" >&2
    exit "$rc"
    ;;

  verify-absent)
    load_token
    need SPIKE_ZONE_ID
    ( cd "$here/tests" && uv run verify_teardown.py --expect absent --allow-unverifiable "${SPIKE_ALLOW_UNVERIFIABLE:-}" )
    ;;

  scrub-state)
    rm -f "$state_file" "$state_file.backup" "$plan_file" "$state_dir/plan.json"
    rm -rf "$state_dir/tfdata"
    rmdir "$state_dir" 2> /dev/null || true
    echo "local tofu state, plan and data dir removed (state dir: $state_dir)"
    ;;

  *)
    sed -n '2,35p' "$0" >&2
    exit 2
    ;;
esac
