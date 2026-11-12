#!/bin/sh
# Recreate the pinned Plane's oracle stack from nothing: down with its volumes, up, wait for the API.
#
#   ORACLE_DIR=<checkout of the Plane fork on the pin branch>/oracle tools/compat-record/fresh-oracle.sh
#
# The oracle is not in this repository (the pin tag is local to the fork, Q-0122.4); `oracle/PIN` in that checkout says
# which commit it is. record.py refuses a stack that has been recorded against, because the journey creates users, so
# run this before every recording. It drops the stack's volumes and nothing else.
set -eu
: "${ORACLE_DIR:?set ORACLE_DIR to the oracle/ directory of the Plane fork checkout on the pin branch}"
BASE="${BASE_URL:-http://127.0.0.1:18080}"

# oracle-assets.yml (next to this script) sets AWS_S3_UPLOAD_METHOD=put on the api, for the asset rows.
HERE="$(cd "$(dirname "$0")" && pwd)"

# ORACLE_COMPOSE_EXTRA: more override files, space-separated, absolute paths without spaces, merged after oracle-assets.yml. Unset
# (the recorder's case), nothing changes. scripts/e2e.sh plane-oracle adds tools/e2e-plane/compose.ui.yml: the stock web app beside
# the API, which the browser journeys need and the recorder does not.
EXTRA=""
for extra_file in ${ORACLE_COMPOSE_EXTRA:-}; do
  EXTRA="$EXTRA -f $extra_file"
done

dc() {
  # $EXTRA is a list of -f arguments, so it is left unquoted on purpose
  docker compose -f "$ORACLE_DIR/compose.oracle.yml" -f "$HERE/oracle-assets.yml" $EXTRA --env-file "$ORACLE_DIR/.env.oracle" "$@"
}

dc down -v --remove-orphans
dc up -d --wait --wait-timeout 300

# Stop the Celery workers. Creating a workspace on the pin dispatches `workspace_seed`, which adds a bot user and a sample
# project with states, labels, cycles, modules, issues and views. Thrust does not reproduce that seed in M1 (deviation
# D-024, to be ported after M1), so the fixtures are recorded with the worker stopped and the seed never runs: the message
# sits in the broker, unconsumed, and the recorded workspace holds exactly what the request created.
dc stop worker beat-worker

# `--wait` returns when the containers are healthy; the API answers a little later.
n=0
until curl -fsS -o /dev/null -m 5 "$BASE/api/instances/"; do
  n=$((n + 1))
  if [ "$n" -gt 60 ]; then
    echo "the API did not answer at $BASE after 5 minutes" >&2
    exit 1
  fi
  sleep 5
done
echo "oracle is up at $BASE"
