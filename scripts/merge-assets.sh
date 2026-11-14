#!/bin/sh
# Put the previous release's web files UNDER this release's, so that a tab still running the last release can still load its scripts
# (issue #49, ADR-0011, ADR-0110 round 29).
#
#   scripts/merge-assets.sh <new-dir> <previous-dir | -> <out-dir>
#
# Why. The Worker serves the web app from Workers static assets with the single-page fallback, and a hashed file that is not there is
# answered `200 text/html`, not 404 (ADR-0011, Q-0011.1). A browser tab that loaded the last release asks for that release's chunks, by
# their hashes, as the user clicks around; if the new deploy dropped them it is given HTML where it wanted a script, and the app breaks
# until the tab is reloaded. A 100 % deploy has no version affinity to hide this (ADR-0110 round 29), so the cure is to keep the last
# release's files until its tabs age out. Each deploy keeps exactly one release back, N and N-1: <previous-dir> is the previous
# release's OWN build, never its merged directory, so files from older releases drop out one release later.
#
# <out-dir> is a copy of <new-dir>, plus every file of <previous-dir> that <new-dir> does not have. A path both have is the new
# release's (index.html, sw.js and the manifests are not hashed, and the new ones are the ones to serve). `-` as <previous-dir> means
# there is no previous release (the first deploy): <out-dir> is a copy of <new-dir>.
#
# What it refuses, so that nothing half-made is uploaded: a <new-dir> without index.html (the build is missing or broken); a
# <previous-dir> without one (that is not a web build); a symlink or a file name with a newline in either; an <out-dir> that already has
# something in it; more than MERGE_ASSETS_LIMIT files (default 20000, the most Workers static assets take on the free plan); and, after
# the copy, any file of <new-dir> that is not in <out-dir> byte for byte.
#
# Prints `merge-assets: new=<n> retained=<r> total=<t>`. Exit status: 0 merged; 1 refused (why, on stderr); 2 called wrongly.
set -u
LC_ALL=C
export LC_ALL

refuse() { echo "merge-assets: refused: $*" >&2; exit 1; }
usage() { echo "merge-assets: $*" >&2; exit 2; }

NEW="${1:-}"; PREV="${2:-}"; OUT="${3:-}"
[ -n "$NEW" ] && [ -n "$PREV" ] && [ -n "$OUT" ] || usage "usage: merge-assets.sh <new-dir> <previous-dir | -> <out-dir>"
LIMIT="${MERGE_ASSETS_LIMIT:-20000}"
case "$LIMIT" in ''|*[!0-9]*) usage "MERGE_ASSETS_LIMIT is not a number" ;; esac

[ -d "$NEW" ] || usage "$NEW is not a directory"
[ -f "$NEW/index.html" ] || refuse "$NEW has no index.html: the web build is missing or broken"
if [ "$PREV" != "-" ]; then
  [ -d "$PREV" ] || usage "$PREV is not a directory (use - for no previous release)"
  [ -f "$PREV/index.html" ] || refuse "$PREV has no index.html: it is not a web build, so it is not what the last release served"
fi
if [ -e "$OUT" ]; then
  [ -d "$OUT" ] || usage "$OUT exists and is not a directory"
  [ -z "$(ls -A "$OUT")" ] || usage "$OUT already has something in it: it is made fresh, never merged into"
fi

for d in "$NEW" "$PREV"; do
  [ "$d" = "-" ] && continue
  [ -z "$(find "$d" -type l | head -1)" ] || refuse "$d has a symlink: $(find "$d" -type l | head -1)"
  # a name with a newline would split in the loop below
  [ -z "$(find "$d" -name '*
*' | head -1)" ] || refuse "$d has a file name with a newline in it"
done

mkdir -p "$OUT" || usage "cannot make $OUT"
cp -R "$NEW/." "$OUT/" || refuse "could not copy $NEW"
new_count="$(cd "$NEW" && find . -type f | wc -l | tr -d ' ')"

retained=0
if [ "$PREV" != "-" ]; then
  # the files the previous release had and this one does not
  ( cd "$PREV" && find . -type f ) > "${TMPDIR:-/tmp}/merge-assets.$$.list" || refuse "could not list $PREV"
  while IFS= read -r f; do
    if [ ! -e "$OUT/$f" ]; then
      mkdir -p "$OUT/$(dirname "$f")" && cp "$PREV/$f" "$OUT/$f" || { rm -f "${TMPDIR:-/tmp}/merge-assets.$$.list"; refuse "could not copy $f from $PREV"; }
      retained=$((retained + 1))
    fi
  done < "${TMPDIR:-/tmp}/merge-assets.$$.list"
  rm -f "${TMPDIR:-/tmp}/merge-assets.$$.list"
fi

total="$(cd "$OUT" && find . -type f | wc -l | tr -d ' ')"
[ "$total" -le "$LIMIT" ] || { rm -rf "$OUT"; refuse "$total files is more than the $LIMIT that Workers static assets take"; }

# the postcondition: nothing of this release was lost or changed on the way
( cd "$NEW" && find . -type f ) | while IFS= read -r f; do
  cmp -s "$NEW/$f" "$OUT/$f" || { echo "merge-assets: refused: $f of the new release is not in $OUT byte for byte" >&2; exit 1; }
done || { rm -rf "$OUT"; exit 1; }
cmp -s "$NEW/index.html" "$OUT/index.html" || { rm -rf "$OUT"; refuse "index.html of $OUT is not the new release's"; }

echo "merge-assets: new=$new_count retained=$retained total=$total"
