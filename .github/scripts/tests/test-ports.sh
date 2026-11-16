#!/bin/sh
# scripts/lib/ports.sh: every port pick_port returns lies in [20000, min(low - 1, 49999)], where `low` is the start of the kernel's
# outgoing-connection range, so no closed client connection can hold it in TIME_WAIT (issue #93: the server's bind then failed with
# EADDRINUSE although nothing listened). Generated: a matrix of range starts (Linux's, macOS's, between, the edge of the room, below
# it) x 300 draws each, which must stay inside the bound and reach both its bottom and top quarter; unreadable ranges; a stubbed
# `lsof` that marks ports busy; the real range of the machine this runs on (procfs, which dash's `read` cannot read); and
# contract.sh's use of the library.
#
#   test-ports.sh [path to a copy of ports.sh]   (a path points it at a deliberately broken copy)
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
LIB="${1:-$HERE/scripts/lib/ports.sh}"
CONTRACT="$HERE/scripts/contract.sh"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT
pass=0; failn=0; n=0
ok() { pass=$((pass + 1)); }
bad() { failn=$((failn + 1)); echo "BAD  $*"; }

# Stubs, first on PATH. `lsof` answers like the real one (exit 0 and a line = something listens): LSOF_BUSY=even marks even ports
# busy, LSOF_BUSY=all every port. `sysctl` answers net.inet.ip.portrange.first with SYSCTL_FIRST, and fails when it is unset.
mkdir "$T/bin"
cat >"$T/bin/lsof" <<'EOF'
#!/bin/sh
for a; do case "$a" in -iTCP:*) p="${a#-iTCP:}" ;; esac; done
case "${LSOF_BUSY:-}" in
  all) echo "x $p LISTEN"; exit 0 ;;
  even) [ $((p % 2)) -eq 0 ] && { echo "x $p LISTEN"; exit 0; } ;;
esac
exit 1
EOF
cat >"$T/bin/sysctl" <<'EOF'
#!/bin/sh
[ "$1" = -n ] && [ "$2" = net.inet.ip.portrange.first ] && [ -n "${SYSCTL_FIRST:-}" ] && { echo "$SYSCTL_FIRST"; exit 0; }
exit 1
EOF
chmod +x "$T/bin/lsof" "$T/bin/sysctl"

# run <range file> <shell code after sourcing the library>: output in $T/out, status returned
run() {
  THRUST_EPHEMERAL_RANGE_FILE="$1" PATH="$T/bin:$PATH" sh -c '. "$1"; eval "$2"' _ "$LIB" "$2" >"$T/out" 2>&1
}
DRAW='i=0; while [ $i -lt 300 ]; do pick_port || exit 1; i=$((i + 1)); done'

# draws_ok <label> <range file> <want top>: ports_top says <top>, and 300 draws stay in [20000, top] and reach both quarters
draws_ok() {
  label="$1"; f="$2"; top="$3"
  n=$((n + 1))
  if ! run "$f" 'ports_top'; then bad "$label: ports_top failed"; sed 's/^/     | /' "$T/out"; return; fi
  got="$(cat "$T/out")"
  if [ "$got" != "$top" ]; then bad "$label: ports_top = '$got', wanted $top"; return; fi
  ok
  n=$((n + 1))
  if ! run "$f" "$DRAW"; then bad "$label: a draw failed"; sed 's/^/     | /' "$T/out"; return; fi
  q=$(((top - 20000 + 1) / 4))
  # every line a number in range; the lowest in the bottom quarter, the highest in the top quarter
  verdict="$(awk -v lo=20000 -v hi="$top" -v q="$q" '
    !/^[0-9]+$/ || $1 < lo || $1 > hi { print "out of bound: " $0; exit }
    { c++; if (min == "" || $1 < min) min = $1; if ($1 > max) max = $1 }
    END { if (c != 300) print "count " c; else if (min > lo + q) print "never in the bottom quarter, min " min; else if (max < hi - q) print "never in the top quarter, max " max }' "$T/out")"
  if [ -z "$verdict" ]; then ok; else bad "$label: $verdict"; fi
}

# refuses <label> <range file> <needle>: pick_port fails and says why
refuses() {
  n=$((n + 1))
  if run "$2" 'pick_port'; then bad "$1: pick_port returned $(cat "$T/out"), wanted a refusal"; return; fi
  if grep -qF -- "$3" "$T/out"; then ok; else bad "$1: output lacks '$3'"; sed 's/^/     | /' "$T/out"; fi
}

range() { printf '%s\t%s\n' "$1" "$2" >"$T/range.$1"; echo "$T/range.$1"; }

# 1. the matrix of range starts (the file as procfs writes it: low, a tab, high)
for low in 32768 49152 40000 60000 50000 21000 21001 20999 20000 1024; do
  f="$(range "$low" 60999)"
  top=$((low - 1)); [ "$top" -gt 49999 ] && top=49999
  if [ $((top - 20000 + 1)) -ge 1000 ]; then draws_ok "low $low" "$f" "$top"; else refuses "low $low" "$f" "leaving fewer than 1000 ports"; fi
done

# 2. a range that cannot be read: not a number, empty, missing with no sysctl; missing with BSD/macOS sysctl
printf 'abc\t60999\n' >"$T/abc"; : >"$T/empty"
refuses "not a number" "$T/abc" "cannot read the kernel's outgoing-connection port range"
refuses "empty file" "$T/empty" "cannot read the kernel's outgoing-connection port range"
refuses "no file, no sysctl" "$T/missing" "cannot read the kernel's outgoing-connection port range"
export SYSCTL_FIRST=49152
draws_ok "no file, sysctl 49152" "$T/missing" 49151
unset SYSCTL_FIRST

# 3. busy ports are never returned; all busy is a refusal
f="$(range 32768 60999)"
n=$((n + 1))
if LSOF_BUSY=even run "$f" "$DRAW" && [ "$(wc -l <"$T/out" | tr -d ' ')" = 300 ] && ! awk '$1 % 2 == 0 { found = 1 } END { exit !found }' "$T/out"; then
  ok
else bad "a port lsof reports as listening was returned (or a draw failed)"; sed 's/^/     | /' "$T/out" | head -5; fi
n=$((n + 1))
if LSOF_BUSY=all run "$f" 'pick_port'; then bad "every port busy, yet pick_port returned $(cat "$T/out")"
elif grep -qF 'were all taken' "$T/out"; then ok; else bad "every port busy: output lacks 'were all taken'"; sed 's/^/     | /' "$T/out"; fi

# 4. the real range of this machine, through the default path (procfs on Linux, where dash's `read` gets one byte; sysctl on macOS)
real=""
if [ -r /proc/sys/net/ipv4/ip_local_port_range ]; then real="$(awk '{ print $1 }' /proc/sys/net/ipv4/ip_local_port_range)"
else real="$(sysctl -n net.inet.ip.portrange.first 2>/dev/null)"; fi
n=$((n + 1))
got="$(sh -c '. "$1"; ephemeral_low' _ "$LIB" 2>&1)"
if [ -n "$real" ] && [ "$got" = "$real" ]; then ok; else bad "this machine's range starts at '$real', ephemeral_low said '$got'"; fi

# 5. contract.sh draws through the library and stops when a draw fails (an empty port would start a server on a random one)
n=$((n + 1))
if grep -qF '. "$ROOT/scripts/lib/ports.sh"' "$CONTRACT" && ! grep -q '^pick_port()' "$CONTRACT"; then ok
else bad "contract.sh must source scripts/lib/ports.sh and define no pick_port of its own"; fi
n=$((n + 1))
unguarded="$(grep -n 'pick_port)' "$CONTRACT" | grep -v '|| exit 3')"
calls="$(grep -c 'pick_port)' "$CONTRACT")"
if [ "$calls" -ge 6 ] && [ -z "$unguarded" ]; then ok; else bad "every pick_port in contract.sh must be followed by '|| exit 3' ($calls calls; unguarded: $unguarded)"; fi

echo "passed=$pass bad=$failn (of $n)"
[ "$failn" -eq 0 ] && [ "$pass" = "$n" ]
