#!/bin/sh
# The shell this repository maintains, run through shellcheck: the `bash` job of lint.yml (Lint phase, stage 1;
# .agents/plans/ci-workflows/phases/0005-lint.md). One script for CI and for a laptop, so that what fails there fails here.
#
#   .github/scripts/check-shell.sh                the control, then every maintained script
#   .github/scripts/check-shell.sh --list         the scripts it would check, one to a line, and nothing else
#   .github/scripts/check-shell.sh --self-test    the control alone
#   SHELLCHECK=<binary>                           (default: shellcheck on PATH)
#   CHECK_SHELL_ANY_VERSION=1                     do not hold the binary to SHELLCHECK_VERSION (for the tests' stand-in, which has none)
#
# WHAT. Every file git tracks that is a shell script: a `*.sh`, or an executable whose first line is the shebang of sh, bash, dash or ksh
# (`scripts/rs` has no extension, and a `*.sh` glob would miss it), less what is not ours to lint (frozen(), below).
#
# CONFIGURATION. The repository's .shellcheckrc, handed over with --rcfile so that CI, a laptop and the control read one file: every level
# fails (as it does for the shell in a workflow's `run:`, which actionlint runs shellcheck over), SC2016 and SC2015 are off, and what a
# script sources is followed.
#
# VERSION. Exactly SHELLCHECK_VERSION, which lint.yml installs at the same number (test_workflow_wiring.py holds the two equal). The runner
# image ships 0.9.0, which does not know SC2329 (a function that is never invoked); a laptop has whatever its package manager has. A
# version nobody chose makes a finding appear on one side of a push and not the other.
#
# THE CONTROL (AGENTS.md rule 8: a check that cannot fail is not evidence). Before the repository is read, three scripts in a directory of
# their own are given to the same binary and configuration: one with an unquoted variable (an `info` finding, SC2086), which must be
# refused; one with a single-quoted `$` (SC2016), which must pass, because it is the configuration that switches it off; and a clean one,
# which must pass. A shellcheck that is not looking, a configuration that was not read, and one that switched a check off each show here.
#
# Exit status: 0 the control behaved and every script is clean; 1 a script has a finding, or the control did not behave; 2 called wrongly
# (no shellcheck, the wrong version, not a git work tree, no script found: a check of nothing would pass).
set -u
LC_ALL=C
export LC_ALL

SHELLCHECK_VERSION="0.11.0"
SC="${SHELLCHECK:-shellcheck}"
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

control() {
  t="$(mktemp -d)" || die "no temporary directory" 2
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
    && { rm -rf "$t"; die "control: $SC passed a script with an unquoted variable (SC2086, an info finding): it is not looking, or the configuration switched the check off"; }
  case "$out" in *SC2086*) ;; *) rm -rf "$t"; die "control: $SC refused the script with the unquoted variable, but not for SC2086: $out" ;; esac
  out="$("$SC" --rcfile="$RC" "$t/idiom.sh" 2>&1)" \
    || { rm -rf "$t"; die "control: $SC refused a single-quoted \$ (SC2016), which $RC switches off: the configuration was not read, or it no longer does: $out"; }
  out="$("$SC" --rcfile="$RC" "$t/good.sh" 2>&1)" \
    || { rm -rf "$t"; die "control: $SC refused a clean script: $out"; }
  rm -rf "$t"
  echo "check-shell: control ok (shellcheck $version refused an unquoted variable, honoured $RC, and passed a clean script)"
}

mode=check
case "${1:-}" in
  "") ;;
  --list) mode=list ;;
  --self-test) mode=self-test ;;
  *) die "usage: check-shell.sh [--list | --self-test]" 2 ;;
esac

git -C "$ROOT" rev-parse --is-inside-work-tree > /dev/null 2>&1 || die "$ROOT is not a git work tree, so there is no list of tracked scripts to read" 2
if [ "$mode" = list ]; then
  list
  exit 0
fi

command -v "$SC" > /dev/null 2>&1 || die "no $SC (lint.yml's bash job installs version $SHELLCHECK_VERSION)" 2
version="$("$SC" --version 2> /dev/null | sed -n 's/^version: //p')"
if [ -z "${CHECK_SHELL_ANY_VERSION:-}" ] && [ "$version" != "$SHELLCHECK_VERSION" ]; then
  die "$SC is version ${version:-unknown}, and this is held to $SHELLCHECK_VERSION (the runner's image has an older one; a laptop's may be newer): install that one, or set CHECK_SHELL_ANY_VERSION=1 to look anyway" 2
fi
[ -f "$RC" ] || die "$RC does not exist" 2

control
[ "$mode" = self-test ] && exit 0

T="$(mktemp -d)" || die "no temporary directory" 2
trap 'rm -rf "$T"' EXIT INT TERM
list > "$T/files"
n="$(wc -l < "$T/files" | tr -d ' ')"
[ "$n" -gt 0 ] || die "git lists no shell script under $ROOT: a check of nothing passes, so this fails" 2
set --
while IFS= read -r f; do set -- "$@" "$f"; done < "$T/files"
cd "$ROOT" || die "cannot enter $ROOT" 2
if "$SC" --rcfile="$RC" "$@"; then
  echo "check-shell: $n scripts, no finding"
else
  status=$?
  echo "check-shell: $n scripts checked; shellcheck exited $status (findings above)" >&2
  exit 1
fi
