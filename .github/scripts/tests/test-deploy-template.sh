#!/bin/sh
# The deployed Worker's template against the config everything else runs on (issue #49): wrangler.deploy.toml and wrangler.toml must
# bind the same things, and the template must hold markers where a real id or name would go. Every check is shown to fail on a copy
# that is broken in exactly that way; a check that has not been seen to fail is not evidence.
#
#   sh .github/scripts/tests/test-deploy-template.sh
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
BASE="$HERE/crates/thrust-worker/wrangler.toml"
DEPLOY="$HERE/crates/thrust-worker/wrangler.deploy.toml"
PARITY="$HERE/.github/scripts/tests/deploy_parity.py"
WORKER_DEV="$HERE/scripts/worker-dev.sh"
T="$(mktemp -d)"
trap 'rm -rf "$T"' EXIT INT TERM
pass=0; failn=0

if command -v uv > /dev/null 2>&1; then
  PYRUN="uv run --no-project --quiet python -I"
elif command -v python3 > /dev/null 2>&1; then
  PYRUN="python3 -I"
else
  echo "BAD  neither uv nor python3 is installed"; exit 1
fi

# expect <label> <want-exit> <want-fragment> <cmd...>
expect() {
  label="$1"; want="$2"; frag="$3"; shift 3
  "$@" > "$T/last.out" 2>&1
  got=$?
  if [ "$got" = "$want" ] && grep -qF -- "$frag" "$T/last.out"; then
    pass=$((pass + 1)); echo "ok   $label (exit $got; saw '$frag')"
  else
    failn=$((failn + 1)); echo "BAD  $label: wanted exit $want + '$frag', got $got"; sed 's/^/     | /' "$T/last.out"
  fi
}
parity() { $PYRUN "$PARITY" "$1" "$2" "${3:-$WORKER_DEV}"; }

expect "the real template and wrangler.toml agree" 0 "deploy_parity: ok" parity "$BASE" "$DEPLOY"

# each copy is broken in one way, and the check has to say which
mk() { # mk <name> <sed script...>: a copy of the template with the change
  name="$1"; shift
  sed "$@" "$DEPLOY" > "$T/$name.toml"
  if cmp -s "$DEPLOY" "$T/$name.toml"; then echo "BAD  the change for '$name' changed nothing, so its control proves nothing"; failn=$((failn + 1)); fi
}
mk renamed-limit -e 's/name = "RL_V1"/name = "RL_V2"/'
expect "a rate limit that is not bound the same way is found" 1 "ratelimits" parity "$BASE" "$T/renamed-limit.toml"

mk other-date -e 's/compatibility_date = "2026-08-01"/compatibility_date = "2026-08-02"/'
expect "a different compatibility date is found" 1 "compatibility_date" parity "$BASE" "$T/other-date.toml"

mk real-hyperdrive -e 's/__HYPERDRIVE_ID__/0123456789abcdef0123456789abcdef/'
expect "a real Hyperdrive id in the template is found" 1 "not a marker" parity "$BASE" "$T/real-hyperdrive.toml"

mk real-bucket -e 's/__BLOBS_BUCKET__/thrust-staging-blobs/'
expect "a real bucket name in the template is found" 1 "not a marker" parity "$BASE" "$T/real-bucket.toml"

mk real-namespace -e 's/__RL_V1_ID__/2103/'
expect "a real rate-limit namespace id in the template is found" 1 "not a marker" parity "$BASE" "$T/real-namespace.toml"

mk real-dlq -e 's/__EVENTS_DLQ__/thrust-staging-events-dlq/'
expect "a real dead-letter queue in the template is found" 1 "must both be markers" parity "$BASE" "$T/real-dlq.toml"

mk real-name -e 's/__WORKER_NAME__/thrust-staging/'
expect "a real Worker name in the template is found" 1 "name is" parity "$BASE" "$T/real-name.toml"

mk no-var -e '/^THRUST_CSRF_ENFORCE/d'
expect "a variable the Worker reads, gone, is found" 1 "var_names" parity "$BASE" "$T/no-var.toml"

mk other-batch -e 's/max_batch_size = 10/max_batch_size = 11/'
expect "a changed queue consumer setting is found" 1 "queue_consumers" parity "$BASE" "$T/other-batch.toml"

mk other-class -e 's/new_sqlite_classes = \["RoomDo"\]/new_sqlite_classes = ["RoomDo", "Other"]/'
expect "a changed Durable Object migration is found" 1 "migrations" parity "$BASE" "$T/other-class.toml"

mk other-cron -e 's/crons = \["\* \* \* \* \*"\]/crons = ["*\/5 * * * *"]/'
expect "a changed cron is found" 1 "crons" parity "$BASE" "$T/other-cron.toml"

mk other-limit -e 's/simple = { limit = 60, period = 60 }/simple = { limit = 61, period = 60 }/'
expect "a changed rate-limit allowance is found" 1 "ratelimits" parity "$BASE" "$T/other-limit.toml"

cp "$DEPLOY" "$T/probe.toml"; printf '\n[[send_email]]\nname = "EMAIL"\n' >> "$T/probe.toml"
expect "a probe-only binding in the template is found" 1 "probe-only bindings" parity "$BASE" "$T/probe.toml"

cp "$DEPLOY" "$T/build.toml"; printf '\n[build]\ncommand = "echo build"\n' >> "$T/build.toml"
expect "a [build] in the template is found" 1 "builds nothing" parity "$BASE" "$T/build.toml"

# the web app (ADR-0011): `[assets]` is in the template, with the list worker-dev.sh writes, and not in wrangler.toml
mk no-live-route -e 's|"/live", "/live/\*", ||'
expect "a Worker-first route dropped from the template is found" 1 "run_worker_first" parity "$BASE" "$T/no-live-route.toml"
mk other-fallback -e 's/not_found_handling = "single-page-application"/not_found_handling = "404-page"/'
expect "a different not-found handling is found" 1 "not_found_handling" parity "$BASE" "$T/other-fallback.toml"
mk other-directory -e 's/^directory = ".assets"/directory = "dist"/'
expect "a different assets directory is found" 1 "assets.directory" parity "$BASE" "$T/other-directory.toml"
sed '/^\[assets\]/,$d' "$DEPLOY" > "$T/no-assets.toml"
expect "a template without [assets] is found" 1 "no [assets]" parity "$BASE" "$T/no-assets.toml"
cp "$BASE" "$T/base-assets.toml"; printf '\n[assets]\ndirectory = "x"\n' >> "$T/base-assets.toml"
expect "an [assets] block in the shared wrangler.toml is found" 1 "wrangler.toml has an [assets] block" parity "$T/base-assets.toml" "$DEPLOY"
sed 's|"/live", "/live/\*", ||' "$WORKER_DEV" > "$T/worker-dev-other.sh"
expect "worker-dev.sh writing a different list is found" 1 "but scripts/worker-dev.sh writes" parity "$BASE" "$DEPLOY" "$T/worker-dev-other.sh"
printf 'nothing here\n' > "$T/worker-dev-none.sh"
expect "a worker-dev.sh with no list to compare is a usage error" 2 "no run_worker_first list" parity "$BASE" "$DEPLOY" "$T/worker-dev-none.sh"

# and the other side: wrangler.toml gets a binding the template has not
cp "$BASE" "$T/base-extra.toml"; printf '\n[[r2_buckets]]\nbinding = "MORE"\nbucket_name = "more"\n' >> "$T/base-extra.toml"
expect "a binding added to wrangler.toml and not to the template is found" 1 "r2" parity "$T/base-extra.toml" "$DEPLOY"

expect "a file that is not there is a usage error" 2 "cannot read" parity "$T/nope.toml" "$DEPLOY"

# ---- the wrangler the deploy installs is the wrangler the rest of CI uses ---------------------------------------------------------
# One version, in two actions that cannot import each other's default: this is what keeps them equal. `dry_run_pin` is the extraction the
# `dry-run` job of deploy.yml does, so a change to the action's layout that breaks it breaks this test, not a pull request's run.
DEPLOY_ACT="$HERE/.github/actions/deploy-worker/action.yml"
RUST_ACT="$HERE/.github/actions/setup-rust/action.yml"
default_of() { # default_of <file> <input name>: the default of one input
  awk -v want="  $2:" '$0 == want { on = 1; next } on && /^    default:/ { v = $0; sub(/^    default: *"?/, "", v); sub(/"? *$/, "", v); print v; exit } on && /^  [A-Za-z]/ { exit }' "$1"
}
dry_run_pin() { sed -n 's/^    default: "\(4\.[0-9.]*\)"$/\1/p' "$1" | head -1; }
same_pin() { # same_pin <deploy action> <setup-rust action>
  a="$(default_of "$1" wrangler-version)"; b="$(default_of "$2" wrangler-version)"
  [ -n "$a" ] && [ "$a" = "$b" ] && echo "same pin: $a" || { echo "different pin: deploy='$a' setup-rust='$b'"; return 1; }
}
expect "the deploy action and setup-rust pin the same wrangler" 0 "same pin: " same_pin "$DEPLOY_ACT" "$RUST_ACT"
sed 's/default: "4\.151\.2"/default: "4.150.0"/' "$DEPLOY_ACT" > "$T/act-other-pin.yml"
expect "CONTROL: a different wrangler in the deploy action is found" 1 "different pin: deploy='4.150.0'" same_pin "$T/act-other-pin.yml" "$RUST_ACT"
check_pin_extraction() {
  # (not `want` and `got`: they are the variables `expect` is using while it runs this)
  pin_want="$(default_of "$DEPLOY_ACT" wrangler-version)"; pin_got="$(dry_run_pin "$DEPLOY_ACT")"
  [ -n "$pin_want" ] && [ "$pin_got" = "$pin_want" ] && echo "extraction ok: $pin_got" || { echo "the dry-run job would read '$pin_got', not '$pin_want'"; return 1; }
}
expect "the dry-run job's extraction reads the same version" 0 "extraction ok: " check_pin_extraction

echo "passed=$pass bad=$failn"
[ "$failn" = 0 ]
