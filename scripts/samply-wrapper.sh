#!/bin/sh
# A nextest wrapper script (.config/nextest.toml, `experimental = ["wrapper-scripts"]`): the `prof` profile runs each prof_cpu_
# test of thrust-profiling under samply, which writes the CPU profile to target/profiling/<test>.json.gz, with its symbols in a
# sidecar (<test>.json.syms.json) so the profile opens without the build (`samply load <file>`).
#
# nextest calls it as:   samply-wrapper.sh <test binary> <libtest arguments, among them --exact <test name>>
# and it must run that binary with those arguments, or the test passes without running (nextest's documentation).
#
# samply 0.13.1 exits with the test's status when the test exits, but with 0 when the test is KILLED BY A SIGNAL (measured on
# macOS: a binary that aborts, exit 134 when run directly, gave samply exit 0). An abort or a crash would then read as a pass.
# So a 0 from samply counts only if libtest printed its pass line for exactly one test; libtest prints it last, only when the
# test ran to its end and passed. samply's output and the test's are kept in target/profiling/<test>.log and printed after.
#
# Needs samply on PATH (cargo install samply; scripts/rs puts ~/.cargo/bin first). On Linux samply needs
# /proc/sys/kernel/perf_event_paranoid at 1 or lower. Tested by .github/scripts/tests/test-samply-wrapper.sh with a fake samply.
set -u

if [ "$#" -eq 0 ]; then
  echo "samply-wrapper: no test binary given; nextest runs this as <wrapper> <test binary> <arguments>" >&2
  exit 2
fi

name=""
prev=""
for arg in "$@"; do
  [ "$prev" = "--exact" ] && name="$arg"
  prev="$arg"
done
if [ -z "$name" ]; then
  echo "samply-wrapper: no --exact <test name> among the arguments: $*" >&2
  exit 2
fi

if ! command -v samply > /dev/null 2>&1; then
  echo "samply-wrapper: samply is not on PATH. Install it with 'cargo install samply', and run nextest through scripts/rs" >&2
  exit 2
fi

root="$(cd "$(dirname "$0")/.." && pwd)"
dir="${SAMPLY_WRAPPER_OUT:-$root/target/profiling}"
mkdir -p "$dir" || exit 2
file="$(printf '%s' "$name" | tr -c 'A-Za-z0-9_.-' '_')"
out="$dir/$file.json.gz"
log="$dir/$file.log"
rm -f "$out" "$dir/$file.json.syms.json" "$log"

# The test refuses to run without this, so a prof_cpu_ test selected without this wrapper fails instead of passing unprofiled.
THRUST_PROF_SAMPLY=1
export THRUST_PROF_SAMPLY
samply record --save-only --no-open --unstable-presymbolicate --profile-name "$name" -o "$out" -- "$@" > "$log" 2>&1
rc=$?
cat "$log"
if [ "$rc" -ne 0 ]; then
  exit "$rc"
fi
if ! grep -q '^test result: ok\. 1 passed' "$log"; then
  echo "samply-wrapper: samply exited 0, but the test printed no pass line: it was killed by a signal, or no test ran" >&2
  exit 1
fi
if [ ! -s "$out" ]; then
  echo "samply-wrapper: the test passed, but samply wrote no profile at $out" >&2
  exit 1
fi
echo "profile: $out (open it with: samply load $out)"
