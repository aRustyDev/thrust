#!/bin/sh
# Build and run the ammonia 4.0.0 probe over the XSS corpus + editor HTML corpus.
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
C="$ROOT/crates/thrust-editor-doc/tests/corpus"
"$ROOT/scripts/rs" cargo run --release --quiet --manifest-path "$HERE/Cargo.toml" --target-dir "$ROOT/target/ammonia-400" -- "$C/xss-corpus.json" "$C/corpus.json" "$ROOT/target/spike-0006/xss-rust-400.json"
