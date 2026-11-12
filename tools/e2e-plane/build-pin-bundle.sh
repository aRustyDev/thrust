#!/bin/sh
# Build the pinned Plane's STOCK web app, for the browser journeys' Plane leg (issue #45).
#
#   tools/e2e-plane/build-pin-bundle.sh <fork checkout> [dest]
#     fork checkout   the directory of the Plane fork with the pin in it (the parent of the oracle/ that ORACLE_DIR names). It is only
#                     READ: `git show` of the files below. Nothing is written into it.
#     dest            where to build; default target/e2e-plane/pin-web
#
# stdout is one line, the directory that holds the built app (<dest>/apps/web/build/client); everything else goes to stderr.
#
# Why this is Thrust's web/ with the ledger reverted, and not a build in the fork. web/apps/web and web/packages are byte-identical
# to the pin except the files in web/ALLOWED-CHANGES, the Q-R2 ledger (scripts/check-web-diff.sh holds CI to that). Thrust's auth client
# posts JSON, which Plane does not take (it takes a form and answers 302), so a Thrust bundle cannot sign anyone in on the pin. Restoring
# every ledger file from the pin gives the pin's app, built with the same lockfile, toolchain and environment as the Thrust bundle, so the
# ONLY variable between the two legs of a journey is the ledger. It also follows the ledger as it grows (base URL, collab provider,
# branding), and leaves the fork checkout without a node_modules.
#
# Each restored file is checked three ways against web/PIN-MANIFEST (`git ls-tree -r <pin> apps/web packages`): the blob the fork says
# the pin has, the manifest's entry, and the hash of the bytes written. A ledger entry that is an ADDED file (the pin has no such file)
# is deleted from the copy, and the manifest must not list it.
#
# The input is the COMMITTED web/ at HEAD (git archive), so a half-edited working tree cannot leak into the oracle's bundle.
set -eu

FORK="${1:?usage: build-pin-bundle.sh <fork checkout> [dest]}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DEST="${2:-$ROOT/target/e2e-plane/pin-web}"
LEDGER="$ROOT/web/ALLOWED-CHANGES"
MANIFEST="$ROOT/web/PIN-MANIFEST"
PIN="$(sed -n 's/^pinned_commit=//p' "$ROOT/tools/oracle-v1/PIN")"

say() { echo "build-pin-bundle: $*" >&2; }
die() { echo "build-pin-bundle: $*" >&2; exit 2; }

[ -n "$PIN" ] || die "tools/oracle-v1/PIN has no pinned_commit"
[ -s "$LEDGER" ] && [ -s "$MANIFEST" ] || die "web/ALLOWED-CHANGES or web/PIN-MANIFEST is missing or empty; refusing to build on nothing"
git -C "$FORK" rev-parse --verify --quiet "$PIN^{commit}" > /dev/null ||
  die "$FORK does not have the pin $PIN: it must be the fork checkout that oracle/PIN names (ORACLE_DIR is its oracle/)"

# an unchanged input and a finished build need no second build
TREE="$(git -C "$ROOT" rev-parse HEAD:web)"
STAMP="$TREE+$PIN+$(git -C "$ROOT" hash-object --no-filters "$LEDGER")"
if [ -s "$DEST/.stamp" ] && [ "$(cat "$DEST/.stamp")" = "$STAMP" ] && [ -s "$DEST/apps/web/build/client/index.html" ]; then
  say "reusing the build at $DEST (same web/ tree, pin and ledger)"
  echo "$DEST/apps/web/build/client"
  exit 0
fi

rm -rf "$DEST"
mkdir -p "$DEST"
TAR="$DEST.tar"
git -C "$ROOT" archive --format=tar -o "$TAR" HEAD web || die "git archive of web/ failed"
tar -xf "$TAR" -C "$DEST" --strip-components=1 || die "could not unpack web/ into $DEST"
rm -f "$TAR"

# the ledger: one path per line, relative to web/; '#' starts a comment (the grammar scripts/check-web-diff.sh reads)
PATHS="$(sed -e 's/#.*//' -e 's/[[:space:]]*$//' -e 's/^[[:space:]]*//' -e '/^$/d' "$LEDGER")"
[ -n "$PATHS" ] || die "web/ALLOWED-CHANGES lists nothing; the ledger is the list of what to restore, so there is nothing to build on"

TAB="$(printf '\t')"
RESTORED=0
DELETED=0
# one path per line; the ledger's paths have no spaces (check-web-diff.sh would not handle them either)
for ledger_path in $PATHS; do
  want="$(awk -F "$TAB" -v p="$ledger_path" '$2 == p { split($1, a, " "); print a[3] }' "$MANIFEST")"
  if git -C "$FORK" cat-file -e "$PIN:$ledger_path" 2> /dev/null; then
    [ -n "$want" ] || die "$ledger_path is in the pin but not in web/PIN-MANIFEST: the manifest is not the pin's"
    have="$(git -C "$FORK" rev-parse "$PIN:$ledger_path")"
    [ "$have" = "$want" ] || die "$ledger_path: the fork's pin has blob $have and web/PIN-MANIFEST says $want"
    git -C "$FORK" cat-file blob "$PIN:$ledger_path" > "$DEST/$ledger_path"
    wrote="$(git -C "$ROOT" hash-object --no-filters "$DEST/$ledger_path")"
    [ "$wrote" = "$want" ] || die "$ledger_path: the bytes restored hash to $wrote, not the pin's $want"
    RESTORED=$((RESTORED + 1))
  else
    [ -z "$want" ] || die "$ledger_path is in web/PIN-MANIFEST but the fork's pin does not have it"
    rm -f "$DEST/$ledger_path"
    DELETED=$((DELETED + 1))
  fi
done
say "restored $RESTORED file(s) from the pin and removed $DELETED added one(s): the ledger is reverted"

# the same commands, with the same environment, as scripts/e2e.sh uses for the Thrust bundle. `--force`: $DEST is inside this git
# repository (target/ is ignored), where turbo finds few input files to hash, and a cache hit from an earlier build of a different
# ledger would hand back the wrong bundle
say "installing and building at $DEST (a few minutes the first time)"
pnpm --dir "$DEST" install --frozen-lockfile --ignore-scripts > "$DEST.install.log" 2>&1 ||
  { tail -8 "$DEST.install.log" >&2; die "the install failed (log: $DEST.install.log)"; }
pnpm --dir "$DEST" exec turbo run build --filter=web --force > "$DEST.build.log" 2>&1 ||
  { tail -8 "$DEST.build.log" >&2; die "the build failed (log: $DEST.build.log)"; }
[ -s "$DEST/apps/web/build/client/index.html" ] || die "the build made no $DEST/apps/web/build/client/index.html"

echo "$STAMP" > "$DEST/.stamp"
say "built the pin's app"
echo "$DEST/apps/web/build/client"
