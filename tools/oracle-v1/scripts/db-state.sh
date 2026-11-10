#!/usr/bin/env bash
# Snapshot and restore the oracle database, so every conformance run starts from the SAME state.
#
#   oracle/scripts/db-state.sh snapshot    # after the corpus is recorded (pristine seed + scratch cleanup)
#   oracle/scripts/db-state.sh restore     # before the gate and before EACH must-fail control
#
# Why this exists: the gate fuzzes PATCH/POST against seeded entities. It leaves a state whose slug is "",
# duplicate names, and so on. A second run on that database sees different data than the first, so a
# "passthrough control" run after the gate failed on failures the gate itself had CAUSED (measured:
# 9 unaccepted failures in the unmodified passthrough run). A control is only meaningful if it starts from
# the same state as the baseline it is compared with.
#
# restore: stop everything that holds a DB connection, drop and recreate the database, pg_restore the
# snapshot, flush Valkey (caches, rate-limit counters), start only the api. worker and beat stay stopped
# during gate/control runs so no asynchronous task mutates the database underneath the harness.
set -euo pipefail

ORACLE="$(cd "$(dirname "$0")/.." && pwd)"
DC=(docker compose -f "$ORACLE/compose.oracle.yml" --env-file "$ORACLE/.env.oracle" --profile ratelimit)
DUMP=/tmp/oracle-snapshot.dump
PGU=plane
DB=plane
# What the seeded database holds, as labels/states/issues/projects rows (seed.py: 6 labels, 12 states, 11 work items, 2 projects).
# A restore that does not give it is not the state the corpus was recorded from, and a gate run on it fails for that reason.
SEED_COUNTS=6/12/11/2

case "${1:-}" in
  snapshot)
    # Give celery a few seconds to finish what the corpus recording queued (activity rows), then
    # stop it and freeze. Best effort: activity LISTS are only checked by class in the register.
    sleep 8
    "${DC[@]}" stop worker beat-worker
    "${DC[@]}" exec -T plane-db pg_dump -U "$PGU" -Fc -f "$DUMP" "$DB"
    "${DC[@]}" exec -T plane-db sh -c "test -s $DUMP && echo snapshot_bytes=\$(wc -c < $DUMP)"
    ;;
  restore)
    "${DC[@]}" stop api api-ratelimit worker beat-worker
    "${DC[@]}" exec -T plane-db psql -U "$PGU" -d postgres -v ON_ERROR_STOP=1 \
      -c "DROP DATABASE IF EXISTS $DB WITH (FORCE)" -c "CREATE DATABASE $DB OWNER $PGU"
    "${DC[@]}" exec -T plane-db pg_restore -U "$PGU" -d "$DB" --exit-on-error "$DUMP"
    "${DC[@]}" exec -T plane-redis valkey-cli FLUSHALL
    "${DC[@]}" up -d --wait --wait-timeout 180 api
    got="$("${DC[@]}" exec -T plane-db psql -U "$PGU" -d "$DB" -At -c \
      "select (select count(*) from labels)||'/'||(select count(*) from states)||'/'||(select count(*) from issues)||'/'||(select count(*) from projects)")"
    echo "restored: labels/states/issues/projects = $got (the seed is $SEED_COUNTS)"
    [ "$got" = "$SEED_COUNTS" ] || { echo "RESTORE DID NOT GIVE THE SEEDED DATABASE: snapshot a freshly seeded stack" >&2; exit 9; }
    ;;
  *)
    echo "usage: $0 snapshot|restore" >&2
    exit 64
    ;;
esac
