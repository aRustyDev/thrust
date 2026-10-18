#!/bin/bash
# Verification of the zip's discriminating claims, each with a must-fail control.
# Runs only inside the scratchpad copy. Writes every exit status INTO the log (no pipes, no trailing echo).
SC=$SCRATCH
Z=$REPO/reference-workspace.zip
R=$SC/ref/reference-workspace
LOG=$SC/verify-2.log
: > "$LOG"
cd "$R" || exit 1

section() { printf '\n##### %s\n' "$1" >> "$LOG"; }
note() { printf '%s\n' "$*" >> "$LOG"; }
run() {
  label=$1; shift
  printf '$ %s\n' "$*" >> "$LOG"
  "$@" >> "$LOG" 2>&1
  st=$?
  printf '[%s] exit=%s\n' "$label" "$st" >> "$LOG"
}
restore() { unzip -o -q "$Z" "$@" -d "$SC/ref" >> "$LOG" 2>&1; }

section "E0 profile lists (test counts per profile)"
run list-default cargo nextest list --workspace --profile default --message-format oneline
run list-ci cargo nextest list --workspace --profile ci --message-format oneline
run list-deep cargo nextest list --workspace --profile deep --message-format oneline
run list-oracle cargo nextest list --workspace --profile oracle --message-format oneline
run list-e2e cargo nextest list --workspace --profile e2e --message-format oneline
run list-mutants cargo nextest list --workspace --profile mutants --message-format oneline

section "E1 retries: plain test is FLAKY-pass, prop_ test must fail under --retries 2"
cp "$SC/probes/zz_flaky.rs" crates/core/tests/zz_flaky.rs
m1=$(mktemp -d); m2=$(mktemp -d); m3=$(mktemp -d); m4=$(mktemp -d)
note "-- control: no retries, plain probe must FAIL (proves the probe fails on attempt 1)"
run e1-control-plain env ZZ_MARKER_DIR=$m1 cargo nextest run -p core-lib --test zz_flaky -E 'test(plain_flaky_probe)'
note "-- plain probe with --retries 2"
run e1-plain-retries env ZZ_MARKER_DIR=$m2 cargo nextest run -p core-lib --test zz_flaky --retries 2 -E 'test(plain_flaky_probe)'
note "-- prop_ probe with --retries 2"
run e1-prop-retries env ZZ_MARKER_DIR=$m3 cargo nextest run -p core-lib --test zz_flaky --retries 2 -E 'test(prop_flaky_probe)'
note "-- prop_ probe with NEXTEST_RETRIES=2"
run e1-prop-envretries env ZZ_MARKER_DIR=$m4 NEXTEST_RETRIES=2 cargo nextest run -p core-lib --test zz_flaky -E 'test(prop_flaky_probe)'
rm -f crates/core/tests/zz_flaky.rs

section "E2 counterexample persistence in tests/: default vs WithSource vs DISABLE env"
cp "$SC/probes/zz_persist.rs" crates/core/tests/zz_persist.rs
note "-- regression files BEFORE"
run e2-find-before find crates/core -name '*proptest-regressions*'
run e2-run cargo nextest run -p core-lib --test zz_persist --no-fail-fast
note "-- regression files AFTER"
run e2-find-after find crates/core -name '*proptest-regressions*'
note "-- content of any saved file"
run e2-cat cat crates/core/tests/zz_persist.proptest-regressions
rm -f crates/core/tests/zz_persist.proptest-regressions
rm -rf crates/core/proptest-regressions/tests
note "-- with PROPTEST_DISABLE_FAILURE_PERSISTENCE=1 (WithSource test only): no file expected"
run e2-disable env PROPTEST_DISABLE_FAILURE_PERSISTENCE=1 cargo nextest run -p core-lib --test zz_persist -E 'test(prop_withsource_persistence)'
run e2-find-disable find crates/core -name '*proptest-regressions*'
rm -f crates/core/tests/zz_persist.rs
rm -f crates/core/tests/zz_persist.proptest-regressions

section "E3 fuzz_replay_: baseline and controls"
run e3a-baseline cargo nextest run -p core-lib -E 'test(fuzz_replay_)'
mv crates/core/fuzz/seeds crates/core/fuzz-seeds.bak
mv crates/core/fuzz/regressions crates/core/fuzz-regressions.bak
note "-- E3b both dirs absent: must FAIL ('no fuzz inputs found')"
run e3b-no-inputs cargo nextest run -p core-lib -E 'test(fuzz_replay_)'
mv crates/core/fuzz-regressions.bak crates/core/fuzz/regressions
note "-- E3c only seeds absent, regressions present: weakness check (PASS here = partial vacuity)"
run e3c-no-seeds cargo nextest run -p core-lib -E 'test(fuzz_replay_)'
mv crates/core/fuzz-seeds.bak crates/core/fuzz/seeds
note "-- the shipped inputs, as bytes:"
run e3-xxd-1 xxd crates/core/fuzz/seeds/parse_frame/empty-payload
run e3-xxd-2 xxd crates/core/fuzz/seeds/parse_frame/abc
run e3-xxd-3 xxd crates/core/fuzz/regressions/parse_frame/crash-too-short
note "-- E3d: break the codec silently (tag + 1): replay with the SHIPPED inputs"
perl -pi -e 's/tag: bytes\[0\],/tag: bytes[0].wrapping_add(1),/' crates/core/src/frame.rs
run e3d-grep grep -n 'wrapping_add' crates/core/src/frame.rs
run e3d-mutated-shipped cargo nextest run -p core-lib -E 'test(fuzz_replay_)'
note "-- E3e: same mutation, but with correct binary seeds added via printf octal"
printf '\001\000\003abc' > crates/core/fuzz/seeds/parse_frame/zz-good-abc
printf '\007\000\000' > crates/core/fuzz/seeds/parse_frame/zz-good-empty
run e3e-xxd xxd crates/core/fuzz/seeds/parse_frame/zz-good-abc
run e3e-mutated-good-seeds cargo nextest run -p core-lib -E 'test(fuzz_replay_)'
restore reference-workspace/crates/core/src/frame.rs
note "-- E3f: original codec + good seeds: must PASS"
run e3f-original-good-seeds cargo nextest run -p core-lib -E 'test(fuzz_replay_)'
rm -f crates/core/fuzz/seeds/parse_frame/zz-good-abc crates/core/fuzz/seeds/parse_frame/zz-good-empty

section "E4 parity: mutated fixture must fail and list every divergence; generator is deterministic"
run e4a-baseline cargo nextest run -p core-lib --test parity
perl -pi -e 's/"expected": "65535"/"expected": "65534"/; s/"expected": "94906265"/"expected": "94906266"/' crates/core/tests/parity/fixtures/isqrt-001-boundaries.json
run e4b-mutated-fixture cargo nextest run -p core-lib --test parity
restore reference-workspace/crates/core/tests/parity/fixtures/isqrt-001-boundaries.json
run e4c-sha-before shasum crates/core/tests/parity/fixtures/isqrt-001-boundaries.json crates/core/tests/parity/fixtures/isqrt-002-random.json
run e4d-regenerate env TZ=UTC node legacy-oracle/gen-fixtures.mjs
run e4e-sha-after shasum crates/core/tests/parity/fixtures/isqrt-001-boundaries.json crates/core/tests/parity/fixtures/isqrt-002-random.json

section "E5 oracle profile: pass; control: broken legacy must fail"
run e5a-oracle cargo nextest run --workspace --profile oracle
perl -pi -e 's/^  return x;/  return x + 1n;/' legacy-oracle/legacy/isqrt.mjs
run e5b-grep grep -n 'return x' legacy-oracle/legacy/isqrt.mjs
run e5c-oracle-broken cargo nextest run --workspace --profile oracle
restore reference-workspace/legacy-oracle/legacy/isqrt.mjs
run e5d-oracle-restored cargo nextest run --workspace --profile oracle

section "E6 e2e profile: pass; control: bad E2E_WEB_DIST must fail"
run e6a-e2e cargo nextest run --workspace --profile e2e
run e6b-e2e-bad-dist env E2E_WEB_DIST=/nonexistent cargo nextest run --workspace --profile e2e

section "E7 -E intersects the profile's default-filter"
run e7a-oracle-under-default cargo nextest run --workspace -E 'test(/\boracle_/)'
run e7b-prop-under-oracle cargo nextest run --workspace --profile oracle -E 'test(/\bprop_/)'
run e7c-e2e-under-default cargo nextest run --workspace -E 'package(e2e)'

section "E8 deep profile at 4096 cases"
run e8-deep env PROPTEST_CASES=4096 cargo nextest run --workspace --profile deep

section "E9 doctests and ci profile junit"
run e9a-doc cargo test --doc --workspace
run e9b-ci cargo nextest run --workspace --profile ci
run e9c-junit ls -l target/nextest/ci/junit.xml

section "DONE"
note "finished"
