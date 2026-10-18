#!/bin/sh
# Spike 0006: regenerate every oracle artefact the Rust tests consume.
#
#   tools/spike-0006/setup-oracle.sh <plane-tree> <workdir>      # once
#   tools/spike-0006/gen-all.sh <workdir>                          # corpus + schema dump (deterministic)
#
# Output: crates/thrust-editor-doc/tests/corpus/{corpus.json,schema.json,todom.json} (committed)
#         target/spike-0006/corpus-large.json                       (git-ignored: big documents for CPU timing)
set -eu
WD="${1:?workdir}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
OUT="$ROOT/crates/thrust-editor-doc/tests/corpus"
mkdir -p "$OUT" "$ROOT/target/spike-0006"
cp "$HERE"/oracle/*.ts "$WD/plane-copy/packages/editor/spike0006/"
"$HERE/run-oracle.sh" "$WD" schema-dump.ts > "$OUT/schema.json"
"$HERE/run-oracle.sh" "$WD" probe-todom.ts > "$OUT/todom.json"
mkdir -p "$ROOT/crates/thrust-editor-doc/data"
"$HERE/run-oracle.sh" "$WD" emoji-dump.ts > "$ROOT/crates/thrust-editor-doc/data/emoji.json"
"$HERE/run-oracle.sh" "$WD" gen-corpus.ts > "$OUT/corpus.json"
"$HERE/run-oracle.sh" "$WD" gen-corpus.ts --large > "$ROOT/target/spike-0006/corpus-large.json"
"$HERE/run-oracle.sh" "$WD" fuzz-html.ts 2000 20261018 > "$ROOT/target/spike-0006/fuzz-html.json"
# second seed, generated AFTER the Rust parser was tuned on the first: the held-out set
"$HERE/run-oracle.sh" "$WD" fuzz-html.ts 4000 777 > "$ROOT/target/spike-0006/fuzz-html-heldout.json"
ls -la "$OUT" "$ROOT/target/spike-0006"
