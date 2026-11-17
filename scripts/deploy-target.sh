#!/bin/sh
# Deploy one target, a Worker Preview of the production Worker, and prove it through the shared hostname (ADR-0111, #195 stage E).
# Everything comes from the environment, as scripts/deploy-worker.sh's does; a test drives it against fakes (test-deploy-target.sh).
#
#   scripts/deploy-target.sh
#
# The order, and why:
#   1. check every input before anything is asked of Cloudflare: a usage error deploys nothing (exit 2). The kind must be a target
#      kind of .github/deploy/flagship/policy.json, and the policy must let CI reach it through TARGET_AUDIENCE, or the smoke could
#      never opt in;
#   2. render production's configuration (scripts/render-wrangler.sh, production's variables) and derive the target's from it
#      (scripts/preview-config.py, `shared`: production's data and rooms);
#   3. `wrangler preview --name <label> --json --ignore-base-config --tag <sha>`, the label being the name with `.` as `-` (a
#      Preview's URL is `https://<label>-<worker>.<subdomain>.workers.dev`);
#   4. read its answer (scripts/preview-result.py): the URL the edge accepts, the deployment id, and the bindings as Cloudflare
#      applied them, rooms on production's Worker and variables as given; a Preview that is not the one asked for fails here;
#   5. write the target's flag (scripts/flagship.py put, which reads it back): {url, deployment, release = the commit, kind,
#      data = shared, expires}. From here the edge routes to it whoever the policy allows;
#   6. opt in at the edge as CI (scripts/edge-opt-in.sh, which waits while the flag propagates);
#   7. the smoke through the edge with that pin (scripts/smoke-deploy.sh): the answers must carry the deployment id and the
#      target's route, and the smoke account's read must work;
#   8. the smoke's own control: the same smoke asked for a version nobody deployed MUST fail, or it could not tell one version
#      from another.
# Any failure from step 3 on deletes the target again: its flag first, so the edge stops routing to it, then the Preview. A deploy
# over a live target that fails therefore leaves no target of that name (the drill of ADR-0111: opted-in requests then reach
# production). A cleanup that fails is said loudly, with what was left behind.
#
# Environment
#   TARGET_NAME       the target (crates/edge/src/pin.rs, valid_name): `v0.3.0`, `staging-1`
#   TARGET_KIND       a target kind of the policy (`rc`, `release`, `dev`)
#   TARGET_AUDIENCE   the opt-in path the smoke uses (`rc`, `dev`): one the policy lets CI reach this kind through
#   TARGET_EXPIRES    YYYY-MM-DDTHH:MM:SSZ, when the target may be removed
#   DEPLOY_SHA        the commit (40 hex digits): the flag's `release`, the Preview deployment's tag
#   THRUST_BASE_URL   production's public origin, which is the edge's shared hostname: https://<host>
#   NEW_ASSETS_DIR    the web build of this commit, with an index.html
#   the renderer's variables (production's), CLOUDFLARE_API_TOKEN and CLOUDFLARE_ACCOUNT_ID (wrangler's)
#   FLAGSHIP_API_TOKEN, FLAGSHIP_APP_ID, PREVIEW_HOST_SUFFIX (flagship.py's; FLAGSHIP_ACCOUNT_ID defaults to CLOUDFLARE_ACCOUNT_ID)
#   EDGE_CI_CLIENT_ID, EDGE_CI_CLIENT_SECRET (edge-opt-in.sh's)
#   the smoke's: SMOKE_EMAIL, SMOKE_PASSWORD, SMOKE_WORKSPACE (production's smoke account: a shared target reads production's
#   data), or SMOKE_AUTH=off
# Each tool sees only the secret it needs: wrangler the Cloudflare token, flagship.py the Flagship token, the opt-in its service
# token, the smoke its password; the render and the readers none.
# Test hooks: WRANGLER, FLAGSHIP_SCRIPT, OPT_IN_SCRIPT, SMOKE_SCRIPT, PYTHON, DEPLOY_CONFIG_OUT, ASSETS_OUT_DIR
#
# Outputs (to $GITHUB_OUTPUT when it is set, else to stdout): deployed, target, target_url, deployment_id, cleaned (n/a|yes|failed).
# Exit status: 0 deployed and proved; 1 anything else (the target removed again, or said to be left); 2 called wrongly (nothing done).
set -u
LC_ALL=C
export LC_ALL

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WRANGLER="${WRANGLER:-wrangler}"
PYTHON="${PYTHON:-python3}"
FLAGSHIP="${FLAGSHIP_SCRIPT:-$ROOT/scripts/flagship.py}"
OPT_IN="${OPT_IN_SCRIPT:-$ROOT/scripts/edge-opt-in.sh}"
SMOKE="${SMOKE_SCRIPT:-$ROOT/scripts/smoke-deploy.sh}"
RENDER="$ROOT/scripts/render-wrangler.sh"
PREVIEW_CONFIG="$ROOT/scripts/preview-config.py"
PREVIEW_RESULT="$ROOT/scripts/preview-result.py"
POLICY="$ROOT/.github/deploy/flagship/policy.json"
CONFIG="${DEPLOY_CONFIG_OUT:-$ROOT/crates/worker/.wrangler.target.run.toml}"
TARGET_CONFIG="${CONFIG%.toml}.json"
ASSETS_OUT="${ASSETS_OUT_DIR:-$ROOT/crates/worker/.assets}"
SECRETS="CLOUDFLARE_API_TOKEN FLAGSHIP_API_TOKEN EDGE_CI_CLIENT_ID EDGE_CI_CLIENT_SECRET SMOKE_PASSWORD GH_TOKEN DATABASE_URL"

WORK="$(mktemp -d)" || { echo "deploy-target: cannot make a temporary directory" >&2; exit 2; }
trap 'rm -rf "$WORK"' EXIT INT TERM

say() { echo "deploy-target: $*"; }
usage() { echo "deploy-target: $*" >&2; exit 2; }
out() { # out <key> <value>: one line of output
  value="$(printf '%s' "$2" | tr -d '\n')"
  if [ -n "${GITHUB_OUTPUT:-}" ]; then printf '%s=%s\n' "$1" "$value" >> "$GITHUB_OUTPUT"; else printf '%s=%s\n' "$1" "$value"; fi
}
# only <secret to keep | EDGE_CI | -> <command...>: run it without every other secret in its environment
only() {
  keep="$1"; shift
  for s in $SECRETS; do
    [ "$s" = "$keep" ] && continue
    if [ "$keep" = EDGE_CI ]; then case "$s" in EDGE_CI_CLIENT_ID|EDGE_CI_CLIENT_SECRET) continue ;; esac; fi
    set -- -u "$s" "$@"
  done
  env "$@"
}

# ---- 1. the inputs --------------------------------------------------------------------------------------------------------
NAME="${TARGET_NAME:-}"; KIND="${TARGET_KIND:-}"; AUD="${TARGET_AUDIENCE:-}"; EXPIRES="${TARGET_EXPIRES:-}"; SHA="${DEPLOY_SHA:-}"
BASE="${THRUST_BASE_URL:-}"
for v in TARGET_NAME TARGET_KIND TARGET_AUDIENCE TARGET_EXPIRES DEPLOY_SHA THRUST_BASE_URL NEW_ASSETS_DIR CLOUDFLARE_API_TOKEN \
  CLOUDFLARE_ACCOUNT_ID FLAGSHIP_API_TOKEN FLAGSHIP_APP_ID PREVIEW_HOST_SUFFIX EDGE_CI_CLIENT_ID EDGE_CI_CLIENT_SECRET; do
  eval "val=\${$v:-}"
  [ -n "$val" ] || usage "$v is not set"
done
case "$NAME" in [a-z0-9]*) ;; *) usage "TARGET_NAME starts with a lowercase letter or a digit: '$NAME'" ;; esac
case "$NAME" in *[!a-z0-9.-]*) usage "TARGET_NAME is lowercase letters, digits, '.' and '-': '$NAME'" ;; esac
[ "${#NAME}" -le 57 ] || usage "TARGET_NAME is at most 57 characters"
LABEL="$(printf '%s' "$NAME" | tr . -)"
case "$LABEL" in *-) usage "TARGET_NAME may not end with '.' or '-' (its Preview's label would)" ;; esac
case "$SHA" in *[!0-9a-f]*) usage "DEPLOY_SHA is a commit: 40 lowercase hex digits" ;; esac
[ "${#SHA}" -eq 40 ] || usage "DEPLOY_SHA is a commit: 40 lowercase hex digits"
case "$BASE" in https://*) ;; *) usage "THRUST_BASE_URL is https://<host>" ;; esac
[ -f "$NEW_ASSETS_DIR/index.html" ] || usage "NEW_ASSETS_DIR has no index.html: '$NEW_ASSETS_DIR'"
# the kind, the audience and the expiry, against the policy and the edge's grammars; prints why when they do not hold
plain_python() { only - "$PYTHON" -I "$@"; }
plain_python - "$POLICY" "$KIND" "$AUD" "$EXPIRES" <<'EOF' || usage "the kind, the audience or the expiry is refused (above)"
import datetime, json, re, sys
policy, kind, aud, expires = json.load(open(sys.argv[1])), sys.argv[2], sys.argv[3], sys.argv[4]
why = []
rules = policy["targets"].get(kind)
if rules is None:
    why.append(f"TARGET_KIND {kind!r} is not one of the policy's target kinds {sorted(policy['targets'])}")
elif not any("ci" in r["kinds"] and aud in r["audiences"] for r in rules):
    why.append(f"the policy does not let CI reach a {kind!r} target through {aud!r}: the smoke could never opt in")
try:
    ok = re.match(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$", expires) and datetime.datetime.strptime(expires, "%Y-%m-%dT%H:%M:%SZ")
except ValueError:
    ok = False
if not ok:
    why.append(f"TARGET_EXPIRES {expires!r} is not YYYY-MM-DDTHH:MM:SSZ")
for w in why:
    print(f"deploy-target: {w}", file=sys.stderr)
sys.exit(1 if why else 0)
EOF
export FLAGSHIP_ACCOUNT_ID="${FLAGSHIP_ACCOUNT_ID:-$CLOUDFLARE_ACCOUNT_ID}"

# ---- 2. the configuration -------------------------------------------------------------------------------------------------
only - sh "$RENDER" --out "$CONFIG" || usage "production's configuration could not be rendered (above); nothing was deployed"
only - "$PYTHON" -I "$PREVIEW_CONFIG" --config "$CONFIG" --profile shared --out "$TARGET_CONFIG" \
  || usage "the target's configuration could not be derived (above); nothing was deployed"
rm -rf "$ASSETS_OUT" && mkdir -p "$ASSETS_OUT" && cp -R "$NEW_ASSETS_DIR/." "$ASSETS_OUT/" \
  || usage "the web files could not be put in place; nothing was deployed"

created=no
flagged=no
cleanup() {
  cleaned=yes
  # `maybe` too: a put that failed its read-back may have written something (flagship.py's delete of an absent flag is a success)
  if [ "$flagged" != no ]; then
    if only FLAGSHIP_API_TOKEN "$PYTHON" -I "$FLAGSHIP" delete "$NAME"; then flagged=no; else cleaned=failed; fi
  fi
  if [ "$created" != no ]; then
    if only CLOUDFLARE_API_TOKEN "$WRANGLER" preview delete --name "$LABEL" --skip-confirmation --config "$TARGET_CONFIG"; then
      created=no
    elif [ "$created" = yes ]; then
      cleaned=failed
    fi
  fi
  out cleaned "$cleaned"
  if [ "$cleaned" = failed ]; then
    left=""
    [ "$flagged" = no ] || left="the flag target-$(printf '%s' "$NAME" | tr . _);"
    [ "$created" = no ] || left="$left the Preview $LABEL of the production Worker;"
    echo "deploy-target: LEFT BEHIND, remove by hand: $left" >&2
  fi
}
die() { echo "deploy-target: FAIL: $*" >&2; out deployed false; cleanup; exit 1; }

out target "$NAME"
# ---- 3. the Preview -------------------------------------------------------------------------------------------------------
say "deploying $NAME as the Preview $LABEL of $(sed -n 's/^name = "\(.*\)"$/\1/p' "$CONFIG" | head -1) ($KIND, shared data, $SHA)"
created=maybe
only CLOUDFLARE_API_TOKEN "$WRANGLER" preview --name "$LABEL" --json --ignore-base-config --tag "$SHA" --config "$TARGET_CONFIG" \
  > "$WORK/preview.out" 2> "$WORK/preview.err" || { cat "$WORK/preview.err" >&2; die "wrangler preview failed"; }
created=yes

# ---- 4. what was deployed -------------------------------------------------------------------------------------------------
only - "$PYTHON" -I "$PREVIEW_RESULT" "$WORK/preview.out" "$TARGET_CONFIG" "$LABEL" "$PREVIEW_HOST_SUFFIX" > "$WORK/result" \
  || die "the Preview is not the one asked for (above)"
URL="$(sed -n 's/^url=//p' "$WORK/result")"
DEPLOYMENT="$(sed -n 's/^deployment=//p' "$WORK/result")"
out target_url "$URL"
out deployment_id "$DEPLOYMENT"
say "the Preview answers at $URL, deployment $DEPLOYMENT"

# ---- 5. the flag ----------------------------------------------------------------------------------------------------------
printf '{"url": "%s", "deployment": "%s", "release": "%s", "kind": "%s", "data": "shared", "expires": "%s"}\n' \
  "$URL" "$DEPLOYMENT" "$SHA" "$KIND" "$EXPIRES" > "$WORK/value.json"
flagged=maybe
only FLAGSHIP_API_TOKEN "$PYTHON" -I "$FLAGSHIP" put "$NAME" "$WORK/value.json" || die "the target's flag was not written"
flagged=yes

# ---- 6 and 7. the opt-in and the smoke, through the edge -----------------------------------------------------------------
only EDGE_CI sh "$OPT_IN" "$BASE" "$AUD" "$NAME" "$WORK/pin.jar" || die "the edge did not let CI opt in to $NAME"
only SMOKE_PASSWORD env SMOKE_COOKIE_FILE="$WORK/pin.jar" SMOKE_EXPECT_ROUTE="$NAME" sh "$SMOKE" "$BASE" "$DEPLOYMENT" \
  || die "the smoke of $NAME through the edge failed"

# ---- 8. the smoke's control -----------------------------------------------------------------------------------------------
only - env SMOKE_COOKIE_FILE="$WORK/pin.jar" SMOKE_EXPECT_ROUTE="$NAME" SMOKE_ATTEMPTS=2 SMOKE_INTERVAL=0 SMOKE_AUTH=off \
  sh "$SMOKE" "$BASE" "not-deployed-$DEPLOYMENT" > "$WORK/control.out" 2>&1
control=$?
[ "$control" = 1 ] || { cat "$WORK/control.out" >&2; die "the smoke's control: asked for a version nobody deployed, the smoke exited $control, not 1; it cannot tell one version from another"; }

out deployed true
out cleaned n/a
say "PASS: $NAME is deployed ($URL, deployment $DEPLOYMENT) and answers through $BASE"
