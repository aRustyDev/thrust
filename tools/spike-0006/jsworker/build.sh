#!/bin/sh
# Bundle the REAL headless editor conversion code into a single Worker module (no DOM shim, no source edits).
#   tools/spike-0006/jsworker/build.sh <workdir> <outdir>
# Produces <outdir>/worker.mjs (+ wrangler.toml). Reports size. Uses the esbuild already in the pinned lockfile.
set -eu
WD="${1:?workdir}"
OUT="${2:?outdir}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ED="$WD/plane-copy/packages/editor"
ESB="$(ls -d "$WD"/plane-copy/node_modules/.pnpm/esbuild@*/node_modules/esbuild/bin/esbuild | head -1)"
mkdir -p "$OUT"
cp "$HERE/worker.ts" "$ED/spike0006/jsworker-entry.ts"
cd "$ED/spike0006"
"$ESB" jsworker-entry.ts --bundle --format=esm --platform=browser --target=es2022 \
  --conditions=workerd,worker,browser,import --main-fields=module,main \
  --tsconfig="$ED/tsconfig.json" --jsx=automatic --loader:.css=empty --loader:.svg=dataurl \
  --define:process.env.NODE_ENV=\"production\" --legal-comments=none \
  ${MINIFY:+--minify} --metafile="$OUT/meta.json" --outfile="$OUT/worker.mjs" > "$OUT/esbuild.log" 2>&1
if [ -n "${NODEJS_COMPAT:-}" ]; then FLAGS='compatibility_flags = ["nodejs_compat"]\n'; else FLAGS=''; fi
printf 'name = "spike-0006-js-worker"\nmain = "worker.mjs"\ncompatibility_date = "2026-08-01"\nworkers_dev = false\n%b' "$FLAGS" > "$OUT/wrangler.toml"
echo "worker.mjs: $(wc -c < "$OUT/worker.mjs") bytes, gzip $(gzip -9 -c "$OUT/worker.mjs" | wc -c)"
tail -n 5 "$OUT/esbuild.log"
