#!/bin/sh
# Do the environments that are meant to wait for a person really have reviewers? (issue #49, ADR-0110 round 29)
#
#   .github/scripts/assert-gated-environments.sh <owner/repo> <environment> [environment ...]
#
# Exit 0 only when every named environment exists and has a `required_reviewers` rule with at least one reviewer in it.
#
# Why: GitHub CREATES an environment, with no protection at all, the first time a job names one that does not exist. A renamed or deleted
# `production-do-lifecycle` would not fail a deploy: it would silently open the gate the deploy relies on, and nothing would show it
# until a deploy that should have waited did not. So the plan job asks, by name, before it lets a deploy start.
#
# Fail closed: an environment that cannot be read (a token without the permission, the API failing) is not "probably fine".
# Needs: gh (authenticated) and jq. Exit status: 0 gated; 1 not gated or not readable (why, on stderr); 2 called wrongly.
set -eu

fail() { echo "NOT GATED: $*" >&2; exit 1; }

REPO="${1:?usage: assert-gated-environments.sh <owner/repo> <environment> [environment ...]}"
shift
[ $# -ge 1 ] || { echo "usage: assert-gated-environments.sh <owner/repo> <environment> [environment ...]" >&2; exit 2; }
for tool in gh jq; do
  command -v "$tool" > /dev/null 2>&1 || { echo "missing tool: $tool" >&2; exit 2; }
done

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

problems=""
for name in "$@"; do
  case "$name" in *[!0-9A-Za-z._-]*|'') echo "assert-gated-environments: '$name' is not an environment name" >&2; exit 2 ;; esac
  if ! gh api "repos/$REPO/environments/$name" > "$WORK/env.json" 2> "$WORK/err"; then
    if grep -q 'HTTP 404' "$WORK/err"; then
      problems="$problems
  $name: does not exist (the first job to name it would create it, with no reviewers)"
    else
      problems="$problems
  $name: could not be read: $(tr '\n' ' ' < "$WORK/err")"
    fi
    continue
  fi
  reviewers="$(jq -r '[.protection_rules[]? | select(.type == "required_reviewers") | (.reviewers // []) | length] | add // 0' "$WORK/env.json" 2> "$WORK/jq.err")" \
    || { problems="$problems
  $name: the answer is not the JSON this reads: $(tr '\n' ' ' < "$WORK/jq.err")"; continue; }
  case "$reviewers" in
    ''|*[!0-9]*) problems="$problems
  $name: the answer is not the JSON this reads"; continue ;;
  esac
  if [ "$reviewers" -lt 1 ]; then
    problems="$problems
  $name: has no required reviewers, so a job in it would not wait for anyone"
  fi
done

if [ -n "$problems" ]; then
  echo "$problems" >&2
  fail "$(echo "$problems" | grep -c .) of $# environments are not gated"
fi
echo "gated: $* each have required reviewers"
