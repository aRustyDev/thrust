#!/bin/sh
# Deploy the edge Worker (`thrust-edge`, ADR-0111, issue #195 stage E3) from a release's attested bundle, and prove it on the shared
# hostname: `wrangler deploy` at 100 % with its secrets, the smoke through the edge to production, the edge's own controls, and an
# automatic `wrangler rollback` when any of them fails. The edge is a single point of failure in front of production, so nothing about
# a new version of it is taken on trust. One script, called by the composite action `.github/actions/deploy-worker` (`script: edge`),
# so that its control flow can be run against fakes (`test-deploy-edge.sh`) instead of being trusted from reading it.
#
#   scripts/deploy-edge.sh          (everything comes from the environment)
#
# A release deploys the edge after production, every release: the edge's tag is then the release's commit, as production's is. (Not
# "only when crates/edge changed": every release bumps the workspace version and the lock, which the edge is built from, so a rule on
# paths could not tell an edge change from a release, and a rule that missed a dependency's fix would be worse than a redeploy.)
#
# The order, and why:
#   1. render the configuration from the environment's variables (scripts/render-wrangler.sh, the edge's template); the edge's build
#      must already be beside it (build/, from the bundle);
#   2. read what is live (`wrangler deployments list` and `versions list`, .github/scripts/active-version.py): the version a rollback
#      goes back to, and its tag. A listing that fails, other than with "the Worker does not exist", is asked for again (as
#      scripts/deploy-worker.sh does: they only read). A live edge whose traffic is split, or whose state cannot be read, STOPS here:
#      nothing a person did not plan is overwritten, and there would be no single version to roll back to;
#   3. `wrangler deploy --tag <sha> --secrets-file <file>`: the edge's three secrets go up with the version (the template's
#      `secrets.required` makes a version without one fail), from a file only wrangler reads, which is removed when this ends;
#   4. the FIRST deploy (nothing was live) stops here, green: the edge has no hostname until the infrastructure moves the shared
#      hostname to it (the cut-over), so there is nothing to smoke. The next release's deploy proves it;
#   5. the smoke (scripts/smoke-deploy.sh) on the shared hostname: the edge that answers must be this version
#      (`x-thrust-edge-version`), the request must have gone to production (`x-thrust-route: prod`), production must be the version
#      expected (`x-thrust-version`), and a real read must work through the edge;
#   6. the controls: the same smoke asked for an edge version nobody deployed MUST fail (a smoke that passes that proves nothing), and
#      the edge's own (scripts/edge-controls.sh): a forged pin still goes to production, and the opt-in gives no pin to a request
#      Access did not admit;
#   7. when 5 or 6 fails and a rollback is possible (AUTO_ROLLBACK=true and one version was live): `wrangler rollback <previous>`, then
#      the smoke without the read must show the previous edge answering and production behind it. The run stays RED either way.
#
# Environment
#   DEPLOY_SHA            the release's commit, 40 hex digits: the version's tag
#   PRODUCTION_VERSION    what production must answer through the edge (default: DEPLOY_SHA; the edge deploys after production
#                         deployed the same release)
#   THRUST_BASE_URL       https://<the shared hostname>, no path
#   THRUST_EDGE_WORKER_NAME, THRUST_WORKER_NAME (production, which the edge's service binding names), and the rest of the edge
#                         template's variables (scripts/render-wrangler.sh): THRUST_FLAGSHIP_APP_ID, THRUST_ACCESS_TEAM_DOMAIN,
#                         THRUST_ACCESS_AUD_RC, THRUST_ACCESS_AUD_DEV, THRUST_WORKERS_DEV_SUBDOMAIN
#   EDGE_PIN_KEY          the pin's HMAC key, at least 32 bytes (the edge refuses every pin under a shorter one)
#   PREVIEW_ACCESS_CLIENT_ID, PREVIEW_ACCESS_CLIENT_SECRET   the service token the edge shows Access on the Previews
#   AUTO_ROLLBACK         true to roll back by itself (default true)
#   CLOUDFLARE_API_TOKEN, CLOUDFLARE_ACCOUNT_ID              wrangler's own
#   SMOKE_EMAIL, SMOKE_PASSWORD, SMOKE_WORKSPACE (SMOKE_PROJECT)   the smoke's (production's smoke account: the edge's smoke reads
#                         production's data, through the edge)
#   WRANGLER_READ_ATTEMPTS (default 3), WRANGLER_READ_RETRY_SECONDS (default 5)   as in scripts/deploy-worker.sh
# Test hooks: WRANGLER, SMOKE_SCRIPT, CONTROLS_SCRIPT, DEPLOY_CONFIG_OUT (default crates/edge/.wrangler.deploy.run.toml; the edge's
# build must be in build/ beside it)
#
# Outputs (to $GITHUB_OUTPUT when it is set, else to stdout): state, previous_version, previous_tag, deployed, smoke
# (passed|failed|skipped), rolled_back (no|yes|failed|unverified). A summary goes to $GITHUB_STEP_SUMMARY.
# Exit status: 0 deployed and proved (or the first deploy, deployed and not yet reachable); 1 anything else; 2 called wrongly.
set -u
LC_ALL=C
export LC_ALL

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WRANGLER="${WRANGLER:-wrangler}"
SMOKE="${SMOKE_SCRIPT:-$ROOT/scripts/smoke-deploy.sh}"
CONTROLS="${CONTROLS_SCRIPT:-$ROOT/scripts/edge-controls.sh}"
RENDER="$ROOT/scripts/render-wrangler.sh"
ACTIVE="$ROOT/.github/scripts/active-version.py"
TEMPLATE="$ROOT/crates/edge/wrangler.deploy.toml"
CONFIG="${DEPLOY_CONFIG_OUT:-$ROOT/crates/edge/.wrangler.deploy.run.toml}"
PYTHON="${PYTHON:-python3}"

say() { echo "deploy-edge: $*"; }
summary() { [ -n "${GITHUB_STEP_SUMMARY:-}" ] && printf '%s\n' "$*" >> "$GITHUB_STEP_SUMMARY"; return 0; }
die() { echo "deploy-edge: FAIL: $*" >&2; summary "FAILED: $*"; exit 1; }
usage() { echo "deploy-edge: $*" >&2; exit 2; }
out() { # out <key> <value>: one line of output, never more than one line
  value="$(printf '%s' "$2" | tr -d '\n')"
  if [ -n "${GITHUB_OUTPUT:-}" ]; then printf '%s=%s\n' "$1" "$value" >> "$GITHUB_OUTPUT"; else printf '%s=%s\n' "$1" "$value"; fi
}

# ---- inputs --------------------------------------------------------------------------------------------------------------------
is_sha() { case "$1" in *[!0-9a-f]*|'') return 1 ;; esac; [ "${#1}" -eq 40 ]; }
is_sha "${DEPLOY_SHA:-}" || usage "DEPLOY_SHA is not a 40-digit commit sha"
SHA="$DEPLOY_SHA"
PROD="${PRODUCTION_VERSION:-$SHA}"
# what a tag or an id can be (scripts/smoke-deploy.sh applies the same rule)
case "$PROD" in *[!0-9A-Za-z._:/@+=-]*) usage "PRODUCTION_VERSION has a character a version tag does not: '$PROD'" ;; esac
case "${THRUST_BASE_URL:-}" in https://*) ;; *) usage "THRUST_BASE_URL must start with https://" ;; esac
case "${THRUST_BASE_URL#https://}" in */*|'') usage "THRUST_BASE_URL must be a host with no path" ;; esac
BASE="$THRUST_BASE_URL"
for v in THRUST_EDGE_WORKER_NAME THRUST_WORKER_NAME CLOUDFLARE_API_TOKEN CLOUDFLARE_ACCOUNT_ID PREVIEW_ACCESS_CLIENT_ID \
  PREVIEW_ACCESS_CLIENT_SECRET EDGE_PIN_KEY; do
  eval "val=\${$v:-}"
  [ -n "$val" ] || usage "$v is not set"
done
NAME="$THRUST_EDGE_WORKER_NAME"
[ "$NAME" != "$THRUST_WORKER_NAME" ] || usage "THRUST_EDGE_WORKER_NAME is production's name ($NAME): the edge would replace production"
# the edge's HMAC is refused under 32 bytes (crates/edge/src/pin.rs, MIN_KEY_LEN): every pin would fail, and nothing would say why
[ "$(printf '%s' "$EDGE_PIN_KEY" | wc -c)" -ge 32 ] || usage "EDGE_PIN_KEY is shorter than 32 bytes, which the edge refuses as a key"
AUTO="${AUTO_ROLLBACK:-true}"
case "$AUTO" in true|false) ;; *) usage "AUTO_ROLLBACK must be true or false, not '$AUTO'" ;; esac
READ_ATTEMPTS="${WRANGLER_READ_ATTEMPTS:-3}"; READ_PAUSE="${WRANGLER_READ_RETRY_SECONDS:-5}"
case "$READ_ATTEMPTS" in ''|*[!0-9]*|0) usage "WRANGLER_READ_ATTEMPTS must be a positive whole number, not '$READ_ATTEMPTS'" ;; esac
case "$READ_PAUSE" in ''|*[!0-9]*) usage "WRANGLER_READ_RETRY_SECONDS must be a whole number of seconds, not '$READ_PAUSE'" ;; esac
BUILD="$(dirname "$CONFIG")/build"
for f in worker/shim.mjs index.js index_bg.wasm; do
  [ -s "$BUILD/$f" ] || usage "the edge's build is not beside its configuration: $BUILD/$f is missing (unpack the bundle's edge/build there)"
done

WORK="$(mktemp -d)" || usage "cannot make a temporary directory"
trap 'rm -rf "$WORK"' EXIT INT TERM

# Each secret reaches only the process that needs it: wrangler gets the Cloudflare token and the FILE of the edge's secrets, never
# their values in its environment or its argv; the smoke gets only its account; the controls get nothing.
plain() { env -u CLOUDFLARE_API_TOKEN -u EDGE_PIN_KEY -u PREVIEW_ACCESS_CLIENT_ID -u PREVIEW_ACCESS_CLIENT_SECRET -u SMOKE_PASSWORD "$@"; }
wr() { env -u EDGE_PIN_KEY -u PREVIEW_ACCESS_CLIENT_ID -u PREVIEW_ACCESS_CLIENT_SECRET -u SMOKE_PASSWORD "$WRANGLER" "$@"; }
smoke() { # smoke <extra VAR=value words, or empty> <expected production version>
  extra="$1"; shift
  # $extra is deliberately split: it is only ever `NAME=value` words written in this file
  env -u CLOUDFLARE_API_TOKEN -u EDGE_PIN_KEY -u PREVIEW_ACCESS_CLIENT_ID -u PREVIEW_ACCESS_CLIENT_SECRET $extra sh "$SMOKE" "$BASE" "$@"
}

summary "### Deploy of the edge \`$NAME\` at \`$SHA\`"

# ---- 1. the configuration ------------------------------------------------------------------------------------------------------
plain sh "$RENDER" --template "$TEMPLATE" --out "$CONFIG" || die "the edge's configuration could not be rendered (see above); nothing was deployed"

# ---- 2. what is live -----------------------------------------------------------------------------------------------------------
list_json() { # list_json <what> <file>: `wrangler <what> list --json` into <file>; a Worker that does not exist is not an error
  try=1
  while :; do
    wr "$1" list --name "$NAME" --json > "$2" 2> "$2.err"
    rc=$?
    [ "$rc" -eq 0 ] && break
    if cat "$2" "$2.err" | grep -qiE '10007|could not find|does not exist|not found'; then
      echo '[]' > "$2"
      break
    fi
    [ "$try" -ge "$READ_ATTEMPTS" ] && return 1
    pause=$((READ_PAUSE * try))
    say "wrangler $1 list failed (try $try of $READ_ATTEMPTS: $(tr -s '\n ' ' ' < "$2.err" | cut -c1-600)); trying again in ${pause}s"
    sleep "$pause"
    try=$((try + 1))
  done
  [ "$try" -gt 1 ] && say "wrangler $1 list answered on try $try of $READ_ATTEMPTS"
  return 0
}
list_json deployments "$WORK/deployments.json" || { sed 's/^/  wrangler: /' "$WORK/deployments.json.err" >&2; die "wrangler could not list the deployments of $NAME; nothing was deployed"; }
list_json versions "$WORK/versions.json" || { sed 's/^/  wrangler: /' "$WORK/versions.json.err" >&2; die "wrangler could not list the versions of $NAME; nothing was deployed"; }
plain "$PYTHON" -I "$ACTIVE" "$WORK/deployments.json" "$WORK/versions.json" > "$WORK/active.txt" || die "the live version of $NAME could not be read; nothing was deployed"
active() { grep "^$1=" "$WORK/active.txt" | head -1 | cut -d= -f2-; }
STATE="$(active state)"; PREV_ID="$(active version_id)"; PREV_TAG="$(active tag)"
say "live: state=$STATE version=${PREV_ID:-none} tag=${PREV_TAG:-none} ($(active reason))"
out state "$STATE"; out previous_version "$PREV_ID"; out previous_tag "$PREV_TAG"
summary "- live before: \`$STATE\` ${PREV_ID:+version \`$PREV_ID\`}${PREV_TAG:+ tag \`$PREV_TAG\`}"
case "$STATE" in
  none|single) ;;
  *) die "the live edge is '$STATE' ($(active reason)): one version must take all of its traffic before this deploys over it (wrangler deployments list --name $NAME); nothing was deployed" ;;
esac
ROLLBACK_OK=false
[ "$AUTO" = true ] && [ "$STATE" = single ] && [ -n "$PREV_ID" ] && ROLLBACK_OK=true
say "automatic rollback: $ROLLBACK_OK"
summary "- automatic rollback: \`$ROLLBACK_OK\`"

# ---- 3. the deploy, with its secrets -------------------------------------------------------------------------------------------
# The file is written by a process that reads the values from its environment, so no value is ever in an argv, and only the owner
# can read it; the trap removes it with the rest of $WORK.
( umask 077 && "$PYTHON" -I -c '
import json, os, sys
names = ("EDGE_PIN_KEY", "PREVIEW_ACCESS_CLIENT_ID", "PREVIEW_ACCESS_CLIENT_SECRET")
with open(sys.argv[1], "w") as f:
    json.dump({n: os.environ[n] for n in names}, f)
' "$WORK/secrets.json" ) || die "the edge's secrets could not be written for wrangler; nothing was deployed"
say "deploying $SHA to $NAME at 100 %"
wr deploy -c "$CONFIG" --tag "$SHA" --message "edge $SHA" --secrets-file "$WORK/secrets.json" \
  || die "wrangler deploy failed; the previous edge should still be live"
rm -f "$WORK/secrets.json"
out deployed true
summary "- deployed with tag \`$SHA\`"

# ---- 4. the first deploy -------------------------------------------------------------------------------------------------------
if [ "$STATE" = none ]; then
  out smoke skipped; out rolled_back no
  say "FIRST DEPLOY: $NAME had no version, so no hostname is attached to it yet and there is nothing to smoke. Move the shared hostname to it (the cut-over); the next release's deploy proves it."
  summary "- **first deploy**: the edge has no hostname yet, so nothing proves it works until the next release's deploy"
  exit 0
fi

# ---- 5., 6. and 7. the smoke, the controls, and the rollback ---------------------------------------------------------------------
rollback() {
  if [ "$ROLLBACK_OK" != true ]; then
    out rolled_back no
    say "NO AUTOMATIC ROLLBACK ($1). If the edge is bad: wrangler rollback ${PREV_ID:-<version>} --name $NAME --message '<reason>'"
    summary "- **no automatic rollback**: $1"
    return
  fi
  say "rolling back $NAME to $PREV_ID ($1)"
  if ! wr rollback "$PREV_ID" --name "$NAME" --message "rollback of $SHA: $1" --yes; then
    out rolled_back failed
    say "ROLLBACK FAILED. The shared hostname may be serving a bad edge: roll back by hand NOW (wrangler rollback $PREV_ID --name $NAME)."
    summary "- **ROLLBACK FAILED**: roll back by hand"
    return
  fi
  expect="${PREV_TAG:-$PREV_ID}"
  if smoke "SMOKE_AUTH=off SMOKE_CONSECUTIVE=2 SMOKE_EXPECT_ROUTE=prod SMOKE_EXPECT_EDGE=$expect" "$PROD"; then
    out rolled_back yes
    say "rolled back: the previous edge ($expect) answers, with production behind it"
    summary "- rolled back to \`$PREV_ID\`: the previous edge answers"
  else
    out rolled_back unverified
    say "ROLLBACK NOT VERIFIED: the previous edge ($expect) did not answer as expected. Look at the deployments below and decide."
    summary "- **rollback not verified**"
  fi
  wr deployments list --name "$NAME" || true
}
fail_after_deploy() { # fail_after_deploy <why>
  out smoke failed
  rollback "$1"
  die "$1"
}

if ! smoke "SMOKE_EXPECT_ROUTE=prod SMOKE_EXPECT_EDGE=$SHA" "$PROD"; then
  summary "- smoke through the edge: **FAILED**"
  fail_after_deploy "the smoke through the edge failed for $SHA"
fi
summary "- smoke through the edge: passed (this edge answered, production behind it at \`$PROD\`, and a real read worked)"

say "control: the smoke must refuse an edge version nobody deployed"
smoke "SMOKE_AUTH=off SMOKE_ATTEMPTS=2 SMOKE_INTERVAL=1 SMOKE_CONSECUTIVE=1 SMOKE_EXPECT_ROUTE=prod SMOKE_EXPECT_EDGE=not-a-deployed-version-$SHA" "$PROD" > "$WORK/control.out" 2>&1
control_rc=$?
if [ "$control_rc" -ne 1 ]; then
  sed 's/^/  control: /' "$WORK/control.out" >&2
  summary "- smoke control: **FAILED** (exit $control_rc, wanted 1)"
  fail_after_deploy "the smoke accepted an edge version that was never deployed (exit $control_rc): it cannot tell one edge from another"
fi
summary "- smoke control: the smoke refuses an edge version nobody deployed"

say "control: the edge's own (a forged pin goes to production; the opt-in admits no one Access did not)"
plain sh "$CONTROLS" "$BASE" "$SHA" > "$WORK/controls.out" 2>&1
controls_rc=$?
sed 's/^/  controls: /' "$WORK/controls.out"
if [ "$controls_rc" -ne 0 ]; then
  summary "- the edge's controls: **FAILED**: $(tail -1 "$WORK/controls.out")"
  fail_after_deploy "the edge's controls failed (exit $controls_rc): $(tail -1 "$WORK/controls.out")"
fi
summary "- the edge's controls: passed"

out smoke passed; out rolled_back no
say "deployed and proved: $SHA is the edge on $BASE"
summary "- **$SHA is the edge on $BASE**"
exit 0
