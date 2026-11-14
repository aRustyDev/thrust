#!/bin/sh
# Get the previous release's bundle, verified and unpacked, so that its web files can be kept under the new release's (issue #49, ADR-0011).
#
#   .github/scripts/fetch-previous-assets.sh <owner/repo> <previous-sha> <dest-dir>
#
# The previous release is the commit whose tag the live Worker version has. Its bundle was uploaded as the artifact
# `deploy-bundle-<previous-sha>` by an earlier run of deploy.yml. This finds it, downloads it, and treats it as hostile until it is not:
#   * only an artifact made by a run on `main`, and not expired, is looked at. Artifact names are not owned by anyone: a pull request from a
#     fork can upload one under any name;
#   * verify-bundle.sh: it must carry an attestation from this repository's deploy.yml on main, which a forged artifact cannot;
#   * unpack-bundle.sh: plain files only, and BUILD_SHA must be <previous-sha>.
# Of a newest-first list it takes the newest that passes the first filter, not the first that passes every check, so a good older one can
# never stand in for a bad newer one.
#
# Prints `bundle=<dest-dir>/bundle` and `web=true|false` (whether the bundle has a web/ with an index.html: bundles from before the web app
# was part of them have none, and that is not an error). Exit status: 0 verified and unpacked; 3 there is no usable artifact; 4 there is
# one and it failed a check, or GitHub could not be asked (why, on stderr); 2 called wrongly.
#
# Test hooks: VERIFY_BUNDLE_SCRIPT, UNPACK_BUNDLE_SCRIPT. Needs gh (GH_TOKEN with `actions: read`), jq and unzip.
set -u
LC_ALL=C
export LC_ALL

HERE="$(cd "$(dirname "$0")" && pwd)"
VERIFY="${VERIFY_BUNDLE_SCRIPT:-$HERE/verify-bundle.sh}"
UNPACK="${UNPACK_BUNDLE_SCRIPT:-$HERE/unpack-bundle.sh}"

usage() { echo "fetch-previous-assets: $*" >&2; exit 2; }
none() { echo "fetch-previous-assets: no usable artifact: $*" >&2; exit 3; }
refuse() { echo "fetch-previous-assets: refused: $*" >&2; exit 4; }

REPO="${1:-}"; SHA="${2:-}"; DEST="${3:-}"
[ -n "$REPO" ] && [ -n "$SHA" ] && [ -n "$DEST" ] || usage "usage: fetch-previous-assets.sh <owner/repo> <previous-sha> <dest-dir>"
case "$REPO" in */*) ;; *) usage "'$REPO' is not owner/repo" ;; esac
case "$SHA" in *[!0-9a-f]*|'') usage "'$SHA' is not a commit sha" ;; esac
[ "${#SHA}" -eq 40 ] || usage "'$SHA' is not 40 hex digits"
for tool in gh jq unzip; do
  command -v "$tool" > /dev/null 2>&1 || usage "$tool is not installed"
done

WORK="$(mktemp -d)" || usage "cannot make a temporary directory"
trap 'rm -rf "$WORK"' EXIT INT TERM

NAME="deploy-bundle-$SHA"
gh api "repos/$REPO/actions/artifacts?name=$NAME&per_page=100" > "$WORK/list.json" 2> "$WORK/err" \
  || refuse "could not list the artifacts of $REPO: $(tr '\n' ' ' < "$WORK/err")"
jq -r --arg name "$NAME" '[.artifacts[]? | select(.name == $name and .expired == false and .workflow_run.head_branch == "main")]
    | sort_by(.created_at) | last | .id // empty' "$WORK/list.json" > "$WORK/id" 2> "$WORK/err" \
  || refuse "the list of artifacts is not the JSON this reads: $(tr '\n' ' ' < "$WORK/err")"
ID="$(cat "$WORK/id")"
if [ -z "$ID" ]; then
  total="$(jq -r '[.artifacts[]? | select(.name == "'"$NAME"'")] | length' "$WORK/list.json" 2> /dev/null || echo "?")"
  none "no unexpired $NAME from a run on main ($total artifact(s) of that name in all; an expired one, or one from another branch, does not count)"
fi

gh api "repos/$REPO/actions/artifacts/$ID/zip" > "$WORK/artifact.zip" 2> "$WORK/err" \
  || refuse "could not download artifact $ID: $(tr '\n' ' ' < "$WORK/err")"
mkdir -p "$WORK/dl"
unzip -q -o "$WORK/artifact.zip" -d "$WORK/dl" 2> "$WORK/err" || refuse "artifact $ID is not a zip file: $(tr '\n' ' ' < "$WORK/err")"
[ -f "$WORK/dl/deploy-bundle.tar.gz" ] || refuse "artifact $ID has no deploy-bundle.tar.gz in it"

sh "$VERIFY" "$WORK/dl/deploy-bundle.tar.gz" "$REPO" >&2 || refuse "artifact $ID did not verify (see above)"
mkdir -p "$DEST" || usage "cannot make $DEST"
rm -rf "$DEST/bundle"
sh "$UNPACK" "$WORK/dl/deploy-bundle.tar.gz" "$DEST/bundle" "$SHA" >&2 || { rm -rf "$DEST/bundle"; refuse "artifact $ID did not unpack (see above)"; }

echo "bundle=$DEST/bundle"
if [ -f "$DEST/bundle/web/index.html" ]; then echo "web=true"; else echo "web=false"; fi
