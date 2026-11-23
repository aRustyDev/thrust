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
# Node must be 26, or 24.12 and later (see below, where it is checked).
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

# The Node the runner starts (it runs `node`). The Worker's inline JavaScript imports `cloudflare:workers` and its like, and every Node 22 and 23, 24 before
# 24.12 and 25 before 25.2 assert on an import of such a specifier from an ES module that was require()d: ERR_INTERNAL_ASSERTION, "a bug in Node.js", before
# any hook is asked (found by the first run of the `workers` job, on 22.22.0; the releases were told apart by reading lib/internal/modules/esm/loader.js of each).
# So it is refused here, in words, and not left to that message.
node_version="$(node --version 2> /dev/null)" || {
  echo "scripts/wasm-test-runner.sh: cannot run node" >&2
  exit 1
}
node_version="${node_version#v}"
major="${node_version%%.*}"
minor="${node_version#*.}"
minor="${minor%%.*}"
case "$major.$minor" in
  . | .* | *. | *[!0-9.]*)
    echo "scripts/wasm-test-runner.sh: cannot read the version of node: '$node_version'" >&2
    exit 1
    ;;
esac
if [ "$major" -lt 24 ] || { [ "$major" -eq 24 ] && [ "$minor" -lt 12 ]; } || { [ "$major" -eq 25 ] && [ "$minor" -lt 2 ]; }; then
  echo "scripts/wasm-test-runner.sh: Node $node_version cannot load the Worker's wasm32 tests: it asserts (ERR_INTERNAL_ASSERTION) on the import of a \`cloudflare:\` module" >&2
  echo "  from an ES module that was require()d, as every Node 22 and 23, 24 before 24.12 and 25 before 25.2 do. Use Node 26 (the \`workers\` job runs 26.13.1)," >&2
  echo "  or 24.12 or later (read in Node's source, not run)." >&2
  exit 1
fi

# shellcheck disable=SC2089 # the quotes are for Node, which reads NODE_OPTIONS as a command line (a path with a blank in it); the shell never splits it
NODE_OPTIONS="--require \"$root/scripts/lib/workerd-stubs.cjs\"${NODE_OPTIONS:+ $NODE_OPTIONS}"
# shellcheck disable=SC2090 # exported, and so handed to Node as it is: nothing in the shell expands it unquoted
export NODE_OPTIONS
exec "$runner" "$@"
