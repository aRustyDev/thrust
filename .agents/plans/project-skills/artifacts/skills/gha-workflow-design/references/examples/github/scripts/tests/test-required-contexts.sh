#!/bin/sh
# Test assert-required-contexts.sh against a fake `gh` that serves fixtures.
# Fixtures live in <case>/fixtures, the script runs in <case>/work. Every expected failure also asserts a
# message fragment, so a test cannot pass for the wrong reason.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
S="$HERE/.github/scripts/assert-required-contexts.sh"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/bin"
cat > "$T/bin/gh" <<'EOF'
#!/bin/sh
# fake gh: $FIX holds rules.json, rollup.json and, optionally, classic.json (HTTP 200) or classic.status
# (an HTTP status to fail with, or the word `network` for a failure with no HTTP response at all).
case "$*" in
  "api repos/o/r/rules/branches/main"*) cat "$FIX/rules.json" ;;
  "api -i repos/o/r/branches/main/protection/required_status_checks"*)
    if [ -f "$FIX/classic.json" ]; then
      printf 'HTTP/2.0 200 OK\r\nContent-Type: application/json\r\n\r\n'
      cat "$FIX/classic.json"
    else
      status="404"
      [ -f "$FIX/classic.status" ] && status="$(cat "$FIX/classic.status")"
      if [ "$status" = "network" ]; then
        echo "gh: error connecting to api.github.com" >&2
      else
        printf 'HTTP/2.0 %s Error\r\nContent-Type: application/json\r\n\r\n{"message":"error"}' "$status"
        echo "gh: error (HTTP $status)" >&2
      fi
      exit 1
    fi ;;
  "pr view 7 -R o/r --json statusCheckRollup"*) cat "$FIX/rollup.json" ;;
  *) echo "fake gh: unexpected: $*" >&2; exit 99 ;;
esac
EOF
chmod +x "$T/bin/gh"
pass=0; bad=0
run() { # run <label> <want-exit> <want-fragment> <case-dir> [STRICT_CONTEXTS]
  label="$1"; want="$2"; frag="$3"; cs="$4"; strict="${5:-}"
  ( cd "$cs/work" && PATH="$T/bin:$PATH" FIX="$cs/fixtures" STRICT_CONTEXTS="$strict" "$S" o/r 7 main ) > "$cs/out.txt" 2>&1
  got=$?
  if [ "$got" = "$want" ] && grep -qF -- "$frag" "$cs/out.txt"; then
    pass=$((pass + 1)); echo "ok   $label (exit $got; saw '$frag')"
  else
    bad=$((bad + 1)); echo "BAD  $label: wanted exit $want + '$frag', got exit $got"; sed 's/^/     | /' "$cs/out.txt"
  fi
}
mk() { mkdir -p "$T/$1/fixtures" "$T/$1/work"; echo "$T/$1"; }

RULES='[{"type":"deletion"},{"type":"required_status_checks","parameters":{"required_status_checks":[{"context":"gate"},{"context":"docs"}]}}]'
ROLL_OK='{"statusCheckRollup":[{"__typename":"CheckRun","name":"gate","status":"COMPLETED","conclusion":"SUCCESS"},{"__typename":"CheckRun","name":"docs","status":"COMPLETED","conclusion":"SKIPPED"},{"__typename":"StatusContext","context":"some-bot","state":"SUCCESS"}]}'

c="$(mk all-green)"; echo "$RULES" > "$c/fixtures/rules.json"; echo "$ROLL_OK" > "$c/fixtures/rollup.json"
run "all required contexts reported and green (classic protection answers 404)" 0 "certified: all 2 required contexts" "$c"

c="$(mk never-ran)"; echo "$RULES" > "$c/fixtures/rules.json"
echo '{"statusCheckRollup":[{"__typename":"StatusContext","context":"bot-a","state":"SUCCESS"},{"__typename":"CheckRun","name":"bot-b","status":"COMPLETED","conclusion":"SUCCESS"}]}' > "$c/fixtures/rollup.json"
run "required contexts never reported (only unrelated checks are green)" 1 "2 of 2 required contexts are absent" "$c"

c="$(mk half-ran)"; echo "$RULES" > "$c/fixtures/rules.json"
echo '{"statusCheckRollup":[{"__typename":"CheckRun","name":"gate","status":"COMPLETED","conclusion":"SUCCESS"}]}' > "$c/fixtures/rollup.json"
run "one required context never reported while the other is green" 1 "1 of 2 required contexts are absent" "$c"

c="$(mk failing)"; echo "$RULES" > "$c/fixtures/rules.json"
echo '{"statusCheckRollup":[{"__typename":"CheckRun","name":"gate","status":"COMPLETED","conclusion":"FAILURE"},{"__typename":"CheckRun","name":"docs","status":"COMPLETED","conclusion":"SUCCESS"}]}' > "$c/fixtures/rollup.json"
run "a required context failed" 1 "gate = FAILURE" "$c"

c="$(mk in-progress)"; echo "$RULES" > "$c/fixtures/rules.json"
echo '{"statusCheckRollup":[{"__typename":"CheckRun","name":"gate","status":"IN_PROGRESS","conclusion":""},{"__typename":"CheckRun","name":"docs","status":"COMPLETED","conclusion":"SUCCESS"}]}' > "$c/fixtures/rollup.json"
run "a required context is still running" 1 "gate = IN_PROGRESS" "$c"

c="$(mk no-required)"; echo '[{"type":"deletion"}]' > "$c/fixtures/rules.json"; echo "$ROLL_OK" > "$c/fixtures/rollup.json"
run "no required checks configured" 1 "no required status checks are configured" "$c"

c="$(mk classic-only)"; echo '[]' > "$c/fixtures/rules.json"
echo '{"contexts":["gate"],"checks":[{"context":"docs","app_id":15368}]}' > "$c/fixtures/classic.json"; echo "$ROLL_OK" > "$c/fixtures/rollup.json"
run "classic protection only (contexts + checks)" 0 "certified: all 2 required contexts" "$c"

c="$(mk empty-rollup)"; echo "$RULES" > "$c/fixtures/rules.json"; echo '{"statusCheckRollup":[]}' > "$c/fixtures/rollup.json"
run "nothing reported at all" 1 "2 of 2 required contexts are absent" "$c"

# The classic-protection lookup must not be read as "none" unless it is a 404.
c="$(mk classic-403)"; echo "$RULES" > "$c/fixtures/rules.json"; echo "$ROLL_OK" > "$c/fixtures/rollup.json"; echo 403 > "$c/fixtures/classic.status"
run "classic protection lookup answers 403: unknown, not none" 1 "HTTP 403" "$c"

c="$(mk classic-502)"; echo "$RULES" > "$c/fixtures/rules.json"; echo "$ROLL_OK" > "$c/fixtures/rollup.json"; echo 502 > "$c/fixtures/classic.status"
run "classic protection lookup answers 502" 1 "HTTP 502" "$c"

c="$(mk classic-network)"; echo "$RULES" > "$c/fixtures/rules.json"; echo "$ROLL_OK" > "$c/fixtures/rollup.json"; echo network > "$c/fixtures/classic.status"
run "classic protection lookup fails with no HTTP response" 1 "HTTP unknown" "$c"

# SKIPPED semantics: GitHub accepts a skipped check for a required context; an aggregate must not be skipped.
c="$(mk gate-skipped)"; echo "$RULES" > "$c/fixtures/rules.json"
echo '{"statusCheckRollup":[{"__typename":"CheckRun","name":"gate","status":"COMPLETED","conclusion":"SKIPPED"},{"__typename":"CheckRun","name":"docs","status":"COMPLETED","conclusion":"SUCCESS"}]}' > "$c/fixtures/rollup.json"
run "a skipped gate is certified by default (GitHub semantics)" 0 "certified: all 2 required contexts" "$c"
run "a skipped gate is NOT certified when named in STRICT_CONTEXTS" 1 "gate = SKIPPED" "$c" "gate"
c="$(mk strict-docs-skipped)"; echo "$RULES" > "$c/fixtures/rules.json"; echo "$ROLL_OK" > "$c/fixtures/rollup.json"
run "STRICT_CONTEXTS=gate leaves other contexts alone (gate SUCCESS, docs SKIPPED)" 0 "certified: all 2 required contexts" "$c" "gate"
echo "passed=$pass bad=$bad"
[ "$bad" -eq 0 ]
