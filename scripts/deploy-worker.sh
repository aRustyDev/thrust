#!/bin/sh
# Deploy the Worker to one environment and prove it: expand migration, `wrangler deploy` at 100 %, the smoke, and an automatic
# `wrangler rollback` when the smoke fails (issue #49, ADR-0110 round 29). One script, called by the composite action
# `.github/actions/deploy-worker`, so that its control flow can be run against a fake Cloudflare (`test-deploy-worker.sh`) instead of
# being trusted from reading it.
#
#   scripts/deploy-worker.sh          (everything comes from the environment)
#
# The order, and why:
#   1. render the configuration from the environment's variables (scripts/render-wrangler.sh); a variable that is missing stops here;
#   2. read what is live (`wrangler deployments list` and `versions list`, .github/scripts/active-version.py): the active version's
#      id, which is what a rollback goes back to, and its tag, which is the commit that is running;
#   3. classify the change against it (.github/scripts/classify-deploy.sh): does the expand migration have to run, and is this a
#      Durable Object lifecycle change? A lifecycle change cannot be rolled back across, and the first deploy to an environment is one.
#      Without ALLOW_DO_LIFECYCLE=true such a deploy, and any deploy over a live state this cannot read, STOPS here and deploys nothing:
#      `deferred=true`, which sends the run to the job whose environment has required reviewers;
#   4. the expand migration (scripts/migrate-ci.sh), only when migration files changed, and before the new code: a failure ends the run;
#   5. `wrangler deploy --tag <sha>`: 100 % at once (there are no stages, ADR-0110 round 29);
#   6. the smoke (scripts/smoke-deploy.sh): the version that answers must be the one just deployed, and a real read must work;
#   7. the smoke's own control: the same smoke asked for a version nobody deployed MUST fail. A smoke that passes that cannot tell one
#      version from another, so the run fails on it;
#   8. when 6 or 7 fails, and a rollback is possible (AUTO_ROLLBACK=true, a live version was there to go back to, no lifecycle change):
#      `wrangler rollback <previous version>`, then the unauthenticated probe must show the previous version answering. The run stays
#      RED either way: a rollback is a repair, not a success. The database keeps the expanded schema (expand/contract: N-1 code runs on it).
#
# Environment
#   DEPLOY_ENV (staging|production), DEPLOY_SHA, THRUST_BASE_URL (https://<host>, no path)
#   ALLOW_DO_LIFECYCLE   true only in the manual-gated job     AUTO_ROLLBACK   true to roll back by itself (default true)
#   INJECT_SMOKE_FAILURE true makes the smoke fail on purpose after it has passed, to prove the rollback; staging only
#   SKIP_SMOKE           true to deploy without the smoke: the FIRST deploy of an environment only (no domain is attached to a Worker
#                        that did not exist), refused for any other, and refused together with INJECT_SMOKE_FAILURE
#   CLOUDFLARE_API_TOKEN, CLOUDFLARE_ACCOUNT_ID                 wrangler's own
#   the renderer's variables (scripts/render-wrangler.sh)
#   the expand step's (scripts/migrate-ci.sh) and the smoke's (scripts/smoke-deploy.sh): used only when they run
# Test hooks: WRANGLER, SMOKE_SCRIPT, MIGRATE_CI_SCRIPT (default wrangler and the scripts of this repository)
#
# Outputs (to $GITHUB_OUTPUT when it is set, else to stdout): deferred, deployed, rolled_back (no|yes|failed|unverified), previous_version,
# previous_tag, migrations_changed, do_lifecycle, state. A summary goes to $GITHUB_STEP_SUMMARY.
# Exit status: 0 deployed and proved (or deferred, nothing deployed); 1 anything else; 2 called wrongly.
set -u
LC_ALL=C
export LC_ALL

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WRANGLER="${WRANGLER:-wrangler}"
SMOKE="${SMOKE_SCRIPT:-$ROOT/scripts/smoke-deploy.sh}"
MIGRATE_CI="${MIGRATE_CI_SCRIPT:-$ROOT/scripts/migrate-ci.sh}"
RENDER="$ROOT/scripts/render-wrangler.sh"
CLASSIFY="$ROOT/.github/scripts/classify-deploy.sh"
ACTIVE="$ROOT/.github/scripts/active-version.py"
CONFIG="${DEPLOY_CONFIG_OUT:-$ROOT/crates/thrust-worker/.wrangler.deploy.run.toml}"
PYTHON="${PYTHON:-python3}"

WORK="$(mktemp -d)" || { echo "deploy-worker: cannot make a temporary directory" >&2; exit 2; }
trap 'rm -rf "$WORK"' EXIT INT TERM

say() { echo "deploy-worker: $*"; }
die() { echo "deploy-worker: FAIL: $*" >&2; summary "FAILED: $*"; exit 1; }
usage() { echo "deploy-worker: $*" >&2; exit 2; }
out() { # out <key> <value>: one line of output, never more than one line
  value="$(printf '%s' "$2" | tr -d '\n')"
  if [ -n "${GITHUB_OUTPUT:-}" ]; then printf '%s=%s\n' "$1" "$value" >> "$GITHUB_OUTPUT"; else printf '%s=%s\n' "$1" "$value"; fi
}
summary() { [ -n "${GITHUB_STEP_SUMMARY:-}" ] && printf '%s\n' "$*" >> "$GITHUB_STEP_SUMMARY"; return 0; }

# ---- inputs --------------------------------------------------------------------------------------------------------------------
case "${DEPLOY_ENV:-}" in staging|production) ;; *) usage "DEPLOY_ENV must be staging or production, not '${DEPLOY_ENV:-}'" ;; esac
case "${DEPLOY_SHA:-}" in *[!0-9a-f]*|'') usage "DEPLOY_SHA is not a commit sha" ;; esac
[ "${#DEPLOY_SHA}" -eq 40 ] || usage "DEPLOY_SHA is not a 40-digit commit sha"
case "${THRUST_BASE_URL:-}" in https://*) ;; *) usage "THRUST_BASE_URL must start with https://" ;; esac
case "${THRUST_BASE_URL#https://}" in */*|'') usage "THRUST_BASE_URL must be a host with no path" ;; esac
[ -n "${THRUST_WORKER_NAME:-}" ] || usage "THRUST_WORKER_NAME is not set"
for v in CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID; do
  eval "val=\${$v:-}"
  [ -n "$val" ] || usage "$v is not set"
done
ALLOW="${ALLOW_DO_LIFECYCLE:-false}"; AUTO="${AUTO_ROLLBACK:-true}"; INJECT="${INJECT_SMOKE_FAILURE:-false}"; SKIP="${SKIP_SMOKE:-false}"
for v in ALLOW AUTO INJECT SKIP; do
  eval "val=\$$v"
  case "$val" in true|false) ;; *) usage "$v must be true or false, not '$val'" ;; esac
done
[ "$INJECT" = true ] && [ "$DEPLOY_ENV" = production ] && usage "INJECT_SMOKE_FAILURE is refused for production"
[ "$INJECT" = true ] && [ "$SKIP" = true ] && usage "INJECT_SMOKE_FAILURE and SKIP_SMOKE together: a smoke that is skipped cannot be made to fail"
BASE="$THRUST_BASE_URL"
NAME="$THRUST_WORKER_NAME"
SHA="$DEPLOY_SHA"

# Each secret reaches only the process that needs it. The environment this script is started in holds all of them, so every child is
# started through one of these, which takes the others away: wrangler gets the Cloudflare token and neither the database URL, the
# tunnel's service token nor the smoke account's password; the expand step gets the database's and the tunnel's and not the Cloudflare
# token; the smoke gets only its own account. (`test-deploy-worker.sh` has each fake report what it could see.)
plain() { env -u CLOUDFLARE_API_TOKEN -u DATABASE_URL -u TUNNEL_SERVICE_TOKEN_ID -u TUNNEL_SERVICE_TOKEN_SECRET -u SMOKE_PASSWORD "$@"; }  # no secret at all
wr() { env -u DATABASE_URL -u TUNNEL_SERVICE_TOKEN_ID -u TUNNEL_SERVICE_TOKEN_SECRET -u SMOKE_PASSWORD "$WRANGLER" "$@"; }
migrate_step() { env -u CLOUDFLARE_API_TOKEN -u SMOKE_PASSWORD sh "$MIGRATE_CI"; }
smoke() { # smoke <extra VAR=value words, or empty> <base> <expected version>
  extra="$1"; shift
  # $extra is deliberately split: it is only ever `NAME=value` words written in this file
  env -u CLOUDFLARE_API_TOKEN -u DATABASE_URL -u TUNNEL_SERVICE_TOKEN_ID -u TUNNEL_SERVICE_TOKEN_SECRET $extra sh "$SMOKE" "$@"
}

summary "### Deploy of \`$SHA\` to \`$DEPLOY_ENV\`"

# ---- 1. the configuration ------------------------------------------------------------------------------------------------------
plain sh "$RENDER" --out "$CONFIG" || die "the deploy configuration could not be rendered (see above); nothing was deployed"

# ---- 2. what is live -----------------------------------------------------------------------------------------------------------
list_json() { # list_json <what> <file>: `wrangler <what> list --json` into <file>; a Worker that does not exist is not an error
  wr "$1" list --name "$NAME" --json > "$2" 2> "$2.err"
  rc=$?
  if [ "$rc" -ne 0 ]; then
    if cat "$2" "$2.err" | grep -qiE '10007|could not find|does not exist|not found'; then
      echo '[]' > "$2"
      return 0
    fi
    return 1
  fi
}
list_json deployments "$WORK/deployments.json" || { sed 's/^/  wrangler: /' "$WORK/deployments.json.err" >&2; die "wrangler could not list the deployments of $NAME; nothing was deployed"; }
list_json versions "$WORK/versions.json" || { sed 's/^/  wrangler: /' "$WORK/versions.json.err" >&2; die "wrangler could not list the versions of $NAME; nothing was deployed"; }
plain "$PYTHON" -I "$ACTIVE" "$WORK/deployments.json" "$WORK/versions.json" > "$WORK/active.txt" || die "the live version could not be read; nothing was deployed"
active() { grep "^$1=" "$WORK/active.txt" | head -1 | cut -d= -f2-; }
STATE="$(active state)"; PREV_ID="$(active version_id)"; PREV_TAG="$(active tag)"
say "live: state=$STATE version=${PREV_ID:-none} tag=${PREV_TAG:-none} ($(active reason))"
out state "$STATE"; out previous_version "$PREV_ID"; out previous_tag "$PREV_TAG"
summary "- live before: \`$STATE\` ${PREV_ID:+version \`$PREV_ID\`}${PREV_TAG:+ tag \`$PREV_TAG\`}"

# the smoke may be skipped for the first deploy and for no other: refused here, before a person is asked to approve anything
if [ "$SKIP" = true ] && [ "$STATE" != none ]; then
  die "SKIP_SMOKE is for the first deploy of an environment, and $NAME has a live version ($STATE); nothing was deployed"
fi

# ---- 3. what the change is -----------------------------------------------------------------------------------------------------
case "$STATE" in
  none) CLASS_ARG=none ;;
  single) CLASS_ARG="${PREV_TAG:-untagged}" ;;
  *) CLASS_ARG="" ;;
esac
if [ -n "$CLASS_ARG" ]; then
  ( cd "$ROOT" && plain sh "$CLASSIFY" "$CLASS_ARG" "$SHA" ) > "$WORK/class.txt" 2> "$WORK/class.err" || { cat "$WORK/class.err" >&2; die "the change could not be classified; nothing was deployed"; }
  MIGRATIONS="$(grep '^migrations_changed=' "$WORK/class.txt" | cut -d= -f2)"
  LIFECYCLE="$(grep '^do_lifecycle=' "$WORK/class.txt" | cut -d= -f2)"
  say "classified: migrations_changed=$MIGRATIONS do_lifecycle=$LIFECYCLE ($(grep '^reason=' "$WORK/class.txt" | cut -d= -f2-))"
else
  # a live state this could not read (several versions share the traffic, or the listing was not what it expects): treat as the worst
  MIGRATIONS=true; LIFECYCLE=true
  say "classified: the live state is '$STATE', so the change counts as a migration and a lifecycle change"
fi
out migrations_changed "$MIGRATIONS"; out do_lifecycle "$LIFECYCLE"
summary "- change: migrations $([ "$MIGRATIONS" = true ] && echo changed || echo unchanged), Durable Object lifecycle $([ "$LIFECYCLE" = true ] && echo changed || echo unchanged)"

if [ "$LIFECYCLE" = true ] && [ "$ALLOW" != true ]; then
  out deferred true; out deployed false; out rolled_back no
  say "DEFERRED: this deploy needs the manual gate (a Durable Object lifecycle change, a first deploy, or a live state that cannot be read). Nothing was deployed."
  summary "- **deferred to the manual-approval job**: nothing was deployed"
  exit 0
fi
out deferred false

# a rollback is possible when there is a single live version to go back to and the change does not cross a lifecycle change
ROLLBACK_OK=false
[ "$AUTO" = true ] && [ "$STATE" = single ] && [ "$LIFECYCLE" = false ] && [ -n "$PREV_ID" ] && ROLLBACK_OK=true
say "automatic rollback: $ROLLBACK_OK"
summary "- automatic rollback: \`$ROLLBACK_OK\`"

# ---- 4. the expand migration ---------------------------------------------------------------------------------------------------
if [ "$MIGRATIONS" = true ]; then
  say "running the expand migration"
  migrate_step || die "the expand migration failed; nothing was deployed (the schema may be partly expanded: every migration is safe to run again)"
  summary "- expand migration: ran"
else
  say "migration files unchanged: no expand migration"
  summary "- expand migration: not needed"
fi

# ---- 5. the deploy -------------------------------------------------------------------------------------------------------------
say "deploying $SHA to $NAME at 100 %"
wr deploy -c "$CONFIG" --tag "$SHA" --message "$DEPLOY_ENV $SHA" || die "wrangler deploy failed; the previous version should still be live"
out deployed true
summary "- deployed with tag \`$SHA\`"

# ---- 6. and 7. the smoke, and the smoke's control ------------------------------------------------------------------------------
rollback() {
  if [ "$ROLLBACK_OK" != true ]; then
    out rolled_back no
    say "NO AUTOMATIC ROLLBACK ($1). If the new version is bad: wrangler rollback ${PREV_ID:-<version>} --name $NAME --message '<reason>' (and see the runbook)"
    summary "- **no automatic rollback**: $1"
    return
  fi
  say "SMOKE FAILED: rolling back to $PREV_ID"
  if ! wr rollback "$PREV_ID" --name "$NAME" --message "rollback of $SHA: $1" --yes; then
    out rolled_back failed
    say "ROLLBACK FAILED. $NAME may be serving a bad version: roll back by hand NOW (wrangler rollback $PREV_ID --name $NAME)."
    summary "- **ROLLBACK FAILED**: roll back by hand"
    return
  fi
  expect="${PREV_TAG:-$PREV_ID}"
  if smoke "SMOKE_AUTH=off SMOKE_CONSECUTIVE=2" "$BASE" "$expect"; then
    out rolled_back yes
    say "rolled back: the previous version ($expect) answers"
    summary "- rolled back to \`$PREV_ID\`: the previous version answers"
  else
    out rolled_back unverified
    say "ROLLBACK NOT VERIFIED: the previous version ($expect) did not answer. Look at the deployments below and decide."
    summary "- **rollback not verified**: the previous version did not answer"
  fi
  wr deployments list --name "$NAME" || true
}

if [ "$SKIP" = true ]; then
  out rolled_back no
  say "SMOKE SKIPPED (the first deploy of $DEPLOY_ENV, which has no hostname yet). Nothing proves this version works: run the smoke once the domain is attached."
  summary "- **smoke skipped** (first deploy): nothing proves this version works yet"
  exit 0
fi

smoke_env=""
[ "$INJECT" = true ] && smoke_env="SMOKE_FORCE_FAIL=1"
if ! smoke "$smoke_env" "$BASE" "$SHA"; then
  summary "- smoke: **FAILED**"
  rollback "the smoke failed"
  die "the smoke failed for $SHA on $DEPLOY_ENV"
fi
summary "- smoke: passed"

say "control: the smoke must refuse a version nobody deployed"
smoke "SMOKE_AUTH=off SMOKE_ATTEMPTS=2 SMOKE_INTERVAL=1 SMOKE_CONSECUTIVE=1" "$BASE" "not-a-deployed-version-$SHA" > "$WORK/control.out" 2>&1
control_rc=$?
if [ "$control_rc" -ne 1 ]; then
  sed 's/^/  control: /' "$WORK/control.out" >&2
  summary "- smoke control: **FAILED** (exit $control_rc, wanted 1)"
  rollback "the smoke's control gave exit $control_rc: it cannot tell versions apart, so its pass proves nothing"
  die "the smoke accepted a version that was never deployed (exit $control_rc): it cannot tell one version from another"
fi
summary "- smoke control: the smoke refuses a version nobody deployed"

out rolled_back no
say "deployed and proved: $SHA is live on $DEPLOY_ENV"
summary "- **$SHA is live on \`$DEPLOY_ENV\`**"
exit 0
