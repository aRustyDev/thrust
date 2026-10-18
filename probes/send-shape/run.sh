#!/bin/sh
# Compile the probe in every (shape x target) cell and print one verdict line per cell.
# Output of each compile is kept under target/ for the evidence appendix.
set -u
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
OUT="$ROOT/target/send-shape"
mkdir -p "$OUT"
for shape in send unsend cfg-switch; do
  for target in native wasm32-unknown-unknown; do
    case "$shape" in
      send) feat="" ;;
      *) feat="--features $shape" ;;
    esac
    case "$target" in
      native) tflag="" ;;
      *) tflag="--target $target" ;;
    esac
    # shellcheck disable=SC2086
    "$ROOT/scripts/rs" cargo check --quiet --manifest-path "$HERE/Cargo.toml" --target-dir "$OUT/target" $feat $tflag >"$OUT/$shape.$target.log" 2>&1
    status=$?
    if [ "$status" -eq 0 ]; then verdict=COMPILES; else verdict="FAILS (exit $status)"; fi
    echo "shape=$shape target=$target -> $verdict"
  done
done
