#!/bin/sh
# One-time bootstrap of an environment's smoke account (issue #49). WRITES to the deployment it is pointed at: run it by hand, once per
# environment, never from CI.
#
#   scripts/smoke-bootstrap.sh <base-url>
#
# scripts/smoke-deploy.sh signs in as a dedicated account and reads a project from the database, so that a deploy that is up but cannot
# reach its data fails the smoke. Nothing in a real deployment is seeded (the seed routes need test routes, which a deployment never
# has), so this makes what the smoke reads: the account, a workspace, and a project in it. It uses only the routes any client uses, the
# open sign-up and the workspace and project creates, so it needs no operator route, no admin token and no secret in the Worker.
#
# Environment (the password is read from the environment and is never printed or put in an argument):
#   SMOKE_EMAIL, SMOKE_PASSWORD, SMOKE_WORKSPACE   the account and the slug of the workspace to make. The password must pass the
#                                                  sign-up's strength rule. Make one with `openssl rand -base64 24`, keep it in the
#                                                  password manager, and give the same three to the deploy job's environment.
#   SMOKE_PROJECT                the project to make (default: smoke)
#   SMOKE_PROJECT_IDENTIFIER     its identifier, capital letters and digits (default: SMK)
#
# Safe to run again: an account that exists is signed in to, a workspace and a project that exist are left alone. Exit status: 0 the
# account, the workspace and the project are all there; 1 a step failed; 2 it was called wrongly (nothing was done).
set -u

fail() { echo "bootstrap: FAIL: $*" >&2; exit 1; }
usage() { echo "bootstrap: $*" >&2; exit 2; }

BASE="${1:-}"
[ -n "$BASE" ] || usage "usage: smoke-bootstrap.sh <base-url>"
case "$BASE" in http://*|https://*) ;; *) usage "the base URL must start with http:// or https://, got '$BASE'" ;; esac
BASE="${BASE%/}"
for v in SMOKE_EMAIL SMOKE_PASSWORD SMOKE_WORKSPACE; do
  eval "val=\${$v:-}"
  [ -n "$val" ] || usage "$v is not set"
done
PROJECT="${SMOKE_PROJECT:-smoke}"
IDENT="${SMOKE_PROJECT_IDENTIFIER:-SMK}"
case "$SMOKE_WORKSPACE" in *[!0-9A-Za-z_-]*) usage "SMOKE_WORKSPACE is a slug (letters, digits, - and _)" ;; esac
case "$PROJECT" in *[\"\\]*) usage "SMOKE_PROJECT must not contain a quote or a backslash" ;; esac
case "$IDENT" in ''|*[!0-9A-Z]*) usage "SMOKE_PROJECT_IDENTIFIER is capital letters and digits" ;; esac

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
JAR="$T/jar"

req() { # req <curl args...> <url>: prints the status code (000 when there was no answer); the body is in $T/b
  curl -sS --connect-timeout 5 --max-time 20 -o "$T/b" -w '%{http_code}' "$@" 2>"$T/e" || true
}
squash() { tr -d ' \t\r\n' < "$1"; }
json_escape() { printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'; }
code_of() { squash "$T/b" | sed -n 's/.*"error_code":\([0-9]*\).*/\1/p'; }
# the CSRF token the server last set, from the cookie jar (a mutation by session cookie carries it where the server enforces it)
csrf_cookie() { awk -F'\t' '$6 == "__Host-thrust_csrf" || $6 == "csrftoken" { v = $7 } END { print v }' "$JAR" 2>/dev/null; }
post_json() { # post_json <path> <json on stdin>: with the session and the current CSRF token
  req -c "$JAR" -b "$JAR" -X POST -H 'Content-Type: application/json' -H "X-CSRFToken: $(csrf_cookie)" --data-binary @- "$BASE$1"
}

status="$(req -c "$JAR" -b "$JAR" "$BASE/auth/get-csrf-token/")"
[ "$status" = 200 ] || fail "/auth/get-csrf-token/ answered $status, not 200: is $BASE the Worker? ($(tr -d '\r\n' < "$T/e"))"
token="$(squash "$T/b" | sed -n 's/.*"csrf_token":"\([^"]*\)".*/\1/p')"
[ -n "$token" ] || fail "/auth/get-csrf-token/ answered 200 with no csrf_token in it"

cred="$(printf '{"email":"%s","password":"%s"}' "$(json_escape "$SMOKE_EMAIL")" "$(json_escape "$SMOKE_PASSWORD")")"
status="$(printf '%s' "$cred" | req -c "$JAR" -b "$JAR" -X POST -H 'Content-Type: application/json' -H "X-CSRFToken: $token" --data-binary @- "$BASE/auth/sign-up/")"
if [ "$status" = 200 ]; then
  echo "account: made, and signed in"
else
  up_code="$(code_of)"
  # not made: an account may already be there, so sign in; a refusal on both is reported with both codes
  status="$(printf '%s' "$cred" | req -c "$JAR" -b "$JAR" -X POST -H 'Content-Type: application/json' -H "X-CSRFToken: $token" --data-binary @- "$BASE/auth/sign-in/")"
  [ "$status" = 200 ] || fail "account: sign-up was refused (error_code ${up_code:-none}) and so was sign-in ($status, error_code $(code_of)): a weak password, or an account with another one"
  echo "account: already there, signed in"
fi

status="$(req -c "$JAR" -b "$JAR" "$BASE/api/workspaces/$SMOKE_WORKSPACE/projects/")"
if [ "$status" = 200 ]; then
  echo "workspace: '$SMOKE_WORKSPACE' already there"
else
  status="$(printf '{"name":"%s","slug":"%s"}' "$SMOKE_WORKSPACE" "$SMOKE_WORKSPACE" | post_json "/api/workspaces/")"
  case "$status" in 200|201) echo "workspace: '$SMOKE_WORKSPACE' made" ;; *) fail "workspace: making '$SMOKE_WORKSPACE' answered $status: $(head -c 300 "$T/b")" ;; esac
  status="$(req -c "$JAR" -b "$JAR" "$BASE/api/workspaces/$SMOKE_WORKSPACE/projects/")"
  [ "$status" = 200 ] || fail "workspace: just made, and its project list answered $status"
fi

needle="\"name\":\"$(printf '%s' "$PROJECT" | tr -d ' \t')\""
if squash "$T/b" | grep -qF "$needle"; then
  echo "project: '$PROJECT' already there"
else
  status="$(printf '{"name":"%s","identifier":"%s","network":2,"description":""}' "$PROJECT" "$IDENT" | post_json "/api/workspaces/$SMOKE_WORKSPACE/projects/")"
  case "$status" in 200|201) echo "project: '$PROJECT' made" ;; *) fail "project: making '$PROJECT' answered $status: $(head -c 300 "$T/b")" ;; esac
fi

echo "bootstrap: DONE. Give the deploy job's environment SMOKE_EMAIL, SMOKE_PASSWORD (a secret) and SMOKE_WORKSPACE='$SMOKE_WORKSPACE', SMOKE_PROJECT='$PROJECT'."
