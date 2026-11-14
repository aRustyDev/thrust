#!/bin/sh
# Unpack a deploy bundle, refusing anything that is not plain files, and check which commit it says it was built from (issue #49).
#
#   .github/scripts/unpack-bundle.sh <deploy-bundle.tar.gz> <dest-dir> <expected-sha>
#
# A bundle is attested (verify-bundle.sh), and this does not replace that: it is the second lock, on what the archive can DO when it is
# extracted. It refuses a member whose path is absolute or has a `..` in it (it would write outside <dest-dir>), and a member that is not a
# regular file or a directory (a symlink or a hard link could point anywhere, a device is not a file), before anything is extracted.
# After it extracts, BUILD_SHA, at the top of the bundle, must be exactly <expected-sha>: the attestation says WHO built the file, and
# this says which commit the file is for, so a bundle of another commit cannot be handed to a deploy of this one.
#
# Exit status: 0 unpacked; 1 refused (why, on stderr; nothing is left in <dest-dir> when the archive itself was refused); 2 called wrongly.
set -u
LC_ALL=C
export LC_ALL

refuse() { echo "unpack-bundle: refused: $*" >&2; exit 1; }
usage() { echo "unpack-bundle: $*" >&2; exit 2; }

BUNDLE="${1:-}"; DEST="${2:-}"; WANT="${3:-}"
[ -n "$BUNDLE" ] && [ -n "$DEST" ] && [ -n "$WANT" ] || usage "usage: unpack-bundle.sh <deploy-bundle.tar.gz> <dest-dir> <expected-sha>"
[ -f "$BUNDLE" ] || usage "$BUNDLE is not a file"
case "$WANT" in *[!0-9a-f]*) usage "the expected sha is not a sha" ;; esac
[ "${#WANT}" -eq 40 ] || usage "the expected sha is not 40 hex digits"

WORK="$(mktemp -d)" || usage "cannot make a temporary directory"
trap 'rm -rf "$WORK"' EXIT INT TERM

# the names, with `./` in front of them as the bundle is made, and then the types
tar -tzf "$BUNDLE" > "$WORK/names" 2> "$WORK/err" || refuse "the archive cannot be read: $(tr '\n' ' ' < "$WORK/err")"
[ -s "$WORK/names" ] || refuse "the archive is empty"
if grep -Eq '(^/|(^|/)\.\.(/|$))' "$WORK/names"; then
  refuse "a member has an absolute path or a '..' in it: $(grep -E '(^/|(^|/)\.\.(/|$))' "$WORK/names" | head -3 | tr '\n' ' ')"
fi
tar -tvzf "$BUNDLE" > "$WORK/verbose" 2> "$WORK/err" || refuse "the archive cannot be listed: $(tr '\n' ' ' < "$WORK/err")"
# the first character of a verbose line is the type: `-` a file, `d` a directory; anything else (l, h, c, b, p) is refused. A link is also
# written ` -> target` or ` link to target`, which is checked too, because tar implementations differ in how they show a hard link.
if grep -Ev '^[-d]' "$WORK/verbose" > "$WORK/odd" && [ -s "$WORK/odd" ]; then
  refuse "a member is not a plain file or directory: $(head -2 "$WORK/odd" | tr '\n' ' ')"
fi
if grep -Eq ' -> | link to ' "$WORK/verbose"; then
  refuse "a member is a link: $(grep -E ' -> | link to ' "$WORK/verbose" | head -2 | tr '\n' ' ')"
fi

mkdir -p "$DEST" || usage "cannot make $DEST"
tar -xzf "$BUNDLE" -C "$DEST" 2> "$WORK/err" || refuse "extraction failed: $(tr '\n' ' ' < "$WORK/err")"

[ -f "$DEST/BUILD_SHA" ] || refuse "the bundle has no BUILD_SHA, so it does not say which commit it is for"
built="$(cat "$DEST/BUILD_SHA")"
[ "$built" = "$WANT" ] || refuse "the bundle was built from '$built', not from $WANT"

echo "unpack-bundle: unpacked $(grep -vc '/$' "$WORK/names") files into $DEST (built from $built)"
