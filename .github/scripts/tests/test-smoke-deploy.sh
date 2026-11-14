#!/bin/sh
# Exercise scripts/smoke-deploy.sh (issue #49) against a stand-in for the deployed Worker (smoke_stub.py), including the cases that
# must FAIL. Every case goes through `expect`, which checks the exit status AND a message fragment, so a case cannot pass for the
# wrong reason and none is merely printed. The passing case is what gives each failing one its meaning: the same stub, the same
# script and the same environment pass until exactly one thing is broken.
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

# ---- hygiene ---------------------------------------------------------------------------------------------------------------
no_stub_left() {
  if ps -eo command | grep -F "smoke_stub.py --port-file $T" | grep -vF grep > /dev/null; then
    echo "a stand-in server is still running"; return 1
  fi
  echo "no stand-in server is left running"
}
expect "hygiene: every stand-in server this test started has been stopped" 0 "no stand-in server is left running" no_stub_left

echo "passed=$pass bad=$failn"
[ "$failn" -eq 0 ]
