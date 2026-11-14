#!/bin/sh
# .github/scripts/assert-sha-checks.sh against a fake `gh` that serves a check-runs listing (issue #49). Every expected failure also
# asserts a message fragment, so a case cannot fail for the wrong reason, and the cases that must fail sit next to the one that passes.
#
#   sh .github/scripts/tests/test-assert-sha-checks.sh
#
# ASSERT_SCRIPT points the test at a copy of the script, so a deliberately broken copy can be shown to fail it.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
S="${ASSERT_SCRIPT:-$HERE/.github/scripts/assert-sha-checks.sh}"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
mkdir -p "$T/bin"
cat > "$T/bin/gh" <<'EOF'
#!/bin/sh
# fake gh: serves $FIX/pages.json for the check-runs request, or fails as $FIX/mode says
case "$*" in
  "api repos/o/r/commits/$WANT_SHA/check-runs?per_page=100 --paginate")
    if [ "$(cat "$FIX/mode" 2> /dev/null)" = "gh-fails" ]; then echo "gh: HTTP 502" >&2; exit 1; fi
    cat "$FIX/pages.json" ;;
  *) echo "fake gh: unexpected: $*" >&2; exit 99 ;;
esac
EOF
chmod +x "$T/bin/gh"
pass=0; bad=0
SHA=0123456789abcdef0123456789abcdef01234567

run() { # run <label> <want-exit> <want-fragment> <fixture-dir> [names...]
  label="$1"; want="$2"; frag="$3"; fx="$4"; shift 4
  PATH="$T/bin:$PATH" FIX="$fx" WANT_SHA="$SHA" "$S" o/r "$SHA" "$@" > "$fx/out.txt" 2>&1
  got=$?
  if [ "$got" = "$want" ] && grep -qF -- "$frag" "$fx/out.txt"; then
    pass=$((pass + 1)); echo "ok   $label (exit $got; saw '$frag')"
  else
    bad=$((bad + 1)); echo "BAD  $label: wanted exit $want + '$frag', got exit $got"; sed 's/^/     | /' "$fx/out.txt"
  fi
}
mk() { mkdir -p "$T/$1"; echo "$T/$1"; }
run_json() { # run_json <name> <json> -> a fixture dir whose one page is the json
  d="$(mk "$1")"; printf '%s' "$2" > "$d/pages.json"; echo "$d"
}
cr() { printf '{"name":"%s","status":"%s","conclusion":%s}' "$1" "$2" "$3"; }

OK="$(cr gate completed '"success"')"
OTHER="$(cr docs completed '"success"')"

d="$(run_json green "{\"total_count\":2,\"check_runs\":[$OK,$OTHER]}")"
run "gate reported and SUCCESS" 0 "certified: gate reported" "$d"

d="$(run_json never-ran "{\"total_count\":1,\"check_runs\":[$OTHER]}")"
run "gate never reported (only an unrelated check is green)" 1 "gate: never reported" "$d"

d="$(run_json empty '{"total_count":0,"check_runs":[]}')"
run "no check runs at all" 1 "gate: never reported" "$d"

d="$(run_json failed "{\"total_count\":2,\"check_runs\":[$(cr gate completed '"failure"'),$OTHER]}")"
run "gate failed" 1 "gate: FAILURE" "$d"

d="$(run_json skipped "{\"total_count\":1,\"check_runs\":[$(cr gate completed '"skipped"')]}")"
run "a SKIPPED gate is not a pass" 1 "gate: SKIPPED" "$d"

d="$(run_json cancelled "{\"total_count\":1,\"check_runs\":[$(cr gate completed '"cancelled"')]}")"
run "a cancelled gate is not a pass" 1 "gate: CANCELLED" "$d"

d="$(run_json neutral "{\"total_count\":1,\"check_runs\":[$(cr gate completed '"neutral"')]}")"
run "a neutral gate is not a pass" 1 "gate: NEUTRAL" "$d"

d="$(run_json running "{\"total_count\":1,\"check_runs\":[$(cr gate in_progress null)]}")"
run "a gate that has not finished (no conclusion) is not a pass" 1 "gate: IN_PROGRESS" "$d"

d="$(run_json queued "{\"total_count\":1,\"check_runs\":[$(cr gate queued null)]}")"
run "a queued gate is not a pass" 1 "gate: QUEUED" "$d"

d="$(run_json two-rows "{\"total_count\":2,\"check_runs\":[$OK,$(cr gate completed '"failure"')]}")"
run "two reports of gate, one of them failed" 1 "gate: FAILURE" "$d"

d="$(run_json two-green "{\"total_count\":2,\"check_runs\":[$OK,$OK]}")"
run "two reports of gate, both SUCCESS" 0 "certified" "$d"

# the pages `gh api --paginate` prints: one JSON object after another
d="$(mk paginated)"
printf '{"total_count":3,"check_runs":[%s]}\n{"total_count":3,"check_runs":[%s]}\n' "$OTHER" "$OK" > "$d/pages.json"
run "gate on the second page" 0 "certified" "$d"
d="$(mk paginated-bad)"
printf '{"total_count":3,"check_runs":[%s]}\n{"total_count":3,"check_runs":[%s]}\n' "$OK" "$(cr gate completed '"failure"')" > "$d/pages.json"
run "a failing report of gate on the second page" 1 "gate: FAILURE" "$d"

# more than one name: all of them, or none
d="$(run_json two-names "{\"total_count\":2,\"check_runs\":[$OK,$OTHER]}")"
run "two names, both green" 0 "certified: gate docs" "$d" gate docs
run "two names, one that never reported" 1 "lint: never reported" "$d" gate lint
run "two names, none reported" 1 "2 of 2 required checks are absent" "$d" nope nada

# the service failing is not a pass
d="$(mk gh-fails)"; echo "gh-fails" > "$d/mode"; echo "{}" > "$d/pages.json"
run "gh failing is not a pass" 1 "could not read the check runs" "$d"
d="$(run_json not-json 'this is not json')"
run "an answer that is not JSON is not a pass" 1 "not the JSON this reads" "$d"
d="$(run_json no-check-runs-key '{"message":"Not Found"}')"
run "an answer without check_runs is not a pass" 1 "not the JSON this reads" "$d"

# usage
PATH="$T/bin:$PATH" "$S" o/r not-a-sha > "$T/out.txt" 2>&1; got=$?
if [ "$got" = 2 ] && grep -q "not a commit sha" "$T/out.txt"; then pass=$((pass + 1)); echo "ok   a sha that is not a sha is a usage error"; else bad=$((bad + 1)); echo "BAD  a bad sha: exit $got"; fi
PATH="$T/bin:$PATH" "$S" o/r 0123456789abcdef > "$T/out.txt" 2>&1; got=$?
if [ "$got" = 2 ]; then pass=$((pass + 1)); echo "ok   a short sha is a usage error"; else bad=$((bad + 1)); echo "BAD  a short sha: exit $got"; fi

echo "passed=$pass bad=$bad"
[ "$bad" = 0 ]
