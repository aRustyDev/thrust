#!/usr/bin/env bash
# Non-hermetic security and lifecycle test; starts a disposable PostgreSQL container.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
container="thrust-provisioner-test-$$"
cleanup() { docker rm -f "$container" >/dev/null 2>&1 || true; }
trap cleanup EXIT
docker run -d --name "$container" -e POSTGRES_PASSWORD=test-admin -e POSTGRES_DB=thrust -e POSTGRES_INITDB_ARGS=--auth-host=scram-sha-256 postgres:16-alpine >/dev/null
for _ in {1..60}; do
  # The image's initialization server listens only on the Unix socket and then shuts down.
  if docker exec "$container" pg_isready -h 127.0.0.1 -U postgres -d thrust >/dev/null 2>&1; then break; fi
  sleep 1
done
admin() { docker exec -i "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d thrust "$@"; }
admin -c 'REVOKE CREATE ON DATABASE thrust FROM PUBLIC' >/dev/null
source "$here/../pr-role-limits.sh"
docker exec -i -e THRUST_PREVIEW_PROVISIONER_PASSWORD=test-provisioner-password-123456 -e THRUST_PR_ROLE_CONNECTION_LIMIT="$THRUST_PR_ROLE_CONNECTION_LIMIT" "$container" psql -X -v ON_ERROR_STOP=1 -U postgres -d thrust <"$here/../provisioner-bootstrap.sql" >/dev/null
admin -v expected_limit="$THRUST_PR_ROLE_CONNECTION_LIMIT" <<'SQL'
SELECT set_config('test.expected_limit', :'expected_limit', false);
SET ROLE thrust_preview_provisioner;
DO $$ BEGIN
  BEGIN CREATE ROLE forbidden LOGIN; RAISE EXCEPTION 'CREATE ROLE unexpectedly succeeded'; EXCEPTION WHEN insufficient_privilege THEN NULL; END;
  BEGIN CREATE SCHEMA forbidden; RAISE EXCEPTION 'CREATE SCHEMA unexpectedly succeeded'; EXCEPTION WHEN insufficient_privilege THEN NULL; END;
END $$;
SELECT preview_admin.provision(42, '0123456789abcdef0123456789abcdef');
SELECT preview_admin.provision(42, 'fedcba9876543210fedcba9876543210');
DO $$ BEGIN
  IF NOT EXISTS (SELECT 1 FROM preview_admin.inventory() WHERE number=42 AND schema_exists AND login_role AND search_path) THEN RAISE EXCEPTION 'inventory missing'; END IF;
  IF NOT EXISTS (SELECT 1 FROM preview_admin.inventory() WHERE number=42 AND pending) THEN RAISE EXCEPTION 'pending marker missing'; END IF;
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname='pr_42' AND rolconnlimit=current_setting('test.expected_limit')::integer AND NOT (rolsuper OR rolcreatedb OR rolcreaterole OR rolreplication OR rolbypassrls)) THEN RAISE EXCEPTION 'unsafe role flags'; END IF;
  BEGIN INSERT INTO preview_admin.managed VALUES (100, 1, 1); RAISE EXCEPTION 'registry writable'; EXCEPTION WHEN insufficient_privilege THEN NULL; END;
END $$;
SELECT preview_admin.acknowledge(42);
DO $$ BEGIN IF EXISTS (SELECT 1 FROM preview_admin.inventory() WHERE number=42 AND pending) THEN RAISE EXCEPTION 'acknowledgement failed'; END IF; END $$;
SELECT preview_admin.deprovision(42);
SELECT preview_admin.deprovision(42);
-- Generated sequence exercises multiple independent resources and repeated deletion.
SELECT preview_admin.provision(n, '0123456789abcdef0123456789abcdef') FROM generate_series(1, 20) AS n;
DO $$ BEGIN IF (SELECT count(*) FROM preview_admin.inventory()) <> 20 THEN RAISE EXCEPTION 'inventory count'; END IF; END $$;
SELECT preview_admin.deprovision(n) FROM generate_series(1, 20) AS n WHERE n % 2 = 0;
DO $$ BEGIN IF (SELECT count(*) FROM preview_admin.inventory()) <> 10 OR EXISTS (SELECT 1 FROM preview_admin.inventory() WHERE number % 2 = 0) THEN RAISE EXCEPTION 'inventory model mismatch'; END IF; END $$;
SELECT preview_admin.deprovision(n) FROM generate_series(1, 20) AS n;
RESET ROLE;
CREATE ROLE pr_123 SUPERUSER;
SET ROLE thrust_preview_provisioner;
DO $$ BEGIN
  BEGIN PERFORM preview_admin.provision(123, '0123456789abcdef0123456789abcdef'); RAISE EXCEPTION 'collision accepted'; EXCEPTION WHEN object_not_in_prerequisite_state THEN NULL; END;
  BEGIN PERFORM preview_admin.deprovision(123); RAISE EXCEPTION 'collision deleted'; EXCEPTION WHEN object_not_in_prerequisite_state THEN NULL; END;
  BEGIN PERFORM preview_admin.inventory(); RAISE EXCEPTION 'unmanaged role omitted from inventory'; EXCEPTION WHEN object_not_in_prerequisite_state THEN NULL; END;
END $$;
RESET ROLE;
DROP ROLE pr_123;
SQL

# Exercise actual TCP authentication, not just role catalog values.
admin -c "SELECT preview_admin.provision(43, '0123456789abcdef0123456789abcdef')" >/dev/null
login() {
  docker exec -e PGPASSWORD="$1" "$container" psql -X -h 127.0.0.1 -U pr_43 -d thrust -tAc 'SELECT current_user' >/dev/null 2>&1
}
login '0123456789abcdef0123456789abcdef' || { echo 'initial login failed' >&2; exit 1; }
admin -c "SELECT preview_admin.provision(43, 'fedcba9876543210fedcba9876543210')" >/dev/null
if login '0123456789abcdef0123456789abcdef'; then echo 'old password still accepted' >&2; exit 1; fi
login 'fedcba9876543210fedcba9876543210' || { echo 'rotated login failed' >&2; exit 1; }

# Hold a real pr_43 backend, then prove deprovision terminates it before dropping the role.
docker exec -e PGPASSWORD=fedcba9876543210fedcba9876543210 -e PGAPPNAME=thrust-session-control \
  "$container" psql -X -h 127.0.0.1 -U pr_43 -d thrust -c 'SELECT pg_sleep(60)' >/dev/null 2>&1 &
session_process=$!
found=0
for _ in {1..100}; do
  if [[ "$(admin -tAc "SELECT count(*) FROM pg_stat_activity WHERE usename='pr_43' AND application_name='thrust-session-control'")" == 1 ]]; then found=1; break; fi
  sleep 0.1
done
[[ "$found" == 1 ]] || { echo 'session control did not connect' >&2; exit 1; }
admin -c 'SELECT preview_admin.deprovision(43)' >/dev/null
if wait "$session_process"; then echo 'active session was not terminated' >&2; exit 1; fi
[[ "$(admin -tAc "SELECT count(*) FROM pg_roles WHERE rolname='pr_43'")" == 0 ]] || { echo 'role survived deprovision' >&2; exit 1; }
echo 'provisioner bootstrap tests: PASS'
