# Ports for the services a contract run starts (the server or `wrangler dev`, the mock identity provider, Postgres, Mailpit,
# the delay proxy). Sourced by scripts/contract.sh; tested by .github/scripts/tests/test-ports.sh.
#
# Why below the kernel's outgoing-connection range (issue #93): a port is chosen here, and bound later by a process that does
# not know about this check. Linux hands out the source ports of outgoing connections from `ip_local_port_range` (32768-60999 by
# default), and a closed client connection keeps its port in TIME_WAIT for a minute. `lsof -sTCP:LISTEN` does not see that
# socket, and because the client did not set SO_REUSEADDR the server's `bind` fails with EADDRINUSE (os error 98) although
# nothing listens there. A `mutants` job starts the server ~45 times among thousands of such connections, and the old range,
# 20000-49999, was 57% inside Linux's. Ports below the range are never handed out that way, so only a listener can hold one,
# and that the `lsof` check sees. (A port bound to 0 and read back cannot be used: the server's own URL, which is in its
# configuration and the identity provider's, must be known before it starts.)

PORTS_FLOOR=20000
# The old upper end: macOS starts its range at 49152, and there is no need to come near it.
PORTS_CAP=49999
# Less room than this below the range and draws would collide with each other.
PORTS_MIN_ROOM=1000

# The lowest port the kernel hands out to outgoing connections: Linux's /proc file, else BSD/macOS sysctl. Fails, naming
# why, when neither gives a number.
ephemeral_low() {
  range_file="${THRUST_EPHEMERAL_RANGE_FILE:-/proc/sys/net/ipv4/ip_local_port_range}"
  low=""
  if [ -r "$range_file" ]; then
    # Not `read`: dash reads a byte at a time, and procfs answers the second byte's read with end of file (measured: `3`).
    low="$(awk '{ print $1; exit }' "$range_file" 2>/dev/null || true)"
  elif command -v sysctl >/dev/null 2>&1; then
    low="$(sysctl -n net.inet.ip.portrange.first 2>/dev/null || true)"
  fi
  case "$low" in
    '' | *[!0-9]*)
      echo "ephemeral_low: cannot read the kernel's outgoing-connection port range ($range_file, sysctl net.inet.ip.portrange.first)" >&2
      return 1
      ;;
  esac
  echo "$low"
}

# The highest port pick_port draws: just below the outgoing range, at most PORTS_CAP. Fails when the room is too small.
ports_top() {
  low="$(ephemeral_low)" || return 1
  top=$((low - 1))
  [ "$top" -gt "$PORTS_CAP" ] && top=$PORTS_CAP
  if [ $((top - PORTS_FLOOR + 1)) -lt "$PORTS_MIN_ROOM" ]; then
    echo "ports_top: the kernel's outgoing-connection range starts at $low, leaving fewer than $PORTS_MIN_ROOM ports above $PORTS_FLOOR" >&2
    return 1
  fi
  echo "$top"
}

# The file that remembers the ports of one run, one a line: the ones pick_port drew and the ones the environment gave
# (ports_remember). The caller sets it, to a file of its own, before the first draw. A run draws the port of the server, of the mock
# identity provider, of Postgres and of Mailpit before any of them listens, and `lsof` cannot see a port nobody has bound yet, so
# without this a later draw can return an earlier one (measured: 400 draws over 1000 ports gave 66 ports twice) and the second
# service's bind is refused by the first. It is a file and not a variable because pick_port runs in `$( )`, a subshell.
#   PORTS_DRAWN_FILE="$LOG.ports"; : > "$PORTS_DRAWN_FILE"

# ports_remember <port>...: ports that were not drawn here (an environment variable gave them), so that no draw returns one of
# them. An empty argument is skipped, so `ports_remember "${THRUST_PORT:-}" ...` takes whatever is set.
ports_remember() {
  for given in "$@"; do
    if [ -n "$given" ]; then
      echo "$given" >>"$PORTS_DRAWN_FILE" || return 1
    fi
  done
  return 0
}

# A port in [PORTS_FLOOR, ports_top] that nothing listens on and that this run has not drawn, chosen at random: fixed ports collide
# with OrbStack-published containers and other sessions' wrangler instances (measured: 18080 and 8787 were both taken). Refuses
# when PORTS_DRAWN_FILE is not set: a picker that forgets what it drew is the defect above.
pick_port() {
  if [ -z "${PORTS_DRAWN_FILE:-}" ]; then
    echo "pick_port: PORTS_DRAWN_FILE is not set; without it a port can be handed out twice" >&2
    return 1
  fi
  top="$(ports_top)" || return 1
  span=$((top - PORTS_FLOOR + 1))
  n=0
  while [ "$n" -lt 50 ]; do
    p=$((PORTS_FLOOR + $(od -An -N2 -tu2 /dev/urandom | tr -d ' ') % span))
    if ! grep -qx "$p" "$PORTS_DRAWN_FILE" 2>/dev/null && ! lsof -nP -iTCP:"$p" -sTCP:LISTEN >/dev/null 2>&1; then
      echo "$p" >>"$PORTS_DRAWN_FILE" || return 1
      echo "$p"
      return 0
    fi
    n=$((n + 1))
  done
  echo "pick_port: 50 ports in $PORTS_FLOOR-$top were all taken" >&2
  return 1
}
