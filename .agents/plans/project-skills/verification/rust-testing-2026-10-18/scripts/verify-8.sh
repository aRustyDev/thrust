#!/bin/bash
# Compile and exercise the rust-test-fuzz skill's template harness in the scratchpad copy of the zip workspace.
SC=$SCRATCH
Z=$REPO/reference-workspace.zip
EX=$WORKTREE/.agents/plans/project-skills/artifacts/skills/rust-test-fuzz/references/examples
R=$SC/ref/reference-workspace
LOG=$SC/verify-8.log
: > "$LOG"
cd "$R" || exit 1
run() {
  label=$1; shift
  printf '$ %s\n' "$*" >> "$LOG"
  "$@" >> "$LOG" 2>&1
  st=$?
  printf '[%s] exit=%s\n' "$label" "$st" >> "$LOG"
}
note() { printf '%s\n' "$*" >> "$LOG"; }
SEEDS=crates/core/fuzz/seeds/parse_frame
REGS=crates/core/fuzz/regressions/parse_frame

# Install the template and the skill's seed recipes (printf octal), replacing the zip's literal-text files.
cp "$EX/fuzz_harness.rs" crates/core/src/fuzz_harness.rs
touch crates/core/src/fuzz_harness.rs
rm -f $SEEDS/empty-payload $SEEDS/abc $REGS/crash-too-short
printf '\007\000\000' > $SEEDS/empty-payload
printf '\001\000\003abc' > $SEEDS/abc
printf '\007\000\002\001' > $SEEDS/length-mismatch
printf '\377\377' > $REGS/too-short
run s0-xxd xxd $SEEDS/empty-payload $SEEDS/abc $SEEDS/length-mismatch $REGS/too-short

note "-- F1 template compiles; replay + two permanent controls pass (expect 3 passed)"
run f1-replay cargo nextest run -p core-lib -E 'test(fuzz_replay_)'

note "-- F2 codec broken (tag + 1): replay must FAIL"
perl -pi -e 's/tag: bytes\[0\],/tag: bytes[0].wrapping_add(1),/' crates/core/src/frame.rs
touch crates/core/src/frame.rs
run f2-grep grep -n wrapping_add crates/core/src/frame.rs
run f2-mutated cargo nextest run -p core-lib -E 'test(fuzz_replay_parse_frame)'
unzip -o -q "$Z" reference-workspace/crates/core/src/frame.rs -d "$SC/ref"
touch crates/core/src/frame.rs

note "-- F3 seeds directory removed (regressions present): replay must FAIL"
mv crates/core/fuzz/seeds crates/core/fuzz-seeds.bak
run f3-no-seeds cargo nextest run -p core-lib -E 'test(fuzz_replay_parse_frame)'
mv crates/core/fuzz-seeds.bak crates/core/fuzz/seeds

note "-- F4 only the text-not-bytes seed (the zip's original defect): replay must FAIL"
mv $SEEDS/empty-payload "$SC/seed.empty"
mv $SEEDS/abc "$SC/seed.abc"
mv $SEEDS/length-mismatch "$SC/seed.lm"
printf '%s' '\x01\x00\x03abc' > $SEEDS/text-not-bytes
run f4-xxd xxd $SEEDS/text-not-bytes
run f4-text-only cargo nextest run -p core-lib -E 'test(fuzz_replay_parse_frame)'
rm -f $SEEDS/text-not-bytes
mv "$SC/seed.empty" $SEEDS/empty-payload
mv "$SC/seed.abc" $SEEDS/abc
mv "$SC/seed.lm" $SEEDS/length-mismatch

note "-- F5 a seed that panics the decoder (control for the panic path): harness panics on purpose"
printf '\007\000\000' > "$SC/ok.seed"
perl -pi -e 's/if bytes\.len\(\) < 3 \{/if bytes.len() < 3 { if bytes.len() == 1 { panic!("boom"); }/' crates/core/src/frame.rs
touch crates/core/src/frame.rs
printf '\005' > $REGS/one-byte-panics
run f5-grep grep -n boom crates/core/src/frame.rs
run f5-panic cargo nextest run -p core-lib -E 'test(fuzz_replay_parse_frame)'
rm -f $REGS/one-byte-panics
unzip -o -q "$Z" reference-workspace/crates/core/src/frame.rs -d "$SC/ref"
touch crates/core/src/frame.rs

note "-- F6 everything restored: expect pass"
run f6-final cargo nextest run -p core-lib -E 'test(fuzz_replay_)'
run f6-workspace cargo nextest run --workspace
printf 'finished\n' >> "$LOG"
