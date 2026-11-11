#!/bin/sh
# The Q-R2 gate for web/: the stock React app and its packages may differ from the oracle pin only where the owner allowed.
#
# web/apps/web and web/packages were imported byte-identical from oracle/v1-pin-20261018 (web/SOURCE). Q-R2 lets the app
# change only in a few places (the auth client, the base URL and service layer, the collab provider, branding). The pin
# exists only in a local clone, so this is checked against two files committed beside the import:
#
#   web/PIN-MANIFEST      `git ls-tree -r <pin> apps/web packages`: the mode and blob hash of every file the pin has there.
#   web/ALLOWED-CHANGES   the paths, relative to web/, that may differ from it. One per line; '#' starts a comment.
#
# The set of paths under web/apps/web and web/packages that differ from the manifest (changed, added or deleted) must EQUAL
# the allowed set. "Equal" and not "inside": an entry that no longer differs is a stale line that would let a later change
# through unreviewed, so it fails too. The file is the ledger of how the app departs from the pin.
#
# A manifest that was edited to hide a change must also rebuild the tree hashes that web/SOURCE records for the pin, so the
# manifest is checked against those too. Nothing here passes on an empty list: the manifest and the tree must both have files.
#
#   scripts/check-web-diff.sh [<commit>]        default HEAD; reads git objects only, never the working tree
#
# To refresh the manifest (only when the import is redone from a new pin), in the fork's clone:
#   git ls-tree -r <pin> apps/web packages > <thrust>/web/PIN-MANIFEST
set -eu

ref="${1:-HEAD}"
LC_ALL=C
export LC_ALL
tab="$(printf '\t')"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

fail() {
  echo "WEB DIFF FAIL: $*" >&2
  exit 1
}

show() { git show "$ref:$1" 2> /dev/null; }

show web/PIN-MANIFEST > "$tmp/pin.raw" || fail "no web/PIN-MANIFEST at $ref"
[ -s "$tmp/pin.raw" ] || fail "web/PIN-MANIFEST is empty; refusing to pass on zero files"
show web/ALLOWED-CHANGES > "$tmp/allowed.raw" || fail "no web/ALLOWED-CHANGES at $ref (an empty list is a file with only comments)"
show web/SOURCE > "$tmp/source" || fail "no web/SOURCE at $ref"

git ls-tree -r "$ref" -- web/apps/web web/packages | sed "s#${tab}web/#${tab}#" > "$tmp/have.raw"
[ -s "$tmp/have.raw" ] || fail "web/apps/web and web/packages have no files at $ref; refusing to pass on zero files"

# 1. The manifest is the pin's: it rebuilds the tree hashes web/SOURCE records. (--missing-ok: a blob the pin has and this
#    repository changed is not in its object database.)
GIT_INDEX_FILE="$tmp/index" git update-index --add --index-info < "$tmp/pin.raw"
tree="$(GIT_INDEX_FILE="$tmp/index" git write-tree --missing-ok)"
for name in apps_web packages; do
  want="$(sed -n "s/^source_tree_${name}=//p" "$tmp/source")"
  [ -n "$want" ] || fail "web/SOURCE has no source_tree_${name}"
  dir="$(printf '%s' "$name" | sed 's#_#/#')"
  got="$(git rev-parse "$tree:$dir")"
  [ "$got" = "$want" ] || fail "web/PIN-MANIFEST does not rebuild the pin's tree $dir: it makes $got, web/SOURCE says $want"
done

# 2. The paths that differ: one 'path<TAB>mode type sha' line per file on each side, the lines only one side has, the paths of those.
awk -F "$tab" '{print $2 "\t" $1}' "$tmp/pin.raw" | sort > "$tmp/pin.kv"
awk -F "$tab" '{print $2 "\t" $1}' "$tmp/have.raw" | sort > "$tmp/have.kv"
comm -3 "$tmp/pin.kv" "$tmp/have.kv" | sed "s/^${tab}*//" | cut -f1 | sort -u > "$tmp/differs"

# 3. The allowed set.
sed -e 's/#.*//' -e 's/[[:space:]]*$//' -e 's/^[[:space:]]*//' -e '/^$/d' "$tmp/allowed.raw" | sort -u > "$tmp/allowed"
outside="$(grep -v -E '^(apps/web|packages)/' "$tmp/allowed" || true)"
[ -z "$outside" ] || fail "web/ALLOWED-CHANGES lists paths outside apps/web and packages, which this does not compare: $outside"

unlisted="$(comm -23 "$tmp/differs" "$tmp/allowed")"
[ -z "$unlisted" ] || fail "these files differ from the pin and are not in web/ALLOWED-CHANGES (Q-R2): $(printf '%s' "$unlisted" | tr '\n' ' ')"
stale="$(comm -13 "$tmp/differs" "$tmp/allowed")"
[ -z "$stale" ] || fail "these are in web/ALLOWED-CHANGES but do not differ from the pin; remove the line: $(printf '%s' "$stale" | tr '\n' ' ')"

echo "ok: $(wc -l < "$tmp/pin.raw" | tr -d ' ') files in the pin's apps/web and packages, $(wc -l < "$tmp/differs" | tr -d ' ') differ, all of them allowed"
