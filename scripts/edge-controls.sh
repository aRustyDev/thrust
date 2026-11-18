#!/bin/sh
# The edge's own controls on the shared hostname (ADR-0111, issue #195 stage E3), run by scripts/deploy-edge.sh after the smoke has
# shown that the new edge answers. Each is something a broken edge would get wrong while production still looked fine through it:
#
#   scripts/edge-controls.sh <base-url> <expected-edge-version>
#
#   1. A forged pin goes to production. `/healthz` with a `__Host-thrust_edge` cookie that no key signed must answer 200 from
#      production (`x-thrust-route: prod`), given by this edge (`x-thrust-edge-version`). An edge that took an unsigned pin at its
#      word would send it wherever it named.
#   2. The opt-in gives no pin to a request Access did not admit. `/_edge/opt-in/rc` asked with a made-up service token and a
#      made-up Access token header must be refused (302 to Access's login, 401 or 403) and must set no `__Host-thrust_edge`
#      cookie. Access refuses it in front of the edge, and the edge refuses it again itself (it verifies the Access token), so
#      either one missing on its own still passes here: what this proves is that the two together let no one in.
#
# Needs no credential, and sends none: the tokens it sends are made up here. A request that gets no answer, or a 5xx, is asked
# again (EDGE_CONTROL_ATTEMPTS times in all, 3 by default, EDGE_CONTROL_INTERVAL seconds apart, 2 by default); any other answer
# is the answer.
#
# Exit status: 0 both hold; 1 one does not (which, and what came back, on stderr); 2 called wrongly.
set -u
LC_ALL=C
export LC_ALL

fail() { echo "edge-controls: FAIL: $*" >&2; exit 1; }
usage() { echo "edge-controls: $*" >&2; exit 2; }

BASE="${1:-}"; EXPECT="${2:-}"
[ -n "$BASE" ] && [ -n "$EXPECT" ] || usage "usage: edge-controls.sh <base-url> <expected-edge-version>"
case "$BASE" in http://*|https://*) ;; *) usage "the base URL must start with http:// or https://, got '$BASE'" ;; esac
BASE="${BASE%/}"
case "$EXPECT" in *[!0-9A-Za-z._:/@+=-]*) usage "the expected edge version has a character a version tag does not: '$EXPECT'" ;; esac
ATTEMPTS="${EDGE_CONTROL_ATTEMPTS:-3}"; INTERVAL="${EDGE_CONTROL_INTERVAL:-2}"
case "$ATTEMPTS" in ''|*[!0-9]*|0) usage "EDGE_CONTROL_ATTEMPTS must be a positive whole number, not '$ATTEMPTS'" ;; esac
case "$INTERVAL" in ''|*[!0-9]*) usage "EDGE_CONTROL_INTERVAL must be a whole number of seconds, not '$INTERVAL'" ;; esac

T="$(mktemp -d)" || usage "cannot make a temporary directory"
trap 'rm -rf "$T"' EXIT INT TERM

# One request, asked again on no answer or a 5xx. Prints the status (000 for no answer); headers to $T/h, the body to $T/b.
ask() { # ask <curl args...> <url>
  n=1
  while :; do
    status="$(curl -sS --connect-timeout 5 --max-time 20 -D "$T/h" -o "$T/b" -w '%{http_code}' "$@" 2>"$T/e" || true)"
    case "$status" in 000|5??) ;; *) break ;; esac
    [ "$n" -ge "$ATTEMPTS" ] && break
    n=$((n + 1))
    sleep "$INTERVAL"
  done
  printf '%s' "$status"
}
header() { # header <name>: the last value of that response header
  tr -d '\r' < "$T/h" 2>/dev/null | grep -i "^$1:" | tail -1 | sed 's/^[^:]*:[[:space:]]*//; s/[[:space:]]*$//'
}
# a Set-Cookie that sets the pin to a value (clearing it, `__Host-thrust_edge=;`, is not setting it)
sets_pin() { tr -d '\r' < "$T/h" 2>/dev/null | grep -iE '^set-cookie:[[:space:]]*__Host-thrust_edge=[^;[:space:]]' > /dev/null; }

# ---- 1. a forged pin goes to production --------------------------------------------------------------------------------------
# shaped like a pin (a payload and a signature), signed by no key
FORGED="__Host-thrust_edge=eyJwcmluY2lwYWwiOiJmb3JnZWQiLCJraW5kIjoiY2kiLCJyZXF1ZXN0ZWQiOiJzdGFnaW5nIn0.AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA"
status="$(ask -H 'Cache-Control: no-cache' -H "Cookie: $FORGED" "$BASE/healthz?edge-control=forged-pin")"
[ "$status" = 200 ] || fail "a forged pin: /healthz answered $status, not 200 ($(tr -d '\r\n' < "$T/e"))"
route="$(header x-thrust-route)"; edge="$(header x-thrust-edge-version)"
[ "$edge" = "$EXPECT" ] || fail "a forged pin: the answer came from the edge '${edge:-no x-thrust-edge-version at all}', not '$EXPECT'"
[ "$route" = prod ] || fail "a forged pin was routed to '${route:-no x-thrust-route at all}', not to production: the edge took an unsigned pin"
echo "edge-controls: ok: a forged pin goes to production"

# ---- 2. the opt-in admits no one Access did not ------------------------------------------------------------------------------
# a made-up service token, and a made-up Access token (a JWT's shape, signed by nobody), as a client that wants a pin would send them
status="$(ask -H 'CF-Access-Client-Id: 0000000000000000000000000000000.access' \
  -H 'CF-Access-Client-Secret: 0000000000000000000000000000000000000000000000000000000000000000' \
  -H 'Cf-Access-Jwt-Assertion: eyJhbGciOiJSUzI1NiIsImtpZCI6ImZvcmdlZCJ9.eyJzdWIiOiJmb3JnZWQifQ.AAAA' \
  "$BASE/_edge/opt-in/rc?target=staging")"
if sets_pin; then
  fail "the opt-in set a pin for a request Access did not admit (it answered $status): anyone could reach a target"
fi
case "$status" in
  302|401|403) ;;
  000) fail "the opt-in: no answer ($(tr -d '\r\n' < "$T/e"))" ;;
  *) fail "the opt-in answered $status to a request Access did not admit, not a refusal (302, 401 or 403)" ;;
esac
echo "edge-controls: ok: the opt-in refuses a request Access did not admit ($status, no pin)"
echo "edge-controls: PASS"
