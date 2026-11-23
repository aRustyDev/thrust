#!/bin/sh
# Exercise scripts/smoke-deploy.sh (issue #49), scripts/edge-opt-in.sh and scripts/edge-controls.sh (ADR-0111) against a stand-in for
# the deployed Worker and the edge in front of it (smoke_stub.py), including the cases that must FAIL. Every case goes through `expect`, which checks the exit status AND a message fragment, so a case cannot pass for the
# wrong reason and none is merely printed. The passing case is what gives each failing one its meaning: the same stub, the same
# script and the same environment pass until exactly one thing is broken.
# shellcheck disable=SC2030,SC2031 # the subshells are the point: a change to PATH or to a variable made in one must not reach the next run, and the `leak` cases prove it
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
SMOKE="$HERE/scripts/smoke-deploy.sh"
STUB="$HERE/.github/scripts/tests/smoke_stub.py"
T="$(mktemp -d)"
stub_pid=""
cleanup() {
  [ -n "$stub_pid" ] && kill "$stub_pid" 2>/dev/null
  rm -rf "$T"
}
trap cleanup EXIT INT TERM
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

# A stand-in needs a Python. The workflow runner has python3; a developer machine has uv (the repo's tool for Python).
if command -v uv > /dev/null 2>&1; then
  PYRUN="uv run --no-project --quiet python -I"
elif command -v python3 > /dev/null 2>&1; then
  PYRUN="python3 -I"
else
  echo "BAD  neither uv nor python3 is installed: the stand-in server cannot run"; exit 1
fi
command -v curl > /dev/null 2>&1 || { echo "BAD  curl is not installed: the smoke cannot run"; exit 1; }

start_stub() { # start the stand-in with $STUB_ARGS; sets STUB_PORT
  rm -f "$T/port"
  # STUB_ARGS is split into words on purpose
  # shellcheck disable=SC2086
  $PYRUN "$STUB" --port-file "$T/port" $STUB_ARGS > "$T/stub.log" 2>&1 &
  stub_pid=$!
  i=0
  while [ ! -s "$T/port" ] && [ "$i" -lt 100 ]; do i=$((i + 1)); sleep 0.1; done
  [ -s "$T/port" ] || { echo "the stand-in server did not start:"; cat "$T/stub.log"; return 1; }
  STUB_PORT="$(cat "$T/port")"
}
stop_stub() {
  kill "$stub_pid" 2>/dev/null
  wait "$stub_pid" 2>/dev/null
  stub_pid=""
}

# smoke_against_stub <expected-version>: start the stand-in with $STUB_ARGS, run the smoke with $SMOKE_ENV, stop the stand-in.
smoke_against_stub() {
  start_stub || return 99
  # SMOKE_ENV is a list of VAR=value words, split on purpose
  # shellcheck disable=SC2086
  env $SMOKE_ENV "$SMOKE" "http://127.0.0.1:$STUB_PORT" "$1"
  rc=$?
  stop_stub
  return $rc
}
# The same, against a port that had a server a moment ago and has none now.
smoke_against_dead_server() {
  start_stub || return 99
  stop_stub
  # shellcheck disable=SC2086
  env $SMOKE_ENV "$SMOKE" "http://127.0.0.1:$STUB_PORT" abc123
}

export STUB_PASSWORD='s3cret-pw'
export SMOKE_PASSWORD='s3cret-pw'
# Few attempts and no waiting: a case that is going to fail must fail fast. The version phase still needs 3 answers in a row.
BASE_ENV="SMOKE_EMAIL=smoke@example.test SMOKE_WORKSPACE=smoke-ws SMOKE_PROJECT=smoke SMOKE_ATTEMPTS=4 SMOKE_INTERVAL=0 SMOKE_STREAK_INTERVAL=0"

# ---- the passing case ------------------------------------------------------------------------------------------------------
STUB_ARGS="--version abc123"; SMOKE_ENV="$BASE_ENV"
expect "pass: the version is the new one and the smoke account reads its project" 0 "smoke: PASS (abc123 at http://127.0.0.1:" smoke_against_stub abc123
expect "pass: the version phase says how many answers it saw" 0 "version: PASS (3 consecutive answers carried abc123)" smoke_against_stub abc123
expect "pass: the auth phase says what it read" 0 "auth: PASS (signed in as the smoke account and read the project 'smoke')" smoke_against_stub abc123

# ---- the version phase: each of these must FAIL ----------------------------------------------------------------------------
# Issue #49, acceptance criterion 2: pointed at a Worker that still serves the previous version, the smoke fails.
STUB_ARGS="--version old000"
expect "version: a Worker still serving the previous version fails" 1 "the Worker serves old000, expected abc123" smoke_against_stub abc123
STUB_ARGS="--no-version"
expect "version: a Worker that sends no version header fails (it cannot be told from the old one)" 1 "has no x-thrust-version header" smoke_against_stub abc123
STUB_ARGS="--alternate old000,abc123"; SMOKE_ENV="$BASE_ENV SMOKE_ATTEMPTS=6"
expect "version: a half-rolled-out deploy (old, new, old, new...) fails" 1 "never saw 3 consecutive answers carrying abc123 in 6 attempts" smoke_against_stub abc123
STUB_ARGS="--health-status 503"; SMOKE_ENV="$BASE_ENV"
expect "version: a /healthz that is not 200 fails" 1 "/healthz answered 503, not 200" smoke_against_stub abc123
STUB_ARGS="--health-body down"
expect "version: a /healthz whose body is not 'ok' fails" 1 "its body is not 'ok'" smoke_against_stub abc123
expect "version: a server that is not there fails and does not hang" 1 "could not reach http://127.0.0.1:" smoke_against_dead_server
# A skipped auth phase must not skip the version phase.
STUB_ARGS="--version old000"; SMOKE_ENV="$BASE_ENV SMOKE_AUTH=off"
expect "version: SMOKE_AUTH=off still checks the version" 1 "the Worker serves old000, expected abc123" smoke_against_stub abc123

# ---- the auth phase: each of these must FAIL -------------------------------------------------------------------------------
STUB_ARGS="--version abc123"; SMOKE_ENV="$BASE_ENV SMOKE_PASSWORD=wrong-pw"
expect "auth: a sign-in that is refused fails, with the error code" 1 "sign-in as the smoke account answered 400 (error_code 5035), not 200" smoke_against_stub abc123
STUB_ARGS="--version abc123 --drop-cookie"; SMOKE_ENV="$BASE_ENV"
expect "auth: a sign-in that sets no session fails at the next read" 1 "the session was not kept" smoke_against_stub abc123
STUB_ARGS="--version abc123 --no-project"
expect "auth: a workspace without the smoke project fails" 1 "does not hold the project 'smoke'" smoke_against_stub abc123
STUB_ARGS="--version abc123"; SMOKE_ENV="$BASE_ENV SMOKE_WORKSPACE=other-ws"
expect "auth: a workspace that does not exist fails" 1 "the project list of 'other-ws' answered 404, not 200" smoke_against_stub abc123
STUB_ARGS="--version abc123"; SMOKE_ENV="$BASE_ENV SMOKE_PROJECT=another"
expect "auth: a project that is not in the list fails" 1 "does not hold the project 'another'" smoke_against_stub abc123

# ---- deliberate failure, the rollback control ------------------------------------------------------------------------------
STUB_ARGS="--version abc123"; SMOKE_ENV="$BASE_ENV SMOKE_FORCE_FAIL=1"
expect "force-fail: every real check passes and the smoke still fails on purpose" 1 "failing on purpose, after every real check passed" smoke_against_stub abc123
expect "force-fail: and it got that far, so the control is not a smoke that failed early" 1 "auth: PASS" smoke_against_stub abc123

# ---- configuration: nothing is checked, and it says so ---------------------------------------------------------------------
STUB_ARGS="--version abc123"; SMOKE_ENV="SMOKE_WORKSPACE=smoke-ws SMOKE_ATTEMPTS=2 SMOKE_INTERVAL=0"
expect "config: no smoke account set fails closed, it does not skip the read" 2 "SMOKE_EMAIL is not set" smoke_against_stub abc123
SMOKE_ENV="$BASE_ENV SMOKE_AUTH=off"
expect "config: SMOKE_AUTH=off skips the read and says nothing was read" 0 "auth: SKIPPED (SMOKE_AUTH=off): nothing was read from the database" smoke_against_stub abc123
SMOKE_ENV="$BASE_ENV SMOKE_AUTH=maybe"
expect "config: SMOKE_AUTH is on or off and nothing else" 2 "SMOKE_AUTH is 'on' or 'off'" smoke_against_stub abc123
SMOKE_ENV="$BASE_ENV SMOKE_WORKSPACE=a/b"
expect "config: a workspace that is not a slug is refused" 2 "SMOKE_WORKSPACE is a slug" smoke_against_stub abc123
SMOKE_ENV="$BASE_ENV SMOKE_CONSECUTIVE=0"
expect "config: zero consecutive answers is refused, it would pass on nothing" 2 "must be positive whole numbers" smoke_against_stub abc123
expect "usage: no arguments" 2 "usage: smoke-deploy.sh <base-url> <expected-version>" "$SMOKE"
expect "usage: a base URL that is not http(s)" 2 "must start with http:// or https://" "$SMOKE" "localhost:8787" abc123
expect "usage: an expected version with a space in it" 2 "the expected version has a character a version tag does not" "$SMOKE" "http://127.0.0.1:9" "abc 123"

# ---- the password: in no argv, in no output ---------------------------------------------------------------------------------
mkdir -p "$T/bin"
REAL_CURL="$(command -v curl)"
cat > "$T/bin/curl" <<EOF
#!/bin/sh
printf '%s\n' "\$*" >> "$T/argv.log"
exec "$REAL_CURL" "\$@"
EOF
chmod +x "$T/bin/curl"
STUB_ARGS="--version abc123"; SMOKE_ENV="$BASE_ENV"
no_leak() {
  : > "$T/argv.log"
  # PATH is set in a subshell and not in SMOKE_ENV: that list is split on spaces, and a developer's PATH can have one in it
  ( PATH="$T/bin:$PATH"; export PATH; smoke_against_stub abc123 ) > "$T/leak.out" 2>&1 || {
    echo "the passing case did not pass:"; cat "$T/leak.out"; return 1
  }
  # not vacuous: the wrapper really was in front of curl, and saw the sign-in
  grep -qF "/auth/sign-in/" "$T/argv.log" || { echo "the argv log has no sign-in: the check looked at nothing"; return 1; }
  if grep -qF "$STUB_PASSWORD" "$T/argv.log" "$T/leak.out"; then echo "the password is in curl's argv or in the output"; return 1; fi
  echo "no leak: the password is in no curl argv and not in the output (the sign-in was seen)"
}
expect "password: it is in no curl argv and not in the output" 0 "no leak: the password is in no curl argv" no_leak
# A password with a quote, a backslash and a space must reach the server as typed: the JSON is escaped, not broken.
odd_password() {
  (
    export STUB_PASSWORD='p"w\x ok' SMOKE_PASSWORD='p"w\x ok'
    STUB_ARGS="--version abc123"; SMOKE_ENV="$BASE_ENV"
    smoke_against_stub abc123
  )
}
expect "password: a quote, a backslash and a space in it still sign in" 0 "auth: PASS" odd_password

# ---- through the edge (ADR-0111): the pin, from scripts/edge-opt-in.sh, and the route ---------------------------------------
# The stub stands for the shared hostname with one target (--pin): only a request with the target's pin cookie reaches it; any
# other is production (another version, `x-thrust-route: prod`, an API that answers 421). The opt-in is Access (the CI token's id
# and secret) and then the edge, which refuses until the target's flag has propagated (--deny-first).
OPTIN="$HERE/scripts/edge-opt-in.sh"
export STUB_CLIENT_ID='ci-id' STUB_CLIENT_SECRET='ci-secret-7f3a9'
OPT_ENV="EDGE_CI_CLIENT_ID=ci-id EDGE_CI_CLIENT_SECRET=ci-secret-7f3a9 OPT_IN_ATTEMPTS=4 OPT_IN_INTERVAL=0"

# opt_in_against_stub <audience> <target>: start the stand-in, opt in into $T/pin.jar, stop it
opt_in_against_stub() {
  start_stub || return 99
  # shellcheck disable=SC2086
  env $OPT_ENV "$OPTIN" "http://127.0.0.1:$STUB_PORT" "$1" "$2" "$T/pin.jar"
  rc=$?
  stop_stub
  return $rc
}
# through_edge <target> <expected-version> [route]: opt in for <target>, then the smoke with that pin (and the route, if given)
through_edge() {
  start_stub || return 99
  # shellcheck disable=SC2086
  env $OPT_ENV "$OPTIN" "http://127.0.0.1:$STUB_PORT" rc "$1" "$T/pin.jar" || { rc=$?; stop_stub; return $rc; }
  # shellcheck disable=SC2086
  env $SMOKE_ENV SMOKE_COOKIE_FILE="$T/pin.jar" ${3:+SMOKE_EXPECT_ROUTE=$3} "$SMOKE" "http://127.0.0.1:$STUB_PORT" "$2"
  rc=$?
  stop_stub
  return $rc
}

SMOKE_ENV="$BASE_ENV"
STUB_ARGS="--version dep-1 --pin v0.3.0"
expect "edge: with the pin and the route, both phases reach the target (its smoke account reads its project)" 0 \
  "smoke: PASS (dep-1 at http://127.0.0.1:" through_edge v0.3.0 dep-1 v0.3.0
expect "edge: the pass names the route" 0 "routed to v0.3.0" through_edge v0.3.0 dep-1 v0.3.0
expect "edge: without the pin the answer is production's, whose version is not the target's" 1 \
  "the Worker serves prod-version, expected dep-1" smoke_against_stub dep-1
STUB_ARGS="--version dep-1 --pin v0.3.0 --route v0.2.0"
expect "edge: an answer routed to another target fails the route" 1 "the edge routed this to 'v0.2.0', not 'v0.3.0'" \
  through_edge v0.3.0 dep-1 v0.3.0
expect "edge: the route is not checked when none is expected (its control)" 0 "smoke: PASS" through_edge v0.3.0 dep-1
STUB_ARGS="--version dep-1 --pin v0.3.0 --no-route"
expect "edge: an answer with no route header (it did not pass the edge) fails the route" 1 "no x-thrust-route at all" \
  through_edge v0.3.0 dep-1 v0.3.0
STUB_ARGS="--version abc123"
SMOKE_ENV="$BASE_ENV SMOKE_COOKIE_FILE=$T/no-such-jar"
expect "edge: SMOKE_COOKIE_FILE that is not a file: a usage error, nothing asked" 2 "is not a readable file" smoke_against_stub abc123
SMOKE_ENV="$BASE_ENV SMOKE_EXPECT_ROUTE=V0.3"
expect "edge: SMOKE_EXPECT_ROUTE that is not a name: a usage error" 2 "SMOKE_EXPECT_ROUTE is a target name" smoke_against_stub abc123
SMOKE_ENV="$BASE_ENV"

# the opt-in itself
STUB_ARGS="--pin v0.3.0 --deny-first 2"
expect "opt-in: refused while the flag propagates, then pinned" 0 "pinned to v0.3.0 through rc (attempt 3/4)" opt_in_against_stub rc v0.3.0
STUB_ARGS="--pin v0.3.0 --deny-first 9"
expect "opt-in: a flag that never arrives fails after the attempts" 1 "no pin for v0.3.0 after 4 attempts" opt_in_against_stub rc v0.3.0
STUB_ARGS="--pin v0.3.0"
expect "opt-in: a target the edge does not route to fails after the attempts" 1 "no pin for v0.9.9" opt_in_against_stub rc v0.9.9
expect "opt-in: an audience the edge has no path for is final" 1 "the edge answered 404" opt_in_against_stub ops v0.3.0
wrong_token() { ( OPT_ENV="EDGE_CI_CLIENT_ID=ci-id EDGE_CI_CLIENT_SECRET=wrong OPT_IN_ATTEMPTS=4 OPT_IN_INTERVAL=0"; opt_in_against_stub rc v0.3.0 ); }
# `FAIL: refused` is printed only by a refusal that is final; one that was retried ends with `FAIL: no pin ... after 4 attempts`
expect "opt-in: a token Access refuses is final, not retried" 1 "FAIL: refused, and not because a flag is propagating (403: an empty body" wrong_token
STUB_ARGS="--pin v0.3.0 --not-admitted"
expect "opt-in: a token the edge refuses is final, not retried" 1 "FAIL: refused, and not because a flag is propagating (403: not admitted" \
  opt_in_against_stub rc v0.3.0
STUB_ARGS="--pin v0.3.0 --unavailable-first 2"
expect "opt-in: an edge that answers 503 for a while, then pins" 0 "pinned to v0.3.0 through rc (attempt 3/4)" opt_in_against_stub rc v0.3.0
STUB_ARGS="--pin v0.3.0 --opt-in-status 503"
expect "opt-in: an edge that keeps answering 503 fails after the attempts" 1 "the edge answered 503" opt_in_against_stub rc v0.3.0
STUB_ARGS="--pin v0.3.0 --opt-in-status 500"
expect "opt-in: any other status is final" 1 "the edge answered 500" opt_in_against_stub rc v0.3.0
STUB_ARGS="--pin v0.3.0 --no-pin-cookie"
expect "opt-in: a 303 that sets no pin is a failure" 1 "answered 303 without the pin cookie" opt_in_against_stub rc v0.3.0
STUB_ARGS="--pin v0.3.0"
for bad in "RC v0.3.0" "rc V0.3.0" "rc -v0" "rc v0/3" "abcdefghijklmnopq v0.3.0"; do
  # shellcheck disable=SC2086
  expect "opt-in: '$bad' is a usage error, nothing asked" 2 "edge-opt-in:" opt_in_against_stub $bad
done
# shellcheck disable=SC2086 # $OPT_ENV is a list of NAME=value words, split on purpose
expect "opt-in: an origin over http elsewhere is a usage error" 2 "must be https" env $OPT_ENV "$OPTIN" "http://example.com" rc v0.3.0 "$T/x.jar"
expect "opt-in: no token is a usage error" 2 "EDGE_CI_CLIENT_ID and EDGE_CI_CLIENT_SECRET are required" \
  env OPT_IN_ATTEMPTS=1 "$OPTIN" "https://thrust.example.test" rc v0.3.0 "$T/x.jar"
opt_in_no_leak() {
  : > "$T/argv.log"
  ( PATH="$T/bin:$PATH"; export PATH; opt_in_against_stub rc v0.3.0 ) > "$T/oleak.out" 2>&1 || {
    echo "the opt-in did not pass:"; cat "$T/oleak.out"; return 1
  }
  grep -qF "/_edge/opt-in/rc" "$T/argv.log" || { echo "the argv log has no opt-in: the check looked at nothing"; return 1; }
  if grep -qF "$STUB_CLIENT_SECRET" "$T/argv.log" "$T/oleak.out"; then echo "the secret is in curl's argv or in the output"; return 1; fi
  echo "no leak: the opt-in secret is in no curl argv and not in the output (the opt-in was seen)"
}
expect "opt-in: the token's secret is in no curl argv and not in the output" 0 "no leak: the opt-in secret" opt_in_no_leak

# ---- the edge's own deploy: SMOKE_EXPECT_EDGE, and scripts/edge-controls.sh ------------------------------------------------
# The stand-in as the shared hostname with the edge in front of production: every answer carries the edge's version, /healthz is
# routed `prod`, and production's API works (the edge's smoke reads production's data through it).
STUB_ARGS="--version rel-1 --route prod --edge-version edge-1"
SMOKE_ENV="$BASE_ENV SMOKE_EXPECT_ROUTE=prod SMOKE_EXPECT_EDGE=edge-1"
expect "edge deploy: this edge answers, routed to production, which reads its project" 0 \
  "smoke: PASS (rel-1 at http://127.0.0.1:" smoke_against_stub rel-1
expect "edge deploy: the pass names the edge" 0 "routed to prod, by the edge edge-1" smoke_against_stub rel-1
SMOKE_ENV="$BASE_ENV SMOKE_EXPECT_ROUTE=prod SMOKE_EXPECT_EDGE=edge-2"
expect "edge deploy: an answer from another edge (the one it replaced) fails" 1 "the edge that answered is 'edge-1', not 'edge-2'" \
  smoke_against_stub rel-1
SMOKE_ENV="$BASE_ENV SMOKE_EXPECT_ROUTE=prod"
expect "edge deploy: the edge is not checked when none is expected (its control)" 0 "smoke: PASS" smoke_against_stub rel-1
STUB_ARGS="--version rel-1 --route prod"
SMOKE_ENV="$BASE_ENV SMOKE_EXPECT_ROUTE=prod SMOKE_EXPECT_EDGE=edge-1"
expect "edge deploy: an answer with no edge version (no edge in front) fails" 1 "no x-thrust-edge-version at all" \
  smoke_against_stub rel-1
STUB_ARGS="--version rel-2 --route prod --edge-version edge-1"
expect "edge deploy: the right edge in front of the wrong production fails" 1 "the Worker serves rel-2, expected rel-1" \
  smoke_against_stub rel-1
SMOKE_ENV="$BASE_ENV SMOKE_EXPECT_EDGE=edge,1"
expect "edge deploy: SMOKE_EXPECT_EDGE that is not a version: a usage error" 2 "SMOKE_EXPECT_EDGE has a character" \
  smoke_against_stub rel-1
SMOKE_ENV="$BASE_ENV"

CONTROLS="$HERE/scripts/edge-controls.sh"
# controls_against_stub <expected-edge-version>: start the stand-in with $STUB_ARGS, run the controls, stop the stand-in
controls_against_stub() {
  start_stub || return 99
  EDGE_CONTROL_ATTEMPTS=2 EDGE_CONTROL_INTERVAL=0 sh "$CONTROLS" "http://127.0.0.1:$STUB_PORT" "$1"
  rc=$?
  stop_stub
  return $rc
}
# The stand-in in front of production with one target, v0.3.0: only `pin-v0.3.0` pins, and Access admits only the CI token.
STUB_ARGS="--version dep-1 --pin v0.3.0 --prod-version rel-1 --edge-version edge-1"
expect "edge controls: a forged pin goes to production, and the opt-in admits no one Access did not" 0 "edge-controls: PASS" \
  controls_against_stub edge-1
expect "edge controls: the refusal it saw was Access's 403" 0 "the opt-in refuses a request Access did not admit (403, no pin)" \
  controls_against_stub edge-1
expect "edge controls: the forged pin's answer from another edge fails" 1 "the answer came from the edge 'edge-1', not 'edge-2'" \
  controls_against_stub edge-2
STUB_ARGS="--version dep-1 --pin v0.3.0 --prod-version rel-1 --edge-version edge-1 --honour-any-pin"
expect "edge controls: an edge that takes an unsigned pin fails" 1 "a forged pin was routed to 'v0.3.0', not to production" \
  controls_against_stub edge-1
STUB_ARGS="--version dep-1 --pin v0.3.0 --prod-version rel-1 --edge-version edge-1 --opt-in-open"
expect "edge controls: an opt-in that pins whoever asks fails" 1 "the opt-in set a pin for a request Access did not admit (it answered 303)" \
  controls_against_stub edge-1
STUB_ARGS="--version dep-1 --route prod --edge-version edge-1 --no-edge-paths"
expect "edge controls: an opt-in path nothing protects (404) is not a refusal" 1 \
  "the opt-in answered 404 to a request Access did not admit, not a refusal" controls_against_stub edge-1
controls_against_dead_server() {
  start_stub || return 99
  stop_stub
  EDGE_CONTROL_ATTEMPTS=2 EDGE_CONTROL_INTERVAL=0 sh "$CONTROLS" "http://127.0.0.1:$STUB_PORT" edge-1
}
expect "edge controls: no answer at all fails" 1 "a forged pin: /healthz answered 000" controls_against_dead_server
# EDGE_CONTROL_ATTEMPTS=2 in controls_against_stub: one 503 is asked again and passes; two are the answer
STUB_ARGS="--version dep-1 --pin v0.3.0 --prod-version rel-1 --edge-version edge-1 --unavailable-first-gets 1"
expect "edge controls: a 5xx is asked again" 0 "edge-controls: PASS" controls_against_stub edge-1
STUB_ARGS="--version dep-1 --pin v0.3.0 --prod-version rel-1 --edge-version edge-1 --unavailable-first-gets 2"
expect "edge controls: a 5xx on every attempt is the answer" 1 "a forged pin: /healthz answered 503, not 200" controls_against_stub edge-1
expect "edge controls: no expected version is a usage error" 2 "usage: edge-controls.sh" sh "$CONTROLS" "http://127.0.0.1:1"
expect "edge controls: a base that is not a URL is a usage error" 2 "must start with http" sh "$CONTROLS" "127.0.0.1" edge-1
expect "edge controls: an expected version with a space is a usage error" 2 "has a character" sh "$CONTROLS" "http://127.0.0.1:1" "edge 1"
expect "edge controls: no attempts is a usage error" 2 "EDGE_CONTROL_ATTEMPTS" \
  env EDGE_CONTROL_ATTEMPTS=0 sh "$CONTROLS" "http://127.0.0.1:1" edge-1

# ---- hygiene ---------------------------------------------------------------------------------------------------------------
no_stub_left() {
  # shellcheck disable=SC2009 # a fixed string in a process's command line: pgrep -f takes a regular expression
  if ps -eo command | grep -F "smoke_stub.py --port-file $T" | grep -vF grep > /dev/null; then
    echo "a stand-in server is still running"; return 1
  fi
  echo "no stand-in server is left running"
}
expect "hygiene: every stand-in server this test started has been stopped" 0 "no stand-in server is left running" no_stub_left

echo "passed=$pass bad=$failn"
[ "$failn" -eq 0 ]
