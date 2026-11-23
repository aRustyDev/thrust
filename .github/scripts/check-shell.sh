#!/bin/sh
# The shell this repository maintains, run through shellcheck: the `bash` job of lint.yml (Lint phase, stage 1;
# .agents/plans/ci-workflows/phases/0005-lint.md). One script for CI and for a laptop, so that what fails there fails here.
#
#   .github/scripts/check-shell.sh                the control, then every maintained script and every composite action's run blocks
#   .github/scripts/check-shell.sh --list         the scripts it would check, one to a line, and nothing else
#   .github/scripts/check-shell.sh --list-actions the composite actions whose `run:` blocks it would check, one to a line
#   .github/scripts/check-shell.sh --self-test    the control alone
#   SHELLCHECK=<binary>, YQ=<binary>, JQ=<binary>  (default: each on PATH)
#   CHECK_SHELL_ANY_VERSION=1                     do not hold the binary to SHELLCHECK_VERSION (for the tests' stand-in, which has none)
#
# WHAT. Every file git tracks that is a shell script: a `*.sh`, or an executable whose first line is the shebang of sh, bash, dash or ksh
# (`scripts/rs` has no extension, and a `*.sh` glob would miss it), less what is not ours to lint (frozen(), below).
#
# AND the shell inside the composite actions (`.github/actions/*/action.yml`), which actionlint does not read (it takes an action.yml for a
# workflow and fails on it): each `run:` block is cut out with yq and jq (both are on the runner image) and checked in the dialect of its
# `shell:`. The number of blocks yq finds in an action is held to the number of `run:` keys grep finds in it, so that an extraction that
# dropped the blocks cannot pass as an action with nothing to say.
#
# CONFIGURATION. The repository's .shellcheckrc, handed over with --rcfile so that CI, a laptop and the control read one file: every level
# fails (as it does for the shell in a workflow's `run:`, which actionlint runs shellcheck over), SC2016 and SC2015 are off, and what a
# script sources is followed.
#
# VERSION. Exactly SHELLCHECK_VERSION, which lint.yml installs at the same number (test_workflow_wiring.py holds the two equal). The runner
# image ships 0.9.0, which does not know SC2329 (a function that is never invoked); a laptop has whatever its package manager has. A
# version nobody chose makes a finding appear on one side of a push and not the other.
#
# THE CONTROL (AGENTS.md rule 8: a check that cannot fail is not evidence). Before the repository is read, scripts in a directory of their
# own are given to the same binary and configuration: one with an unquoted variable (an `info` finding, SC2086), which must be refused; one
# with a single-quoted `$` (SC2016), which must pass, because it is the configuration that switches it off; a clean one, which must pass;
# and two composite actions, one whose run block has the unquoted variable (refused, and both of its blocks found) and a clean one (passed).
# A shellcheck that is not looking, a configuration that was not read, one that switched a check off, and an extraction that finds nothing
# each show here.
#
# Exit status: 0 the control behaved and every script is clean; 1 a script has a finding, or the control did not behave; 2 called wrongly
# (no shellcheck, yq or jq, the wrong version, not a git work tree, no script found: a check of nothing would pass).
set -u
LC_ALL=C
export LC_ALL

SHELLCHECK_VERSION="0.11.0"
SC="${SHELLCHECK:-shellcheck}"
YQ="${YQ:-yq}"
JQ="${JQ:-jq}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
RC="$ROOT/.shellcheckrc"
TAB="$(printf '\t')"

die() { echo "check-shell: $1" >&2; exit "${2:-1}"; }

# frozen <path>: succeeds for a file that is not ours to lint. The stock web app and its dependencies have their own pipeline. The plans'
# scripts, the spikes, the probes and the zxcvbn measurement are the records of finished experiments: they say what was run, and are not
# edited to satisfy a linter.
frozen() {
  case "$1" in
    web/* | node_modules/* | */node_modules/*) return 0 ;;
    .agents/plans/* | tools/spike-* | tools/measure-zxcvbn/* | probes/*) return 0 ;;
  esac
  return 1
}

# shell_shebang <file>: succeeds when the first line is the shebang of a shell: `#!/bin/sh`, `#!/usr/bin/env bash`, and so on.
shell_shebang() {
  first=
  IFS= read -r first < "$1" || [ -n "$first" ]
  case "$first" in '#!'*) ;; *) return 1 ;; esac
  # shellcheck disable=SC2086 # word splitting is the point: the interpreter, then its arguments
  set -- ${first#\#!}
  interp="${1:-}"
  interp="${interp##*/}"
  if [ "$interp" = env ]; then
    shift
    while case "${1:-}" in -*) true ;; *) false ;; esac; do shift; done
    interp="${1:-}"
    interp="${interp##*/}"
  fi
  case "$interp" in sh | bash | dash | ksh) return 0 ;; esac
  return 1
}

# list: the scripts to check, one to a line, as git tracks them (a path relative to the root)
list() {
  git -C "$ROOT" ls-files -s | while IFS="$TAB" read -r meta path; do
    frozen "$path" && continue
    case "$path" in
      *.sh) printf '%s\n' "$path" ;;
      *) case "$meta" in "100755 "*) shell_shebang "$ROOT/$path" && printf '%s\n' "$path" ;; esac ;;
    esac
  done
}

# list_actions: the composite actions, as git tracks them
list_actions() { git -C "$ROOT" ls-files '.github/actions/*/action.yml'; }

# check_blocks <base> <list>: every `run:` block of the actions that file lists (paths under <base>), written out and checked in the dialect of
# its `shell:`. A finding is printed under the action, the line its block starts at and the step's name (line 1 of shellcheck's report is
# the line after it). Sets NB to the number of blocks. Returns 0 clean, 1 when a block has a finding, and 2 with BERR set when yq cannot read an
# action, when yq and grep disagree on how many blocks it has, or when a block is in a shell shellcheck does not read.
check_blocks() {
  bb="$T/blocks"; mkdir -p "$bb"; NB=0; bstatus=0; BERR=
  while IFS= read -r action; do
    "$YQ" -o=json -I=0 '.runs.steps[] | select(has("run")) | {"shell": (.shell // "bash"), "run": .run, "line": (.run | line), "name": (.name // "")}' "$1/$action" > "$bb/steps" \
      || { BERR="yq cannot read the steps of $action"; return 2; }
    found="$(wc -l < "$bb/steps" | tr -d ' ')"
    expected="$(grep -c -E '^[[:space:]]+run:' "$1/$action")"
    [ "$found" = "$expected" ] || { BERR="$action: yq found $found run blocks and grep found $expected run: keys, so the extraction dropped some"; return 2; }
    while IFS= read -r step; do
      NB=$((NB + 1))
      printf '%s' "$step" | "$JQ" -r '.run' > "$bb/$NB.sh"
      shell="$(printf '%s' "$step" | "$JQ" -r '.shell')"
      case "$shell" in sh | bash) ;; *) BERR="$action: a step runs in $shell, which shellcheck does not read (sh and bash only)"; return 2 ;; esac
      if ! out="$("$SC" --rcfile="$RC" -s "$shell" "$bb/$NB.sh" 2>&1)"; then
        bstatus=1
        printf 'check-shell: %s, the run block at line %s (%s), whose line 1 is the next line:\n%s\n' "$action" \
          "$(printf '%s' "$step" | "$JQ" -r '.line')" "$(printf '%s' "$step" | "$JQ" -r '.name')" "$out" >&2
      fi
    done < "$bb/steps"
  done < "$2"
  return "$bstatus"
}

control() {
  t="$T/control"; mkdir -p "$t/bad" "$t/good"
  cat > "$t/bad.sh" << 'EOF'
#!/bin/sh
x="a b"
ls $x
EOF
  cat > "$t/idiom.sh" << 'EOF'
#!/bin/sh
echo '$HOME'
EOF
  cat > "$t/good.sh" << 'EOF'
#!/bin/sh
x="a b"
ls "$x"
EOF
  out="$("$SC" --rcfile="$RC" "$t/bad.sh" 2>&1)" \
    && die "control: $SC passed a script with an unquoted variable (SC2086, an info finding): it is not looking, or the configuration switched the check off"
  case "$out" in *SC2086*) ;; *) die "control: $SC refused the script with the unquoted variable, but not for SC2086: $out" ;; esac
  out="$("$SC" --rcfile="$RC" "$t/idiom.sh" 2>&1)" \
    || die "control: $SC refused a single-quoted \$ (SC2016), which $RC switches off: the configuration was not read, or it no longer does: $out"
  out="$("$SC" --rcfile="$RC" "$t/good.sh" 2>&1)" \
    || die "control: $SC refused a clean script: $out"
  # the composite actions: one with two blocks (a block scalar and a one-line `run:`), the first with the unquoted variable; one clean
  cat > "$t/bad/action.yml" << 'EOF'
name: bad
description: a composite action whose first run block has an unquoted variable
runs:
  using: composite
  steps:
    - shell: bash
      run: |
        x="a b"
        ls $x
    - shell: bash
      run: echo hi
EOF
  cat > "$t/good/action.yml" << 'EOF'
name: good
description: a composite action with a clean run block
runs:
  using: composite
  steps:
    - shell: bash
      run: echo "$HOME"
EOF
  printf 'bad/action.yml\n' > "$t/bad.list"
  printf 'good/action.yml\n' > "$t/good.list"
  check_blocks "$t" "$t/bad.list" 2> "$t/bad.err"; st=$?
  [ "$st" != 2 ] || die "control: $BERR"
  [ "$st" = 1 ] || die "control: a composite action whose run block has an unquoted variable was passed: the blocks are not cut out, or not read"
  case "$(cat "$t/bad.err")" in *SC2086*) ;; *) die "control: that action was refused, but not for SC2086: $(cat "$t/bad.err")" ;; esac
  [ "$NB" = 2 ] || die "control: $NB run blocks were found in an action that has two"
  check_blocks "$t" "$t/good.list" 2> "$t/good.err"; st=$?
  [ "$st" != 2 ] || die "control: $BERR"
  [ "$st" = 0 ] || die "control: a clean composite action was refused: $(cat "$t/good.err")"
  [ "$NB" = 1 ] || die "control: $NB run blocks were found in an action that has one"
  echo "check-shell: control ok (shellcheck $version refused an unquoted variable, honoured $RC, passed a clean script, and read the run blocks of two composite actions)"
}

mode=check
case "${1:-}" in
  "") ;;
  --list) mode=list ;;
  --list-actions) mode=list-actions ;;
  --self-test) mode=self-test ;;
  *) die "usage: check-shell.sh [--list | --list-actions | --self-test]" 2 ;;
esac

git -C "$ROOT" rev-parse --is-inside-work-tree > /dev/null 2>&1 || die "$ROOT is not a git work tree, so there is no list of tracked scripts to read" 2
case "$mode" in
  list) list; exit 0 ;;
  list-actions) list_actions; exit 0 ;;
esac

command -v "$SC" > /dev/null 2>&1 || die "no $SC (lint.yml's bash job installs version $SHELLCHECK_VERSION)" 2
command -v "$JQ" > /dev/null 2>&1 || die "no $JQ, which cuts the run blocks of the composite actions out (the runner image has it)" 2
command -v "$YQ" > /dev/null 2>&1 || die "no $YQ, which cuts the run blocks of the composite actions out (the runner image has it)" 2
"$YQ" --version 2>&1 | grep -q mikefarah || die "$YQ is not mikefarah's (https://github.com/mikefarah/yq), whose syntax the extraction uses: $("$YQ" --version 2>&1 | head -n 1)" 2
version="$("$SC" --version 2> /dev/null | sed -n 's/^version: //p')"
if [ -z "${CHECK_SHELL_ANY_VERSION:-}" ] && [ "$version" != "$SHELLCHECK_VERSION" ]; then
  die "$SC is version ${version:-unknown}, and this is held to $SHELLCHECK_VERSION (the runner's image has an older one; a laptop's may be newer): install that one, or set CHECK_SHELL_ANY_VERSION=1 to look anyway" 2
fi
[ -f "$RC" ] || die "$RC does not exist" 2

T="$(mktemp -d)" || die "no temporary directory" 2
trap 'rm -rf "$T"' EXIT INT TERM
control
[ "$mode" = self-test ] && exit 0

list > "$T/files"
n="$(wc -l < "$T/files" | tr -d ' ')"
[ "$n" -gt 0 ] || die "git lists no shell script under $ROOT: a check of nothing passes, so this fails" 2
list_actions > "$T/actions"
na="$(wc -l < "$T/actions" | tr -d ' ')"
set --
while IFS= read -r f; do set -- "$@" "$f"; done < "$T/files"
cd "$ROOT" || die "cannot enter $ROOT" 2
files_status=0
"$SC" --rcfile="$RC" "$@" || files_status=$?
blocks_status=0
check_blocks "$ROOT" "$T/actions" || blocks_status=$?
[ "$blocks_status" != 2 ] || die "$BERR" 2
if [ "$files_status" = 0 ] && [ "$blocks_status" = 0 ]; then
  echo "check-shell: $n scripts and $NB run blocks of $na composite actions, no finding"
else
  echo "check-shell: $n scripts (shellcheck exited $files_status) and $NB run blocks of $na composite actions (status $blocks_status); the findings are above" >&2
  exit 1
fi
