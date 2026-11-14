#!/bin/sh
# After a deploy: is a file that ONLY the previous release had still served, as that file? (issue #49 acceptance 5, ADR-0011)
#
#   scripts/check-asset-retention.sh <base-url> <previous-dir> <new-dir>
#
# <previous-dir> and <new-dir> are the two releases' own builds (the web files of their bundles). The check takes up to
# CHECK_ASSET_COUNT (default 3) scripts under assets/ that the previous release had and the new one does not, asks the live hostname for
# each, and requires all of this: status 200; a JavaScript content type, and not `text/html`; and the very bytes of the previous release's
# file. The last is what makes the first two mean something. Workers static assets answer a missing hashed file with the single-page
# fallback, `200 text/html` (ADR-0011, Q-0011.1), so a status of 200 alone passes for exactly the failure this exists to catch.
#
# When the new release dropped no script, there is nothing to look for, and it says so and exits 0: a release that changed only the API
# keeps every hash. That is stated, not hidden: the deploy's summary says the check did not look at anything.
#
# Propagation to the edge takes a few seconds: each file is asked for up to CHECK_ASSET_ATTEMPTS times (default 6), CHECK_ASSET_INTERVAL
# seconds apart (default 3).
#
# Exit status: 0 every file asked for was served as itself (or none was asked for); 1 one was not; 2 called wrongly.
set -u
LC_ALL=C
export LC_ALL

fail() { echo "check-asset-retention: FAIL: $*" >&2; exit 1; }
usage() { echo "check-asset-retention: $*" >&2; exit 2; }

BASE="${1:-}"; PREV="${2:-}"; NEW="${3:-}"
[ -n "$BASE" ] && [ -n "$PREV" ] && [ -n "$NEW" ] || usage "usage: check-asset-retention.sh <base-url> <previous-dir> <new-dir>"
case "$BASE" in http://*|https://*) ;; *) usage "the base URL must start with http:// or https://" ;; esac
BASE="${BASE%/}"
[ -d "$PREV" ] || usage "$PREV is not a directory"
[ -d "$NEW" ] || usage "$NEW is not a directory"
COUNT="${CHECK_ASSET_COUNT:-3}"; ATTEMPTS="${CHECK_ASSET_ATTEMPTS:-6}"; INTERVAL="${CHECK_ASSET_INTERVAL:-3}"
for v in "$COUNT" "$ATTEMPTS"; do
  case "$v" in ''|*[!0-9]*|0) usage "CHECK_ASSET_COUNT and CHECK_ASSET_ATTEMPTS must be positive whole numbers" ;; esac
done
command -v curl > /dev/null 2>&1 || usage "curl is not installed"

WORK="$(mktemp -d)" || usage "cannot make a temporary directory"
trap 'rm -rf "$WORK"' EXIT INT TERM

# the scripts the previous release served from assets/ that the new one does not have
( cd "$PREV" && find assets -type f -name '*.js' 2> /dev/null | sort ) > "$WORK/prev.list"
( cd "$NEW" && find assets -type f -name '*.js' 2> /dev/null | sort ) > "$WORK/new.list"
comm -23 "$WORK/prev.list" "$WORK/new.list" | head -"$COUNT" > "$WORK/retired.list"

if [ ! -s "$WORK/retired.list" ]; then
  echo "check-asset-retention: nothing to look for: the new release has every script the previous one had (it dropped none)"
  exit 0
fi

checked=0
while IFS= read -r rel; do
  # a space in a path is the only character a hashed build file name may have that a URL may not
  url="$BASE/$(printf '%s' "$rel" | sed 's/ /%20/g')"
  n=0; ok=0; why=""
  while [ "$n" -lt "$ATTEMPTS" ]; do
    n=$((n + 1))
    code="$(curl -sS --max-time 20 -o "$WORK/body" -D "$WORK/headers" -w '%{http_code}' "$url" 2> "$WORK/curl.err")" || code="000"
    ctype="$(grep -i '^content-type:' "$WORK/headers" 2> /dev/null | tail -1 | tr -d '\r' | tr 'A-Z' 'a-z')"
    if [ "$code" != 200 ]; then
      why="answered $code, not 200"
    elif printf '%s' "$ctype" | grep -q 'text/html'; then
      why="answered 200 text/html: the single-page fallback, which is what a dropped hashed file gets (ADR-0011)"
    elif ! printf '%s' "$ctype" | grep -q 'javascript'; then
      why="answered 200 with the content type '${ctype#content-type: }', not JavaScript"
    elif ! cmp -s "$WORK/body" "$PREV/$rel"; then
      why="answered 200 JavaScript, but not the bytes of the previous release's file"
    else
      ok=1; break
    fi
    [ "$n" -lt "$ATTEMPTS" ] && sleep "$INTERVAL"
  done
  [ "$ok" = 1 ] || fail "$url $why (after $n attempts): a tab still running the previous release would be given this instead of its script"
  checked=$((checked + 1))
done < "$WORK/retired.list"

echo "check-asset-retention: $checked script(s) of the previous release are still served as themselves"
