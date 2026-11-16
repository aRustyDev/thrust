#!/bin/sh
# Build the book (docs/) and fail on what `mdbook build` lets through with exit 0: an ERROR line in its log (a broken
# {{#include}}, a mistyped key of a book.toml table) and a page under docs/src/ that SUMMARY.md does not list (mdBook leaves it
# out of the book without a word). Needs mdbook 0.5 and mdbook-mermaid on PATH (CI: the `docs` job of ci.yml).
#
#   scripts/check-book.sh               the book as it is
#   scripts/check-book.sh --self-test   the controls: a copy of the book broken four ways, each of which must be refused
#
# Exit status: 0 clean; 1 the book is broken (why, on stderr); 2 a call gone wrong (mdbook missing, no book).
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# check <repository root>: build that root's docs/ into a temporary directory and judge it
check() {
  book="$1/docs"
  [ -f "$book/book.toml" ] && [ -f "$book/src/SUMMARY.md" ] || { echo "check-book: no book at $book" >&2; return 2; }
  out="$(mktemp -d)"
  mdbook build "$book" -d "$out/html" > "$out/build.log" 2>&1
  st=$?
  failed=0
  if [ "$st" -ne 0 ]; then
    echo "check-book: mdbook build exited $st" >&2; failed=1
  fi
  if grep -qE '^ERROR' "$out/build.log"; then
    echo "check-book: mdbook build logged errors (it exits 0 on some of them):" >&2
    grep -E '^ERROR' "$out/build.log" >&2
    failed=1
  fi
  # SUMMARY.md links, with an optional ./ stripped, against the pages on disk
  grep -oE '\]\([^)#]+\.md' "$book/src/SUMMARY.md" | sed -E 's/^\]\((\.\/)?//' | sort -u > "$out/listed.txt"
  ( cd "$book/src" && find . -name '*.md' ! -name SUMMARY.md | sed 's|^\./||' | sort -u ) > "$out/present.txt"
  orphans="$(comm -13 "$out/listed.txt" "$out/present.txt")"
  if [ -n "$orphans" ]; then
    echo "check-book: pages under docs/src/ that SUMMARY.md does not list, so the book leaves them out:" >&2
    printf '  %s\n' $orphans >&2
    failed=1
  fi
  pages="$(wc -l < "$out/present.txt" | tr -d ' ')"
  [ "$pages" -ge 1 ] || { echo "check-book: no page found under $book/src" >&2; failed=1; }
  rm -rf "$out"
  [ "$failed" = 0 ] && echo "check-book: $book builds with no error, and all $pages pages are in SUMMARY.md"
  return "$failed"
}

command -v mdbook > /dev/null 2>&1 || { echo "check-book: mdbook is not on PATH" >&2; exit 2; }

if [ "${1:-}" != --self-test ]; then
  check "$ROOT"
  exit $?
fi

# --self-test: a copy of what the book reads (docs/, AGENTS.md through an include, .config/branding/ through the docs/branding
# symlink), checked clean, then broken one way at a time. Each must be refused, and the copy restored must pass again.
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/.config"
cp -R "$ROOT/docs" "$T/docs"; cp "$ROOT/AGENTS.md" "$T/"; cp -R "$ROOT/.config/branding" "$T/.config/branding"
rm -rf "$T/docs/book"
bad=0
expect() { # expect <label> <want exit>
  check "$T" > "$T/out" 2>&1; got=$?
  if [ "$got" = "$2" ]; then echo "ok   $1 (exit $got)"; else echo "BAD  $1: exit $got, wanted $2"; sed 's/^/     | /' "$T/out"; bad=1; fi
}
expect "the copy, as it is" 0
inc="$(grep -rlF '{{#include ../../../AGENTS.md' "$T/docs/src" | head -1)"
[ -n "$inc" ] || { echo "BAD  no page includes AGENTS.md: the include control has nothing to break"; exit 1; }
cp "$inc" "$T/inc.bak"
sed 's|{{#include ../../../AGENTS.md|{{#include ../../../NO-SUCH-FILE.md|' "$T/inc.bak" > "$inc"
expect "a broken include (mdbook exits 0 on it)" 1
cp "$T/inc.bak" "$inc"
cp "$T/docs/book.toml" "$T/toml.bak"
sed 's/^additional-css/addtional-css/' "$T/toml.bak" > "$T/docs/book.toml"
expect "a mistyped key of [output.html] (mdbook exits 0 and drops the table)" 1
sed 's/^command = "mdbook-mermaid"$/command = "mdbook-mermaid-not-installed"/' "$T/toml.bak" > "$T/docs/book.toml"
expect "the mermaid preprocessor missing" 1
cp "$T/toml.bak" "$T/docs/book.toml"
mkdir -p "$T/docs/src/stray"; echo '# A page nobody linked' > "$T/docs/src/stray/page.md"
expect "a page SUMMARY.md does not list" 1
rm -rf "$T/docs/src/stray"
expect "the copy, restored" 0
[ "$bad" = 0 ] && echo "check-book: self-test passed"
exit "$bad"
