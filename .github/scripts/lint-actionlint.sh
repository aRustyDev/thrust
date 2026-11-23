#!/bin/sh
# actionlint over every workflow of this repository, and the control that shows it is looking (ADR-0102, research 0018 F11).
#
#   .github/scripts/lint-actionlint.sh [repository root]   (default: this one)
#   ACTIONLINT=<the binary>                                (default: actionlint on PATH)
#   SHELLCHECK=<the binary>                                (default: shellcheck on PATH)
#
# actionlint checks a call of a local reusable workflow (the `with:` it passes against the inputs the callee declares, the outputs it
# reads through `needs`) only when it can find the project, which it does by the git repository. In a plain directory it checks
# nothing, prints nothing and exits 0. So this refuses to run outside a git work tree, and before it lints the repository it
# runs a control in a git repository of its own: a caller that passes an input the callee does not declare, omits one it requires and
# reads an output it does not declare must be refused, with all three messages; a caller that does none of that must pass. If the
# control does not behave, actionlint is not looking, and the run fails whatever the repository holds.
#
# actionlint also reads the shell in every `run:` block, by running shellcheck over it, and only when it is given one: with none on PATH
# it skips the blocks and says nothing, and with the runner image's it reads them with 0.9.0, which does not know the codes that the
# repository's own scripts are held to (check-shell.sh holds them to SHELLCHECK_VERSION; SC2329 is the first it lacks). So this hands
# actionlint the one it was given (-shellcheck), refuses to run without one, and the control also needs two more things of it: an unquoted
# variable in a `run:` block must be refused (SC2086: it is being run), and a function that is never called, in a block that ends in
# `exit`, must be refused (SC2329: it is one that knows the code). A block that quotes and calls what it defines must pass.
#
# Exit status: 0 the control behaved and every workflow is clean; 1 a workflow has a finding, or the control did not behave;
# 2 called wrongly (no binary, no shellcheck, not a git work tree).
set -u
ROOT="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
AL="${ACTIONLINT:-actionlint}"
command -v "$AL" > /dev/null 2>&1 || { echo "lint-actionlint: no $AL (lint.yml's deps job installs a pinned one)" >&2; exit 2; }
SC="${SHELLCHECK:-shellcheck}"
command -v "$SC" > /dev/null 2>&1 \
  || { echo "lint-actionlint: no $SC (lint.yml's deps job installs the pinned one): without it actionlint reads no workflow's run: blocks and says nothing" >&2; exit 2; }
SC="$(command -v "$SC")"
sc_version="$("$SC" --version 2> /dev/null | sed -n 's/^version: //p')"
git -C "$ROOT" rev-parse --is-inside-work-tree > /dev/null 2>&1 \
  || { echo "lint-actionlint: $ROOT is not a git work tree, where actionlint would check no call and say nothing" >&2; exit 2; }
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM

# The control: one callee, one caller that keeps its contract and one that breaks it three ways. Each in a git repository of its own.
callee() { # callee <dir>
  mkdir -p "$1/.github/workflows"
  git -C "$1" init -q
  cat > "$1/.github/workflows/callee.yml" <<'EOF'
name: callee
on:
  workflow_call:
    inputs:
      needed:
        type: string
        required: true
    outputs:
      made:
        value: ${{ jobs.j.outputs.made }}
permissions: {}
jobs:
  j:
    runs-on: ubuntu-24.04
    permissions: {}
    outputs:
      made: ${{ steps.s.outputs.made }}
    steps:
      - id: s
        run: echo "made=1" >> "$GITHUB_OUTPUT"
EOF
}
callee "$T/good"
cat > "$T/good/.github/workflows/caller.yml" <<'EOF'
name: caller
on: workflow_dispatch
permissions: {}
jobs:
  call:
    uses: ./.github/workflows/callee.yml
    with:
      needed: x
    permissions: {}
  after:
    needs: call
    runs-on: ubuntu-24.04
    permissions: {}
    env:
      MADE: ${{ needs.call.outputs.made }}
    steps:
      - run: echo "$MADE"
  shell:
    runs-on: ubuntu-24.04
    permissions: {}
    env:
      WORDS: a b
    steps:
      - name: a block that quotes its variable and calls the function it defines
        run: |
          say() { echo "$WORDS"; }
          say
          exit 0
EOF
callee "$T/bad"
cat > "$T/bad/.github/workflows/caller.yml" <<'EOF'
name: caller
on: workflow_dispatch
permissions: {}
jobs:
  undeclared:
    uses: ./.github/workflows/callee.yml
    with:
      needed: x
      bogus: nobody-declared-this
    permissions: {}
  missing:
    uses: ./.github/workflows/callee.yml
    permissions: {}
  reads-an-output-that-is-not-there:
    needs: undeclared
    runs-on: ubuntu-24.04
    permissions: {}
    env:
      NOPE: ${{ needs.undeclared.outputs.nope }}
    steps:
      - run: echo "$NOPE"
  unquoted:
    runs-on: ubuntu-24.04
    permissions: {}
    env:
      WORDS: a b
    steps:
      - name: a block with a variable that is not quoted
        run: echo $WORDS
  never-called:
    runs-on: ubuntu-24.04
    permissions: {}
    steps:
      - name: a block that defines a function and never calls it
        run: |
          say() { echo hi; }
          echo finished
          exit 0
EOF
control_failed=0
( cd "$T/good" && "$AL" -shellcheck "$SC" ) > "$T/good.out" 2>&1; good=$?
( cd "$T/bad" && "$AL" -shellcheck "$SC" ) > "$T/bad.out" 2>&1; bad=$?
if [ "$good" != 0 ]; then
  echo "lint-actionlint: the control failed: actionlint refused a caller that keeps its contract (exit $good):" >&2
  sed 's/^/  | /' "$T/good.out" >&2
  control_failed=1
fi
if [ "$bad" = 0 ]; then
  echo "lint-actionlint: the control failed: actionlint accepted a caller that breaks its contract three ways, so it is not looking" >&2
  control_failed=1
else
  for want in '"bogus" is not defined' '"needed" is required' 'property "nope" is not defined'; do
    grep -qF -- "$want" "$T/bad.out" || {
      echo "lint-actionlint: the control failed: actionlint did not say '$want' of the caller that breaks its contract:" >&2
      sed 's/^/  | /' "$T/bad.out" >&2
      control_failed=1
    }
  done
  # the shell in a run: block, which only the shellcheck it was given can read: shellcheck is run at all, and it is one that knows SC2329
  grep -qF -- 'SC2086' "$T/bad.out" || {
    echo "lint-actionlint: the control failed: actionlint did not say 'SC2086' of a run: block with an unquoted variable, so it is not running shellcheck over the blocks ($SC, version ${sc_version:-unknown}):" >&2
    sed 's/^/  | /' "$T/bad.out" >&2
    control_failed=1
  }
  grep -qF -- 'SC2329' "$T/bad.out" || {
    echo "lint-actionlint: the control failed: actionlint did not say 'SC2329' of a run: block with a function it never calls, so the shellcheck it was given ($SC, version ${sc_version:-unknown}) is older than the one check-shell.sh holds the repository to:" >&2
    sed 's/^/  | /' "$T/bad.out" >&2
    control_failed=1
  }
fi
[ "$control_failed" = 0 ] || exit 1

# the repository itself
( cd "$ROOT" && "$AL" -shellcheck "$SC" ) > "$T/repo.out" 2>&1; repo=$?
if [ "$repo" != 0 ]; then
  cat "$T/repo.out" >&2
  echo "lint-actionlint: actionlint has findings in the workflows of $ROOT" >&2
  exit 1
fi
echo "lint-actionlint: the control behaved (a broken call refused three ways, an unquoted variable and a function never called refused, a sound caller accepted; shellcheck ${sc_version:-unknown}) and every workflow of $ROOT is clean"
