#!/bin/sh
# The deploy's expand migration, over the Access tunnel (issue #49, ADR-0110 round 29): open a TCP tunnel to `thrust-pg` with
# `cloudflared access tcp`, run `migrate` through it over verified TLS, close the tunnel.
#
#   scripts/migrate-ci.sh
#
# Why this shape. `thrust-pg` has no public endpoint and accepts only TLS, so CI reaches it as a client of an Access application with a
# service token (Q-0094, ADR-0094 Option 2). `cloudflared access tcp` listens on 127.0.0.1 and carries the bytes, so the database sees
# a TLS client and the client sees a certificate that names a host and not 127.0.0.1: `migrate` runs under `verify-ca`, which checks
# the chain and the dates and not the name (crates/thrust-adapters-postgres/src/tls.rs).
#
# Environment. Nothing secret goes through argv, and nothing goes to a process that does not need it: `cloudflared` is started
# without the database URL, `migrate` without the service token.
#   DB_ACCESS_HOSTNAME                                   the hostname of the Access application in front of thrust-pg
#   TUNNEL_SERVICE_TOKEN_ID, TUNNEL_SERVICE_TOKEN_SECRET the Access service token (cloudflared reads these names itself)
#   DATABASE_URL           the database URL, host and port 127.0.0.1:$MIGRATE_LOCAL_PORT. Anything else is refused: it would run
#                          the migration somewhere that is not the tunnel
#   DATABASE_SSLROOTCERT   the PEM file of the CA the server's certificate chains to. `DATABASE_SSLMODE` is set to verify-ca here; set
#                          to anything else it is refused, because this route is TLS only
#   MIGRATE_BIN            the built `migrate` binary. It is not built here: the job that holds the service token and the password
#                          runs no build of this repository's code
#   MIGRATE_LOCAL_PORT     the local end of the tunnel (default 15432)
#   MIGRATE_WAIT_ATTEMPTS, MIGRATE_WAIT_INTERVAL   how long to wait for the tunnel's local end to accept connections (default 50 x 0.2 s)
#
# What it cannot know. The local end of the tunnel accepts connections before the edge has accepted the service token, so a wrong or
# expired token is not seen here: it is seen as `migrate` failing to connect, and the tail of cloudflared's log is printed then.
#
# Exit status: `migrate`'s own when it ran; 1 when the tunnel did not come up; 2 when it was called wrongly (nothing was started).
set -u

fail() { echo "migrate-ci: FAIL: $*" >&2; exit 1; }
usage() { echo "migrate-ci: $*" >&2; exit 2; }

for v in DB_ACCESS_HOSTNAME TUNNEL_SERVICE_TOKEN_ID TUNNEL_SERVICE_TOKEN_SECRET DATABASE_URL DATABASE_SSLROOTCERT MIGRATE_BIN; do
  eval "val=\${$v:-}"
  [ -n "$val" ] || usage "$v is not set"
done
PORT="${MIGRATE_LOCAL_PORT:-15432}"
case "$PORT" in ''|*[!0-9]*) usage "MIGRATE_LOCAL_PORT must be a port number" ;; esac
ATTEMPTS="${MIGRATE_WAIT_ATTEMPTS:-50}"
INTERVAL="${MIGRATE_WAIT_INTERVAL:-0.2}"
case "$ATTEMPTS" in ''|*[!0-9]*|0) usage "MIGRATE_WAIT_ATTEMPTS must be a positive whole number" ;; esac

case "${DATABASE_SSLMODE:-verify-ca}" in
  verify-ca) ;;
  *) usage "DATABASE_SSLMODE is set and is not verify-ca: this route is TLS only, with the server's CA verified" ;;
esac
# the URL is never printed: it holds the password
case "$DATABASE_URL" in
  *"@127.0.0.1:$PORT/"*|*"@127.0.0.1:$PORT?"*|*"@127.0.0.1:$PORT") ;;
  *) usage "DATABASE_URL must point at 127.0.0.1:$PORT, the local end of the tunnel: any other host would run the migration without it" ;;
esac
[ -f "$DATABASE_SSLROOTCERT" ] || usage "DATABASE_SSLROOTCERT is not a file"
[ -x "$MIGRATE_BIN" ] || usage "MIGRATE_BIN is not an executable file"
command -v cloudflared > /dev/null 2>&1 || usage "cloudflared is not installed"
command -v nc > /dev/null 2>&1 || usage "nc is not installed (it is how the tunnel's local end is probed)"

T="$(mktemp -d)"
TUNNEL_PID=""
cleanup() {
  if [ -n "$TUNNEL_PID" ]; then
    kill "$TUNNEL_PID" 2> /dev/null
    wait "$TUNNEL_PID" 2> /dev/null
  fi
  rm -rf "$T"
}
trap cleanup EXIT INT TERM

export TUNNEL_SERVICE_TOKEN_ID TUNNEL_SERVICE_TOKEN_SECRET
# The hostname and the local address are not secret and go on argv; the token is read from the environment by cloudflared itself.
env -u DATABASE_URL -u DATABASE_SSLROOTCERT -u DATABASE_SSLMODE \
  cloudflared access tcp --hostname "$DB_ACCESS_HOSTNAME" --url "127.0.0.1:$PORT" > "$T/cloudflared.log" 2>&1 &
TUNNEL_PID=$!

tail_log() { sed 's/^/    cloudflared: /' "$T/cloudflared.log" | tail -15 >&2; }

n=0
until nc -z 127.0.0.1 "$PORT" > /dev/null 2>&1; do
  n=$((n + 1))
  if ! kill -0 "$TUNNEL_PID" 2> /dev/null; then
    tail_log
    fail "cloudflared exited before the tunnel was up (a wrong or expired service token, or a hostname that is not an Access application, looks like this)"
  fi
  if [ "$n" -ge "$ATTEMPTS" ]; then
    tail_log
    fail "the local end of the tunnel, 127.0.0.1:$PORT, did not accept a connection in $ATTEMPTS attempts"
  fi
  sleep "$INTERVAL"
done
echo "migrate-ci: the tunnel's local end is up on 127.0.0.1:$PORT"

# DATABASE_SSLMODE is set for `migrate` and not left to the caller. The service token is not passed on to it.
DATABASE_SSLMODE=verify-ca env -u TUNNEL_SERVICE_TOKEN_ID -u TUNNEL_SERVICE_TOKEN_SECRET "$MIGRATE_BIN"
rc=$?
if [ "$rc" -ne 0 ]; then
  echo "migrate-ci: migrate exited $rc; the end of cloudflared's log, which says why the connection failed if it did:" >&2
  tail_log
fi
exit "$rc"
