#!/bin/sh
# Did the checks that matter pass on THIS commit? (issue #49, ADR-0110 round 29)
#
#   .github/scripts/assert-sha-checks.sh <owner/repo> <sha> [check-name ...]      (default check: gate)
#
# Exit 0 only when every named check has reported on the commit and every report of it is SUCCESS. It reasons over the SET, never over
# the absence of failures: a workflow that never started (a typo in `on:`, a path filter, startup_failure) creates no check at all, so a
# summary that only looks at what reported can say "nothing failed" while nothing ran. A SKIPPED `gate` is not a pass either: GitHub lets
# a skipped check satisfy a required context, and a deploy must not.
#
# Why not the whole required set, as assert-required-contexts.sh certifies a pull request: a push to main reports fewer checks than a
# pull request does (some bots run on pull requests only), so the push's required set would never be complete. `gate` is the one check
# that stands for everything `ci` runs (ci.yml), and it is the one this asks for. Pass other names to ask for more.
#
# Needs: gh (authenticated) and jq. Intermediate files live in a temporary directory removed on exit.
# Exit status: 0 certified; 1 not certified (what is missing or not green is on stderr); 2 called wrongly or a tool is missing.
set -eu

fail() { echo "NOT CERTIFIED: $*" >&2; exit 1; }

REPO="${1:?usage: assert-sha-checks.sh <owner/repo> <sha> [check-name ...]}"
SHA="${2:?usage: assert-sha-checks.sh <owner/repo> <sha> [check-name ...]}"
shift 2
NAMES="${*:-gate}"

case "$SHA" in *[!0-9a-f]*|'') echo "assert-sha-checks: '$SHA' is not a commit sha" >&2; exit 2 ;; esac
[ "${#SHA}" -eq 40 ] || { echo "assert-sha-checks: '$SHA' is not a 40-digit commit sha" >&2; exit 2; }
for tool in gh jq; do
  command -v "$tool" > /dev/null 2>&1 || { echo "missing tool: $tool" >&2; exit 2; }
done

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# one JSON object per page; jq reads them one after the other
gh api "repos/$REPO/commits/$SHA/check-runs?per_page=100" --paginate > "$WORK/pages.json" 2> "$WORK/err" \
  || fail "could not read the check runs of $SHA: $(tr '\n' ' ' < "$WORK/err")"

# one "<name><TAB><state>" row per check run; a run that has not finished has no conclusion, so its status stands for it.
# Not `jq | sort || fail`: that tests sort's status, and a jq that failed would read as "nothing reported".
jq -r '.check_runs[] | [.name, ((.conclusion // .status // "UNKNOWN") | ascii_upcase)] | @tsv' "$WORK/pages.json" \
  > "$WORK/rows.tsv" 2> "$WORK/jq.err" || fail "the check runs of $SHA are not the JSON this reads: $(tr '\n' ' ' < "$WORK/jq.err")"
sort "$WORK/rows.tsv" > "$WORK/reported.tsv"

problems=""
for name in $NAMES; do
  rows="$(awk -F'\t' -v n="$name" '$1 == n { print $2 }' "$WORK/reported.tsv")"
  if [ -z "$rows" ]; then
    problems="$problems
  $name: never reported on $SHA"
    continue
  fi
  not_green="$(echo "$rows" | grep -vx 'SUCCESS' | sort -u | tr '\n' ' ' || true)"
  if [ -n "$not_green" ]; then
    problems="$problems
  $name: ${not_green}(every report of it must be SUCCESS)"
  fi
done

if [ -n "$problems" ]; then
  echo "$problems" >&2
  fail "$(echo "$problems" | grep -c .) of $(echo "$NAMES" | wc -w | tr -d ' ') required checks are absent or not green on $SHA"
fi
echo "certified: $NAMES reported on $SHA and every report is SUCCESS"
