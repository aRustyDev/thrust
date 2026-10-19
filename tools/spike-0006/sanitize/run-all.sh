#!/bin/sh
# Spike 0006 sanitiser evidence, end to end (needs the pinned Plane tree for the Python oracle):
#   tools/spike-0006/sanitize/run-all.sh <plane-tree> [uv-env-dir]
# Output: target/spike-0006/sanitize-report.json (+ the raw outputs next to it)
set -eu
PLANE="${1:?plane tree}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
C="$ROOT/crates/thrust-editor-doc/tests/corpus"
O="$ROOT/target/spike-0006"
mkdir -p "$O"
[ -n "${2:-}" ] && export UV_PROJECT_ENVIRONMENT="$2"
cd "$HERE"
uv run --quiet run.py corpus --out "$C/xss-corpus.json"
uv run --quiet run.py oracle --plane-tree "$PLANE" --corpus "$C/xss-corpus.json" --html "$C/corpus.json" --out "$O/xss-oracle.json" > "$O/xss-oracle.log" 2>&1
"$ROOT/scripts/rs" cargo run --release --quiet -p thrust-editor-doc --example sanitize_check -- "$C/xss-corpus.json" "$C/corpus.json" "$O/xss-rust.json"
"$ROOT/probes/ammonia-400/run.sh"
uv run --quiet run.py check --corpus "$C/xss-corpus.json" --oracle "$O/xss-oracle.json" --rust "$O/xss-rust.json" --rust-old "$O/xss-rust-400.json" > "$O/sanitize-report.json" 2> "$O/sanitize-check.log"
echo "report: $O/sanitize-report.json"
