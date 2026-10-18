#!/bin/sh
# Global-scope (startup) CPU of a Worker with wrangler's own local profiler: no login, nothing deployed.
#   tools/spike-0006/bench/startup.sh <label> <dir-with-wrangler.toml>
set -u
LABEL="${1:?label}"
DIR="${2:?dir}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
OUT="$ROOT/target/spike-0006/bench"
mkdir -p "$OUT"
cd "$DIR" || exit 99
rc=0
env -i HOME="$HOME" PATH="$PATH" TMPDIR="${TMPDIR:-/tmp}" WRANGLER_SEND_METRICS=false NO_COLOR=1 \
  wrangler check startup -c wrangler.toml --outfile "$OUT/$LABEL.cpuprofile" > "$OUT/$LABEL.startup.log" 2>&1 || rc=$?
echo "rc=$rc"
cat "$OUT/$LABEL.startup.log"
exit $rc
