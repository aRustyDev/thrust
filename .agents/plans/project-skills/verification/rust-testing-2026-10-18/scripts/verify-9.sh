#!/bin/bash
SC=$SCRATCH
R=$SC/ref/reference-workspace
LOG=$SC/verify-9.log
: > "$LOG"
run() {
  label=$1; shift
  printf '$ %s\n' "$*" >> "$LOG"
  "$@" >> "$LOG" 2>&1
  st=$?
  printf '[%s] exit=%s\n' "$label" "$st" >> "$LOG"
}
note() { printf '%s\n' "$*" >> "$LOG"; }

note "##### T1 JS vs Rust numeric claims"
run t1-node node "$SC/roundtrip.mjs"
run t1-rustc rustc -O -o "$SC/roundtrip-bin" "$SC/roundtrip.rs"
run t1-rust "$SC/roundtrip-bin"

note "##### T2 does the CLI override the per-test flaky-result = fail? (prop_ probe fails once then passes)"
cd "$R" || exit 1
cp "$SC/probes/zz_flaky.rs" crates/core/tests/zz_flaky.rs
d1=$(mktemp -d); d2=$(mktemp -d); d3=$(mktemp -d); d4=$(mktemp -d)
run t2-help sh -c "cargo nextest run --help | grep -i -B1 -A6 flaky"
note "-- baseline: --retries 2 only (expect FAIL, the zip's claim)"
run t2a-retries env ZZ_MARKER_DIR=$d1 cargo nextest run -p core-lib --test zz_flaky --retries 2 -E 'test(prop_flaky_probe)'
note "-- --retries 2 --flaky-result pass"
run t2b-cli-pass env ZZ_MARKER_DIR=$d2 cargo nextest run -p core-lib --test zz_flaky --retries 2 --flaky-result pass -E 'test(prop_flaky_probe)'
note "-- env NEXTEST_RETRIES=2 NEXTEST_FLAKY_RESULT=pass"
run t2c-env-pass env ZZ_MARKER_DIR=$d3 NEXTEST_RETRIES=2 NEXTEST_FLAKY_RESULT=pass cargo nextest run -p core-lib --test zz_flaky -E 'test(prop_flaky_probe)'
note "-- plain test with --retries 2 --flaky-result fail (CLI the other way)"
run t2d-plain-fail env ZZ_MARKER_DIR=$d4 cargo nextest run -p core-lib --test zz_flaky --retries 2 --flaky-result fail -E 'test(plain_flaky_probe)'
rm -f crates/core/tests/zz_flaky.rs
note "finished"
