#!/bin/sh
# Compile the sqlx probe for wasm32 in both configurations and keep the compiler output.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
OUT="$ROOT/target/sqlx-wasm"
mkdir -p "$OUT"
for feat in "" "--features tokio"; do
  name="$(echo "${feat:-no-runtime}" | tr ' ' '_' | tr -d '-')"
  # shellcheck disable=SC2086
  "$ROOT/scripts/rs" cargo check --quiet --manifest-path "$HERE/Cargo.toml" --target-dir "$OUT/target" --target wasm32-unknown-unknown $feat >"$OUT/$name.log" 2>&1
  status=$?
  if [ "$status" -eq 0 ]; then verdict=COMPILES; else verdict="FAILS (exit $status)"; fi
  echo "sqlx postgres on wasm32, ${feat:-no runtime feature}: $verdict"
  grep -m3 -E "^error" "$OUT/$name.log" | cut -c1-220
done
