#!/bin/sh
# Run one oracle script with tsx inside the prepared editor copy.
#   tools/spike-0006/run-oracle.sh <workdir> <script.ts> [args...]   (stdout is the script's stdout)
set -eu
WD="${1:?workdir}"; shift
S="${1:?script}"; shift
ED="$WD/plane-copy/packages/editor"
TSX="$(ls -d "$WD"/plane-copy/node_modules/.pnpm/tsx@*/node_modules/tsx/dist/cli.mjs 2>/dev/null | head -1)"
# Without this check a missing tsx runs `node ""`, which prints nothing and exits 0 (measured 2026-10-19).
[ -f "$TSX" ] || { echo "run-oracle: no tsx under $WD/plane-copy (run tools/spike-0006/setup-oracle.sh)" >&2; exit 1; }
cd "$ED/spike0006"
exec node --no-warnings "$TSX" --tsconfig "$ED/tsconfig.json" "$S" "$@"
