#!/bin/sh
# .github/scripts/check-shell.sh, which runs shellcheck over the shell the repository maintains (the `bash` job of lint.yml; Lint phase, stage 1).
# The job runs the real thing against the real tree; what this shows is what the job cannot show about itself, in a fixture work tree of its own:
#   1. WHICH scripts: a `*.sh`, or an executable whose first line is the shebang of a shell (`env bash`, and `env -S bash -e`, which an earlier
#      draft choked on), and not a plain file with a shebang, a non-shell script, a non-executable file without `.sh`, the stock app (`web/`),
#      anything under a `node_modules/`, the plans, the spikes, the probes or the zxcvbn measurement; a name with a blank in it is one name;
#   2. WHAT it hands to shellcheck: the configuration file by --rcfile, every one of those scripts, one argument each, nothing else;
#   3. WHEN it refuses (exit 2, "called wrongly"): no shellcheck, a version other than the pinned one (unless asked to look anyway), no
#      configuration file, a directory that is not a git work tree, and a tree with no script (a check of nothing passes, so it fails);
#   4. THE CONTROL: a shellcheck that passes everything (not looking), one that refuses the unquoted variable without a code, one that refuses a
#      single-quoted `$` (the configuration was not read), and one that refuses a clean script are each refused, before any script is read;
#   5. THE RESULT: a finding is exit 1, and a clean tree is exit 0;
#   6. with the real shellcheck, when it is exactly the pinned version: the real control, a real finding and a real clean tree.
# Every check has a control: a deliberately broken copy of the helper that the same check must reject (AGENTS.md rule 8).
#
#   test-check-shell.sh
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
pass=0; failn=0; n=0
ok() { pass=$((pass + 1)); }
bad() { failn=$((failn + 1)); echo "BAD  $*"; }
check() {
  label="$1"; shift
  n=$((n + 1))
  if out="$("$@" 2>&1)"; then ok; else bad "$label: $out"; fi
}
control() {
  label="$1"; shift
  n=$((n + 1))
  if out="$("$@" 2>&1)"; then bad "control '$label' was not caught (the check passed on a broken copy)"; else ok; if [ -n "${VERBOSE:-}" ]; then printf 'caught  %s\n          %s\n' "$label" "$(printf '%s' "$out" | head -n 1)"; fi; fi
}
PINNED="$(sed -n 's/^SHELLCHECK_VERSION="\(.*\)"$/\1/p' "$HERE/.github/scripts/check-shell.sh")"
[ -n "$PINNED" ] || { echo "no SHELLCHECK_VERSION in check-shell.sh"; exit 1; }

# a stand-in shellcheck. FAKE_VERSION is what it says it is; FAKE_ARGS, a file, collects what it was given; FAKE_MODE is how it judges. It knows
# the control's three files by their names (bad.sh, idiom.sh, good.sh: the helper's own text holds their contents, so the content cannot tell
# them apart) and any other file by the marker FIXTURE-FINDING:
#   judge       as shellcheck does with the repository's configuration: bad.sh is refused with SC2086, idiom.sh and good.sh pass
#   lax         passes everything (not looking)
#   silent      refuses bad.sh, and says no code
#   unconfigured  refuses idiom.sh too (the configuration that switches SC2016 off was not read)
#   fussy       refuses good.sh too (a clean script)
cat > "$T/fake-sc" << 'EOF'
#!/bin/sh
if [ "$1" = "--version" ]; then printf 'ShellCheck - shell script analysis tool\nversion: %s\n' "${FAKE_VERSION:-0.0.0}"; exit 0; fi
[ -n "${FAKE_ARGS:-}" ] && printf '%s\n' "$@" >> "$FAKE_ARGS"
status=0
mode="${FAKE_MODE:-judge}"
for a in "$@"; do
  case "$a" in --*) continue ;; esac
  [ -f "$a" ] || continue
  [ "$mode" = lax ] && continue
  case "${a##*/}" in
    bad.sh) if [ "$mode" = silent ]; then echo "refused, and no code"; else echo "In $a line 3: SC2086 (info): Double quote to prevent globbing"; fi; status=1 ;;
    idiom.sh) if [ "$mode" = unconfigured ]; then echo "In $a line 2: SC2016 (info): Expressions do not expand in single quotes"; status=1; fi ;;
    good.sh) if [ "$mode" = fussy ]; then echo "In $a: refused a clean script"; status=1; fi ;;
    *) if grep -q FIXTURE-FINDING "$a"; then echo "In $a line 1: SC2086 (info): a finding"; status=1; fi ;;
  esac
done
exit "$status"
EOF
chmod +x "$T/fake-sc"
FAKE="$T/fake-sc"

# a fixture: a git work tree holding a copy of the helper and the repository's configuration, and these files (path, mode, first line)
fixture() { # fixture <dir>
  d="$1"; mkdir -p "$d/.github/scripts"
  cp "$HERE/.github/scripts/check-shell.sh" "$d/.github/scripts/"; cp "$HERE/.shellcheckrc" "$d/"
  git -C "$d" init -q
}
put() { # put <dir> <path> <mode> <first line>
  mkdir -p "$(dirname "$1/$2")"; printf '%s\n' "$4" > "$1/$2"; chmod "$3" "$1/$2"
}
tree="$T/tree"; fixture "$tree"
put "$tree" a.sh 644 '#!/bin/sh'
put "$tree" bin/tool 755 '#!/bin/sh'
put "$tree" bin/other 755 '#!/usr/bin/env bash'
put "$tree" bin/env-s 755 '#!/usr/bin/env -S bash -e'
put "$tree" "dir with space/b.sh" 644 '#!/bin/sh'
put "$tree" infra/x.sh 644 '#!/bin/sh'
put "$tree" tools/oracle-v1/run.sh 644 '#!/bin/sh'
# what must not be listed
put "$tree" bin/py 755 '#!/usr/bin/env python3'
put "$tree" bin/data 644 '#!/bin/sh'
put "$tree" bin/noshebang 755 'echo hi'
put "$tree" web/x.sh 644 '#!/bin/sh'
put "$tree" node_modules/x.sh 644 '#!/bin/sh'
put "$tree" web/node_modules/y.sh 644 '#!/bin/sh'
put "$tree" sub/node_modules/z.sh 644 '#!/bin/sh'
put "$tree" .agents/plans/p/v.sh 644 '#!/bin/sh'
put "$tree" tools/spike-0001/s.sh 644 '#!/bin/sh'
put "$tree" tools/measure-zxcvbn/m.sh 644 '#!/bin/sh'
put "$tree" probes/p/q.sh 644 '#!/bin/sh'
git -C "$tree" add -A
EXPECTED="$T/expected"
# (the helper's own copy is one of the scripts of the fixture, as it is one of the repository's)
printf '%s\n' .github/scripts/check-shell.sh a.sh bin/env-s bin/other bin/tool "dir with space/b.sh" infra/x.sh tools/oracle-v1/run.sh | LC_ALL=C sort > "$EXPECTED"

list_is_right() { # list_is_right <dir>: --list prints exactly the expected scripts
  got="$(sh "$1/.github/scripts/check-shell.sh" --list | LC_ALL=C sort)" || return 1
  [ "$got" = "$(cat "$EXPECTED")" ] || { echo "listed:"; echo "$got"; return 1; }
}
check "the listing is exactly the maintained scripts" list_is_right "$tree"

# a broken copy of the whole fixture: <name>, a sed script for the helper (the copy must differ from the original)
mutant() { # mutant <name> <sed script>
  cp -R "$tree" "$T/$1"
  sed "$2" "$tree/.github/scripts/check-shell.sh" > "$T/$1/.github/scripts/check-shell.sh.new" && mv "$T/$1/.github/scripts/check-shell.sh.new" "$T/$1/.github/scripts/check-shell.sh"
  cmp -s "$tree/.github/scripts/check-shell.sh" "$T/$1/.github/scripts/check-shell.sh" && { echo "mutant $1 changed nothing"; exit 1; }
  return 0
}
mutant m-frozen '/^frozen() {/,/^}/ s/return 0 ;;/return 1 ;;/'
mutant m-ext '/100755 /d'
mutant m-env 's/if \[ "\$interp" = env \]; then/if false; then/'
control "listing: nothing is frozen" list_is_right "$T/m-frozen"
control "listing: only the *.sh files" list_is_right "$T/m-ext"
control "listing: a shebang through env is read as the shell it names" list_is_right "$T/m-env"

# 2. what it hands over
args_are_right() { # args_are_right <dir>
  rm -f "$T/args"
  FAKE_ARGS="$T/args" FAKE_MODE=judge FAKE_VERSION="$PINNED" SHELLCHECK="$FAKE" sh "$1/.github/scripts/check-shell.sh" > /dev/null || return 1
  # the control's three files come first (they sit in a temporary directory), then the rc file and the scripts, in git's order
  got="$(grep -v -e '/bad.sh$' -e '/idiom.sh$' -e '/good.sh$' "$T/args" | LC_ALL=C sort)"
  want="$(printf '%s\n' "--rcfile=$1/.shellcheckrc" | cat - "$EXPECTED" | LC_ALL=C sort)"
  # the control passes the rc file too: it is handed over once per call, and there are four calls (three control files, then the scripts)
  [ "$(grep -c -e '^--rcfile=' "$T/args")" = 4 ] || { echo "the rc file was handed over $(grep -c -e '^--rcfile=' "$T/args") times, not 4"; return 1; }
  [ "$(printf '%s\n' "$got" | sort -u)" = "$(printf '%s\n' "$want" | sort -u)" ] || { echo "got:"; echo "$got"; echo "wanted:"; echo "$want"; return 1; }
  [ "$(grep -c 'dir with space/b.sh$' "$T/args")" = 1 ] || { echo "the name with a blank in it was not one argument"; return 1; }
}
check "the real run hands over the configuration and every script, one argument each" args_are_right "$tree"
mutant m-rc 's/--rcfile="\$RC" "\$@"/"$@"/'
control "args: the configuration is not handed over to the run over the scripts" args_are_right "$T/m-rc"

# 3. when it refuses
run() { # run <dir> <env assignments...> -- <args>: the status and the output of the helper, with the stand-in; prints the output, returns the status
  rdir="$1"; shift
  env "$@" sh "$rdir/.github/scripts/check-shell.sh"
}
refuses() { # refuses <status> <words> <dir> <env assignments...>: the helper exits with the status and says the words
  want_status="$1"; words="$2"; rd="$3"; shift 3
  out="$(run "$rd" FAKE_MODE=judge SHELLCHECK="$FAKE" "$@" 2>&1)"; st=$?
  [ "$st" = "$want_status" ] || { echo "exit $st, wanted $want_status: $out"; return 1; }
  printf '%s' "$out" | grep -q -F -e "$words" || { echo "the output lacks '$words': $out"; return 1; }
}
check "a shellcheck of another version is refused, and the pinned one named" refuses 2 "held to $PINNED" "$tree" FAKE_VERSION=0.9.0
check "a shellcheck of another version is accepted when asked to look anyway" refuses 0 "no finding" "$tree" FAKE_VERSION=0.9.0 CHECK_SHELL_ANY_VERSION=1
check "a shellcheck that is not there is refused" refuses 2 "no /nonexistent/shellcheck" "$tree" SHELLCHECK=/nonexistent/shellcheck
mutant m-version 's/\[ "\$version" != "\$SHELLCHECK_VERSION" \]/false/'
control "version: any version is accepted" refuses 2 "held to $PINNED" "$T/m-version" FAKE_VERSION=0.9.0
empty="$T/empty"; fixture "$empty"
check "a tree with no script is refused (a check of nothing passes)" refuses 2 "git lists no shell script" "$empty" FAKE_VERSION="$PINNED"
mutant m-empty 's/\[ "\$n" -gt 0 \]/true/'
rm -rf "$T/m-empty-tree"; fixture "$T/m-empty-tree"; cp "$T/m-empty/.github/scripts/check-shell.sh" "$T/m-empty-tree/.github/scripts/check-shell.sh"
control "empty: a tree with no script passes" refuses 2 "git lists no shell script" "$T/m-empty-tree" FAKE_VERSION="$PINNED"
norc="$T/norc"; fixture "$norc"; rm "$norc/.shellcheckrc"; put "$norc" a.sh 644 '#!/bin/sh'; git -C "$norc" add -A
check "a tree with no .shellcheckrc is refused" refuses 2 ".shellcheckrc does not exist" "$norc" FAKE_VERSION="$PINNED"
nogit="$T/nogit"; mkdir -p "$nogit/.github/scripts"; cp "$HERE/.github/scripts/check-shell.sh" "$nogit/.github/scripts/"; cp "$HERE/.shellcheckrc" "$nogit/"
check "a directory that is not a git work tree is refused" refuses 2 "is not a git work tree" "$nogit" FAKE_VERSION="$PINNED"

# 4. the control, before any script is read
check "with a shellcheck that judges, the control passes and the run is clean" refuses 0 "control ok" "$tree" FAKE_VERSION="$PINNED"
check "a shellcheck that passes everything is refused" refuses 1 "passed a script with an unquoted variable" "$tree" FAKE_VERSION="$PINNED" FAKE_MODE=lax
check "a shellcheck that refuses the unquoted variable without SC2086 is refused" refuses 1 "but not for SC2086" "$tree" FAKE_VERSION="$PINNED" FAKE_MODE=silent
check "a shellcheck that refuses a single-quoted \$ (the configuration was not read) is refused" refuses 1 "the configuration was not read" "$tree" FAKE_VERSION="$PINNED" FAKE_MODE=unconfigured
check "a shellcheck that refuses a clean script is refused" refuses 1 "refused a clean script" "$tree" FAKE_VERSION="$PINNED" FAKE_MODE=fussy
mutant m-control 's/^control$/:/'
control "control: it is not run before the scripts" refuses 1 "passed a script with an unquoted variable" "$T/m-control" FAKE_VERSION="$PINNED" FAKE_MODE=lax
self_test_stops() { # self_test_stops <dir>: --self-test runs the control, says so, and reads no script
  out="$(env FAKE_MODE=judge FAKE_VERSION="$PINNED" SHELLCHECK="$FAKE" sh "$1/.github/scripts/check-shell.sh" --self-test 2>&1)" || { echo "exit $?: $out"; return 1; }
  printf '%s' "$out" | grep -q 'control ok' || { echo "no control ok: $out"; return 1; }
  printf '%s' "$out" | grep -q 'no finding' && { echo "it went on to the scripts: $out"; return 1; }
  return 0
}
check "the self-test runs the control and stops there" self_test_stops "$tree"
mutant m-selftest 's/^\[ "\$mode" = self-test \] && exit 0$/:/'
control "self-test: it goes on to read the scripts" self_test_stops "$T/m-selftest"

# 5. the result
dirty="$T/dirty"; cp -R "$tree" "$dirty"; printf 'FIXTURE-FINDING\n' >> "$dirty/infra/x.sh"
check "a finding in a script is exit 1, and the output says so" refuses 1 "findings above" "$dirty" FAKE_VERSION="$PINNED"
mutant m-exit '/findings above/{n;s/exit 1/exit 0/;}'
cp -R "$dirty/infra" "$T/m-exit/"
control "result: a finding is exit 0" refuses 1 "findings above" "$T/m-exit" FAKE_VERSION="$PINNED"

# 6. the real shellcheck, when it is the pinned version
real_version="$(shellcheck --version 2> /dev/null | sed -n 's/^version: //p')"
if [ "$real_version" = "$PINNED" ]; then
  real_clean="$T/real-clean"; fixture "$real_clean"; printf '#!/bin/sh\nx="a b"\nls "$x"\n' > "$real_clean/ok.sh"; git -C "$real_clean" add -A
  real_bad="$T/real-bad"; fixture "$real_bad"; printf '#!/bin/sh\nx="a b"\nls $x\n' > "$real_bad/bad.sh"; git -C "$real_bad" add -A
  real_run() { env -u SHELLCHECK -u FAKE_MODE sh "$1/.github/scripts/check-shell.sh"; }
  check "the real control passes, and the real shellcheck says a clean tree is clean" real_run "$real_clean"
  control "the real shellcheck refuses an unquoted variable in a script of the tree" real_run "$real_bad"
  real_names_code() { out="$(real_run "$1" 2>&1)"; printf '%s' "$out" | grep -q 'SC2086'; }
  check "and names the code" real_names_code "$real_bad"
else
  echo "skipped: the checks with the real shellcheck need exactly $PINNED (this one is ${real_version:-not installed}); the \`bash\` job runs them on the pinned one"
fi

echo "$pass of $n checks passed"
[ "$failn" -eq 0 ]
