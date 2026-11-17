#!/bin/sh
# Opt in at the edge as CI (ADR-0111, #195): get the pin cookie that makes this client's requests on the shared hostname reach one
# target, for that target's smoke (scripts/smoke-deploy.sh with SMOKE_COOKIE_FILE).
#
#   scripts/edge-opt-in.sh <shared-origin> <audience> <target> <cookie-jar>
#
# It asks `<shared-origin>/_edge/opt-in/<audience>?target=<target>` with the CI opt-in service token, which Access admits on that
# path; the edge then sets its pin, `__Host-thrust_edge`, in a 303, and curl writes it to <cookie-jar>. The edge sets a pin only for
# a target the rules route this principal to NOW (crates/edge/src/runtime.rs, opt_in), so right after the target's flag was
# written it refuses until the flag has reached it (up to 30 s, Flagship's documentation): that refusal, and an edge that answers
# 503 or not at all, are asked again. Any other answer is final: a token Access or the edge does not admit, an unknown audience,
# a bad name.
#
# Environment:
#   EDGE_CI_CLIENT_ID, EDGE_CI_CLIENT_SECRET   the CI opt-in service token (`thrust-ci-opt-in`). They reach curl in a header file
#                                              made 0600 in a private directory, never in argv, and are never printed.
#   OPT_IN_ATTEMPTS (default 12), OPT_IN_INTERVAL (default 5 seconds)
#
# Exit status: 0 the jar holds the pin; 1 the edge did not give one; 2 called wrongly (nothing was asked).
set -u
LC_ALL=C
export LC_ALL

usage() { echo "edge-opt-in: $*" >&2; exit 2; }
fail() { echo "edge-opt-in: FAIL: $*" >&2; exit 1; }

[ "$#" -eq 4 ] || usage "usage: edge-opt-in.sh <shared-origin> <audience> <target> <cookie-jar>"
BASE="${1%/}"; AUD="$2"; TARGET="$3"; JAR="$4"
case "$BASE" in
  https://*) ;;
  http://127.0.0.1:*) ;; # the tests' stand-in for the edge
  *) usage "the shared origin must be https://<host>, got '$BASE'" ;;
esac
case "$BASE" in https://*/*|http://127.0.0.1:*/*) usage "the shared origin is an origin, with no path: '$BASE'" ;; esac
# the edge's own grammars (crates/edge/src/pin.rs): an audience is 1-16 lowercase letters; a target name starts with a letter or digit
# and is at most 57 of lowercase letters, digits, '.' and '-'. Both are then safe in a URL as they are.
case "$AUD" in ''|*[!a-z]*) usage "the audience is lowercase letters only, got '$AUD'" ;; esac
[ "${#AUD}" -le 16 ] || usage "the audience is at most 16 letters"
case "$TARGET" in [a-z0-9]*) ;; *) usage "a target name starts with a lowercase letter or a digit, got '$TARGET'" ;; esac
case "$TARGET" in *[!a-z0-9.-]*) usage "a target name is lowercase letters, digits, '.' and '-', got '$TARGET'" ;; esac
[ "${#TARGET}" -le 57 ] || usage "a target name is at most 57 characters"
[ -n "${EDGE_CI_CLIENT_ID:-}" ] && [ -n "${EDGE_CI_CLIENT_SECRET:-}" ] || usage "EDGE_CI_CLIENT_ID and EDGE_CI_CLIENT_SECRET are required"
ATTEMPTS="${OPT_IN_ATTEMPTS:-12}"
INTERVAL="${OPT_IN_INTERVAL:-5}"
case "$ATTEMPTS" in ''|*[!0-9]*|0) usage "OPT_IN_ATTEMPTS is a positive whole number" ;; esac
case "$INTERVAL" in ''|*[!0-9]*) usage "OPT_IN_INTERVAL is a whole number of seconds" ;; esac

T="$(mktemp -d)" || usage "cannot make a temporary directory"
trap 'rm -rf "$T"' EXIT INT TERM
chmod 700 "$T"
( umask 077; printf 'CF-Access-Client-Id: %s\nCF-Access-Client-Secret: %s\n' "$EDGE_CI_CLIENT_ID" "$EDGE_CI_CLIENT_SECRET" > "$T/headers" )
rm -f "$JAR"

URL="$BASE/_edge/opt-in/$AUD?target=$TARGET"
n=0
while [ "$n" -lt "$ATTEMPTS" ]; do
  n=$((n + 1))
  rm -f "$JAR"
  status="$(curl -sS --connect-timeout 5 --max-time 20 -D "$T/h" -o "$T/b" -c "$JAR" -H @"$T/headers" -w '%{http_code}' "$URL" 2>"$T/e" || true)"
  body="$(head -c 200 "$T/b" 2>/dev/null | tr -d '\r\n')"
  if [ "$status" = 303 ] && grep -q '__Host-thrust_edge' "$JAR" 2>/dev/null; then
    echo "edge-opt-in: pinned to $TARGET through $AUD (attempt $n/$ATTEMPTS)"
    exit 0
  fi
  case "$status" in
    303) fail "the edge answered 303 without the pin cookie: $body" ;;
    403)
      case "$body" in
        "that target does not exist or is not open to you"*) why="the edge does not route this principal to $TARGET yet (its flag may still be propagating)" ;;
        *) fail "refused, and not because a flag is propagating (403: ${body:-an empty body, as Access refuses})" ;;
      esac
      ;;
    503) why="the edge answered 503: $body" ;;
    000) why="no answer from $BASE: $(tr -d '\r\n' < "$T/e")" ;;
    *) fail "the edge answered $status: $body" ;;
  esac
  echo "edge-opt-in: attempt $n/$ATTEMPTS: $why"
  [ "$n" -lt "$ATTEMPTS" ] && sleep "$INTERVAL"
done
rm -f "$JAR"
fail "no pin for $TARGET after $ATTEMPTS attempts (last: $why)"
