#!/bin/sh
# Native first-call timing and heap (research/0015, section 4.1): build once in release, then run ONE PROCESS PER RUN so that every run's first
# call is cold. One line per run, in the format main.rs documents.
#
#   tools/measure-zxcvbn/native/run.sh <runs>
set -eu
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
"$ROOT/scripts/rs" cargo build --release --manifest-path "$HERE/Cargo.toml" >&2
bin="$HERE/target/release/zxcvbn-first-call"
i=0
while [ "$i" -lt "${1:?usage: run.sh <runs>}" ]; do
  i=$((i + 1))
  "$bin"
done
