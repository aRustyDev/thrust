#!/bin/sh
# Spike 0006: prepare the JS oracle (the REAL headless @plane/editor code) outside the Plane tree.
#
#   tools/spike-0006/setup-oracle.sh <plane-tree> <workdir>
#
# <plane-tree> is the pinned Plane checkout (tag oracle/v1-pin-20261018); it is only READ.
# <workdir> receives a copy of the packages the editor needs (no node_modules) plus a pnpm
# install that uses the Plane lockfile, so versions are exactly the pinned ones.
# Then it copies tools/spike-0006/oracle/*.ts into <workdir>/plane-copy/packages/editor/spike0006/.
set -eu
PL="${1:?plane tree}"
WD="${2:?workdir}"
HERE="$(cd "$(dirname "$0")" && pwd)"
CP="$WD/plane-copy"
mkdir -p "$CP/packages"
cp "$PL/package.json" "$PL/pnpm-lock.yaml" "$PL/.npmrc" "$PL/.node-version" "$CP/"
# the real workspace file, but only packages/* (apps are not needed to run the editor headlessly)
sed -e '/^  - apps\/\*$/d' -e '/^  - "!apps\//d' "$PL/pnpm-workspace.yaml" > "$CP/pnpm-workspace.yaml"
for d in editor blocks constants hooks types utils typescript-config tailwind-config; do
  rsync -a --exclude node_modules --exclude .turbo --exclude '*.map' "$PL/packages/$d" "$CP/packages/"
done
(cd "$CP" && pnpm install --ignore-scripts --filter "@plane/editor..." > "$WD/pnpm-install.log" 2>&1)
# the editor imports its workspace dependencies through their dist/ builds, which a fresh checkout of the pin does not
# have (measured 2026-10-19: ERR_MODULE_NOT_FOUND for packages/utils/dist/index.js), so build them in the copy
(cd "$CP" && pnpm --filter "@plane/editor^..." run build > "$WD/build-deps.log" 2>&1)
mkdir -p "$CP/packages/editor/spike0006"
cp "$HERE"/oracle/*.ts "$CP/packages/editor/spike0006/"
# jsdom (the pinned catalog version, 23.2.0) only for the "does a REAL editor open this binary" check
mkdir -p "$WD/extra"
cp "$HERE/extra-package.json" "$WD/extra/package.json"
(cd "$WD/extra" && pnpm install --ignore-scripts --ignore-workspace > "$WD/extra-install.log" 2>&1)
echo "oracle ready: $CP/packages/editor/spike0006"
