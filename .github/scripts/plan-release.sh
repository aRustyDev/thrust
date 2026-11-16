#!/bin/sh
# What should this run of release.yml do, and to which commit? (the release flow, docs/src/architecture/release.md)
#
#   .github/scripts/plan-release.sh        (everything comes from the environment)
#
# Prints `key=value` lines, ready for $GITHUB_OUTPUT:
#   mode     release   build the bundle at `tag`, attest it, attach it to the tag's draft release and publish it
#            dry-run   build the bundle and the SBOM; attest and publish nothing (a pull request)
#            skip      do nothing
#   tag      the release tag v<version> (mode release only; empty otherwise)
#   sha      the commit (40 hex digits; empty for a skip)
#   reason   one line
#
# The rules, which are the reason this is a script with a test and not an `if:` in YAML:
#   * a workflow_dispatch ON a release tag v<version> releases it. release-plz.yml dispatches it on the tag it just made, because a
#     draft release starts no workflow (GitHub runs nothing for the `created` event of a draft), and a person can dispatch it the
#     same way when that run never started. The tag must point at the run's own commit, and that commit must be on main: release-plz
#     tags the merge of an rc pull request, and only the release App can make a v* tag. `<version>` has no slash, as the tag ruleset
#     matches one path segment. A dispatch on any other ref is refused, not skipped: a person asked for something that cannot happen.
#     Whether the release is still a draft is the `publish` job's to check, with the token that can see drafts.
#   * a pull request is a dry run.
#   * anything else does nothing.
#
# Environment: EVENT_NAME, GITHUB_SHA; RUN_REF (github.ref) for workflow_dispatch, and MAIN_REF (default origin/main).
# Exit status: 0 it planned (possibly `skip`); 1 the request is refused; 2 it was called wrongly.
set -u
LC_ALL=C
export LC_ALL

MAIN_REF="${MAIN_REF:-origin/main}"

refuse() { echo "plan-release: refused: $*" >&2; exit 1; }
misuse() { echo "plan-release: $*" >&2; exit 2; }
emit() { printf 'mode=%s\ntag=%s\nsha=%s\nreason=%s\n' "$1" "$2" "$3" "$4"; exit 0; }
is_sha() { case "$1" in *[!0-9a-f]*|'') return 1 ;; esac; [ "${#1}" -eq 40 ]; }
is_release_name() { case "$1" in v*/*|v) return 1 ;; v*) return 0 ;; *) return 1 ;; esac; }

[ -n "${EVENT_NAME:-}" ] || misuse "EVENT_NAME is not set"
case "$EVENT_NAME" in
  pull_request|workflow_dispatch) is_sha "${GITHUB_SHA:-}" || misuse "GITHUB_SHA '${GITHUB_SHA:-}' is not a commit sha" ;;
esac

case "$EVENT_NAME" in
  pull_request) emit dry-run "" "$GITHUB_SHA" "a pull request: build the bundle and the SBOM; attest and publish nothing" ;;
  workflow_dispatch) ;;
  *) emit skip "" "" "'$EVENT_NAME' does not release" ;;
esac

REF="${RUN_REF:-}"
case "$REF" in
  refs/tags/*) TAG="${REF#refs/tags/}" ;;
  *) refuse "a release is run on its tag (refs/tags/v<version>), not on '$REF'" ;;
esac
is_release_name "$TAG" || refuse "'$TAG' is not a release tag (v<version>)"
git rev-parse --verify --quiet "$MAIN_REF^{commit}" > /dev/null || misuse "'$MAIN_REF' is not a commit: fetch main first"
tagged="$(git rev-parse --verify --quiet "refs/tags/$TAG^{commit}")" || refuse "there is no tag $TAG here"
[ "$tagged" = "$GITHUB_SHA" ] || refuse "the tag $TAG points at $tagged, not at this run's commit $GITHUB_SHA"
git merge-base --is-ancestor "$GITHUB_SHA" "$MAIN_REF" || refuse "$TAG ($GITHUB_SHA) is not a commit of main: a release tag marks a merge"
emit release "$TAG" "$GITHUB_SHA" "the release $TAG of $GITHUB_SHA: build, attest, attach, publish"
