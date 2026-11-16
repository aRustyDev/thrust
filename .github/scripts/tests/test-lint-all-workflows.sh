#!/bin/sh
# lint-all-workflows.sh: every workflow and action is linted, each workflow with the credentials its name allows, and a workflow
# it does not name gets none. Generated: every workflow name x every kind of credential x every job name, against the policy
# written out again here. Then .yaml files, actions, several files with one bad, no workflow at all, and this repository.
#
#   test-lint-all-workflows.sh [path to a copy of lint-all-workflows.sh]   (a path points it at a deliberately broken copy)
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
WRAP="${1:-$HERE/.github/scripts/lint-all-workflows.sh}"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
pass=0; failn=0
ok() { pass=$((pass + 1)); }
bad() { failn=$((failn + 1)); echo "BAD  $*"; }
# The wrapper finds lint-workflow.sh beside itself, so a copy under test gets the real one beside it.
W="$T/bin"; mkdir -p "$W"; cp "$WRAP" "$W/lint-all-workflows.sh"; cp "$HERE/.github/scripts/lint-workflow.sh" "$W/"

# allowed <file> <kind> <job>: 0 when the policy lets that job have that credential in that file
allowed() {
  case "$2:$1" in
    secrets:deploy.yml | environment:deploy.yml) case "$3" in deploy | deploy-do) return 0 ;; esac ;;
    secrets:release-plz.yml | environment:release-plz.yml) case "$3" in release | release-pr) return 0 ;; esac ;;
    id-token:deploy.yml | id-token:release.yml) [ "$3" = build ] && return 0 ;;
  esac
  return 1
}
workflow() { # workflow <job> <kind>: a workflow that keeps every rule but, possibly, the credential
  printf 'name: t\non: workflow_dispatch\npermissions: {}\njobs:\n  %s:\n    runs-on: ubuntu-24.04\n    permissions:\n      contents: read\n' "$1"
  case "$2" in
    id-token) printf '      id-token: write\n' ;;
    secrets) printf '    env:\n      TOKEN: ${{ secrets.TOKEN }}\n' ;;
    environment) printf '    environment: staging\n' ;;
  esac
  printf '    steps:\n      - run: echo hi\n'
}
n=0
for file in deploy.yml release.yml release-plz.yml ci.yml new.yml; do
  for kind in none secrets environment id-token; do
    for job in deploy deploy-do build release release-pr other; do
      n=$((n + 1)); r="$T/r$n"; mkdir -p "$r/.github/workflows"
      workflow "$job" "$kind" > "$r/.github/workflows/$file"
      want=1
      if [ "$kind" = none ] || allowed "$file" "$kind" "$job"; then want=0; fi
      sh "$W/lint-all-workflows.sh" "$r" > "$T/out" 2>&1; got=$?
      label="$file: $kind in job $job"
      if [ "$got" != "$want" ]; then bad "$label: exit $got, wanted $want"; sed 's/^/     | /' "$T/out"
      elif [ "$want" = 1 ] && ! grep -q ': R7: ' "$T/out"; then bad "$label: refused, but not by R7"; sed 's/^/     | /' "$T/out"
      else ok; fi
    done
  done
done

expect() { # expect <label> <want exit> <fragment> <root>
  sh "$W/lint-all-workflows.sh" "$4" > "$T/out" 2>&1; got=$?
  if [ "$got" = "$2" ] && grep -qF -- "$3" "$T/out"; then ok; else bad "$1: exit $got, wanted $2 + '$3'"; sed 's/^/     | /' "$T/out"; fi
}
r="$T/yaml"; mkdir -p "$r/.github/workflows"; workflow other secrets > "$r/.github/workflows/new.yaml"
expect "a .yaml workflow is linted too" 1 ": R7: " "$r"
r="$T/act"; mkdir -p "$r/.github/workflows" "$r/.github/actions/x"; workflow other none > "$r/.github/workflows/ci.yml"
printf 'name: x\nruns:\n  using: composite\n  steps:\n    - uses: some/action@v1\n' > "$r/.github/actions/x/action.yml"
expect "an action is linted (an unpinned uses)" 1 ": R1: " "$r"
r="$T/two"; mkdir -p "$r/.github/workflows"; workflow other secrets > "$r/.github/workflows/a.yml"; workflow other none > "$r/.github/workflows/b.yml"
expect "one bad workflow among clean ones fails the run" 1 "a.yml" "$r"
r="$T/two2"; mkdir -p "$r/.github/workflows"; workflow other none > "$r/.github/workflows/a.yml"; workflow other secrets > "$r/.github/workflows/b.yml"
expect "a bad workflow after a clean one fails the run" 1 "b.yml" "$r"
r="$T/empty"; mkdir -p "$r/.github/workflows"
expect "no workflow at all is a call gone wrong" 2 "no workflow under" "$r"
# this repository: clean, and every workflow and action file it has is counted
nw="$(find "$HERE/.github/workflows" -maxdepth 1 \( -name '*.yml' -o -name '*.yaml' \) | wc -l | tr -d ' ')"
na="$(find "$HERE/.github/actions" -mindepth 2 -maxdepth 2 \( -name 'action.yml' -o -name 'action.yaml' \) | wc -l | tr -d ' ')"
expect "this repository's workflows and actions are clean, all of them" 0 "$nw workflows and $na actions" "$HERE"

echo "passed=$pass bad=$failn (of $((n + 6)))"
[ "$failn" -eq 0 ]
