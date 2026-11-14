#!/bin/sh
# The deploy's smoke (issue #49, ADR-0110 round 29): is the version that was just deployed the one that answers, and does it work?
#
#   scripts/smoke-deploy.sh <base-url> <expected-version>
#
# <expected-version> is what the deploy tagged the version with (`wrangler deploy --tag <git sha>`). The Worker sends its own tag back
# as `x-thrust-version` (crates/thrust-api, `set_version`), so a Worker that still serves the version it replaced FAILS this smoke
# instead of passing it: that is the guard against a vacuous pass, and it is why the header has to be there, not merely right when
# present. It needs no test route, no admin token and no seed route, none of which a real deployment has.
#
# Phase 1, the version. `/healthz` must answer 200 `ok` with `x-thrust-version` equal to <expected-version>, on SMOKE_CONSECUTIVE
# answers in a row (a deploy reaches the edge over a few seconds, and one answer from a half-rolled-out state is not evidence).
# Phase 2, a real read. Sign in as the dedicated smoke account, check `/api/users/me/`, and list the smoke workspace's projects: the
# project that the one-time bootstrap made (scripts/smoke-bootstrap.sh) must be in the list, so the read went through the database.
#
# Environment (nothing secret is ever taken from argv, and the password is never printed):
#   SMOKE_EMAIL, SMOKE_PASSWORD, SMOKE_WORKSPACE   the smoke account, and the slug of its workspace. Required, so a deploy cannot
#                                                  go green by skipping the phase that reads data; SMOKE_AUTH=off skips it, loudly.
#   SMOKE_PROJECT       the project that must be listed (default: smoke)
#   SMOKE_AUTH=off      skip phase 2. Say so in the job's name or you will not know later that nothing was read
#   SMOKE_ATTEMPTS      how many times phase 1 asks (default 30)
#   SMOKE_INTERVAL      seconds to wait after an answer that is not the new version (default 5)
#   SMOKE_CONSECUTIVE   answers in a row that must carry the version (default 3)
#   SMOKE_STREAK_INTERVAL  seconds between two answers that did (default 1)
#   SMOKE_FORCE_FAIL=1  after every real check has passed, fail on purpose. The deploy's rollback control (a failing smoke must
#                       end with the previous version active) runs on staging with it set; it is refused for prod by the workflow.
#
# Exit status: 0 the smoke passed; 1 a check failed; 2 it was called wrongly (a usage or configuration error, nothing was checked).
set -u

fail() { echo "smoke: FAIL: $*" >&2; exit 1; }
usage() { echo "smoke: $*" >&2; exit 2; }

BASE="${1:-}"
EXPECT="${2:-}"
[ -n "$BASE" ] && [ -n "$EXPECT" ] || usage "usage: smoke-deploy.sh <base-url> <expected-version>"
case "$BASE" in http://*|https://*) ;; *) usage "the base URL must start with http:// or https://, got '$BASE'" ;; esac
BASE="${BASE%/}"
# what a tag or an id can be: visible ASCII, no space (the same rule the Worker applies before it sends the header)
case "$EXPECT" in *[!0-9A-Za-z._:/@+=-]*) usage "the expected version has a character a version tag does not: '$EXPECT'" ;; esac

ATTEMPTS="${SMOKE_ATTEMPTS:-30}"
INTERVAL="${SMOKE_INTERVAL:-5}"
NEED="${SMOKE_CONSECUTIVE:-3}"
STREAK_INTERVAL="${SMOKE_STREAK_INTERVAL:-1}"
for v in "$ATTEMPTS" "$NEED"; do
  case "$v" in ''|*[!0-9]*|0) usage "SMOKE_ATTEMPTS and SMOKE_CONSECUTIVE must be positive whole numbers" ;; esac
done

AUTH="${SMOKE_AUTH:-on}"
case "$AUTH" in
  on)
    for v in SMOKE_EMAIL SMOKE_PASSWORD SMOKE_WORKSPACE; do
      eval "val=\${$v:-}"
      [ -n "$val" ] || usage "$v is not set. Set it, or set SMOKE_AUTH=off to skip the phase that reads data."
    done
    ;;
  off) ;;
  *) usage "SMOKE_AUTH is 'on' or 'off', got '$AUTH'" ;;
esac
PROJECT="${SMOKE_PROJECT:-smoke}"
case "${SMOKE_WORKSPACE:-x}" in *[!0-9A-Za-z_-]*) usage "SMOKE_WORKSPACE is a slug (letters, digits, - and _)" ;; esac
case "$PROJECT" in *[\"\\]*) usage "SMOKE_PROJECT must not contain a quote or a backslash" ;; esac

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM

# One request. Prints the status code (000 when there was no answer); the headers go to $T/h, the body to $T/b, the transport
# error to $T/e. Never follows a redirect: a 3xx where a 200 is expected is a failure, not something to chase.
req() { # req <curl args...> <url>
  curl -sS --connect-timeout 5 --max-time 20 -D "$T/h" -o "$T/b" -w '%{http_code}' "$@" 2>"$T/e" || true
}
header() { # header <name>: the last value of that response header, without the CR and the padding
  tr -d '\r' < "$T/h" 2>/dev/null | grep -i "^$1:" | tail -1 | sed 's/^[^:]*:[[:space:]]*//; s/[[:space:]]*$//'
}
squash() { tr -d ' \t\r\n' < "$1"; } # a JSON body without its whitespace, whatever the server's pretty-printing
json_escape() { printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'; }

# ---- phase 1: the version --------------------------------------------------------------------------------------------------
n=0
streak=0
last="no attempt was made"
while [ "$n" -lt "$ATTEMPTS" ]; do
  n=$((n + 1))
  status="$(req -H 'Cache-Control: no-cache' "$BASE/healthz?smoke=$n")"
  got="$(header x-thrust-version)"
  body="$(cat "$T/b" 2>/dev/null)"
  if [ "$status" = 000 ]; then
    last="could not reach $BASE/healthz: $(tr -d '\r\n' < "$T/e")"
    streak=0
  elif [ "$status" != 200 ]; then
    last="/healthz answered $status, not 200"
    streak=0
  elif [ "$body" != ok ]; then
    last="/healthz answered 200 but its body is not 'ok'"
    streak=0
  elif [ -z "$got" ]; then
    last="/healthz has no x-thrust-version header, so it cannot be told from the version it replaced"
    streak=0
  elif [ "$got" != "$EXPECT" ]; then
    last="the Worker serves $got, expected $EXPECT"
    streak=0
  else
    streak=$((streak + 1))
    last="ok"
  fi
  echo "version: attempt $n/$ATTEMPTS: $last (streak $streak/$NEED)"
  [ "$streak" -ge "$NEED" ] && break
  if [ "$n" -lt "$ATTEMPTS" ]; then
    if [ "$last" = ok ]; then sleep "$STREAK_INTERVAL"; else sleep "$INTERVAL"; fi
  fi
done
[ "$streak" -ge "$NEED" ] || fail "never saw $NEED consecutive answers carrying $EXPECT in $ATTEMPTS attempts (last: $last)"
echo "version: PASS ($NEED consecutive answers carried $EXPECT)"

# ---- phase 2: a real read --------------------------------------------------------------------------------------------------
if [ "$AUTH" = off ]; then
  echo "auth: SKIPPED (SMOKE_AUTH=off): nothing was read from the database"
else
  JAR="$T/jar"
  # The CSRF token goes with the sign-in whether this Worker enforces it or not: enforcement is a per-environment setting.
  status="$(req -c "$JAR" -b "$JAR" "$BASE/auth/get-csrf-token/")"
  [ "$status" = 200 ] || fail "auth: /auth/get-csrf-token/ answered $status, not 200"
  token="$(squash "$T/b" | sed -n 's/.*"csrf_token":"\([^"]*\)".*/\1/p')"
  [ -n "$token" ] || fail "auth: /auth/get-csrf-token/ answered 200 with no csrf_token in it"

  # The password reaches curl on stdin: printf is a builtin, so it is in no process's argv.
  status="$(printf '{"email":"%s","password":"%s"}' "$(json_escape "$SMOKE_EMAIL")" "$(json_escape "$SMOKE_PASSWORD")" |
    req -c "$JAR" -b "$JAR" -X POST -H 'Content-Type: application/json' -H "X-CSRFToken: $token" --data-binary @- "$BASE/auth/sign-in/")"
  if [ "$status" != 200 ]; then
    code="$(squash "$T/b" | sed -n 's/.*"error_code":\([0-9]*\).*/\1/p')"
    fail "auth: sign-in as the smoke account answered $status${code:+ (error_code $code)}, not 200"
  fi

  status="$(req -c "$JAR" -b "$JAR" "$BASE/api/users/me/")"
  [ "$status" = 200 ] || fail "auth: /api/users/me/ answered $status after a sign-in that answered 200: the session was not kept"
  # case-insensitive: the server stores an email normalised, and the account may have been made with another case
  want_email="$(printf '%s' "$SMOKE_EMAIL" | tr -d ' \t\r\n')"
  squash "$T/b" | grep -qiF "\"email\":\"$want_email\"" || fail "auth: /api/users/me/ is not the smoke account"

  status="$(req -c "$JAR" -b "$JAR" "$BASE/api/workspaces/$SMOKE_WORKSPACE/projects/")"
  [ "$status" = 200 ] || fail "auth: the project list of '$SMOKE_WORKSPACE' answered $status, not 200"
  needle="\"name\":\"$(printf '%s' "$PROJECT" | tr -d ' \t')\""
  squash "$T/b" | grep -qF "$needle" || fail "auth: the project list of '$SMOKE_WORKSPACE' does not hold the project '$PROJECT': run scripts/smoke-bootstrap.sh once for this environment"
  echo "auth: PASS (signed in as the smoke account and read the project '$PROJECT')"
fi

if [ "${SMOKE_FORCE_FAIL:-}" = 1 ]; then
  fail "SMOKE_FORCE_FAIL=1: failing on purpose, after every real check passed"
fi
echo "smoke: PASS ($EXPECT at $BASE)"
