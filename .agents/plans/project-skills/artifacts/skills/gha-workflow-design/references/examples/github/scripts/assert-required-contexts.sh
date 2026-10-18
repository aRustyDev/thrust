#!/bin/sh
# Certify a PR's checks by SET, never by the absence of failures.
#
#   .github/scripts/assert-required-contexts.sh <owner/repo> <pr-number> [base-branch]
#
# Exit 0 only when EVERY required status context of the base branch has reported on the PR and is
# SUCCESS, SKIPPED or NEUTRAL. Exit 1 otherwise, naming what is missing or not green. A workflow that
# never started (startup_failure, a path filter, a typo in `on:`) creates no check at all, so a
# summary that only looks at what reported can say "all passed" while the required set is empty.
#
# Environment:
#   STRICT_CONTEXTS   space-separated context names that must be exactly SUCCESS. GitHub lets a
#                     SKIPPED check satisfy a required context; name an aggregate such as `gate` here so
#                     that a skipped gate is not certified.
#
# Needs: gh (authenticated), jq. Intermediate files live in a temporary directory removed on exit.
set -eu

fail() {
  echo "NOT CERTIFIED: $*" >&2
  exit 1
}

REPO="${1:?usage: assert-required-contexts.sh <owner/repo> <pr-number> [base-branch]}"
PR="${2:?pr number}"
BASE="${3:-main}"
STRICT_CONTEXTS="${STRICT_CONTEXTS:-}"

for tool in gh jq; do
  command -v "$tool" > /dev/null 2>&1 || { echo "missing tool: $tool" >&2; exit 2; }
done

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# 1a. Required contexts from the rulesets that apply to the branch. Any failure stops the script.
gh api "repos/$REPO/rules/branches/$BASE" > "$WORK/rules.json" || fail "could not read the rulesets for '$BASE'"
jq -r '.[] | select(.type == "required_status_checks") | .parameters.required_status_checks[].context' \
  "$WORK/rules.json" > "$WORK/required.raw"

# 1b. Classic branch protection. ONLY an HTTP 404 means "none configured"; a 403, a 5xx or a network
# failure means the answer is unknown, and certifying on a partial set is the failure this script prevents.
if gh api -i "repos/$REPO/branches/$BASE/protection/required_status_checks" > "$WORK/classic.raw" 2> "$WORK/classic.err"; then
  awk 'b { print } /^\r?$/ { b = 1 }' "$WORK/classic.raw" > "$WORK/classic.json"
  jq -r '(.contexts[]?), (.checks[]?.context)' "$WORK/classic.json" >> "$WORK/required.raw"
else
  status="$(sed -n '1s/^HTTP\/[0-9.]* \([0-9][0-9]*\).*/\1/p' "$WORK/classic.raw")"
  if [ "$status" != "404" ]; then
    fail "classic branch-protection lookup failed (HTTP ${status:-unknown}): $(tr '\n' ' ' < "$WORK/classic.err")"
  fi
fi
sort -u "$WORK/required.raw" > "$WORK/required.txt"
[ -s "$WORK/required.txt" ] || fail "no required status checks are configured for '$BASE'; there is nothing to certify against"

# 2. What actually reported: one "<name><TAB><state>" row per check run or commit status.
gh pr view "$PR" -R "$REPO" --json statusCheckRollup > "$WORK/rollup.json" || fail "could not read the checks of PR $PR"
jq -r '.statusCheckRollup[]
  | [ (.name // .context),
      (if .__typename == "StatusContext" then .state
       elif (.conclusion // "") != "" then .conclusion
       else .status end) ]
  | @tsv' "$WORK/rollup.json" | sort > "$WORK/reported.tsv"
cut -f1 "$WORK/reported.tsv" | sort -u > "$WORK/reported.txt"

# 3. The set difference: required but never reported.
missing="$(comm -23 "$WORK/required.txt" "$WORK/reported.txt")"
if [ -n "$missing" ]; then
  echo "required contexts that never reported:" >&2
  echo "$missing" | sed 's/^/  /' >&2
  fail "$(echo "$missing" | wc -l | tr -d ' ') of $(wc -l < "$WORK/required.txt" | tr -d ' ') required contexts are absent"
fi

# 4. Reported, but not green (a re-run leaves several rows per name; this is deliberately conservative).
notgreen="$(awk -F'\t' -v strict="$STRICT_CONTEXTS" '
  BEGIN { n = split(strict, s, " "); for (i = 1; i <= n; i++) must[s[i]] = 1 }
  NR == FNR { req[$1] = 1; next }
  ($1 in req) {
    ok = ($2 == "SUCCESS" || $2 == "SKIPPED" || $2 == "NEUTRAL")
    if (($1 in must) && $2 != "SUCCESS") ok = 0
    if (!ok) print "  " $1 " = " $2
  }' "$WORK/required.txt" "$WORK/reported.tsv")"
[ -z "$notgreen" ] || { echo "$notgreen" >&2; fail "required contexts are not green"; }

echo "certified: all $(wc -l < "$WORK/required.txt" | tr -d ' ') required contexts reported and are green"
