#!/bin/sh
# Spike 0006: regenerate every oracle artefact the Rust tests consume.
#
#   tools/spike-0006/setup-oracle.sh <plane-tree> <workdir>      # once
#   tools/spike-0006/gen-all.sh <workdir>                          # corpus + schema dump (deterministic)
#
# Output: crates/thrust-editor-doc/tests/corpus/{corpus.json,schema.json,todom.json} (committed)
#         target/spike-0006/corpus-large.json                       (git-ignored: big documents for CPU timing)
#         target/spike-0006/{fuzz-html,fuzz-html-heldout,entities-fuzz,deep}.json  (git-ignored: read by the oracle_ tests)
set -eu
WD="${1:?workdir}"
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
OUT="$ROOT/crates/thrust-editor-doc/tests/corpus"
mkdir -p "$OUT" "$ROOT/target/spike-0006"
cp "$HERE"/oracle/*.ts "$WD/plane-copy/packages/editor/spike0006/"
# Write to a temp file and move it into place only when the oracle succeeded AND printed something: a plain
# `> file` truncates the committed fixture before the oracle runs, so a failing oracle leaves it empty (measured
# 2026-10-19: schema.json emptied).
gen() {
  dest="$1"; shift
  if "$HERE/run-oracle.sh" "$WD" "$@" > "$dest.tmp" && [ -s "$dest.tmp" ]; then
    mv "$dest.tmp" "$dest"
  else
    rm -f "$dest.tmp"; echo "gen-all: oracle failed or printed nothing for $*; $dest left unchanged" >&2; exit 1
  fi
}
gen "$OUT/schema.json" schema-dump.ts
gen "$OUT/todom.json" probe-todom.ts
mkdir -p "$ROOT/crates/thrust-editor-doc/data"
gen "$ROOT/crates/thrust-editor-doc/data/emoji.json" emoji-dump.ts
gen "$OUT/corpus.json" gen-corpus.ts
gen "$ROOT/target/spike-0006/corpus-large.json" gen-corpus.ts --large
gen "$ROOT/target/spike-0006/fuzz-html.json" fuzz-html.ts 2000 20261018
gen "$ROOT/target/spike-0006/entities-fuzz.json" entities-fuzz.ts 6000 99
gen "$ROOT/target/spike-0006/deep.json" deep.ts
# second seed, generated AFTER the Rust parser was tuned on the first: the held-out set
gen "$ROOT/target/spike-0006/fuzz-html-heldout.json" fuzz-html.ts 4000 777
ls -la "$OUT" "$ROOT/target/spike-0006"
