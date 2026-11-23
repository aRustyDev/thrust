#!/bin/sh
# check-ci-index.sh: the indexes of .github/workflows/ and .github/actions/ are complete and true. Built roots, each broken in one way
# (a file with no row, a row with no file, a repeated row, a missing README), the cells and lines that are not rows, and this repository.
# Generated: for each of five workflows, the index without that one row must be refused, naming it.
#
#   test-check-ci-index.sh [path to a copy of check-ci-index.sh]   (a path points it at a deliberately broken copy)
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
S="${1:-$HERE/.github/scripts/check-ci-index.sh}"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
pass=0; failn=0

expect() { # expect <label> <want exit> <fragment> <root>
  sh "$S" "$4" > "$T/out" 2>&1; got=$?
  if [ "$got" = "$2" ] && grep -qF -- "$3" "$T/out"; then
    pass=$((pass + 1))
  else
    failn=$((failn + 1)); echo "BAD  $1: exit $got, wanted $2 + '$3'"; sed 's/^/     | /' "$T/out"
  fi
}

root() { r="$T/$1"; rm -rf "$r"; mkdir -p "$r/.github/workflows" "$r/.github/actions"; echo "$r"; }
wf() { printf 'name: x\n' > "$1/.github/workflows/$2"; }
act() { mkdir -p "$1/.github/actions/$2"; printf 'name: x\nruns:\n  using: composite\n  steps: []\n' > "$1/.github/actions/$2/action.yml"; }
rows() { # rows <README> <name>...: a README with a header and one row for each name
  out="$1"; shift
  printf '# x\n\n| File | Does |\n|---|---|\n' > "$out"
  for item in "$@"; do printf '| `%s` | does a thing |\n' "$item" >> "$out"; done
}
# (not `complete`: that is a bash builtin, and shellcheck reads every call of the name as one, SC3044)
full_root() { # full_root <name>: a root with two workflows and two actions, each with its row
  r="$(root "$1")"; wf "$r" a.yml; wf "$r" b.yml; act "$r" x; act "$r" y
  rows "$r/.github/workflows/README.md" a.yml b.yml; rows "$r/.github/actions/README.md" x y
  echo "$r"
}

r="$(full_root ok)"
expect "a complete index passes" 0 "2 workflows and 2 actions, each with its row" "$r"

r="$(full_root w-no-row)"; rows "$r/.github/workflows/README.md" a.yml
expect "a workflow with no row is refused, and named" 1 'workflows/README.md has no row for `b.yml`' "$r"
r="$(full_root w-extra)"; rows "$r/.github/workflows/README.md" a.yml b.yml gone.yml
expect "a row for a workflow that is not there is refused" 1 'has a row for `gone.yml`, which is not there' "$r"
r="$(full_root w-twice)"; rows "$r/.github/workflows/README.md" a.yml b.yml a.yml
expect "a workflow with two rows is refused" 1 'more than one row for `a.yml`' "$r"
r="$(full_root w-none)"; rm "$r/.github/workflows/README.md"
expect "no workflows README is refused" 1 "workflows has no README.md" "$r"
r="$(full_root a-no-row)"; rows "$r/.github/actions/README.md" x
expect "an action with no row is refused, and named" 1 'actions/README.md has no row for `y`' "$r"
r="$(full_root a-extra)"; rows "$r/.github/actions/README.md" x y gone
expect "a row for an action that is not there is refused" 1 'has a row for `gone`, which is not there' "$r"
r="$(full_root a-twice)"; rows "$r/.github/actions/README.md" x y y
expect "an action with two rows is refused" 1 'more than one row for `y`' "$r"
r="$(full_root a-none)"; rm "$r/.github/actions/README.md"
expect "no actions README is refused" 1 "actions has no README.md" "$r"

# what is not a row
r="$(full_root second-cell)"
printf '# x\n\n| File | Does |\n|---|---|\n| `a.yml` | calls `b.yml` |\n' > "$r/.github/workflows/README.md"
expect "a name in the second cell is not a row" 1 'has no row for `b.yml`' "$r"
r="$(full_root prose)"
printf '# x\n\nSee `b.yml` for details.\n\n| File | Does |\n|---|---|\n| `a.yml` | a |\n' > "$r/.github/workflows/README.md"
expect "a name in prose is not a row" 1 'has no row for `b.yml`' "$r"
r="$(full_root yaml)"; wf "$r" z.yaml
expect "a .yaml workflow needs its row too" 1 'has no row for `z.yaml`' "$r"
r="$(full_root not-an-action)"; mkdir -p "$r/.github/actions/notes"
expect "a directory with no action.yml is not an action" 0 "each with its row" "$r"
r="$(full_root readme-is-no-workflow)"
expect "the README is not a workflow" 0 "2 workflows" "$r"

# a call gone wrong
r="$T/no-workflows"; mkdir -p "$r/.github/workflows" "$r/.github/actions"
expect "no workflow at all is a call gone wrong" 2 "no workflow under" "$r"
r="$T/no-github"; mkdir -p "$r"
expect "a root with no .github is a call gone wrong" 2 "has no .github directory" "$r"

# generated: five workflows, each row dropped in turn
for drop in 1 2 3 4 5; do
  r="$(root "g$drop")"; names=""
  for i in 1 2 3 4 5; do wf "$r" "w$i.yml"; [ "$i" = "$drop" ] || names="$names w$i.yml"; done
  # shellcheck disable=SC2086
  rows "$r/.github/workflows/README.md" $names
  rows "$r/.github/actions/README.md"
  expect "five workflows, the row of w$drop.yml dropped" 1 "has no row for \`w$drop.yml\`" "$r"
done

# this repository: both indexes complete and true
expect "this repository's indexes are complete" 0 "each with its row" "$HERE"

echo "passed=$pass bad=$failn"
[ "$failn" -eq 0 ]
