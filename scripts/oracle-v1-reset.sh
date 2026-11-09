#!/bin/sh
# Empty a Thrust target and load the v1 oracle's seed into it (issue #58).
#
#   THRUST_BASE_URL=http://127.0.0.1:8080 THRUST_ADMIN_TOKEN=... scripts/oracle-v1-reset.sh
#
# The oracle's gate (and its controls) must start from the rows its recorded corpus was made from: the workspace `oracle-ws`, two
# projects, their states, labels and eleven work items, and seven API tokens, under the uuid5 ids of `tools/oracle-v1/corpus/
# seed-manifest.json`. A run that creates things (the gate does) leaves the target different, so this runs before every gate and
# control run. It does three things, and says which one failed:
#
#   1. POST /internal/reset   every row of every table, gone (the schema stays)                       exit 3 if not a 204
#   2. POST /internal/seed    the fixture that `oracle-fixture` prints (crates/thrust-oracle-fixture) exit 4 if not a 204
#   3. two requests through /api/v1 that show the load took: the owner's token lists both projects (200), and the oracle's
#      `expired` token is refused as Plane refuses it (403). A load that put nothing there cannot pass both.   exit 5 if not
#
# DESTRUCTIVE. `/internal/reset` deletes everything in the target's database. It is behind the target's test routes and the admin
# token, and the target must be one that nobody keeps anything in: a local server, or the contract suite's own database. Never point
# this at a database with data you want.
#
# Secrets: the admin token is read from the environment and handed to curl in a private (0600) config file, never on argv; the
# oracle's own tokens are fake. The temp directory is removed on exit.
set -u

URL="${THRUST_BASE_URL:?usage: THRUST_BASE_URL=<target> THRUST_ADMIN_TOKEN=<token> scripts/oracle-v1-reset.sh}"
TOKEN="${THRUST_ADMIN_TOKEN:?usage: THRUST_BASE_URL=<target> THRUST_ADMIN_TOKEN=<token> scripts/oracle-v1-reset.sh}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

TMP="$(mktemp -d)" || exit 1
trap 'rm -rf "$TMP"' EXIT INT TERM
FIXTURE="$TMP/fixture.json"
MANIFEST="$TMP/manifest.json"
ADMIN="$TMP/admin.curl"
( umask 077; printf 'header = "x-admin-token: %s"\n' "$TOKEN" > "$ADMIN" ) || exit 1

# The fixture is made at this moment: its dates are relative to today and its `expired` token expired a day ago.
cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p thrust-oracle-fixture --bin oracle-fixture > "$FIXTURE" || {
  echo "oracle-v1-reset: could not make the fixture" >&2
  exit 1
}
cargo run --quiet --manifest-path "$ROOT/Cargo.toml" -p thrust-oracle-fixture --bin oracle-fixture -- --manifest > "$MANIFEST" || {
  echo "oracle-v1-reset: could not make the manifest" >&2
  exit 1
}

# The status of one request, and nothing else on stdout.
status() {
  curl -sS --max-time 120 -o "$TMP/body" -w '%{http_code}' "$@"
}

code="$(status --config "$ADMIN" -X POST "$URL/internal/reset")"
if [ "$code" != 204 ]; then
  echo "oracle-v1-reset: POST /internal/reset answered $code, not 204 (is the target started with test routes and this admin token?)" >&2
  head -c 400 "$TMP/body" >&2
  exit 3
fi

code="$(status --config "$ADMIN" -X POST -H 'content-type: application/json' --data-binary "@$FIXTURE" "$URL/internal/seed")"
if [ "$code" != 204 ]; then
  echo "oracle-v1-reset: POST /internal/seed answered $code, not 204" >&2
  head -c 400 "$TMP/body" >&2
  exit 4
fi

# A token's line in the manifest is `"name": "plane_api_..."`; the users section has `"owner": {` and so cannot match.
token_of() {
  sed -n "s/^ *\"$1\": \"\(plane_api_[A-Za-z0-9_]*\)\",\{0,1\}$/\1/p" "$MANIFEST"
}
OWNER="$(token_of owner)"
EXPIRED="$(token_of expired)"
if [ -z "$OWNER" ] || [ -z "$EXPIRED" ]; then
  echo "oracle-v1-reset: the manifest has no owner or expired token" >&2
  exit 5
fi

# the key goes to curl on stdin as a config, not on argv
with_key() {
  printf 'header = "x-api-key: %s"\n' "$1" | status --config - "$URL/api/v1/workspaces/oracle-ws/projects/"
}

code="$(with_key "$OWNER")"
if [ "$code" != 200 ]; then
  echo "oracle-v1-reset: the owner's token got $code from GET /api/v1/workspaces/oracle-ws/projects/, not 200" >&2
  head -c 400 "$TMP/body" >&2
  exit 5
fi
if ! grep -q '"ORA"' "$TMP/body" || ! grep -q '"ORB"' "$TMP/body"; then
  echo "oracle-v1-reset: the owner's project list does not name both ORA and ORB" >&2
  head -c 400 "$TMP/body" >&2
  exit 5
fi

code="$(with_key "$EXPIRED")"
if [ "$code" != 403 ]; then
  echo "oracle-v1-reset: the expired token got $code, not the 403 Plane gives it" >&2
  exit 5
fi

echo "oracle-v1-reset: $URL is emptied and holds the oracle's seed (reset 204, seed 204, owner 200, expired 403)"
