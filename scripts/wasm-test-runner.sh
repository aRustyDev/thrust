#!/bin/sh
# Cargo's runner for wasm32-unknown-unknown (.cargo/config.toml): `wasm-bindgen-test-runner`, which runs a wasm32 test binary in Node, with the
# stand-ins for the workerd-only modules the Worker imports (scripts/lib/workerd-stubs.cjs). Cargo and nextest call it as
#   scripts/wasm-test-runner.sh <test binary>.wasm [--list --format terse | <test name> --exact | ...]
# so that `cargo nextest run --target wasm32-unknown-unknown -p thrust-worker` runs the Worker's unit tests (scripts/wasm-tests.sh).
#
# The runner must be the exact version of the `wasm-bindgen` in Cargo.lock: the test binary carries that version's glue, and a runner of
# another one can read the format and still disagree about it. CI installs it at the lock's version (the `workers` job of test.yml); a
# machine that has another (`cargo install wasm-bindgen-cli --version =<the lock's>`, or a prebuilt binary of that release) is refused here,
# by naming both versions, before any test runs.
#
#   WASM_BINDGEN_TEST_RUNNER  the runner to use (default: wasm-bindgen-test-runner on PATH)
#   WASM_TEST_CARGO           the cargo that says which wasm-bindgen the lock pins (default: cargo; for the test of this script)
set -eu
root="$(cd "$(dirname "$0")/.." && pwd)"
runner="${WASM_BINDGEN_TEST_RUNNER:-wasm-bindgen-test-runner}"

# Cargo says which version the lock pins: `cargo pkgid` prints `registry+<index>#wasm-bindgen@0.2.130` (or `...#0.2.130`, for a source named like its
# last path part), and fails when there is none or more than one, which is the case where "the" version is not defined. From the root, so that the
# toolchain of rust-toolchain.toml answers whatever directory cargo started this runner in.
id="$(cd "$root" && "${WASM_TEST_CARGO:-cargo}" pkgid --locked wasm-bindgen)" || {
  echo "scripts/wasm-test-runner.sh: no single version of wasm-bindgen to match in Cargo.lock" >&2
  exit 1
}
wanted="${id##*[#@]}"
if ! found="$("$runner" --version 2> /dev/null)"; then
  echo "scripts/wasm-test-runner.sh: cannot run $runner. Install the runner of wasm-bindgen $wanted (the version Cargo.lock pins):" >&2
  echo "  cargo install wasm-bindgen-cli --version =$wanted --locked   (or a prebuilt binary of release $wanted of github.com/wasm-bindgen/wasm-bindgen)" >&2
  exit 1
fi
# `wasm-bindgen-test-runner 0.2.130`: the last word
if [ "${found##* }" != "$wanted" ]; then
  echo "scripts/wasm-test-runner.sh: $runner is '$found', and Cargo.lock pins wasm-bindgen $wanted: the runner must be $wanted exactly." >&2
  exit 1
fi

NODE_OPTIONS="--require \"$root/scripts/lib/workerd-stubs.cjs\"${NODE_OPTIONS:+ $NODE_OPTIONS}"
export NODE_OPTIONS
exec "$runner" "$@"
