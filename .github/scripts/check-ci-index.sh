#!/bin/sh
# The indexes of the CI directories are complete and true: every workflow in .github/workflows/ and every composite action in
# .github/actions/ has exactly one row in its directory's README.md, and every row names a file or a directory that is there.
# A row is a table line whose FIRST cell is the name in backticks (`ci.yml`, `setup-rust`); a name in any other cell, or in
# prose, is not one. A README that lists files that are gone, or leaves out one that was added, is a lie, and this is what stops it.
#
#   .github/scripts/check-ci-index.sh [repository root]   (default: this one)
#
# Exit status: 0 both indexes are complete and true; 1 a row is missing, unknown or repeated, or a README is missing; 2 called wrongly.
set -u
ROOT="${1:-$(cd "$(dirname "$0")/../.." && pwd)}"
[ -d "$ROOT/.github" ] || { echo "check-ci-index: $ROOT has no .github directory" >&2; exit 2; }
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
status=0

bad() {
  echo "check-ci-index: $*" >&2
  status=1
}

# check <label> <README> <file with the names that exist, one per line>
check() {
  label="$1"; readme="$2"; found="$3"
  if [ ! -f "$readme" ]; then
    bad "$label has no README.md"
    return
  fi
  grep -E '^\| `[^`]+` \|' "$readme" | sed -E 's/^\| `([^`]+)` \|.*/\1/' | sort > "$T/rows"
  sort -u "$found" > "$T/found"
  for name in $(comm -23 "$T/found" "$T/rows"); do
    bad "$label/README.md has no row for \`$name\`"
  done
  for name in $(comm -13 "$T/found" "$T/rows"); do
    bad "$label/README.md has a row for \`$name\`, which is not there"
  done
  for name in $(uniq -d "$T/rows"); do
    bad "$label/README.md has more than one row for \`$name\`"
  done
}

# workflows: .yml and .yaml files of the directory itself
: > "$T/workflows"
for f in "$ROOT"/.github/workflows/*.yml "$ROOT"/.github/workflows/*.yaml; do
  [ -f "$f" ] && basename "$f" >> "$T/workflows"
done
[ -s "$T/workflows" ] || { echo "check-ci-index: no workflow under $ROOT/.github/workflows" >&2; exit 2; }
check workflows "$ROOT/.github/workflows/README.md" "$T/workflows"

# actions: a directory with an action.yml or action.yaml
: > "$T/actions"
for f in "$ROOT"/.github/actions/*/action.yml "$ROOT"/.github/actions/*/action.yaml; do
  [ -f "$f" ] && basename "$(dirname "$f")" >> "$T/actions"
done
check actions "$ROOT/.github/actions/README.md" "$T/actions"

[ "$status" = 0 ] && echo "check-ci-index: $(wc -l < "$T/workflows" | tr -d ' ') workflows and $(sort -u "$T/actions" | wc -l | tr -d ' ') actions, each with its row"
exit "$status"
