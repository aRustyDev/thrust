#!/bin/sh
# lint-actionlint.sh: it fails when actionlint is not looking, and not otherwise. Stand-in actionlint binaries that each misbehave in one
# way (they say nothing, refuse everything, leave out one message, find a fault in the repository), a root that is not a git work tree,
# a missing binary, a missing shellcheck, the shellcheck being handed to actionlint; and, when the real actionlint is on PATH, the real one
# on this repository and on a repository with a broken call, with the real shellcheck and with two that are not looking (one that says
# nothing, one that does not know SC2329, as the runner image's 0.9.0 does not).
# The stand-ins keep this test independent of the version of actionlint and of the runner (ci-scripts has none; deps installs the real one).
#
#   test-lint-actionlint.sh [path to a copy of lint-actionlint.sh]   (a path points it at a deliberately broken copy)
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
S="${1:-$HERE/.github/scripts/lint-actionlint.sh}"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
pass=0; failn=0

expect() { # expect <label> <want exit> <fragment> <binary> <root>   (the shellcheck is $SC_FOR_TEST, a stand-in, unless a case sets it)
  ACTIONLINT="$4" SHELLCHECK="${SC_FOR_TEST:-$T/shellcheck-stub}" sh "$S" "$5" > "$T/out" 2>&1; got=$?
  if [ "$got" = "$2" ] && grep -qF -- "$3" "$T/out"; then
    pass=$((pass + 1))
  else
    failn=$((failn + 1)); echo "BAD  $1: exit $got, wanted $2 + '$3'"; sed 's/^/     | /' "$T/out"
  fi
}
stub() { # stub <name> <body>
  printf '#!/bin/sh\n%s\n' "$2" > "$T/$1"; chmod +x "$T/$1"
}
ALL3='echo "x.yml:1:1: input \"bogus\" is not defined in callee"; echo "x.yml:1:1: input \"needed\" is required by callee"; echo "x.yml:1:1: property \"nope\" is not defined in object type"'

# a repository to lint: a git work tree with one workflow
R="$T/repo"; mkdir -p "$R/.github/workflows"; git -C "$R" init -q; printf 'name: x\n' > "$R/.github/workflows/ci.yml"

SH2086='echo "x.yml:1:1: shellcheck reported issue in this script: SC2086:info:1:6: Double quote to prevent globbing and word splitting [shellcheck]"'
SH2329='echo "x.yml:1:1: shellcheck reported issue in this script: SC2329:info:1:1: This function is never invoked. [shellcheck]"'
ALL5="$ALL3; $SH2086; $SH2329"
SMART_TAIL="if [ -n \"\${STUB_FINDINGS:-}\" ] && ! [ -f .github/workflows/callee.yml ]; then echo '.github/workflows/ci.yml:1:1: a finding'; exit 1; fi
exit 0"
stub smart "if grep -q bogus .github/workflows/caller.yml 2>/dev/null; then $ALL5; exit 1; fi
$SMART_TAIL"
# the same, and it writes down what it was called with
stub argv "printf '%s\\n' \"\$*\" >> \"$T/args\"
if grep -q bogus .github/workflows/caller.yml 2>/dev/null; then $ALL5; exit 1; fi
$SMART_TAIL"
# one that is not looking at the shell in a run: block (no shellcheck given, or the one given says nothing), and one whose shellcheck does not know SC2329
stub no-shell "if grep -q bogus .github/workflows/caller.yml 2>/dev/null; then $ALL3; exit 1; fi
$SMART_TAIL"
stub old-shellcheck "if grep -q bogus .github/workflows/caller.yml 2>/dev/null; then $ALL3; $SH2086; exit 1; fi
$SMART_TAIL"
stub silent "exit 0"
stub refuse-all "$ALL5; exit 1"
stub half "echo 'x.yml:1:1: input \"bogus\" is not defined in callee'; echo 'x.yml:1:1: input \"needed\" is required by callee'; exit 1"
# a shellcheck for the cases that are about something else: it answers --version and is never asked anything else by the stand-in actionlints
stub shellcheck-stub 'echo "version: 9.9.9"'

expect "an actionlint that behaves, on a clean repository" 0 "the control behaved" "$T/smart" "$R"
STUB_FINDINGS=1 expect "a finding in the repository fails the run and is shown" 1 "a finding" "$T/smart" "$R"
STUB_FINDINGS=1 expect "and the run says the workflows have findings" 1 "actionlint has findings in the workflows" "$T/smart" "$R"
unset STUB_FINDINGS   # an assignment before a function call outlives it in some shells: the cases below want a clean repository
expect "an actionlint that says nothing is not looking" 1 "so it is not looking" "$T/silent" "$R"
expect "an actionlint that refuses everything refuses a sound call" 1 "refused a caller that keeps its contract" "$T/refuse-all" "$R"
expect "an actionlint that leaves out one of the three messages" 1 "did not say 'property \"nope\" is not defined'" "$T/half" "$R"
mkdir -p "$T/plain/.github/workflows"; printf 'name: x\n' > "$T/plain/.github/workflows/ci.yml"
expect "a root that is not a git work tree is refused, since actionlint would say nothing there" 2 "is not a git work tree" "$T/smart" "$T/plain"
expect "no actionlint at all is a call gone wrong" 2 "no /nonexistent/actionlint" /nonexistent/actionlint "$R"

# the shell in a run: block (the codes are the ones the repository's own scripts are held to: check-shell.sh)
expect "an actionlint that reads no shell in a run: block is not looking at it" 1 "did not say 'SC2086'" "$T/no-shell" "$R"
expect "and the run says what that means" 1 "not running shellcheck over the blocks" "$T/no-shell" "$R"
expect "an actionlint whose shellcheck does not know SC2329 is older than the repository is held to" 1 "did not say 'SC2329'" "$T/old-shellcheck" "$R"
expect "and the run says so, with the version it was given" 1 ", version 9.9.9) is older than the one check-shell.sh holds the repository to" "$T/old-shellcheck" "$R"
SC_FOR_TEST=/nonexistent/shellcheck; expect "no shellcheck at all is a call gone wrong" 2 "no /nonexistent/shellcheck" "$T/smart" "$R"
expect "and it says what actionlint does without one" 2 "reads no workflow's run: blocks and says nothing" "$T/smart" "$R"
SC_FOR_TEST=
rm -f "$T/args"
expect "an actionlint that is given a shellcheck" 0 "the control behaved" "$T/argv" "$R"
if [ "$(grep -cF -- "-shellcheck $T/shellcheck-stub" "$T/args" 2> /dev/null)" = 3 ]; then
  pass=$((pass + 1))
else
  failn=$((failn + 1)); echo "BAD  the shellcheck was not handed to actionlint with -shellcheck on all three runs (the control's two and the repository's):"; sed 's/^/     | /' "$T/args" 2> /dev/null
fi

# the real actionlint, when this machine has it and a shellcheck that knows SC2329 (the runner image's 0.9.0 does not; ci-scripts has no actionlint)
RSC="$(command -v shellcheck 2> /dev/null)"
knows_sc2329() { printf 'f() { echo hi; }\nexit 0\n' | "$1" --norc -f gcc --shell bash - 2>&1 | grep -q SC2329; }
if command -v actionlint > /dev/null 2>&1 && [ -n "$RSC" ] && knows_sc2329 "$RSC"; then
  SC_FOR_TEST="$RSC"
  expect "the real actionlint: the control behaves and this repository is clean" 0 "the control behaved" actionlint "$HERE"
  # two shellchecks that are not looking: one that does not know SC2329 (the image's 0.9.0 is one: -e removes the report of it, nothing else), one that says nothing
  printf '#!/bin/sh\nexec "%s" -e SC2329 "$@"\n' "$RSC" > "$T/sc-not-knowing"; chmod +x "$T/sc-not-knowing"
  stub sc-silent 'case "$1" in --version) echo "version: 0.0.0" ;; *) echo "[]" ;; esac'
  SC_FOR_TEST="$T/sc-not-knowing"
  expect "the real actionlint with a shellcheck that does not know SC2329 is refused, and says which code it lacked" 1 "did not say 'SC2329'" actionlint "$HERE"
  SC_FOR_TEST="$T/sc-silent"
  expect "the real actionlint with a shellcheck that says nothing is refused, and says which code it lacked" 1 "did not say 'SC2086'" actionlint "$HERE"
  SC_FOR_TEST="$RSC"
  B="$T/broken"; mkdir -p "$B/.github/workflows"; git -C "$B" init -q
  printf 'name: callee\non:\n  workflow_call:\n    inputs:\n      needed:\n        type: string\n        required: true\npermissions: {}\njobs:\n  j:\n    runs-on: ubuntu-24.04\n    permissions: {}\n    steps:\n      - run: echo hi\n' > "$B/.github/workflows/callee.yml"
  printf 'name: c\non: workflow_dispatch\npermissions: {}\njobs:\n  call:\n    uses: ./.github/workflows/callee.yml\n    with:\n      needed: x\n      bogus: y\n    permissions: {}\n' > "$B/.github/workflows/caller.yml"
  expect "the real actionlint finds the undeclared input in a repository that has one" 1 '"bogus" is not defined' actionlint "$B"
  # a finding that only a shellcheck of the repository's version reports, in a workflow of the repository itself (the image's 0.9.0 passes it)
  W="$T/shell-finding"; mkdir -p "$W/.github/workflows"; git -C "$W" init -q
  printf 'name: s\non: workflow_dispatch\npermissions: {}\njobs:\n  j:\n    runs-on: ubuntu-24.04\n    permissions: {}\n    steps:\n      - run: |\n          say() { echo hi; }\n          exit 0\n' > "$W/.github/workflows/s.yml"
  expect "the real actionlint reads a workflow's run: block with the shellcheck it was given: an unused function is a finding" 1 "SC2329" actionlint "$W"
  SC_FOR_TEST=
else
  echo "note: no actionlint on PATH, or no shellcheck that knows SC2329, so the cases that run the real ones were skipped (the deps job runs them)"
fi

echo "passed=$pass bad=$failn"
[ "$failn" -eq 0 ]
