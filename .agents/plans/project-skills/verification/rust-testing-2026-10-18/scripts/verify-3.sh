#!/bin/bash
# Re-run of E3f: the restore in verify-2.sh kept the zip's old mtime, so cargo reused the mutated build.
SC=$SCRATCH
R=$SC/ref/reference-workspace
LOG=$SC/verify-3.log
: > "$LOG"
cd "$R" || exit 1
run() {
  label=$1; shift
  printf '$ %s\n' "$*" >> "$LOG"
  "$@" >> "$LOG" 2>&1
  st=$?
  printf '[%s] exit=%s\n' "$label" "$st" >> "$LOG"
}
run v3-grep-original grep -n 'tag: bytes' crates/core/src/frame.rs
touch crates/core/src/frame.rs
printf '\001\000\003abc' > crates/core/fuzz/seeds/parse_frame/zz-good-abc
printf '\007\000\000' > crates/core/fuzz/seeds/parse_frame/zz-good-empty
run v3-original-good-seeds cargo nextest run -p core-lib -E 'test(fuzz_replay_)'
rm -f crates/core/fuzz/seeds/parse_frame/zz-good-abc crates/core/fuzz/seeds/parse_frame/zz-good-empty
run v3-original-shipped-seeds cargo nextest run -p core-lib -E 'test(fuzz_replay_)'
run v3-full cargo nextest run --workspace
printf 'finished\n' >> "$LOG"
