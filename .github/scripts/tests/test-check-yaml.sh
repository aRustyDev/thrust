#!/bin/sh
# .github/scripts/check-yaml.sh, which runs yamllint over the YAML the repository maintains (the `yaml` job of lint.yml; Lint phase, stage 2).
# The job runs the real thing against the real tree; what this shows is what the job cannot show about itself, in a fixture work tree of its own:
#   1. WHICH files: a tracked `*.yml` or `*.yaml`, workflows and composite actions included, and not the stock app (`web/`), the plans, the spikes,
#      the probes, anything under a `node_modules/`, the lockfile pnpm writes, or a file that is not YAML;
#   2. WHAT it hands to yamllint: the configuration by -c, and the files, one argument each;
#   3. WHEN it refuses (exit 2, "called wrongly"): no yamllint, a version other than the pinned one (unless asked to look anyway), no configuration,
#      a directory that is not a git work tree, no YAML to read (a check of nothing passes, so it fails), a name with a blank in it;
#   4. THE CONTROL, before any file is read: a yamllint that passes everything (not looking), one that refuses a key written twice without saying
#      key-duplicates, one that passes a file that does not parse, and one that refuses a file that is only unstyled (the configuration was not read)
#      are each refused (exit 1);
#   5. THE RESULT: a finding is exit 1, and a clean tree is exit 0;
#   6. with the real yamllint, when it is exactly the pinned version: the real control, a real duplicate key and a real clean tree.
# Every check has a control: a deliberately broken copy of the helper that the same checks must reject (AGENTS.md rule 8).
#
#   test-check-yaml.sh
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
REAL="$HERE/.github/scripts/check-yaml.sh"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
PINNED="$(sed -n 's/^YAMLLINT_VERSION="\(.*\)"$/\1/p' "$REAL")"
[ -n "$PINNED" ] || { echo "no YAMLLINT_VERSION in check-yaml.sh"; exit 1; }

# A stand-in yamllint. FAKE_VERSION is what it says it is; FAKE_ARGS, a file, collects what it was given; FAKE_MODE is how it judges. It knows the
# control's three files by their names (the helper writes them, and their contents say nothing a stand-in should read) and any other file by the
# marker FIXTURE-FINDING:
#   judge        as yamllint does with the repository's configuration: dup.yml and broken.yml are refused, style.yml passes
#   lax          passes everything (not looking)
#   nocode       refuses dup.yml and says no rule
#   nosyntax     passes broken.yml
#   unconfigured refuses style.yml too (the configuration that leaves style alone was not read)
cat > "$T/fake-yl" << 'EOF'
#!/bin/sh
if [ "$1" = "--version" ]; then echo "yamllint ${FAKE_VERSION:-0.0.0}"; exit 0; fi
if [ -n "${FAKE_ARGS:-}" ]; then printf '%s\n' "$@" >> "$FAKE_ARGS"; printf '%s\n' "$@" > "$FAKE_ARGS.last"; fi
status=0
mode="${FAKE_MODE:-judge}"
for a in "$@"; do
  [ -f "$a" ] || continue
  [ "$mode" = lax ] && continue
  case "${a##*/}" in
    dup.yml) if [ "$mode" = nocode ]; then echo "$a:3:1: [error] refused, and no rule"; else echo "$a:3:1: [error] duplication of key \"a\" in mapping (key-duplicates)"; fi; status=1 ;;
    broken.yml) [ "$mode" = nosyntax ] || { echo "$a:1:7: [error] syntax error: did not find expected ',' or ']' (syntax)"; status=1; } ;;
    style.yml) if [ "$mode" = unconfigured ]; then echo "$a:3:81: [error] line too long (line-length)"; status=1; fi ;;
    *) if grep -q FIXTURE-FINDING "$a"; then echo "$a:1:1: [error] a finding (key-duplicates)"; status=1; fi ;;
  esac
done
exit "$status"
EOF
chmod +x "$T/fake-yl"

# a fixture: a git work tree holding the helper under test and the repository's configuration, and these files
fixture() { # fixture <helper> <dir>
  d="$2"; mkdir -p "$d/.github/scripts" "$d/b" "$d/.github/workflows" "$d/.github/actions/x" "$d/web/apps" "$d/.agents/plans/p" "$d/tools/spike-0001" \
    "$d/probes" "$d/e2e" "$d/sub/node_modules"
  cp "$1" "$d/.github/scripts/check-yaml.sh"; cp "$HERE/.yamllint.yaml" "$d/"
  git -C "$d" init -q
  for f in a.yml b/c.yaml .github/workflows/w.yml .github/actions/x/action.yml web/apps/x.yml .agents/plans/p/q.yml tools/spike-0001/s.yml \
           probes/p.yml e2e/pnpm-lock.yaml sub/node_modules/m.yml; do
    printf 'key: value\n' > "$d/$f"
  done
  printf 'text\n' > "$d/notes.txt"; printf '{}\n' > "$d/x.json"
  git -C "$d" add -A
}
# run <dir> [VAR=value ...]: the helper of the fixture against the stand-in; the output in $out, the status in $rc
run() {
  d="$1"; shift
  : > "$T/args"; rm -f "$T/args.last"
  out="$(env YAMLLINT="$T/fake-yl" FAKE_VERSION="$PINNED" FAKE_ARGS="$T/args" "$@" sh "$d/.github/scripts/check-yaml.sh" "$d" 2>&1)"; rc=$?
}

# The checks, against the helper it is given. Prints a line for each one that does not hold, and returns how many.
suite() { # suite <helper>
  H="$1"; bad=0; n=0
  want() { # want <label> <exit> <fragment of the output>
    n=$((n + 1))
    if [ "$rc" = "$2" ] && printf '%s' "$out" | grep -qF -- "$3"; then :; else bad=$((bad + 1)); echo "BAD  $1: exit $rc, wanted $2 + '$3'"; printf '%s\n' "$out" | sed 's/^/     | /'; fi
  }
  holds() { # holds <label> <true|false: did the condition hold>
    n=$((n + 1)); [ "$2" = true ] || { bad=$((bad + 1)); echo "BAD  $1"; }
  }
  has() { grep -qFx -- "$1" "$T/args" && echo true || echo false; }
  # (the last call of yamllint is the one over the tree: the control's three calls come before it and also pass -c)
  haslast() { grep -qFx -- "$1" "$T/args.last" 2> /dev/null && echo true || echo false; }

  W="$T/w$$"; rm -rf "$W"; fixture "$H" "$W"
  run "$W"
  want "a clean tree passes" 0 "no finding"
  holds "it hands over the files a tracked yml or yaml file names, workflows and actions included" \
    "$( [ "$(has a.yml)$(has b/c.yaml)$(has .github/workflows/w.yml)$(has .github/actions/x/action.yml)" = truetruetruetrue ] && echo true || echo false )"
  holds "and not the stock app, the plans, the spikes, the probes, node_modules or the lockfile" \
    "$( [ "$(has web/apps/x.yml)$(has .agents/plans/p/q.yml)$(has tools/spike-0001/s.yml)$(has probes/p.yml)$(has sub/node_modules/m.yml)$(has e2e/pnpm-lock.yaml)" = falsefalsefalsefalsefalsefalse ] && echo true || echo false )"
  holds "and not a file that is not YAML" "$( [ "$(has notes.txt)$(has x.json)" = falsefalse ] && echo true || echo false )"
  holds "it hands over the configuration by -c, in the call over the tree and not only in the control's" \
    "$( [ "$(haslast -c)$(haslast "$W/.yamllint.yaml")" = truetrue ] && echo true || echo false )"

  printf 'key: value\nmarker: FIXTURE-FINDING\n' > "$W/a.yml"; git -C "$W" add -A
  run "$W"; want "a finding is exit 1, and says where to look" 1 "the findings are above"
  printf 'key: value\n' > "$W/a.yml"; git -C "$W" add -A

  run "$W" YAMLLINT=/nonexistent/yamllint; want "no yamllint is a call gone wrong" 2 "no /nonexistent/yamllint"
  run "$W" FAKE_VERSION=9.9.9; want "another version is refused" 2 "is version 9.9.9, and this is held to $PINNED"
  run "$W" FAKE_VERSION=9.9.9 CHECK_YAML_ANY_VERSION=1; want "unless asked to look anyway" 0 "no finding"
  run "$W" FAKE_MODE=lax; want "a yamllint that passes everything is not looking" 1 "did not refuse a mapping with a key twice"
  holds "and no file of the tree was read once the control failed" "$( [ "$(has a.yml)" = false ] && echo true || echo false )"
  run "$W" FAKE_MODE=nocode; want "one that refuses the duplicate key and does not say key-duplicates is refused" 1 "did not refuse a mapping with a key twice"
  run "$W" FAKE_MODE=nosyntax; want "one that passes a file that does not parse is refused" 1 "did not refuse a file that does not parse"
  run "$W" FAKE_MODE=unconfigured; want "one that refuses a file that is only unstyled says the configuration is not in effect" 1 "is not the configuration in effect"

  mv "$W/.yamllint.yaml" "$W/.yamllint.off"; run "$W"; want "no configuration is refused" 2 "no $W/.yamllint.yaml"; mv "$W/.yamllint.off" "$W/.yamllint.yaml"
  mv "$W/.git" "$W/.git.off"; run "$W"; want "a directory that is not a git work tree is refused" 2 "is not a git work tree"; mv "$W/.git.off" "$W/.git"
  printf 'k: v\n' > "$W/has blank.yml"; git -C "$W" add -A; run "$W"; want "a name with a blank is refused, not split in two" 2 "a blank in its name"
  git -C "$W" rm -q -f --cached "has blank.yml"; rm -f "$W/has blank.yml"
  E="$T/e$$"; rm -rf "$E"; mkdir -p "$E/.github/scripts" "$E/web"; cp "$H" "$E/.github/scripts/check-yaml.sh"; cp "$HERE/.yamllint.yaml" "$E/"
  # (the configuration stays untracked: it is YAML too, and a tracked one is a file to read)
  printf 'k: v\n' > "$E/web/x.yml"; git -C "$E" init -q; git -C "$E" add web/x.yml
  run "$E"; want "a tree with no YAML to read is refused: a check of nothing passes" 2 "a check of nothing passes"
  rm -rf "$W" "$E"
  SUITE_N="$n"
  return "$bad"
}

suite "$REAL"; real_bad=$?
echo "the helper: $SUITE_N checks, $real_bad did not hold"
caught=0; missed=0

# The controls: a broken copy of the helper for each check, which the same checks must reject.
broken() { # broken <label> <sed expression>
  mkdir -p "$T/b"; sed "$2" "$REAL" > "$T/b/check-yaml.sh"
  if cmp -s "$REAL" "$T/b/check-yaml.sh"; then missed=$((missed + 1)); echo "BAD  control '$1' did not change the helper (the sed expression matched nothing)"; return; fi
  if out="$(suite "$T/b/check-yaml.sh")"; then
    missed=$((missed + 1)); echo "BAD  control '$1' was not caught: the same checks passed a broken copy of the helper"
  else caught=$((caught + 1)); if [ -n "${VERBOSE:-}" ]; then echo "caught  $1"; fi; fi
}
broken "nothing is left out" "s#grep -v -E '[^']*'#grep -v -E '^NOTHING\$'#"
broken "the configuration is not handed over" 's/xargs "\$YL" -c "\$CFG" /xargs "$YL" /'
broken "any version is accepted" 's/\[ "\$version" != "\$YAMLLINT_VERSION" \]/false/'
broken "a tree with no YAML passes" 's/^\[ "\$count" -gt 0 \] ||/true ||/'
broken "the control is skipped" 's/^\[ "\$control_failed" = 0 \] || exit 1$/true/'
broken "a finding is exit 0" 's/the findings are above" 1/the findings are above" 0/'
broken "the duplicate key need not be named" "s/grep -qF 'key-duplicates'/grep -qF ''/"
broken "a name with a blank is split" "s/grep -q '\[\[:space:\]\]' \"\$T\/files\" \&\& die/false \&\& die/"
echo "the controls: $caught caught, $missed not"

# With the real yamllint, when it is exactly the pinned version
REALYL="${YAMLLINT:-yamllint}"
if command -v "$REALYL" > /dev/null 2>&1 && [ "$("$REALYL" --version 2> /dev/null | sed -n 's/^yamllint //p')" = "$PINNED" ]; then
  W="$T/real"; fixture "$REAL" "$W"
  out="$(YAMLLINT="$REALYL" sh "$W/.github/scripts/check-yaml.sh" "$W" 2>&1)"; rc=$?
  if [ "$rc" = 0 ] && printf '%s' "$out" | grep -qF "no finding"; then real=ok; else real="BAD: exit $rc: $out"; fi
  printf 'a: 1\nb: 2\na: 3\n' > "$W/a.yml"; git -C "$W" add -A
  out="$(YAMLLINT="$REALYL" sh "$W/.github/scripts/check-yaml.sh" "$W" 2>&1)"; rc=$?
  if [ "$rc" = 1 ] && printf '%s' "$out" | grep -qF "a.yml:3:1: [error] duplication of key"; then dup=ok; else dup="BAD: exit $rc: $out"; fi
  echo "real yamllint $PINNED: a clean tree $real; a real duplicate key $dup"
  case "$real$dup" in okok) ;; *) real_bad=$((real_bad + 1)) ;; esac
else
  echo "note: no yamllint $PINNED on PATH (or YAMLLINT), so the cases that run the real one were skipped (the yaml job runs it)"
fi

[ "$real_bad" -eq 0 ] && [ "$missed" -eq 0 ]
