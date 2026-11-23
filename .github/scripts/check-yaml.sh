#!/bin/sh
# yamllint over the YAML the repository maintains: that it parses, and that no mapping has a key twice (the `yaml` job of lint.yml; Lint phase,
# stage 2). One script for CI and a laptop.
#
#   .github/scripts/check-yaml.sh [repository root]   (default: this one)
#   YAMLLINT=<the binary>                              (default: yamllint on PATH)
#   CHECK_YAML_ANY_VERSION=1                           do not hold the binary to YAMLLINT_VERSION (for the tests' stand-in, which has none)
#
# FILES. What git tracks as `*.yml` or `*.yaml`, less web/ (the stock app's), the records of finished experiments (.agents/plans/, tools/spike-*,
# probes/), any node_modules/, and e2e/pnpm-lock.yaml, which pnpm writes. Workflows and composite actions are in: actionlint reads the workflows
# and the workflow lint reads both, but neither says a key is written twice in an action, and this costs a second to read them.
#
# CONFIGURATION. .yamllint.yaml, handed over with -c: the one rule that is on is key-duplicates, and a file that does not parse fails whatever
# the rules say. Everything else yamllint can say is style, which the owner chose not to enforce.
#
# VERSION. Exactly YAMLLINT_VERSION, which lint.yml installs at the same number (test_workflow_wiring.py holds the two equal): a laptop's is
# whatever its package manager has, and a rule that one version has and the next renames is a finding on one side of a push and not the other.
#
# CONTROL. yamllint that is not looking passes everything, and a job that reads 40 files with it is green. So before any file is read, with the
# same binary and configuration, three files in a directory of their own: a key written twice must be refused (key-duplicates), a file that
# does not parse must be refused (syntax), and a file that is only unstyled (no `---`, a long line, `yes` as a value) must pass, which shows
# that .yamllint.yaml is the configuration in effect and not yamllint's default. If any of the three is wrong the run fails, whatever the tree holds.
#
# Exit status: 0 the control behaved and every file is clean; 1 a file has a finding, or the control did not behave; 2 called wrongly (no
# yamllint, another version, no configuration, not a git work tree, no YAML to read, a name with a blank in it).
set -u
YAMLLINT_VERSION="1.38.0"
ROOT="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
YL="${YAMLLINT:-yamllint}"
CFG="$ROOT/.yamllint.yaml"

die() { echo "check-yaml: $1" >&2; exit "${2:-1}"; }

command -v "$YL" > /dev/null 2>&1 || die "no $YL (lint.yml's yaml job installs version $YAMLLINT_VERSION)" 2
version="$("$YL" --version 2> /dev/null | sed -n 's/^yamllint //p')"
if [ -z "${CHECK_YAML_ANY_VERSION:-}" ] && [ "$version" != "$YAMLLINT_VERSION" ]; then
  die "$YL is version ${version:-unknown}, and this is held to $YAMLLINT_VERSION (a laptop's may be newer): install that one, or set CHECK_YAML_ANY_VERSION=1 to look anyway" 2
fi
[ -f "$CFG" ] || die "no $CFG: the configuration that says what fails" 2
git -C "$ROOT" rev-parse --is-inside-work-tree > /dev/null 2>&1 || die "$ROOT is not a git work tree, so there is no list of the files it tracks" 2

T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM

# The control, before any file of the repository.
mkdir "$T/control"
printf 'a: 1\nb: 2\na: 3\n' > "$T/control/dup.yml"
printf 'a: [1, 2\n' > "$T/control/broken.yml"
printf '# unstyled, and not wrong: no document start, a long line, a truthy value, wide spacing\nkey:     yes\nlong: "%s"\n' \
  "0123456789012345678901234567890123456789012345678901234567890123456789012345678901234567890123456789" > "$T/control/style.yml"
control_failed=0
out="$("$YL" -c "$CFG" -f parsable "$T/control/dup.yml" 2>&1)"; rc=$?
if [ "$rc" = 0 ] || ! printf '%s' "$out" | grep -qF 'key-duplicates'; then
  echo "check-yaml: the control failed: $YL did not refuse a mapping with a key twice, saying key-duplicates (exit $rc):" >&2
  printf '%s\n' "$out" | sed 's/^/  | /' >&2
  control_failed=1
fi
out="$("$YL" -c "$CFG" -f parsable "$T/control/broken.yml" 2>&1)"; rc=$?
if [ "$rc" = 0 ] || ! printf '%s' "$out" | grep -qF '(syntax)'; then
  echo "check-yaml: the control failed: $YL did not refuse a file that does not parse, saying syntax (exit $rc):" >&2
  printf '%s\n' "$out" | sed 's/^/  | /' >&2
  control_failed=1
fi
out="$("$YL" -c "$CFG" -f parsable "$T/control/style.yml" 2>&1)"; rc=$?
if [ "$rc" != 0 ]; then
  echo "check-yaml: the control failed: $YL refused a file that is only unstyled, so $CFG is not the configuration in effect (exit $rc):" >&2
  printf '%s\n' "$out" | sed 's/^/  | /' >&2
  control_failed=1
fi
[ "$control_failed" = 0 ] || exit 1

# The files.
git -C "$ROOT" ls-files '*.yml' '*.yaml' \
  | grep -v -E '^(web/|\.agents/plans/|tools/spike-|probes/)|(^|/)node_modules/|^e2e/pnpm-lock\.yaml$' > "$T/files"
count="$(grep -c '' "$T/files")"
[ "$count" -gt 0 ] || die "no YAML file found under $ROOT: a check of nothing passes, so this fails" 2
grep -q '[[:space:]]' "$T/files" && die "a YAML file has a blank in its name, and this hands names to yamllint one word at a time" 2

( cd "$ROOT" && xargs "$YL" -c "$CFG" -f parsable < "$T/files" ) > "$T/out" 2>&1; rc=$?
if [ "$rc" != 0 ]; then
  cat "$T/out" >&2
  die "$count files (yamllint exited $rc); the findings are above" 1
fi
echo "check-yaml: $count files, no finding (yamllint $version, $CFG)"
