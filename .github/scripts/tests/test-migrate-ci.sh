#!/bin/sh
# Exercise scripts/migrate-ci.sh (issue #49) with a fake `cloudflared` and a fake `migrate`, including the cases that must FAIL.
# Every case goes through `expect`, which checks the exit status AND a message fragment, so a case cannot pass for the wrong reason.
#
# What this can and cannot show. The fakes stand in for Cloudflare and for the database, so it shows that the script starts the tunnel
# with the token in the environment and not in argv, waits for it, runs `migrate` with verified TLS asked for and without the token,
# stops the tunnel whatever happens, never runs `migrate` when the tunnel is not up, and says nothing secret. It cannot show that a
# real Access application accepts the token: that is the live run against staging.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
# MIGRATE_CI_SCRIPT points the test at a copy of the script, so that a broken copy can be shown to fail it without touching the real one.
SCRIPT="${MIGRATE_CI_SCRIPT:-$HERE/scripts/migrate-ci.sh}"
LISTEN="$HERE/.github/scripts/tests/listen_stub.py"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
pass=0; failn=0
expect() { # expect <label> <want-exit> <want-fragment> <cmd...>
  label="$1"; want="$2"; frag="$3"; shift 3
  "$@" > "$T/last.out" 2>&1
  got=$?
  if [ "$got" = "$want" ] && grep -qF -- "$frag" "$T/last.out"; then
    pass=$((pass + 1)); echo "ok   $label (exit $got; saw '$frag')"
  else
    failn=$((failn + 1)); echo "BAD  $label: wanted exit $want + '$frag', got $got"; sed 's/^/     | /' "$T/last.out"
  fi
}

if command -v uv > /dev/null 2>&1; then
  PYRUN="uv run --no-project --quiet python -I"
elif command -v python3 > /dev/null 2>&1; then
  PYRUN="python3 -I"
else
  echo "BAD  neither uv nor python3 is installed: the stand-in tunnel cannot run"; exit 1
fi
command -v nc > /dev/null 2>&1 || { echo "BAD  nc is not installed: the script cannot probe the tunnel"; exit 1; }

FAKE_DIR="$T/fake"
mkdir -p "$T/bin"
printf 'not a real CA, the script only checks that the file is there\n' > "$T/ca.pem"

# A free port: one that had a listener a moment ago and has none now.
PORT="$($PYRUN -c 'import socket; s = socket.socket(); s.bind(("127.0.0.1", 0)); print(s.getsockname()[1])')"

# The fake tunnel. It records what it was given (never the values of the secrets), then does what FAKE_CLOUDFLARED says.
cat > "$T/bin/cloudflared" <<'EOF'
#!/bin/sh
printf 'argv: %s\n' "$*" >> "$FAKE_DIR/cloudflared.calls"
[ -n "${TUNNEL_SERVICE_TOKEN_ID:-}" ] && echo "token-id-in-env" >> "$FAKE_DIR/cloudflared.calls"
[ -n "${TUNNEL_SERVICE_TOKEN_SECRET:-}" ] && echo "token-secret-in-env" >> "$FAKE_DIR/cloudflared.calls"
[ -n "${DATABASE_URL:-}" ] && echo "LEAK: DATABASE_URL is in cloudflared's environment" >> "$FAKE_DIR/cloudflared.calls"
echo $$ > "$FAKE_DIR/cloudflared.pid"
case "${FAKE_CLOUDFLARED:-listen}" in
  die)    echo "failed to authenticate: access denied by policy" >&2; exit 1 ;;
  silent) exec sleep 60 ;;
  listen)
    url=""
    while [ $# -gt 0 ]; do [ "$1" = "--url" ] && url="$2"; shift; done
    echo "INF Start Websocket listener host=$url" >&2
    exec $PYRUN "$LISTEN_STUB" "${url##*:}" ;;
esac
EOF
# The fake `migrate`. It records what it was given and exits as FAKE_MIGRATE_EXIT says.
cat > "$T/bin/migrate" <<'EOF'
#!/bin/sh
echo ran > "$FAKE_DIR/migrate.ran"
echo "sslmode=${DATABASE_SSLMODE:-unset}" >> "$FAKE_DIR/migrate.env"
echo "rootcert=${DATABASE_SSLROOTCERT:-unset}" >> "$FAKE_DIR/migrate.env"
[ -n "${TUNNEL_SERVICE_TOKEN_ID:-}${TUNNEL_SERVICE_TOKEN_SECRET:-}" ] && echo "LEAK: the service token is in migrate's environment" >> "$FAKE_DIR/migrate.env"
exit "${FAKE_MIGRATE_EXIT:-0}"
EOF
chmod +x "$T/bin/cloudflared" "$T/bin/migrate"

SECRET_TOKEN="tok-secret-SHOULD-NOT-APPEAR"
SECRET_PW="pw-SHOULD-NOT-APPEAR"

# The good environment; a case then changes it with an `eval`-ed snippet. Fast waits: a case that is going to fail must fail fast.
run_with() { # run_with '<shell snippet that changes the environment>'
  rm -rf "$FAKE_DIR"; mkdir -p "$FAKE_DIR"
  (
    PATH="$T/bin:$PATH"; export PATH
    export FAKE_DIR PYRUN LISTEN_STUB="$LISTEN"
    export DB_ACCESS_HOSTNAME=thrust-pg-access.example.test
    export TUNNEL_SERVICE_TOKEN_ID=tok-id-xyz.access TUNNEL_SERVICE_TOKEN_SECRET="$SECRET_TOKEN"
    export DATABASE_URL="postgres://migrator:$SECRET_PW@127.0.0.1:$PORT/thrust"
    export DATABASE_SSLROOTCERT="$T/ca.pem" MIGRATE_BIN="$T/bin/migrate" MIGRATE_LOCAL_PORT="$PORT"
    export MIGRATE_WAIT_ATTEMPTS=100 MIGRATE_WAIT_INTERVAL=0.1
    eval "$1"
    "$SCRIPT"
  )
}

# ---- it works ----------------------------------------------------------------------------------------------------------------
expect "pass: the tunnel comes up, migrate runs and the script exits 0" 0 "the tunnel's local end is up on 127.0.0.1:$PORT" run_with ':'

check_pass_run() { # what the pass run did, read from the fakes' own records
  run_with ':' > /dev/null 2>&1 || { echo "the passing run did not pass"; return 1; }
  grep -qF "argv: access tcp --hostname thrust-pg-access.example.test --url 127.0.0.1:$PORT" "$FAKE_DIR/cloudflared.calls" \
    || { echo "cloudflared was not started as 'access tcp --hostname <host> --url 127.0.0.1:<port>'"; cat "$FAKE_DIR/cloudflared.calls"; return 1; }
  grep -qx "token-id-in-env" "$FAKE_DIR/cloudflared.calls" && grep -qx "token-secret-in-env" "$FAKE_DIR/cloudflared.calls" \
    || { echo "cloudflared did not get the service token through its environment"; return 1; }
  grep -qx "sslmode=verify-ca" "$FAKE_DIR/migrate.env" || { echo "migrate was not told verify-ca"; cat "$FAKE_DIR/migrate.env"; return 1; }
  grep -qx "rootcert=$T/ca.pem" "$FAKE_DIR/migrate.env" || { echo "migrate was not given the root certificate"; return 1; }
  if grep -q "LEAK" "$FAKE_DIR/cloudflared.calls" "$FAKE_DIR/migrate.env"; then echo "a secret went to a process that does not need it"; return 1; fi
  echo "pass-run ok: argv, environment and verify-ca are as designed"
}
expect "pass: cloudflared gets the token in its environment and none on argv, migrate gets verify-ca and no token" 0 "pass-run ok" check_pass_run

tunnel_is_stopped() { # the PID the fake tunnel wrote, after the script has ended
  pid="$(cat "$FAKE_DIR/cloudflared.pid" 2> /dev/null)"
  [ -n "$pid" ] || { echo "the fake tunnel never ran: this check looked at nothing"; return 1; }
  if kill -0 "$pid" 2> /dev/null; then echo "the tunnel (pid $pid) is still running"; kill "$pid" 2> /dev/null; return 1; fi
  echo "tunnel-stopped ok"
}
stopped_after() { # stopped_after '<snippet>': run the script, ignore how it went, and look at the tunnel it started
  run_with "$1" > /dev/null 2>&1
  tunnel_is_stopped
}
expect "pass: the tunnel is stopped when the script ends" 0 "tunnel-stopped ok" stopped_after ':'

migrate_did_not_run() {
  [ -f "$FAKE_DIR/migrate.ran" ] && { echo "migrate RAN"; return 1; }
  echo "migrate-not-run ok"
}
ran_and_migrate_did_not() { # run the case, then say whether migrate ran
  run_with "$1" > /dev/null 2>&1
  migrate_did_not_run
}

# ---- the tunnel does not come up: migrate must NOT run ---------------------------------------------------------------------
# A wrong or expired token: cloudflared goes away. The control is that the database is never touched.
expect "tunnel: cloudflared that exits (a refused token) fails the script and says so" 1 "cloudflared exited before the tunnel was up" run_with 'export FAKE_CLOUDFLARED=die'
expect "tunnel: and it shows what cloudflared said" 1 "access denied by policy" run_with 'export FAKE_CLOUDFLARED=die'
expect "tunnel: and migrate did not run" 0 "migrate-not-run ok" ran_and_migrate_did_not 'export FAKE_CLOUDFLARED=die'
expect "tunnel: a tunnel that never listens fails the script and says how many tries" 1 "did not accept a connection in 3 attempts" \
  run_with 'export FAKE_CLOUDFLARED=silent MIGRATE_WAIT_ATTEMPTS=3 MIGRATE_WAIT_INTERVAL=0'
expect "tunnel: and migrate did not run" 0 "migrate-not-run ok" ran_and_migrate_did_not 'export FAKE_CLOUDFLARED=silent MIGRATE_WAIT_ATTEMPTS=3 MIGRATE_WAIT_INTERVAL=0'
expect "tunnel: and the tunnel that never listened is stopped" 0 "tunnel-stopped ok" stopped_after 'export FAKE_CLOUDFLARED=silent MIGRATE_WAIT_ATTEMPTS=3 MIGRATE_WAIT_INTERVAL=0'

# ---- migrate fails: its status is the script's, and the tunnel still stops ---------------------------------------------------
expect "migrate: a failing migrate fails the script with ITS status" 7 "migrate exited 7" run_with 'export FAKE_MIGRATE_EXIT=7'
expect "migrate: and the tail of cloudflared's log is shown, where a token the edge refused would be" 7 "cloudflared:" run_with 'export FAKE_MIGRATE_EXIT=7 FAKE_CLOUDFLARED=listen'
expect "migrate: and the tunnel is stopped" 0 "tunnel-stopped ok" stopped_after 'export FAKE_MIGRATE_EXIT=7'

# ---- called wrongly: exit 2, and nothing was started --------------------------------------------------------------------------
nothing_started() { # after a refused call: neither the tunnel nor migrate ran
  [ -f "$FAKE_DIR/cloudflared.calls" ] && { echo "cloudflared was started"; return 1; }
  migrate_did_not_run
}
refused() { # refused '<snippet>' '<fragment>': exit 2 with the fragment, and nothing started
  run_with "$1" > "$T/refused.out" 2>&1
  rc=$?
  [ "$rc" = 2 ] || { echo "wanted exit 2, got $rc"; cat "$T/refused.out"; return 1; }
  grep -qF -- "$2" "$T/refused.out" || { echo "wanted '$2' in:"; cat "$T/refused.out"; return 1; }
  nothing_started > /dev/null || { echo "something was started"; return 1; }
  echo "refused ok: $2"
}
expect "config: no access hostname" 0 "refused ok: DB_ACCESS_HOSTNAME is not set" refused 'unset DB_ACCESS_HOSTNAME' 'DB_ACCESS_HOSTNAME is not set'
expect "config: no token id" 0 "refused ok: TUNNEL_SERVICE_TOKEN_ID is not set" refused 'unset TUNNEL_SERVICE_TOKEN_ID' 'TUNNEL_SERVICE_TOKEN_ID is not set'
expect "config: no token secret" 0 "refused ok: TUNNEL_SERVICE_TOKEN_SECRET is not set" refused 'unset TUNNEL_SERVICE_TOKEN_SECRET' 'TUNNEL_SERVICE_TOKEN_SECRET is not set'
expect "config: no database URL" 0 "refused ok: DATABASE_URL is not set" refused 'unset DATABASE_URL' 'DATABASE_URL is not set'
expect "config: no root certificate" 0 "refused ok: DATABASE_SSLROOTCERT is not set" refused 'unset DATABASE_SSLROOTCERT' 'DATABASE_SSLROOTCERT is not set'
expect "config: no migrate binary" 0 "refused ok: MIGRATE_BIN is not set" refused 'unset MIGRATE_BIN' 'MIGRATE_BIN is not set'
expect "config: a migrate binary that is not executable" 0 "refused ok: MIGRATE_BIN is not an executable file" refused 'export MIGRATE_BIN="$T/ca.pem"' 'MIGRATE_BIN is not an executable file'
expect "config: a root certificate that is not a file" 0 "refused ok: DATABASE_SSLROOTCERT is not a file" refused 'export DATABASE_SSLROOTCERT="$T/nope.pem"' 'DATABASE_SSLROOTCERT is not a file'
# The URL must end at the tunnel. Any other host would run the migration without it, with the password sent somewhere else.
expect "config: a URL for another host is refused, not run without the tunnel" 0 "refused ok: DATABASE_URL must point at 127.0.0.1:$PORT" \
  refused 'export DATABASE_URL="postgres://u:p@db.example.test:5432/thrust"' "DATABASE_URL must point at 127.0.0.1:$PORT"
expect "config: a URL for the right host and another port is refused" 0 "refused ok: DATABASE_URL must point at 127.0.0.1:$PORT" \
  refused 'export DATABASE_URL="postgres://u:p@127.0.0.1:5432/thrust"' "DATABASE_URL must point at 127.0.0.1:$PORT"
expect "config: sslmode disable is refused: this route is TLS only" 0 "refused ok: DATABASE_SSLMODE is set and is not verify-ca" \
  refused 'export DATABASE_SSLMODE=disable' 'DATABASE_SSLMODE is set and is not verify-ca'
expect "config: sslmode require is refused" 0 "refused ok: DATABASE_SSLMODE is set and is not verify-ca" \
  refused 'export DATABASE_SSLMODE=require' 'DATABASE_SSLMODE is set and is not verify-ca'
expect "config: a port that is not a number" 0 "refused ok: MIGRATE_LOCAL_PORT must be a port number" \
  refused 'export MIGRATE_LOCAL_PORT=abc' 'MIGRATE_LOCAL_PORT must be a port number'
expect "config: sslmode verify-ca said out loud is fine" 0 "the tunnel's local end is up" run_with 'export DATABASE_SSLMODE=verify-ca'

# ---- the secrets are in no output and no argv ---------------------------------------------------------------------------------
no_secret_anywhere() { # the token secret and the password, over a run that passes and runs that fail three different ways
  : > "$T/all.out"
  for snippet in ':' 'export FAKE_CLOUDFLARED=die' 'export FAKE_MIGRATE_EXIT=7' 'export DATABASE_SSLMODE=disable'; do
    run_with "$snippet" >> "$T/all.out" 2>&1
    cat "$FAKE_DIR/cloudflared.calls" >> "$T/all.out" 2> /dev/null
  done
  [ -s "$T/all.out" ] || { echo "no output at all: this check looked at nothing"; return 1; }
  # not vacuous: the passing run really ran and said so, and a failing one really failed and said so
  grep -qF "the tunnel's local end is up" "$T/all.out" || { echo "the passing run did not run: the output has nothing of it"; return 1; }
  grep -qF "cloudflared exited before the tunnel was up" "$T/all.out" || { echo "the failing run did not fail the way it should"; return 1; }
  if grep -qF -e "$SECRET_TOKEN" -e "$SECRET_PW" "$T/all.out"; then echo "a secret is in the output"; return 1; fi
  echo "no-secret ok: over a pass and three failures, neither the token nor the password appears"
}
expect "secrets: neither the token secret nor the password is in any output or in cloudflared's argv" 0 "no-secret ok" no_secret_anywhere

# ---- hygiene -----------------------------------------------------------------------------------------------------------------
no_listener_left() {
  if ps -eo command | grep -F "listen_stub.py $PORT" | grep -vF grep > /dev/null; then echo "a stand-in tunnel is still listening"; return 1; fi
  echo "no stand-in tunnel is left running"
}
expect "hygiene: no stand-in tunnel is left running" 0 "no stand-in tunnel is left running" no_listener_left

echo "passed=$pass bad=$failn"
[ "$failn" -eq 0 ]
