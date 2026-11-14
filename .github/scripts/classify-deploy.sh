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
#                        commits. `migrate` re-runs every file each time, including DROP and ADD CONSTRAINT, so a deploy that did not
#                        change them does not run it.
#   do_lifecycle         true when the deploy changes the Durable Object lifecycle, so it can neither be gradual nor be rolled back
#                        across (ADR-0110): the `[[migrations]]` tables of the deploy template differ. It goes through the manual gate.
#   reason               one line saying why, for the job summary
#
# Fail safe. Anything it cannot establish counts as a change, and as a lifecycle change, so the deploy takes the manual gate and does
# not roll back by itself: no active version (the first deploy to an environment carries `[[migrations]] v1`); a previous tag that is
# not a commit in this checkout (deployed some other way, or history not fetched); a previous commit that is not an ancestor of
# <head-sha> (a redeploy of older code, a force-pushed branch); a template that did not exist at the previous commit.
#
# Environment: DEPLOY_TEMPLATE (default crates/thrust-worker/wrangler.deploy.toml), MIGRATIONS_DIR (default
# crates/thrust-adapters-postgres/migrations), both relative to the repository root.
# Exit status: 0 it classified (whatever the answer); 2 it was called wrongly or the head commit has no template.
set -u
LC_ALL=C
export LC_ALL

TEMPLATE="${DEPLOY_TEMPLATE:-crates/thrust-worker/wrangler.deploy.toml}"
MIGRATIONS="${MIGRATIONS_DIR:-crates/thrust-adapters-postgres/migrations}"

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
  || emit true true "the active version's tag $PREV is not a commit in this checkout (history not fetched, or deployed from elsewhere)"
git merge-base --is-ancestor "$PREV" "$HEAD_SHA" \
  || emit true true "the active version ($PREV) is not an ancestor of $HEAD_SHA: a redeploy of older or diverged code needs a person's decision"
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

emit "$migrations" "$lifecycle" "compared with the active version $PREV: migrations $( [ "$migrations" = true ] && echo changed || echo unchanged ), Durable Object migrations $( [ "$lifecycle" = true ] && echo changed || echo unchanged )"
