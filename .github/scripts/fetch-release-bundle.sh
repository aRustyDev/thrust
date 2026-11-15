#!/bin/sh
# Get the deploy bundle of a published release, for production (the release flow, .agents/plans/cicd-review PR 3).
#
#   .github/scripts/fetch-release-bundle.sh <owner/repo> <tag> <dest-dir>
#
# Production deploys what its release carries, not a new build: release.yml built the bundle at the tag, attested it, and attached it
# to the release before publishing it, and releases are immutable once published (a repository setting), so the asset cannot be
# swapped afterwards. This takes it only when:
#   * the tag is a release tag, v<version> with no slash (the tag ruleset matches one segment);
#   * the release is published (not a draft: a draft's assets can still change) and immutable;
#   * it has exactly one asset named deploy-bundle.tar.gz;
#   * `gh release verify-asset` (GitHub's own attestation of an immutable release and its assets) says the downloaded file is that
#     release's asset.
# Who BUILT it is the next check, verify-bundle.sh (release.yml's build provenance on refs/tags/<tag>), and which commit it is for is
# unpack-bundle.sh's (BUILD_SHA).
#
# Prints the bundle's path. Needs gh (GH_TOKEN with contents: read) and jq.
# Exit status: 0 fetched and verified; 1 refused (why, on stderr); 2 called wrongly.
set -u
LC_ALL=C
export LC_ALL

usage() { echo "fetch-release-bundle: $*" >&2; exit 2; }
refuse() { echo "fetch-release-bundle: refused: $*" >&2; exit 1; }

REPO="${1:-}"; TAG="${2:-}"; DEST="${3:-}"
[ -n "$REPO" ] && [ -n "$TAG" ] && [ -n "$DEST" ] || usage "usage: fetch-release-bundle.sh <owner/repo> <tag> <dest-dir>"
case "$REPO" in */*) ;; *) usage "'$REPO' is not owner/repo" ;; esac
case "$TAG" in v*/*|v) refuse "'$TAG' is not a release tag (v<version>)" ;; v*) ;; *) refuse "'$TAG' is not a release tag (v<version>)" ;; esac
for tool in gh jq; do
  command -v "$tool" > /dev/null 2>&1 || usage "$tool is not installed"
done

WORK="$(mktemp -d)" || usage "cannot make a temporary directory"
trap 'rm -rf "$WORK"' EXIT INT TERM

gh release view "$TAG" --repo "$REPO" --json tagName,isDraft,isImmutable,assets > "$WORK/release.json" 2> "$WORK/err" \
  || refuse "could not read the release $TAG: $(tr '\n' ' ' < "$WORK/err")"
jq -e --arg tag "$TAG" '.tagName == $tag' "$WORK/release.json" > /dev/null 2>&1 || refuse "the release read is not $TAG"
jq -e '.isDraft == false' "$WORK/release.json" > /dev/null 2>&1 \
  || refuse "the release $TAG is a draft: release.yml has not published it yet (or failed); a draft's assets can still change"
jq -e '.isImmutable == true' "$WORK/release.json" > /dev/null 2>&1 \
  || refuse "the release $TAG is not immutable: turn immutable releases on (repository settings) before trusting its assets"
n="$(jq '[.assets[]? | select(.name == "deploy-bundle.tar.gz")] | length' "$WORK/release.json" 2> /dev/null)" \
  || refuse "the release's JSON is not what this reads"
[ "$n" = 1 ] || refuse "the release $TAG has $n assets named deploy-bundle.tar.gz, not one"

mkdir -p "$DEST" || usage "cannot make $DEST"
rm -f "$DEST/deploy-bundle.tar.gz"
gh release download "$TAG" --repo "$REPO" --pattern deploy-bundle.tar.gz --dir "$DEST" 2> "$WORK/err" \
  || refuse "could not download the bundle of $TAG: $(tr '\n' ' ' < "$WORK/err")"
[ -s "$DEST/deploy-bundle.tar.gz" ] || refuse "the download of $TAG's bundle is empty or missing"
gh release verify-asset "$TAG" "$DEST/deploy-bundle.tar.gz" --repo "$REPO" > /dev/null 2> "$WORK/err" || {
  rm -f "$DEST/deploy-bundle.tar.gz"
  refuse "the downloaded bundle is not $TAG's attested asset: $(tr '\n' ' ' < "$WORK/err")"
}
echo "$DEST/deploy-bundle.tar.gz"
