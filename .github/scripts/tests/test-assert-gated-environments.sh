#!/bin/sh
# .github/scripts/assert-gated-environments.sh against a fake `gh` that serves one JSON file per environment (issue #49). Every expected
# failure asserts a message fragment, so a case cannot fail for the wrong reason, and the cases that must fail sit next to the one that
# passes.
#
#   sh .github/scripts/tests/test-assert-gated-environments.sh
#
# GATED_SCRIPT points the test at a copy of the script, so a deliberately broken copy can be shown to fail it.
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
S="${GATED_SCRIPT:-$HERE/.github/scripts/assert-gated-environments.sh}"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
mkdir -p "$T/bin" "$T/fix"
cat > "$T/bin/gh" <<'EOF'
#!/bin/sh
# fake gh: `gh api repos/o/r/environments/<name>` serves $FIX/<name>.json; $FIX/<name>.status says to fail with an HTTP status instead
case "$*" in
  "api repos/o/r/environments/"*)
    name="${2##*/}"
    if [ -f "$FIX/$name.status" ]; then
      echo "gh: error (HTTP $(cat "$FIX/$name.status"))" >&2
      exit 1
    fi
    if [ -f "$FIX/$name.json" ]; then cat "$FIX/$name.json"; exit 0; fi
    echo "gh: Not Found (HTTP 404)" >&2; exit 1 ;;
  *) echo "fake gh: unexpected: $*" >&2; exit 99 ;;
esac
EOF
chmod +x "$T/bin/gh"
pass=0; bad=0
run() { # run <label> <want-exit> <want-fragment> <environment ...>
  label="$1"; want="$2"; frag="$3"; shift 3
  PATH="$T/bin:$PATH" FIX="$T/fix" "$S" o/r "$@" > "$T/out.txt" 2>&1
  got=$?
  if [ "$got" = "$want" ] && grep -qF -- "$frag" "$T/out.txt"; then
    pass=$((pass + 1)); echo "ok   $label (exit $got; saw '$frag')"
  else
    bad=$((bad + 1)); echo "BAD  $label: wanted exit $want + '$frag', got exit $got"; sed 's/^/     | /' "$T/out.txt"
  fi
}
env_json() { printf '%s' "$2" > "$T/fix/$1.json"; }

GATED='{"name":"x","protection_rules":[{"id":1,"type":"required_reviewers","reviewers":[{"type":"User","reviewer":{"login":"someone"}}]},{"id":2,"type":"branch_policy"}]}'
env_json staging-do-lifecycle "$GATED"
env_json production "$GATED"
env_json production-do-lifecycle "$GATED"
run "one gated environment" 0 "gated: staging-do-lifecycle" staging-do-lifecycle
run "three gated environments" 0 "gated: production production-do-lifecycle" production production-do-lifecycle

env_json open '{"name":"open","protection_rules":[]}'
run "an environment with no protection rules" 1 "open: has no required reviewers" open
env_json timer-only '{"name":"timer-only","protection_rules":[{"id":1,"type":"wait_timer","wait_timer":5}]}'
run "a wait timer is not a reviewer" 1 "timer-only: has no required reviewers" timer-only
env_json empty-reviewers '{"name":"empty-reviewers","protection_rules":[{"id":1,"type":"required_reviewers","reviewers":[]}]}'
run "a reviewers rule with nobody in it" 1 "empty-reviewers: has no required reviewers" empty-reviewers
env_json no-rules-key '{"name":"no-rules-key"}'
run "an environment whose answer has no protection_rules" 1 "no-rules-key: has no required reviewers" no-rules-key
env_json other-rule-with-reviewers '{"name":"other-rule-with-reviewers","protection_rules":[{"id":1,"type":"wait_timer","wait_timer":5,"reviewers":[{"type":"User"}]}]}'
run "pinned: only a required_reviewers rule counts, whatever another rule carries" 1 "other-rule-with-reviewers: has no required reviewers" other-rule-with-reviewers
env_json branches-only '{"name":"branches-only","protection_rules":[{"id":1,"type":"branch_policy"}]}'
run "a branch policy is not a reviewer" 1 "branches-only: has no required reviewers" branches-only

run "an environment that does not exist" 1 "ghost: does not exist" ghost
echo 403 > "$T/fix/forbidden.status"
run "an environment that cannot be read is not 'probably fine'" 1 "forbidden: could not be read" forbidden
echo 502 > "$T/fix/flaky.status"
run "the API failing is not a pass" 1 "flaky: could not be read" flaky
env_json not-json 'this is not json'
run "an answer that is not JSON is not a pass" 1 "not-json: the answer is not the JSON this reads" not-json

run "one ungated environment among gated ones fails the lot, and names it" 1 "open: has no required reviewers" production open production-do-lifecycle
grep -q "1 of 3 environments are not gated" "$T/out.txt" && pass=$((pass + 1)) || { bad=$((bad + 1)); echo "BAD  the count of ungated environments is wrong"; }
run "two problems are both reported" 1 "ghost: does not exist" open ghost
grep -q "open: has no required reviewers" "$T/out.txt" && pass=$((pass + 1)) || { bad=$((bad + 1)); echo "BAD  the second problem was not listed"; }

run "an environment name that is not a name" 2 "is not an environment name" 'a b'
PATH="$T/bin:$PATH" FIX="$T/fix" "$S" o/r > "$T/out.txt" 2>&1; got=$?
if [ "$got" = 2 ]; then pass=$((pass + 1)); echo "ok   no environment named is a usage error"; else bad=$((bad + 1)); echo "BAD  no environment named: exit $got"; fi

echo "passed=$pass bad=$bad"
[ "$bad" = 0 ]
