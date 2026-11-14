#!/bin/sh
# Render the deployed Worker's wrangler config (issue #49, ADR-0110 round 29): fill the `__MARKER__`s of the template from the
# deploy environment's variables, refusing to write anything that is not whole.
#
#   scripts/render-wrangler.sh [--template FILE] [--out FILE]
#   scripts/render-wrangler.sh --self-test
#
# Defaults: --template crates/thrust-worker/wrangler.deploy.toml, --out crates/thrust-worker/.wrangler.deploy.run.toml (ignored by git).
#
# Variables, one per marker. They belong to a GitHub environment and none of them is secret:
#   THRUST_WORKER_NAME        __WORKER_NAME__      the Worker's name
#   THRUST_HYPERDRIVE_ID      __HYPERDRIVE_ID__    32 hex digits, not all zero
#   THRUST_R2_BUCKET          __BLOBS_BUCKET__     the R2 bucket
#   THRUST_QUEUE              __EVENTS_QUEUE__     the queue
#   THRUST_QUEUE_DLQ          __EVENTS_DLQ__       its dead-letter queue, a different queue
#   THRUST_RL_CREATE_ID, THRUST_RL_SIGNIN_ID, THRUST_RL_V1_ID
#                             __RL_CREATE_ID__ ... three rate-limit namespace ids: positive whole numbers, different from one another
#                                                  and from 1001-1004, which local runs use (the namespace is account-wide)
#   THRUST_CSRF_ENFORCE       __CSRF_ENFORCE__     `on` or `off`. No default: turning it on for production is a decision
#
# What it refuses, all at once (it lists every problem, not the first): a marker whose variable is unset or empty; a value of the wrong
# shape; an all-zero Hyperdrive id (what wrangler.toml carries for local runs); a name that still says `thrust-spike`; a marker in the
# template that this script does not know; and a rendered file that still has a marker in it. A refused render leaves no output file.
# Values are checked against a strict alphabet before they are substituted, so none can carry a character that changes the TOML.
#
# Exit status: 0 rendered; 1 refused (the reasons are on stderr); 2 called wrongly.
set -u
# Byte order for `[a-z]` in the `case` patterns below. In a UTF-8 locale on macOS a range such as `a-z` also matches capitals, and a
# name with a capital letter passed the check (found by this script's own self-test).
LC_ALL=C
export LC_ALL

SELF="$0"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
TEMPLATE="$ROOT/crates/thrust-worker/wrangler.deploy.toml"
OUT="$ROOT/crates/thrust-worker/.wrangler.deploy.run.toml"
LOCAL_RL_IDS="1001 1002 1003 1004"

# marker, variable, kind
MARKERS="__WORKER_NAME__:THRUST_WORKER_NAME:name
__HYPERDRIVE_ID__:THRUST_HYPERDRIVE_ID:hex32
__BLOBS_BUCKET__:THRUST_R2_BUCKET:name
__EVENTS_QUEUE__:THRUST_QUEUE:name
__EVENTS_DLQ__:THRUST_QUEUE_DLQ:name
__RL_CREATE_ID__:THRUST_RL_CREATE_ID:nsid
__RL_SIGNIN_ID__:THRUST_RL_SIGNIN_ID:nsid
__RL_V1_ID__:THRUST_RL_V1_ID:nsid
__CSRF_ENFORCE__:THRUST_CSRF_ENFORCE:onoff"

usage() { echo "render-wrangler: $*" >&2; exit 2; }

# problem(): the reasons are collected and printed together
PROBLEMS=""
problem() { PROBLEMS="$PROBLEMS
  - $*"; }

# check_value <variable> <kind> <value>: adds a problem for a value that is not of its kind
check_value() {
  var="$1"; kind="$2"; val="$3"
  case "$kind" in
    name)
      case "$val" in
        *[!a-z0-9-]*) problem "$var='$val': only lowercase letters, digits and '-' are allowed"; return ;;
        -*|*-) problem "$var='$val': must not start or end with '-'"; return ;;
      esac
      [ "${#val}" -ge 3 ] && [ "${#val}" -le 63 ] || problem "$var='$val': must be 3 to 63 characters"
      case "$val" in thrust-spike*) problem "$var='$val': a thrust-spike resource is a placeholder from local runs, not a deployed one" ;; esac
      ;;
    hex32)
      case "$val" in *[!0-9a-f]*) problem "$var: must be 32 lowercase hex digits"; return ;; esac
      [ "${#val}" -eq 32 ] || { problem "$var: must be 32 lowercase hex digits (got ${#val} characters)"; return; }
      case "$val" in 00000000000000000000000000000000) problem "$var is all zeros: that is the placeholder wrangler.toml carries for local runs" ;; esac
      ;;
    nsid)
      case "$val" in ''|*[!0-9]*|0*) problem "$var='$val': must be a positive whole number without leading zeros"; return ;; esac
      [ "${#val}" -le 18 ] || { problem "$var='$val': too many digits"; return; }
      for local_id in $LOCAL_RL_IDS; do
        [ "$val" = "$local_id" ] && problem "$var=$val is one of the ids local runs use ($LOCAL_RL_IDS); the namespace is account-wide"
      done
      ;;
    onoff)
      case "$val" in on|off) ;; *) problem "$var='$val': must be 'on' or 'off'" ;; esac
      ;;
  esac
}

render() {
  tmpl="$1"; out="$2"
  [ -f "$tmpl" ] || usage "the template '$tmpl' is not a file"
  PROBLEMS=""

  # every marker the template has must be one this script knows how to fill
  found="$(grep -o '__[A-Z0-9_][A-Z0-9_]*__' "$tmpl" | sort -u)"
  for m in $found; do
    echo "$MARKERS" | grep -q "^$m:" || problem "the template has the marker $m, which this script does not know"
  done

  # and each known marker the template has must have a value of the right kind
  for line in $MARKERS; do
    m="${line%%:*}"; rest="${line#*:}"; var="${rest%%:*}"; kind="${rest#*:}"
    echo "$found" | grep -qx "$m" || continue
    eval "set_p=\${$var+x}"
    eval "val=\${$var:-}"
    if [ -z "$set_p" ] || [ -z "$val" ]; then
      problem "$var is not set (it fills $m)"
      continue
    fi
    check_value "$var" "$kind" "$val"
  done

  # relations between values
  if [ -n "${THRUST_RL_CREATE_ID:-}" ] && [ -n "${THRUST_RL_SIGNIN_ID:-}" ] && [ -n "${THRUST_RL_V1_ID:-}" ]; then
    if [ "$THRUST_RL_CREATE_ID" = "$THRUST_RL_SIGNIN_ID" ] || [ "$THRUST_RL_CREATE_ID" = "$THRUST_RL_V1_ID" ] || [ "$THRUST_RL_SIGNIN_ID" = "$THRUST_RL_V1_ID" ]; then
      problem "the three rate-limit namespace ids must differ from one another (equal ids share one counter)"
    fi
  fi
  if [ -n "${THRUST_QUEUE:-}" ] && [ "${THRUST_QUEUE:-}" = "${THRUST_QUEUE_DLQ:-}" ]; then
    problem "THRUST_QUEUE_DLQ must be a different queue from THRUST_QUEUE"
  fi

  if [ -n "$PROBLEMS" ]; then
    echo "render-wrangler: refused:$PROBLEMS" >&2
    rm -f "$out"
    return 1
  fi

  work="$(mktemp)" || usage "cannot make a temporary file"
  cp "$tmpl" "$work"
  for line in $MARKERS; do
    m="${line%%:*}"; rest="${line#*:}"; var="${rest%%:*}"
    echo "$found" | grep -qx "$m" || continue
    eval "val=\${$var}"
    # the alphabet was checked above, so no escaping is needed for sed
    sed -e "s|$m|$val|g" "$work" > "$work.next" && mv "$work.next" "$work"
  done

  left="$(grep -n '__[A-Z0-9_][A-Z0-9_]*__' "$work" || true)"
  if [ -n "$left" ]; then
    echo "render-wrangler: refused: the rendered file still has a marker:" >&2
    echo "$left" >&2
    rm -f "$work" "$out"
    return 1
  fi
  if grep -Eq '"0{32}"' "$work"; then
    echo "render-wrangler: refused: the rendered file has an all-zero id" >&2
    rm -f "$work" "$out"
    return 1
  fi
  mv "$work" "$out"
  chmod 644 "$out"
  echo "render-wrangler: wrote $out from $tmpl"
  return 0
}

# ---- the self-test -----------------------------------------------------------------------------------------------------------
self_test() {
  T="$(mktemp -d)"
  trap 'rm -rf "$T"' EXIT INT TERM
  pass=0; bad=0
  good_env() {
    THRUST_WORKER_NAME=thrust-staging
    THRUST_HYPERDRIVE_ID=0123456789abcdef0123456789abcdef
    THRUST_R2_BUCKET=thrust-staging-blobs
    THRUST_QUEUE=thrust-staging-events
    THRUST_QUEUE_DLQ=thrust-staging-events-dlq
    THRUST_RL_CREATE_ID=2101
    THRUST_RL_SIGNIN_ID=2102
    THRUST_RL_V1_ID=2103
    THRUST_CSRF_ENFORCE=off
    export THRUST_WORKER_NAME THRUST_HYPERDRIVE_ID THRUST_R2_BUCKET THRUST_QUEUE THRUST_QUEUE_DLQ \
      THRUST_RL_CREATE_ID THRUST_RL_SIGNIN_ID THRUST_RL_V1_ID THRUST_CSRF_ENFORCE
  }
  # run <tmpl> <label> <want-exit> <want-fragment> '<snippet that changes the good environment>'
  run() {
    tmpl="$1"; label="$2"; want="$3"; frag="$4"; change="$5"
    # a STALE file from an earlier render is there before every run: a refused render has to remove it, or the deploy would go on
    # to ship the configuration of the last good run
    echo "stale configuration from an earlier run" > "$T/out.toml"
    ( good_env; eval "$change"; sh "$SELF" --template "$tmpl" --out "$T/out.toml" ) > "$T/log" 2>&1
    got=$?
    if [ "$got" = "$want" ] && grep -qF -- "$frag" "$T/log"; then
      pass=$((pass + 1)); echo "ok   $label"
    else
      bad=$((bad + 1)); echo "BAD  $label: wanted exit $want and '$frag', got $got"; sed 's/^/     | /' "$T/log"
      return
    fi
    # a refusal leaves no file behind (not even the stale one), a render replaces it
    if [ "$want" = 1 ] && [ -e "$T/out.toml" ]; then bad=$((bad + 1)); echo "BAD  $label: a refused render left an output file"; fi
    if [ "$want" = 0 ] && grep -q "stale configuration" "$T/out.toml" 2> /dev/null; then bad=$((bad + 1)); echo "BAD  $label: the render did not replace the old file"; fi
  }

  cat > "$T/mini.toml" <<'EOF'
name = "__WORKER_NAME__"
id = "__HYPERDRIVE_ID__"
bucket = "__BLOBS_BUCKET__"
queue = "__EVENTS_QUEUE__"
dlq = "__EVENTS_DLQ__"
a = "__RL_CREATE_ID__"
b = "__RL_SIGNIN_ID__"
c = "__RL_V1_ID__"
csrf = "__CSRF_ENFORCE__"
EOF
  # the real template, so the template and this script cannot drift apart
  REAL="$TEMPLATE"

  run "$T/mini.toml" "a filled template renders" 0 "wrote" ':'
  run "$REAL" "the real template renders with a good environment" 0 "wrote" ':'
  if grep -q '__[A-Z]' "$T/out.toml" 2> /dev/null; then bad=$((bad + 1)); echo "BAD  the rendered real template has a marker left"; fi
  run "$T/mini.toml" "the rendered file has the values in it" 0 "wrote" ':'
  grep -q 'name = "thrust-staging"' "$T/out.toml" && grep -q 'id = "0123456789abcdef0123456789abcdef"' "$T/out.toml" \
    && grep -q 'dlq = "thrust-staging-events-dlq"' "$T/out.toml" \
    && pass=$((pass + 1)) && echo "ok   ...and they are the right values in the right places" \
    || { bad=$((bad + 1)); echo "BAD  the rendered values are not where the markers were"; }

  # every variable, taken away, is refused, and the message names it
  for v in THRUST_WORKER_NAME THRUST_HYPERDRIVE_ID THRUST_R2_BUCKET THRUST_QUEUE THRUST_QUEUE_DLQ THRUST_RL_CREATE_ID THRUST_RL_SIGNIN_ID THRUST_RL_V1_ID THRUST_CSRF_ENFORCE; do
    run "$T/mini.toml" "$v unset is refused" 1 "$v is not set" "unset $v"
    run "$T/mini.toml" "$v empty is refused" 1 "$v is not set" "$v="
  done
  # a template that is the real one, for the one variable most likely to be forgotten when a marker is added
  run "$REAL" "the real template needs THRUST_CSRF_ENFORCE" 1 "THRUST_CSRF_ENFORCE is not set" "unset THRUST_CSRF_ENFORCE"

  # values of the wrong shape: the controls that make the checks above mean something
  run "$T/mini.toml" "an all-zero Hyperdrive id is refused" 1 "all zeros" 'THRUST_HYPERDRIVE_ID=00000000000000000000000000000000'
  run "$T/mini.toml" "a Hyperdrive id that is too short is refused" 1 "32 lowercase hex digits" 'THRUST_HYPERDRIVE_ID=0123456789abcdef'
  run "$T/mini.toml" "a Hyperdrive id with a capital letter is refused" 1 "32 lowercase hex digits" 'THRUST_HYPERDRIVE_ID=0123456789ABCDEF0123456789abcdef'
  run "$T/mini.toml" "a thrust-spike bucket is refused" 1 "thrust-spike" 'THRUST_R2_BUCKET=thrust-spike-blobs'
  run "$T/mini.toml" "a thrust-spike queue is refused" 1 "thrust-spike" 'THRUST_QUEUE=thrust-spike-events'
  run "$T/mini.toml" "a name with a capital letter is refused" 1 "only lowercase letters" 'THRUST_WORKER_NAME=Thrust-Staging'
  run "$T/mini.toml" "a name that could break the TOML is refused" 1 "only lowercase letters" 'THRUST_WORKER_NAME=x\"y'
  run "$T/mini.toml" "a name that starts with a dash is refused" 1 "must not start or end" 'THRUST_WORKER_NAME=-staging'
  run "$T/mini.toml" "a name that is too short is refused" 1 "3 to 63" 'THRUST_WORKER_NAME=ab'
  run "$T/mini.toml" "the dead-letter queue must differ from the queue" 1 "different queue" 'THRUST_QUEUE_DLQ=thrust-staging-events'
  run "$T/mini.toml" "a rate-limit id that is not a number is refused" 1 "positive whole number" 'THRUST_RL_V1_ID=abc'
  run "$T/mini.toml" "a rate-limit id with a leading zero is refused" 1 "positive whole number" 'THRUST_RL_V1_ID=0123'
  run "$T/mini.toml" "a rate-limit id that local runs use is refused" 1 "ids local runs use" 'THRUST_RL_V1_ID=1004'
  run "$T/mini.toml" "equal rate-limit ids are refused (create = v1)" 1 "must differ from one another" 'THRUST_RL_V1_ID=2101'
  run "$T/mini.toml" "equal rate-limit ids are refused (sign-in = v1)" 1 "must differ from one another" 'THRUST_RL_V1_ID=2102'
  run "$T/mini.toml" "equal rate-limit ids are refused (create = sign-in)" 1 "must differ from one another" 'THRUST_RL_SIGNIN_ID=2101'
  run "$T/mini.toml" "a CSRF value that is not on or off is refused" 1 "'on' or 'off'" 'THRUST_CSRF_ENFORCE=yes'

  # every problem at once, not only the first
  run "$T/mini.toml" "all the problems are listed together" 1 "THRUST_R2_BUCKET is not set" 'unset THRUST_R2_BUCKET; THRUST_HYPERDRIVE_ID=0000'
  grep -qF "32 lowercase hex digits" "$T/log" && pass=$((pass + 1)) && echo "ok   ...and the second one too" \
    || { bad=$((bad + 1)); echo "BAD  only the first problem was listed"; }

  # templates that are not whole
  printf 'name = "__WORKER_NAME__"\nextra = "__NOT_A_MARKER__"\n' > "$T/unknown.toml"
  run "$T/unknown.toml" "a marker the script does not know is refused" 1 "does not know" ':'
  printf 'name = "__WORKER_NAME__"\n' > "$T/one.toml"
  run "$T/one.toml" "a template with only some markers needs only their variables" 0 "wrote" 'unset THRUST_R2_BUCKET'
  run "$T/nope.toml" "a template that is not there is a usage error" 2 "is not a file" ':'

  echo "passed=$pass bad=$bad"
  [ "$bad" = 0 ]
}

# ---- arguments ---------------------------------------------------------------------------------------------------------------
MODE=render
while [ $# -gt 0 ]; do
  case "$1" in
    --template) [ $# -ge 2 ] || usage "--template needs a file"; TEMPLATE="$2"; shift ;;
    --out) [ $# -ge 2 ] || usage "--out needs a file"; OUT="$2"; shift ;;
    --self-test) MODE=self-test ;;
    -h|--help) sed -n '2,/^set -u/p' "$0" | sed '$d; s/^# \{0,1\}//'; exit 0 ;;
    *) usage "unknown argument '$1'" ;;
  esac
  shift
done

if [ "$MODE" = self-test ]; then
  self_test
  exit $?
fi
render "$TEMPLATE" "$OUT"
