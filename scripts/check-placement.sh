#!/bin/sh
# Placement guard (issue #39): the Worker runs next to its database.
#
# `wrangler.toml` and `wrangler.stage2.toml` must each carry exactly one `[placement]` table that holds exactly
# `region = "aws:us-east-1"` (ADR-0094). Cloudflare makes `region` exclusive with `mode`, `host` and `hostname`, so the
# whole table is compared and not only the `region` line.
#
# `wrangler.pg.stage.toml` must carry no literal table. Its `# __PLACEMENT__` marker is replaced by
# `scripts/spike-0002-pg.sh`, and a literal table beside the injected one would be a second `[placement]`. The marker
# must also still be there, or the script would inject nothing and say nothing.
#
# This reads the text. It does not prove that wrangler or Cloudflare accepts it: `wrangler deploy --dry-run` and the
# worker-cf contract cell do that.
#
#   scripts/check-placement.sh [dir]     dir defaults to crates/thrust-worker
set -u
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIR="${1:-$ROOT/crates/thrust-worker}"
WANT='region=aws:us-east-1'
STATUS=0

# The `key=value` lines of a file's `[placement]` table(s), with spaces, quotes and comments removed.
placement_keys() {
  awk '
    /^\[/ { in_table = ($0 ~ /^\[placement\][ \t]*(#.*)?$/); next }
    in_table && /^[ \t]*[A-Za-z_][A-Za-z_-]*[ \t]*=/ { line = $0; sub(/#.*/, "", line); gsub(/[ \t"]/, "", line); print line }
  ' "$1"
}

table_count() {
  grep -c '^\[placement\]' "$1"
}

for f in wrangler.toml wrangler.stage2.toml; do
  file="$DIR/$f"
  if [ ! -f "$file" ]; then
    echo "FAIL: $f is missing"
    STATUS=1
    continue
  fi
  n="$(table_count "$file")"
  keys="$(placement_keys "$file" | tr '\n' ' ')"
  if [ "$n" = 1 ] && [ "$keys" = "$WANT " ]; then
    echo "ok: $f carries [placement] $WANT"
  else
    echo "FAIL: $f needs exactly one [placement] table holding only $WANT (found $n table(s); keys: ${keys:-none})"
    STATUS=1
  fi
done

stage="$DIR/wrangler.pg.stage.toml"
if [ ! -f "$stage" ]; then
  echo "FAIL: wrangler.pg.stage.toml is missing"
  STATUS=1
elif [ "$(table_count "$stage")" != 0 ]; then
  echo "FAIL: wrangler.pg.stage.toml carries a literal [placement] table; spike-0002-pg.sh would add a second one"
  STATUS=1
elif ! grep -q '^# __PLACEMENT__$' "$stage"; then
  echo "FAIL: wrangler.pg.stage.toml lost its '# __PLACEMENT__' marker, so spike-0002-pg.sh injects no placement"
  STATUS=1
else
  echo "ok: wrangler.pg.stage.toml carries only the marker"
fi

exit "$STATUS"
