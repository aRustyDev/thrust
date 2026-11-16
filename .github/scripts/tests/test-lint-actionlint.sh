#!/bin/sh
# lint-actionlint.sh: it fails when actionlint is not looking, and not otherwise. Stand-in actionlint binaries that each misbehave in one
# way (they say nothing, refuse everything, leave out one message, find a fault in the repository), a root that is not a git work tree,
# a missing binary; and, when the real actionlint is on PATH, the real one on this repository and on a repository with a broken call.
# The stand-ins keep this test independent of the version of actionlint and of the runner (ci-scripts has none; deps installs the real one).
#
#   test-lint-actionlint.sh [path to a copy of lint-actionlint.sh]   (a path points it at a deliberately broken copy)
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
S="${1:-$HERE/.github/scripts/lint-actionlint.sh}"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
pass=0; failn=0

expect() { # expect <label> <want exit> <fragment> <binary> <root>
  ACTIONLINT="$4" sh "$S" "$5" > "$T/out" 2>&1; got=$?
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

stub smart "if grep -q bogus .github/workflows/caller.yml 2>/dev/null; then $ALL3; exit 1; fi
if [ -n \"\${STUB_FINDINGS:-}\" ] && ! [ -f .github/workflows/callee.yml ]; then echo '.github/workflows/ci.yml:1:1: a finding'; exit 1; fi
exit 0"
stub silent "exit 0"
stub refuse-all "$ALL3; exit 1"
stub half "echo 'x.yml:1:1: input \"bogus\" is not defined in callee'; echo 'x.yml:1:1: input \"needed\" is required by callee'; exit 1"

expect "an actionlint that behaves, on a clean repository" 0 "the control behaved" "$T/smart" "$R"
STUB_FINDINGS=1 expect "a finding in the repository fails the run and is shown" 1 "a finding" "$T/smart" "$R"
STUB_FINDINGS=1 expect "and the run says the workflows have findings" 1 "actionlint has findings in the workflows" "$T/smart" "$R"
expect "an actionlint that says nothing is not looking" 1 "so it is not looking" "$T/silent" "$R"
expect "an actionlint that refuses everything refuses a sound call" 1 "refused a caller that keeps its contract" "$T/refuse-all" "$R"
expect "an actionlint that leaves out one of the three messages" 1 "did not say 'property \"nope\" is not defined'" "$T/half" "$R"
mkdir -p "$T/plain/.github/workflows"; printf 'name: x\n' > "$T/plain/.github/workflows/ci.yml"
expect "a root that is not a git work tree is refused, since actionlint would say nothing there" 2 "is not a git work tree" "$T/smart" "$T/plain"
expect "no actionlint at all is a call gone wrong" 2 "no /nonexistent/actionlint" /nonexistent/actionlint "$R"

# the real actionlint, when this machine has it
if command -v actionlint > /dev/null 2>&1; then
  expect "the real actionlint: the control behaves and this repository is clean" 0 "the control behaved" actionlint "$HERE"
  B="$T/broken"; mkdir -p "$B/.github/workflows"; git -C "$B" init -q
  printf 'name: callee\non:\n  workflow_call:\n    inputs:\n      needed:\n        type: string\n        required: true\npermissions: {}\njobs:\n  j:\n    runs-on: ubuntu-24.04\n    permissions: {}\n    steps:\n      - run: echo hi\n' > "$B/.github/workflows/callee.yml"
  printf 'name: c\non: workflow_dispatch\npermissions: {}\njobs:\n  call:\n    uses: ./.github/workflows/callee.yml\n    with:\n      needed: x\n      bogus: y\n    permissions: {}\n' > "$B/.github/workflows/caller.yml"
  expect "the real actionlint finds the undeclared input in a repository that has one" 1 '"bogus" is not defined' actionlint "$B"
else
  echo "note: no actionlint on PATH, so the two cases that run the real one were skipped (the deps job runs it)"
fi

echo "passed=$pass bad=$failn"
[ "$failn" -eq 0 ]
