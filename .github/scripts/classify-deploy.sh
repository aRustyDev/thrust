#!/bin/sh
# What does this deploy change, compared with the version that is live? (issue #49, ADR-0110 round 29)
#
#   .github/scripts/classify-deploy.sh <previous-tag|none> <head-sha>
#
# <previous-tag> is the tag of the active Worker version, which is the commit the pipeline deployed (`wrangler deploy --tag <sha>`), or
# `none` when the Worker has no active version. Run it inside a checkout that has the history (fetch-depth: 0).
#
# Prints `key=value` lines, ready for $GITHUB_OUTPUT:
#   migrations_changed   true when the expand migration has to run: a file under the migrations directory differs between the two
#                        commits. A deploy that changed none has nothing for `migrate` to apply (it applies only the files the
#                        database's ledger lacks, issue #195), so it does not open the tunnel to run it.
#   do_lifecycle         true when the deploy changes the Durable Object lifecycle, so it can neither be gradual nor be rolled back
#                        across (ADR-0110): the `[[migrations]]` tables of the deploy template differ. It goes through the manual gate.
#   reason               one line saying why, for the job summary
#
# The two commits are compared by what they hold, so neither has to descend from the other. release-plz force-updates the
# release-candidate branch, so the commit that is live is usually in no branch's history any more, and a deploy whose only difference
# from it is a release commit has nothing for a person to decide. What a difference in either direction means is the same: a migration
# file that differs makes `migrate` look (it applies what the ledger lacks and refuses what is out of order), and a `[[migrations]]`
# table that differs is a lifecycle change.
#
# Fail safe. Anything it cannot establish counts as a change, and as a lifecycle change, so the deploy takes the manual gate and does
# not roll back by itself: no active version (the first deploy to an environment carries `[[migrations]] v1`); a previous tag that is
# not a commit in this checkout (deployed some other way, or a commit nobody could fetch: the caller asks for it first); a head that
# is OLDER than the active version, an ancestor of it (a redeploy of older code, which is a person's decision); a checkout too shallow
# to tell which of the two is older; a template that did not exist at the previous commit.
#
# Environment: DEPLOY_TEMPLATE (default crates/worker/wrangler.deploy.toml), MIGRATIONS_DIR (default
# crates/adapters/postgres/migrations), both relative to the repository root.
# Exit status: 0 it classified (whatever the answer); 2 it was called wrongly or the head commit has no template.
set -u
LC_ALL=C
export LC_ALL

TEMPLATE="${DEPLOY_TEMPLATE:-crates/worker/wrangler.deploy.toml}"
MIGRATIONS="${MIGRATIONS_DIR:-crates/adapters/postgres/migrations}"

usage() { echo "classify-deploy: $*" >&2; exit 2; }
emit() { printf 'migrations_changed=%s\ndo_lifecycle=%s\nreason=%s\n' "$1" "$2" "$3"; exit 0; }

PREV="${1:-}"
HEAD_SHA="${2:-}"
[ -n "$PREV" ] && [ -n "$HEAD_SHA" ] || usage "usage: classify-deploy.sh <previous-tag|none> <head-sha>"

git cat-file -e "$HEAD_SHA^{commit}" 2> /dev/null || usage "head '$HEAD_SHA' is not a commit in this checkout"
git cat-file -e "$HEAD_SHA:$TEMPLATE" 2> /dev/null || usage "the deploy template $TEMPLATE does not exist at $HEAD_SHA"

[ "$PREV" = none ] && emit true true "no active version: the first deploy to this environment (it carries the Durable Object migration and the whole schema)"

# a tag that is not a commit here is not something to diff against: hex only, then it has to resolve to a commit
case "$PREV" in
  *[!0-9a-f]*) emit true true "the active version's tag '$PREV' is not a commit sha: it was not deployed by this pipeline" ;;
esac
git cat-file -e "$PREV^{commit}" 2> /dev/null \
  || emit true true "the active version's tag $PREV is not a commit in this checkout (it could not be fetched, or it was deployed from elsewhere)"

# Which of the two is older. Both are resolved first, so that a short sha and the same commit under its full name are one commit.
PREV_ID="$(git rev-parse --verify "$PREV^{commit}" 2> /dev/null)" || emit true true "the active version's tag $PREV does not resolve to a commit"
HEAD_ID="$(git rev-parse --verify "$HEAD_SHA^{commit}" 2> /dev/null)" || usage "head '$HEAD_SHA' does not resolve to a commit"
if [ "$PREV_ID" != "$HEAD_ID" ]; then
  # a shallow checkout cuts the history short, so "not an ancestor" there may only mean "the history is not here": no guess is made
  [ "$(git rev-parse --is-shallow-repository 2> /dev/null)" = false ] \
    || emit true true "this checkout is shallow, so it cannot be told whether $HEAD_SHA is older than the active version ($PREV)"
  git merge-base --is-ancestor "$HEAD_ID" "$PREV_ID"
  case $? in
    0) emit true true "$HEAD_SHA is older than the active version ($PREV): a redeploy of older code needs a person's decision" ;;
    1) ;;
    *) emit true true "it could not be told whether $HEAD_SHA is older than the active version ($PREV)" ;;
  esac
fi
git cat-file -e "$PREV:$TEMPLATE" 2> /dev/null \
  || emit true true "the deploy template did not exist at the active version ($PREV)"

# the `[[migrations]]` tables of the template at a commit: comments, blank lines and trailing spaces removed, so only meaning is compared
lifecycle_tables() {
  git show "$1:$TEMPLATE" | awk '
    /^\[\[migrations\]\][ \t]*(#.*)?$/ { keep = 1; print "[[migrations]]"; next }
    /^\[/ { keep = 0; next }
    keep {
      line = $0
      sub(/[ \t]+#.*$/, "", line)
      sub(/^[ \t]+/, "", line)
      sub(/[ \t]+$/, "", line)
      if (line != "" && line !~ /^#/) print line
    }
  '
}

if git diff --quiet "$PREV" "$HEAD_SHA" -- "$MIGRATIONS"; then
  migrations=false
else
  migrations=true
fi

before="$(lifecycle_tables "$PREV")"
after="$(lifecycle_tables "$HEAD_SHA")"
if [ "$before" = "$after" ]; then
  lifecycle=false
else
  lifecycle=true
fi

# neither an ancestor of the other: say so, so that a person reading the summary knows the comparison was of content alone
how="compared with the active version $PREV"
if [ "$PREV_ID" != "$HEAD_ID" ] && ! git merge-base --is-ancestor "$PREV_ID" "$HEAD_ID"; then
  how="compared by content with the active version $PREV, which $HEAD_SHA does not descend from (a rewritten release-candidate branch)"
fi
emit "$migrations" "$lifecycle" "$how: migrations $( [ "$migrations" = true ] && echo changed || echo unchanged ), Durable Object migrations $( [ "$lifecycle" = true ] && echo changed || echo unchanged )"
