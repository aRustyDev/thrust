#!/bin/sh
# scripts/samply-wrapper.sh against a fake samply and a fake test binary, so it runs where samply is not installed (CI). The fake
# samply does what samply 0.13.1 was measured to do: it runs the command, exits with its status, but exits 0 when the command is
# killed by a signal. Every combination of how the test ends, whether samply is installed and writes a profile, and the test's name
# (sanitised into a file name) goes through `expect`, which checks the exit status AND a message fragment.
#
#   test-samply-wrapper.sh [path to a copy of samply-wrapper.sh]   (a path points it at a deliberately broken copy)
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
SCRIPT="${1:-$HERE/scripts/samply-wrapper.sh}"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
pass=0; failn=0
# The helpers name their variables e_* and c_*: sh has no local variables, and the loop below keeps its own `label`.
expect() { # expect <label> <want-exit> <want-fragment> <cmd...>
  e_label="$1"; e_want="$2"; e_frag="$3"; shift 3
  "$@" > "$T/last.out" 2>&1
  e_got=$?
  if [ "$e_got" = "$e_want" ] && grep -qF -- "$e_frag" "$T/last.out"; then
    pass=$((pass + 1))
  else
    failn=$((failn + 1)); echo "BAD  $e_label: wanted exit $e_want + '$e_frag', got $e_got"; sed 's/^/     | /' "$T/last.out"
  fi
}
check() { # check <label> <condition...>
  c_label="$1"; shift
  if "$@"; then pass=$((pass + 1)); else failn=$((failn + 1)); echo "BAD  $c_label"; fi
}

# The fake samply: `samply record [options] -o <out> [options] -- <command...>`.
mkdir -p "$T/bin" "$T/nobin"
cat > "$T/bin/samply" << 'EOF'
#!/bin/sh
[ "$1" = record ] || exit 64
shift
out=""
while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do
  [ "$1" = "-o" ] && { out="$2"; shift; }
  shift
done
shift
printf '%s\n' "$out" > "$FAKE_LOG/samply-out"
"$@"
rc=$?
echo "Stopping profile."
[ "${FAKE_PROFILE:-yes}" = yes ] && echo profile > "$out"
[ "$rc" -gt 128 ] && exit 0
exit "$rc"
EOF
# The fake test binary: libtest's output for the way FAKE_TEST says the test ends; it records its arguments and the marker.
cat > "$T/testbin" << 'EOF'
#!/bin/sh
printf '%s\n' "$@" > "$FAKE_LOG/args"
printf '%s\n' "${THRUST_PROF_SAMPLY:-unset}" > "$FAKE_LOG/marker"
echo "running 1 test"
case "$FAKE_TEST" in
  pass) echo "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s"; exit 0 ;;
  fail) echo "test result: FAILED. 0 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s"; exit 101 ;;
  none) echo "test result: ok. 0 passed; 0 failed; 0 ignored; 0 measured; 1 filtered out; finished in 0.01s"; exit 0 ;;
  killed) kill -s ABRT $$ ;;
esac
EOF
chmod +x "$T/bin/samply" "$T/testbin"
export FAKE_LOG="$T"
OUT="$T/out"
export SAMPLY_WRAPPER_OUT="$OUT"
WITH="$T/bin:/usr/bin:/bin"
WITHOUT="$T/nobin:/usr/bin:/bin"

# The fake itself: a killed command makes it exit 0, as samply does, so the cases below test the wrapper's guard.
FAKE_TEST=killed expect "fake samply: a killed command exits 0" 0 "Stopping profile." \
  env PATH="$WITH" samply record -o "$T/x.json.gz" -- "$T/testbin"

for name in prof_cpu_router 'cpu::prof_cpu_router' 'a b/c'; do
  file="$(printf '%s' "$name" | tr -c 'A-Za-z0-9_.-' '_')"
  for ending in pass fail none killed; do
    for installed in yes no; do
      for profile in yes no; do
        rm -rf "$OUT" "$T/args" "$T/marker" "$T/samply-out"
        label="name='$name' ending=$ending samply=$installed profile=$profile"
        if [ "$installed" = yes ]; then p="$WITH"; else p="$WITHOUT"; fi
        if [ "$installed" = no ]; then want=2; frag="samply is not on PATH"
        elif [ "$ending" = fail ]; then want=101; frag="test result: FAILED"
        elif [ "$ending" = none ] || [ "$ending" = killed ]; then want=1; frag="printed no pass line"
        elif [ "$profile" = no ]; then want=1; frag="wrote no profile"
        else want=0; frag="profile: $OUT/$file.json.gz"; fi
        FAKE_TEST="$ending" FAKE_PROFILE="$profile" expect "$label" "$want" "$frag" \
          env PATH="$p" sh "$SCRIPT" "$T/testbin" --exact "$name" --nocapture
        if [ "$installed" = yes ]; then
          check "$label: the profile goes to $OUT/$file.json.gz" [ "$(cat "$T/samply-out" 2> /dev/null)" = "$OUT/$file.json.gz" ]
          check "$label: the test binary gets its arguments unchanged" \
            [ "$(cat "$T/args" 2> /dev/null)" = "$(printf '%s\n' --exact "$name" --nocapture)" ]
          check "$label: the test binary sees the marker" [ "$(cat "$T/marker" 2> /dev/null)" = 1 ]
        else
          check "$label: without samply the test binary does not run" [ ! -e "$T/args" ]
        fi
      done
    done
  done
done

expect "no arguments" 2 "no test binary given" env PATH="$WITH" sh "$SCRIPT"
rm -f "$T/args"
FAKE_TEST=pass expect "no --exact" 2 "no --exact <test name>" env PATH="$WITH" sh "$SCRIPT" "$T/testbin" --nocapture
check "no --exact: the test binary does not run" [ ! -e "$T/args" ]
FAKE_TEST=pass expect "--exact with no name after it" 2 "no --exact <test name>" env PATH="$WITH" sh "$SCRIPT" "$T/testbin" --exact

echo "passed=$pass bad=$failn"
[ "$failn" = 0 ]
