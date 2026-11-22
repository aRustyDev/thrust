#!/bin/sh
# The scripts around the Worker's wasm32 unit tests (G-6, stage 8 of the Test phase), which run before any test does and so cannot be shown by the
# tests themselves:
#   1. scripts/wasm-test-runner.sh, cargo's runner for wasm32: it passes its arguments to the real runner unchanged, sets NODE_OPTIONS to load the
#      stand-ins for workerd's modules (keeping what was there), and refuses a runner that is not the lock's wasm-bindgen EXACTLY (an older patch,
#      a newer one, a prefix of the right one, another crate's version), one that does not run, a Node that asserts on the Worker's `cloudflare:` imports
#      (every 22 and 23, 24 before 24.12, 25 before 25.2: stand-in nodes of each side of each boundary), and a Cargo that cannot name one version of
#      wasm-bindgen (none in the lock, or two). Which version the lock pins is Cargo's to say (`cargo pkgid --locked`): a stand-in Cargo answers here,
#      in the two forms of its output, and the real one answers once, against the real lock, when there is a cargo;
#   2. scripts/wasm-tests.sh: every `#[wasm_bindgen_test]` the source declares must have a test in the report (a run that drops one passes
#      nextest and must not pass this), a report left by an earlier run is not read, a failing run fails, and a run with arguments (a filter) is
#      not held to the count;
#   3. scripts/lib/workerd-stubs.cjs, when Node has `module.registerHooks` (22.15 or later; the `workers` job runs the real thing on 26): each
#      workerd module the Worker imports loads and exports what it is asked for (`tracing` answers as workerd does outside an invocation), the global
#      `FixedLengthStream` errors on a body of any other length than it was given, an unlisted `cloudflare:` module is refused naming the file, and an
#      ordinary module is not touched.
# Every check has a control: a deliberately broken copy of the script that the same check must reject (AGENTS.md rule 8).
#
#   test-wasm-tests.sh
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
pass=0; failn=0; n=0
ok() { pass=$((pass + 1)); }
bad() { failn=$((failn + 1)); echo "BAD  $*"; }
check() {
  label="$1"; shift
  n=$((n + 1))
  if out="$("$@" 2>&1)"; then ok; else bad "$label: $out"; fi
}
control() {
  label="$1"; shift
  n=$((n + 1))
  if out="$("$@" 2>&1)"; then bad "control '$label' was not caught (the check passed on a broken copy)"; else ok; if [ -n "${VERBOSE:-}" ]; then printf 'caught  %s\n          %s\n' "$label" "$(printf '%s' "$out" | head -n 1)"; fi; fi
}
# refuses <command...>: succeeds when the command fails (a refusal is the behaviour under test)
refuses() { if "$@" > /dev/null 2>&1; then return 1; else return 0; fi; }

# a copy of the scripts in a tree of their own (each finds its root from its own place), so that a broken copy can be made beside the real one
tree="$T/tree"
mkdir -p "$tree/scripts/lib" "$tree/crates/worker/src" "$tree/target/nextest/workers"
cp "$HERE/scripts/wasm-test-runner.sh" "$HERE/scripts/wasm-tests.sh" "$tree/scripts/"
cp "$HERE/scripts/lib/workerd-stubs.cjs" "$tree/scripts/lib/"
chmod +x "$tree"/scripts/*.sh

# 1. wasm-test-runner.sh, with stand-in cargos that answer only `pkgid --locked wasm-bindgen`, the question the wrapper must ask, and stub runners
# that say a version and otherwise report what they were given
cargo_says() { # cargo_says <name> <exit status> <what it prints>
  printf '#!/bin/sh\n[ "$*" = "pkgid --locked wasm-bindgen" ] || { echo "stand-in cargo: asked for: $*" >&2; exit 64; }\necho %s\nexit %s\n' "'$3'" "$2" > "$T/$1"
  chmod +x "$T/$1"
}
cargo_says cargo-registry 0 'registry+https://github.com/rust-lang/crates.io-index#wasm-bindgen@0.2.130'
cargo_says cargo-short 0 'file:///src/wasm-bindgen#0.2.130'
cargo_says cargo-ambiguous 101 'error: specification `wasm-bindgen` is ambiguous'
cargo_says cargo-none 101 'error: package ID specification `wasm-bindgen` did not match any packages'
cg="$T/cargo-registry"
stub() { # stub <version>: a runner that answers --version and otherwise prints its arguments, one per line, and NODE_OPTIONS
  printf '#!/bin/sh\nif [ "$1" = "--version" ]; then echo "wasm-bindgen-test-runner %s"; exit 0; fi\nfor a in "$@"; do echo "arg:$a"; done\necho "NODE_OPTIONS=${NODE_OPTIONS:-}"\n' "$1" > "$T/runner-$1"
  chmod +x "$T/runner-$1"
}
for v in 0.2.130 0.2.129 0.2.131 0.2.13 0.2.1300 0.4.80; do stub "$v"; done
node_stub() { # node_stub <version>: a `node` that says this version, first on PATH, so that no check depends on the Node of the machine it runs on
  mkdir -p "$T/node-$1"
  printf '#!/bin/sh\nif [ "$1" = "--version" ]; then echo "v%s"; exit 0; fi\nexit 1\n' "$1" > "$T/node-$1/node"
  chmod +x "$T/node-$1/node"
}
node_stub 26.13.1
run_wrapper() { # run_wrapper <wrapper> <runner> <cargo> <args...>
  rw_wrapper="$1"; rw_runner="$2"; rw_cargo="$3"; shift 3
  PATH="$T/node-26.13.1:$PATH" WASM_BINDGEN_TEST_RUNNER="$rw_runner" WASM_TEST_CARGO="$rw_cargo" "$rw_wrapper" "$@"
}
wrapper="$tree/scripts/wasm-test-runner.sh"
passes_args_through() { # the arguments arrive as they were, spaces and all, one each
  out="$(run_wrapper "$1" "$T/runner-0.2.130" "$cg" "/x/y.wasm" --list --format terse "a test name" --exact)" || return 1
  [ "$(printf '%s\n' "$out" | grep -c '^arg:')" = 6 ] || return 1
  printf '%s\n' "$out" | grep -q '^arg:a test name$'
}
# with NODE_OPTIONS absent from the environment, so that a wrapper that forgets to export it is not saved by a variable that already was
sets_node_options() {
  out="$(env -u NODE_OPTIONS PATH="$T/node-26.13.1:$PATH" WASM_BINDGEN_TEST_RUNNER="$T/runner-0.2.130" WASM_TEST_CARGO="$cg" "$1" x.wasm)" || return 1
  printf '%s\n' "$out" | grep -q '^NODE_OPTIONS=--require ".*scripts/lib/workerd-stubs.cjs"$'
}
keeps_node_options() {
  out="$(NODE_OPTIONS="--max-old-space-size=4096" run_wrapper "$1" "$T/runner-0.2.130" "$cg" x.wasm)" || return 1
  printf '%s\n' "$out" | grep -q 'NODE_OPTIONS=.*--require ".*scripts/lib/workerd-stubs.cjs".*--max-old-space-size=4096'
}
runs_with_the_registry_id() { run_wrapper "$1" "$T/runner-0.2.130" "$cg" x.wasm > /dev/null; }
check "runner: the lock's own version runs, arguments unchanged" passes_args_through "$wrapper"
check "runner: NODE_OPTIONS loads the stand-ins" sets_node_options "$wrapper"
check "runner: and keeps what was set" keeps_node_options "$wrapper"
check "runner: the short form of Cargo's id (#version) is read the same" run_wrapper "$wrapper" "$T/runner-0.2.130" "$T/cargo-short" x.wasm
for v in 0.2.129 0.2.131 0.2.13 0.2.1300 0.4.80; do
  control "runner: $v is not 0.2.130" run_wrapper "$wrapper" "$T/runner-$v" "$cg" x.wasm
done
control "runner: one that does not run" run_wrapper "$wrapper" "$T/no-such-runner" "$cg" x.wasm
control "runner: a lock with two wasm-bindgen (Cargo says it is ambiguous)" run_wrapper "$wrapper" "$T/runner-0.2.130" "$T/cargo-ambiguous" x.wasm
control "runner: a lock with none (Cargo says nothing matched)" run_wrapper "$wrapper" "$T/runner-0.4.80" "$T/cargo-none" x.wasm
control "runner: no cargo" run_wrapper "$wrapper" "$T/runner-0.2.130" "$T/no-such-cargo" x.wasm
# The Node the runner starts: the Worker's inline JavaScript imports `cloudflare:*`, and every Node 22 and 23, 24 before 24.12 and 25 before 25.2 assert on
# that from an ES module that was require()d (the first run of the `workers` job, on 22.22.0). The boundaries are the ones of Node's own source.
with_node() { # with_node <version> <wrapper>: the wrapper, with a node of this version first on PATH
  node_stub "$1"
  PATH="$T/node-$1:$PATH" WASM_BINDGEN_TEST_RUNNER="$T/runner-0.2.130" WASM_TEST_CARGO="$cg" "$2" x.wasm
}
for v in 24.12.0 24.22.3 25.2.0 25.9.0 26.9.0 26.13.1; do
  check "runner: Node $v runs" with_node "$v" "$wrapper"
done
for v in 20.20.2 22.15.0 22.22.0 22.23.5 23.11.1 24.11.1 25.0.0 25.1.0; do
  control "runner: Node $v is refused" with_node "$v" "$wrapper"
done
mkdir -p "$T/node-odd"
printf '#!/bin/sh\nif [ "$1" = "--version" ]; then echo "nightly"; exit 0; fi\n' > "$T/node-odd/node"; chmod +x "$T/node-odd/node"
control "runner: a node that does not say a version" env PATH="$T/node-odd:$PATH" WASM_BINDGEN_TEST_RUNNER="$T/runner-0.2.130" WASM_TEST_CARGO="$cg" "$wrapper" x.wasm
printf '#!/bin/sh\nexit 1\n' > "$T/node-odd/node"
control "runner: a node that does not run" env PATH="$T/node-odd:$PATH" WASM_BINDGEN_TEST_RUNNER="$T/runner-0.2.130" WASM_TEST_CARGO="$cg" "$wrapper" x.wasm
# the controls of the checks themselves: the wrapper without its export, without its version comparison, asking Cargo about another crate, and
# not asking for the locked version
sed 's/^export NODE_OPTIONS$/:/' "$wrapper" > "$T/no-export.sh"; chmod +x "$T/no-export.sh"; cp "$T/no-export.sh" "$tree/scripts/no-export.sh"
sed 's/ != "\$wanted" \]/ = "never" ]/' "$wrapper" > "$tree/scripts/no-compare.sh"; chmod +x "$tree/scripts/no-compare.sh"
sed 's/pkgid --locked wasm-bindgen)"/pkgid --locked wasm-bindgen-macro)"/' "$wrapper" > "$tree/scripts/other-crate.sh"; chmod +x "$tree/scripts/other-crate.sh"
sed 's/pkgid --locked wasm-bindgen)"/pkgid wasm-bindgen)"/' "$wrapper" > "$tree/scripts/not-locked.sh"; chmod +x "$tree/scripts/not-locked.sh"
control "runner check: a wrapper that does not export NODE_OPTIONS" sets_node_options "$tree/scripts/no-export.sh"
control "runner check: a wrapper that asks Cargo about another crate" runs_with_the_registry_id "$tree/scripts/other-crate.sh"
control "runner check: a wrapper that does not ask for the locked version" runs_with_the_registry_id "$tree/scripts/not-locked.sh"
refuses_stale() { refuses run_wrapper "$1" "$T/runner-0.2.129" "$cg" x.wasm; }
check "runner check: the real wrapper refuses a stale runner" refuses_stale "$wrapper"
control "runner check: a wrapper that does not compare versions" refuses_stale "$tree/scripts/no-compare.sh"
# the Node guard, one boundary at a time: a wrapper that lets Node 22 through, one that lets 24.11 through and one that lets 25.1 through
sed 's/"\$major" -lt 24/"$major" -lt 0/' "$wrapper" > "$tree/scripts/any-major.sh"
sed 's/"\$minor" -lt 12/"$minor" -lt 0/' "$wrapper" > "$tree/scripts/any-24.sh"
sed 's/"\$minor" -lt 2 /"$minor" -lt 0 /' "$wrapper" > "$tree/scripts/any-25.sh"
chmod +x "$tree/scripts/any-major.sh" "$tree/scripts/any-24.sh" "$tree/scripts/any-25.sh"
refuses_old_nodes() { refuses with_node 22.22.0 "$1" && refuses with_node 24.11.1 "$1" && refuses with_node 25.1.0 "$1"; }
check "runner check: the real wrapper refuses Node 22.22.0, 24.11.1 and 25.1.0" refuses_old_nodes "$wrapper"
control "runner check: a wrapper that lets Node 22 through" refuses_old_nodes "$tree/scripts/any-major.sh"
control "runner check: a wrapper that lets Node 24.11 through" refuses_old_nodes "$tree/scripts/any-24.sh"
control "runner check: a wrapper that lets Node 25.1 through" refuses_old_nodes "$tree/scripts/any-25.sh"
# the real cargo, on the real lock, through the real wrapper: the version it names is the one a runner must be, and a runner of another is refused
if command -v cargo > /dev/null 2>&1 && real_id="$(cd "$HERE" && cargo pkgid --locked wasm-bindgen 2> /dev/null)"; then
  real_version="${real_id##*[#@]}"
  stub "$real_version"; stub 0.0.1
  real_cargo_runs() { run_wrapper "$HERE/scripts/wasm-test-runner.sh" "$T/runner-$real_version" cargo x.wasm > /dev/null; }
  real_cargo_refuses_another() { refuses run_wrapper "$HERE/scripts/wasm-test-runner.sh" "$T/runner-0.0.1" cargo x.wasm; }
  check "runner: the real cargo names the version of the real lock ($real_version) and its runner runs" real_cargo_runs
  check "runner: and a runner of another version is refused" real_cargo_refuses_another
else
  echo "skipped: the check of the real cargo against the real lock needs cargo and a lock it can read offline"
fi

# 2. wasm-tests.sh, with a stub for the run: writes a report of the names it is given
src="$tree/crates/worker/src/wasm_tests.rs"
junit="$tree/target/nextest/workers/junit.xml"
printf '%s\n' '#[wasm_bindgen_test]' 'fn a() {}' '#[wasm_bindgen_test]' 'async fn b() {}' '    #[wasm_bindgen_test] // indented: not a declaration of this file' '#[wasm_bindgen_test]' 'fn c() {}' > "$src"
report_of() { # report_of <n>: a run that writes n testcases
  printf '#!/bin/sh\n{ echo "<testsuites>"; i=0; while [ "$i" -lt %s ]; do i=$((i + 1)); echo "<testcase name=\\"t$i\\" classname=\\"thrust-worker\\"/>"; done; echo "</testsuites>"; } > "$WASM_TESTS_JUNIT"\n' "$1" > "$T/run-$1.sh"
  chmod +x "$T/run-$1.sh"
}
for k in 0 2 3 4; do report_of "$k"; done
printf '#!/bin/sh\nexit 0\n' > "$T/run-nothing.sh"; chmod +x "$T/run-nothing.sh"
printf '#!/bin/sh\n{ echo "<testsuites>"; echo "<testcase name=\\"a\\"/><testcase name=\\"b\\"/><testcase name=\\"c\\"/>"; echo "</testsuites>"; } > "$WASM_TESTS_JUNIT"\nexit 1\n' > "$T/run-fails.sh"; chmod +x "$T/run-fails.sh"
run_tests() { # run_tests <script> <run command> [source]
  WASM_TESTS_RUN="$2" WASM_TESTS_JUNIT="$junit" WASM_TESTS_SOURCE="${3:-$src}" "$1"
}
tests_script="$tree/scripts/wasm-tests.sh"
check "tests: 3 declared, 3 in the report" run_tests "$tests_script" "$T/run-3.sh"
control "tests: 3 declared, 2 in the report" run_tests "$tests_script" "$T/run-2.sh"
control "tests: 3 declared, 4 in the report" run_tests "$tests_script" "$T/run-4.sh"
control "tests: a run that writes no report" run_tests "$tests_script" "$T/run-nothing.sh"
control "tests: a run that fails, whatever it wrote" run_tests "$tests_script" "$T/run-fails.sh"
printf 'nothing declared here\n' > "$T/empty.rs"
control "tests: nothing declared" run_tests "$tests_script" "$T/run-0.sh" "$T/empty.rs"
stale_not_read() { # a report from an earlier run, and a run that writes none
  printf '<testsuites><testcase name="a"/><testcase name="b"/><testcase name="c"/></testsuites>\n' > "$junit"
  run_tests "$1" "$T/run-nothing.sh"
}
control "tests: a report left by an earlier run" stale_not_read "$tests_script"
# the controls of the checks themselves
sed 's/"\$declared" -ne "\$ran"/"$declared" -ne "$declared"/' "$tests_script" > "$tree/scripts/no-count.sh"; chmod +x "$tree/scripts/no-count.sh"
sed 's/^rm -f "\$junit"$/:/' "$tests_script" > "$tree/scripts/no-rm.sh"; chmod +x "$tree/scripts/no-rm.sh"
refuses_dropped() { refuses run_tests "$1" "$T/run-2.sh"; }
check "tests check: the real script refuses a dropped test" refuses_dropped "$tests_script"
control "tests check: a script that does not count" refuses_dropped "$tree/scripts/no-count.sh"
with_args() { # with_args <script> <run command>: the arguments a person gives for one test
  WASM_TESTS_RUN="$2" WASM_TESTS_JUNIT="$junit" WASM_TESTS_SOURCE="$src" "$1" -E 'test(a)'
}
filtered_run() { with_args "$1" "$T/run-2.sh"; }
failing_filtered_run() { with_args "$1" "$T/run-fails.sh"; }
sed 's/^if \[ "\$#" -gt 0 \]; then$/if false; then/' "$tests_script" > "$tree/scripts/always-count.sh"; chmod +x "$tree/scripts/always-count.sh"
check "tests: a run with arguments may run fewer than are declared" filtered_run "$tests_script"
control "tests: a run with arguments that fails" failing_filtered_run "$tests_script"
control "tests check: a script that counts a filtered run too" filtered_run "$tree/scripts/always-count.sh"
refuses_stale_report() { refuses stale_not_read "$1"; }
check "tests check: the real script ignores a stale report" refuses_stale_report "$tests_script"
control "tests check: a script that keeps a stale report" refuses_stale_report "$tree/scripts/no-rm.sh"

# 3. the stand-ins, where Node can load them
node_hooks() { command -v node > /dev/null 2>&1 && node -e 'process.exit(typeof require("node:module").registerHooks === "function" ? 0 : 1)' 2> /dev/null; }
loads() { # loads <stubs file> <specifier> <export>
  node --require "$1" -e 'import(process.argv[1]).then((m) => process.exit(process.argv[2] in m ? 0 : 3), () => process.exit(4))' "$2" "$3"
}
all_four_load() {
  loads "$1" cloudflare:workers tracing && loads "$1" cloudflare:sockets connect && loads "$1" cloudflare:node handleAsNodeConnection && loads "$1" cloudflare:email EmailMessage
}
# the entry points call tracing.getActiveSpan() on every request (worker::observability::in_current_span): workerd answers undefined outside an invocation
tracing_has_no_active_span() {
  node --require "$1" -e 'import("cloudflare:workers").then((m) => process.exit(m.tracing.getActiveSpan() === undefined && m.tracing.startSpan === undefined ? 0 : 3)).catch(() => process.exit(5))'
}
# worker::http::response pipes every body of a known size through workerd's FixedLengthStream, which errors on a body of any other length
fixed_length_counts() {
  node --require "$1" -e '
    const via = (size, length) => new Response(new Blob([new Uint8Array(size)]).stream().pipeThrough(new FixedLengthStream(length))).arrayBuffer().then((b) => b.byteLength, () => -1);
    // the bytes a reader is handed before the error: a body longer than its length must not reach the consumer, even though the stream ends in an error either way
    const got = async (size, length) => {
      const reader = new Blob([new Uint8Array(size)]).stream().pipeThrough(new FixedLengthStream(length)).getReader();
      let n = 0;
      try { for (;;) { const { done, value } = await reader.read(); if (done) break; n += value.byteLength; } } catch { /* the error is the point */ }
      return n;
    };
    Promise.all([via(5, 5), via(5, 4), via(5, 6), via(0, 0), via(3, 3n), via(7, 3n), got(5, 5), got(5, 4)]).then((r) => process.exit(JSON.stringify(r) === "[5,-1,-1,0,3,-1,5,0]" ? 0 : 3)).catch(() => process.exit(5))
  '
}
unlisted_refused_by_name() {
  out="$(node --require "$1" -e 'import("cloudflare:nope").then(() => process.exit(0), (e) => { console.log(e.message); process.exit(1) })' 2>&1)" && return 1
  printf '%s' "$out" | grep -q 'scripts/lib/workerd-stubs.cjs'
}
ordinary_untouched() { loads "$1" node:path join; }
if node_hooks; then
  stubs="$tree/scripts/lib/workerd-stubs.cjs"
  check "stubs: the four workerd modules load and export their names" all_four_load "$stubs"
  check "stubs: an unlisted cloudflare: module is refused, naming the file" unlisted_refused_by_name "$stubs"
  check "stubs: an ordinary module is not touched" ordinary_untouched "$stubs"
  check "stubs: tracing has no active span and no startSpan, as workerd outside an invocation" tracing_has_no_active_span "$stubs"
  check "stubs: FixedLengthStream passes exactly its length and errors on more or less" fixed_length_counts "$stubs"
  sed 's|if (seen > expected) throw mismatch();|;|' "$stubs" > "$tree/scripts/lib/fixed-no-max.cjs"
  sed 's|if (seen !== expected) throw mismatch();|;|' "$stubs" > "$tree/scripts/lib/fixed-no-min.cjs"
  sed 's|^globalThis.FixedLengthStream ??= |const notDefined = |' "$stubs" > "$tree/scripts/lib/fixed-absent.cjs"
  control "stubs check: a FixedLengthStream that lets a longer body through" fixed_length_counts "$tree/scripts/lib/fixed-no-max.cjs"
  control "stubs check: a FixedLengthStream that lets a shorter body through" fixed_length_counts "$tree/scripts/lib/fixed-no-min.cjs"
  control "stubs check: no FixedLengthStream at all (what the first fetch-test run hit)" fixed_length_counts "$tree/scripts/lib/fixed-absent.cjs"
  sed "s/'cloudflare:email': undefinedExports('EmailMessage'),//" "$stubs" > "$tree/scripts/lib/no-email.cjs"
  sed 's/shortCircuit: true };$/shortCircuit: false };/' "$stubs" > "$tree/scripts/lib/no-short.cjs"
  sed 's/= { getActiveSpan() { return undefined; } };/= undefined;/' "$stubs" > "$tree/scripts/lib/tracing-undefined.cjs"
  sed 's/getActiveSpan() { return undefined; }/getActiveSpan() { return 1; }/' "$stubs" > "$tree/scripts/lib/tracing-span.cjs"
  control "stubs check: a file without the email module" all_four_load "$tree/scripts/lib/no-email.cjs"
  control "stubs check: a hook that does not short-circuit" all_four_load "$tree/scripts/lib/no-short.cjs"
  control "stubs check: tracing as undefined (what the first entry-point run hit)" tracing_has_no_active_span "$tree/scripts/lib/tracing-undefined.cjs"
  control "stubs check: tracing that reports an active span" tracing_has_no_active_span "$tree/scripts/lib/tracing-span.cjs"
else
  echo "skipped: the checks of scripts/lib/workerd-stubs.cjs need Node with module.registerHooks (22.15 or later); the \`workers\` job runs it on the pinned Node"
fi

echo "$pass of $n checks passed"
[ "$failn" -eq 0 ]
