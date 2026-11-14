#!/bin/sh
# scripts/check-asset-retention.sh against a stand-in Worker (assets_stub.py) in each behaviour it has to tell apart (issue #49
# acceptance 5). The pass is with the previous release's files kept (scripts/merge-assets.sh, the deploy's own step), and the controls
# are the same check against the same release with them NOT kept, where the stand-in answers as Workers static assets do for a missing
# hashed file: `200 text/html`, the single-page fallback (ADR-0011). A check that is happy with that is the failure this exists to catch.
#
#   sh .github/scripts/tests/test-check-asset-retention.sh
#
# ASSET_CHECK_SCRIPT points the test at a copy of the script, so a deliberately broken copy can be shown to fail it.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
S="${ASSET_CHECK_SCRIPT:-$HERE/scripts/check-asset-retention.sh}"
MERGE="$HERE/scripts/merge-assets.sh"
STUB="$HERE/.github/scripts/tests/assets_stub.py"
T="$(mktemp -d)"
STUB_PID=""
cleanup() { [ -n "$STUB_PID" ] && kill "$STUB_PID" 2> /dev/null; rm -rf "$T"; }
trap cleanup EXIT INT TERM
pass=0; bad=0

# two releases: the previous one has two scripts the new one does not (one with a space in its name), and both have one in common
mkdir -p "$T/prev/assets" "$T/new/assets"
printf '<html>previous</html>' > "$T/prev/index.html"
printf '<html>new</html>' > "$T/new/index.html"
printf '// OLD app chunk\n' > "$T/prev/assets/app-old1.js"
printf '// OLD second chunk\n' > "$T/prev/assets/with space-old2.js"
printf '// shared\n' > "$T/prev/assets/shared-1.js"
cp "$T/prev/assets/shared-1.js" "$T/new/assets/shared-1.js"
printf '// NEW app chunk\n' > "$T/new/assets/app-new1.js"
sh "$MERGE" "$T/new" "$T/prev" "$T/merged" > /dev/null || { echo "BAD  the fixture could not be merged"; exit 1; }

if command -v uv > /dev/null 2>&1; then PYRUN="uv run --no-project --quiet python -I"; else PYRUN="python3 -I"; fi

serve() { # serve <dir> <mode>: starts the stand-in, sets BASE
  [ -n "$STUB_PID" ] && { kill "$STUB_PID" 2> /dev/null; wait "$STUB_PID" 2> /dev/null; STUB_PID=""; }
  rm -f "$T/port"
  $PYRUN "$STUB" "$1" "$2" "$T/port" > /dev/null 2>&1 &
  STUB_PID=$!
  i=0
  while [ ! -s "$T/port" ] && [ "$i" -lt 100 ]; do sleep 0.1; i=$((i + 1)); done
  [ -s "$T/port" ] || { echo "BAD  the stand-in did not start"; exit 1; }
  BASE="http://127.0.0.1:$(cat "$T/port")"
}
run() { # run <label> <want-exit> <want-fragment> <prev> <new> [VAR=value ...]
  label="$1"; want="$2"; frag="$3"; p="$4"; n="$5"; shift 5
  env CHECK_ASSET_INTERVAL=0 CHECK_ASSET_ATTEMPTS=3 "$@" sh "$S" "$BASE" "$p" "$n" > "$T/out.txt" 2>&1
  got=$?
  if [ "$got" = "$want" ] && grep -qF -- "$frag" "$T/out.txt"; then
    pass=$((pass + 1)); echo "ok   $label (exit $got; saw '$frag')"
  else
    bad=$((bad + 1)); echo "BAD  $label: wanted exit $want + '$frag', got exit $got"; sed 's/^/     | /' "$T/out.txt"
  fi
}

# ---- the pass: the previous release's files were kept -------------------------------------------------------------------------------
serve "$T/merged" serve
run "the previous release's scripts are still served as themselves" 0 "2 script(s) of the previous release are still served" "$T/prev" "$T/new"

# ---- the controls: the same releases, the previous one's files NOT kept ---------------------------------------------------------------
serve "$T/new" fallback
run "CONTROL: a dropped script is answered with the HTML fallback, and the check says so" 1 "answered 200 text/html: the single-page fallback" "$T/prev" "$T/new"
serve "$T/new" serve
run "CONTROL: a dropped script that is a 404" 1 "answered 404, not 200" "$T/prev" "$T/new"

# ---- the ways a 200 can still be wrong --------------------------------------------------------------------------------------------------
serve "$T/merged" wrong-type
run "a script served with a content type that is not JavaScript" 1 "not JavaScript" "$T/prev" "$T/new"
serve "$T/merged" wrong-bytes
run "a script served as JavaScript with other bytes in it" 1 "not the bytes of the previous release's file" "$T/prev" "$T/new"

# ---- propagation: the edge does not have the new version for the first few requests ------------------------------------------------------
serve "$T/merged" flaky:2
run "an edge that takes two requests to catch up, with attempts to spare" 0 "still served as themselves" "$T/prev" "$T/new" CHECK_ASSET_ATTEMPTS=6 CHECK_ASSET_COUNT=1
serve "$T/merged" flaky:2
run "CONTROL: the same edge with too few attempts is a failure, so the retries are what passed it" 1 "after 2 attempts" "$T/prev" "$T/new" CHECK_ASSET_ATTEMPTS=2 CHECK_ASSET_COUNT=1

# ---- nothing was dropped -----------------------------------------------------------------------------------------------------------------
serve "$T/merged" serve
run "a release that dropped no script has nothing to look for, and says so" 0 "nothing to look for" "$T/new" "$T/merged"

# ---- the server is not there -------------------------------------------------------------------------------------------------------------
BASE="http://127.0.0.1:1"
run "no server answering is a failure" 1 "answered 000" "$T/prev" "$T/new"

# ---- usage ---------------------------------------------------------------------------------------------------------------------------------
sh "$S" > "$T/out.txt" 2>&1; got=$?
if [ "$got" = 2 ]; then pass=$((pass + 1)); echo "ok   no arguments is a usage error"; else bad=$((bad + 1)); echo "BAD  no arguments: exit $got"; fi
sh "$S" ftp://x "$T/prev" "$T/new" > "$T/out.txt" 2>&1; got=$?
if [ "$got" = 2 ]; then pass=$((pass + 1)); echo "ok   a base URL that is not http is a usage error"; else bad=$((bad + 1)); echo "BAD  bad URL: exit $got"; fi
sh "$S" http://x "$T/nope" "$T/new" > "$T/out.txt" 2>&1; got=$?
if [ "$got" = 2 ]; then pass=$((pass + 1)); echo "ok   a directory that is not there is a usage error"; else bad=$((bad + 1)); echo "BAD  missing dir: exit $got"; fi

echo "passed=$pass bad=$bad"
[ "$bad" = 0 ]
