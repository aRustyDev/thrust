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
#   6. with the real shellcheck, when it is exactly the pinned version: the real control, a real finding and a real clean tree;
#   7. the run blocks of the composite actions, cut out with the real yq and jq: which files are actions, a block's finding (named by action, line
#      and step), the dialect of a block's `shell:`, yq and grep disagreeing on the number of blocks, a shell shellcheck does not read, a yq or
#      jq that is missing or the wrong yq, and a shellcheck that passes a block with the unquoted variable (the control refuses it).
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
    [0-9]*.sh) # a run block of a composite action, written out by the helper
      if [ "$mode" = blind-blocks ]; then :; elif grep -q -e FIXTURE-FINDING -e '^ls \$x$' "$a"; then echo "In $a line 1: SC2086 (info): a finding"; status=1; fi ;;
    check-shell.sh) ;; # the helper's own copy in a fixture: its text holds the control's scripts
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
  # the control's three scripts and three run blocks come first (they sit in a temporary directory, the blocks with a `-s bash` each), then the
  # rc file and the scripts, in git's order
  got="$(grep -v -e '/bad.sh$' -e '/idiom.sh$' -e '/good.sh$' -e '/blocks/[0-9]*\.sh$' -e '^-s$' -e '^bash$' "$T/args" | LC_ALL=C sort)"
  want="$(printf '%s\n' "--rcfile=$1/.shellcheckrc" | cat - "$EXPECTED" | LC_ALL=C sort)"
  # the rc file is handed over once per call: three control scripts, three control blocks, then the scripts of the tree (a tree with no action has no block)
  [ "$(grep -c -e '^--rcfile=' "$T/args")" = 7 ] || { echo "the rc file was handed over $(grep -c -e '^--rcfile=' "$T/args") times, not 7"; return 1; }
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
check "a finding in a script is exit 1, and the output says so" refuses 1 "the findings are above" "$dirty" FAKE_VERSION="$PINNED"
mutant m-exit '/the findings are above/{n;s/exit 1/exit 0/;}'
cp -R "$dirty/infra" "$T/m-exit/"
control "result: a finding is exit 0" refuses 1 "the findings are above" "$T/m-exit" FAKE_VERSION="$PINNED"

# 7. the composite actions' run blocks, cut out with the real yq and jq (the runner image has both)
if command -v yq > /dev/null 2>&1 && yq --version 2>&1 | grep -q mikefarah && command -v jq > /dev/null 2>&1; then
  mkact() { # mkact <dir> <name>: a composite action; its steps come on stdin
    mkdir -p "$1/.github/actions/$2"
    { printf 'name: %s\ndescription: a fixture\nruns:\n  using: composite\n  steps:\n' "$2"; cat; } > "$1/.github/actions/$2/action.yml"
  }
  atree="$T/atree"; fixture "$atree"; put "$atree" a.sh 644 '#!/bin/sh'
  mkact "$atree" clean << 'EOF'
    - name: One
      shell: bash
      run: echo one
    - name: Two
      shell: bash
      run: |
        echo two
EOF
  mkact "$atree" posix << 'EOF'
    - name: Posix
      shell: sh
      run: echo posix
EOF
  mkdir -p "$atree/.github/actions/notes"; printf 'steps: []\n' > "$atree/.github/actions/notes/other.yml"; printf '# x\n' > "$atree/.github/actions/README.md"
  git -C "$atree" add -A
  adirty="$T/adirty"; cp -R "$atree" "$adirty"
  mkact "$adirty" dirty << 'EOF'
    - name: Fine
      shell: bash
      run: echo fine
    - name: Second
      shell: bash
      run: |
        echo FIXTURE-FINDING
EOF
  git -C "$adirty" add -A
  actions_are_right() { # actions_are_right <dir> <list...>: --list-actions prints exactly these
    ar="$1"; shift
    got="$(sh "$ar/.github/scripts/check-shell.sh" --list-actions | LC_ALL=C sort)"
    [ "$got" = "$(printf '%s\n' "$@" | LC_ALL=C sort)" ] || { echo "listed: $got"; return 1; }
  }
  check "the composite actions are the action.yml files under .github/actions/ and no other file" actions_are_right "$atree" .github/actions/clean/action.yml .github/actions/posix/action.yml
  mutant m-actions "s|ls-files '.github/actions/\*/action.yml'|ls-files 'nothing'|"
  rm -rf "$T/m-actions/.github/actions"; cp -R "$atree/.github/actions" "$T/m-actions/.github/"; git -C "$T/m-actions" add -A
  control "actions: none are found" actions_are_right "$T/m-actions" .github/actions/clean/action.yml .github/actions/posix/action.yml
  check "a run block with a finding refuses the run, names the action, the line of the block and the step" refuses 1 "dirty/action.yml, the run block at line" "$adirty" FAKE_VERSION="$PINNED"
  check "and the step's name" refuses 1 "(Second)" "$adirty" FAKE_VERSION="$PINNED"
  check "clean actions pass, and the summary counts their blocks" refuses 0 "3 run blocks of 2 composite actions, no finding" "$atree" FAKE_VERSION="$PINNED"
  mutant m-blocks 's/check_blocks "\$ROOT" "\$T\/actions" || blocks_status=\$?/check_blocks "$ROOT" "$T\/actions" || true/'
  rm -rf "$T/m-blocks/.github/actions"; cp -R "$adirty/.github/actions" "$T/m-blocks/.github/"; git -C "$T/m-blocks" add -A
  control "blocks: a finding in a run block is not a failure" refuses 1 "dirty/action.yml, the run block at line" "$T/m-blocks" FAKE_VERSION="$PINNED"
  dialect_is_right() { # dialect_is_right <dir>: the block of `shell: sh` is read as sh, and the blocks of `shell: bash` as bash
    rm -f "$T/args"
    FAKE_ARGS="$T/args" FAKE_MODE=judge FAKE_VERSION="$PINNED" SHELLCHECK="$FAKE" sh "$1/.github/scripts/check-shell.sh" > /dev/null || return 1
    [ "$(grep -c -x sh "$T/args")" = 1 ] || { echo "sh was named $(grep -c -x sh "$T/args") times, not once"; return 1; }
  }
  check "a block is read in the dialect of its shell:" dialect_is_right "$atree"
  mutant m-dialect 's/-s "\$shell"/-s bash/'
  rm -rf "$T/m-dialect/.github/actions"; cp -R "$atree/.github/actions" "$T/m-dialect/.github/"; git -C "$T/m-dialect" add -A
  control "dialect: every block is read as bash" dialect_is_right "$T/m-dialect"
  # a `run:` key that yq does not count (a line of a block's text) makes grep and yq disagree, and the run is refused (exit 2), and not trusted
  anest="$T/anest"; cp -R "$atree" "$anest"
  mkact "$anest" nested << 'EOF'
    - name: Heredoc
      shell: bash
      run: |
        cat << 'X'
          run: nested
        X
EOF
  git -C "$anest" add -A
  check "yq and grep disagreeing on the number of blocks is refused" refuses 2 "yq found 1 run blocks and grep found 2" "$anest" FAKE_VERSION="$PINNED"
  mutant m-count 's/\[ "\$found" = "\$expected" \] ||/true ||/'
  rm -rf "$T/m-count/.github/actions"; cp -R "$anest/.github/actions" "$T/m-count/.github/"; git -C "$T/m-count" add -A
  control "count: the disagreement is not refused" refuses 2 "yq found 1 run blocks and grep found 2" "$T/m-count" FAKE_VERSION="$PINNED"
  apy="$T/apy"; cp -R "$atree" "$apy"
  mkact "$apy" python << 'EOF'
    - name: Py
      shell: python
      run: print("hi")
EOF
  git -C "$apy" add -A
  check "a block in a shell shellcheck does not read is refused" refuses 2 "a step runs in python" "$apy" FAKE_VERSION="$PINNED"
  mutant m-shell 's/case "\$shell" in sh | bash) ;;/case "$shell" in *) ;;/'
  rm -rf "$T/m-shell/.github/actions"; cp -R "$apy/.github/actions" "$T/m-shell/.github/"; git -C "$T/m-shell" add -A
  control "shell: a block in python is read" refuses 2 "a step runs in python" "$T/m-shell" FAKE_VERSION="$PINNED"
  check "a yq that is not there is refused" refuses 2 "no /nonexistent/yq" "$atree" FAKE_VERSION="$PINNED" YQ=/nonexistent/yq
  check "a jq that is not there is refused" refuses 2 "no /nonexistent/jq" "$atree" FAKE_VERSION="$PINNED" JQ=/nonexistent/jq
  printf '#!/bin/sh\necho "yq 3.4.1"\n' > "$T/yq-python"; chmod +x "$T/yq-python"
  check "a yq that is not mikefarah's is refused" refuses 2 "is not mikefarah's" "$atree" FAKE_VERSION="$PINNED" YQ="$T/yq-python"
  check "a shellcheck that passes the run blocks of an action with an unquoted variable is refused by the control" refuses 1 "a composite action whose run block has an unquoted variable was passed" "$atree" FAKE_VERSION="$PINNED" FAKE_MODE=blind-blocks
else
  echo "skipped: the checks of the composite actions' run blocks need mikefarah's yq and jq (the runner image has both)"
fi

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
