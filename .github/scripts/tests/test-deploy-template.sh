#!/bin/sh
# The deployed Worker's template against the config everything else runs on (issue #49): wrangler.deploy.toml and wrangler.toml must
# bind the same things, and the template must hold markers where a real id or name would go. Every check is shown to fail on a copy
# that is broken in exactly that way; a check that has not been seen to fail is not evidence.
#
#   sh .github/scripts/tests/test-deploy-template.sh
HERE="$(cd "$(dirname "$0")/../../.." && pwd)"
BASE="$HERE/crates/worker/wrangler.toml"
DEPLOY="$HERE/crates/worker/wrangler.deploy.toml"
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
expect "a real dead-letter queue in the template is found" 1 "must be markers" parity "$BASE" "$T/real-dlq.toml"

# the queues (ADR-0070, FR-21, issue #53): the dead-letter queue is read by a consumer of its own, which has no dead-letter queue (a queue
# cannot be its own); THRUST_EVENTS_DLQ names it; the events consumer's retries cover the delivery policy; and each file says so alone
# (a backslash and a newline in the replacement: the one way to put a line in that both BSD sed and GNU sed take)
mk dlq-has-dlq -e 's/^max_retries = 10$/max_retries = 10\
dead_letter_queue = "__EVENTS_DLQ__"/'
expect "a dead-letter queue that is its own dead-letter queue is found" 1 "a queue cannot be its own" parity "$BASE" "$T/dlq-has-dlq.toml"

mk dlq-var-wrong -e 's/^THRUST_EVENTS_DLQ = "__EVENTS_DLQ__"/THRUST_EVENTS_DLQ = "__EVENTS_QUEUE__"/'
expect "a THRUST_EVENTS_DLQ that names another queue is found" 1 "the deploy template: THRUST_EVENTS_DLQ is" parity "$BASE" "$T/dlq-var-wrong.toml"

mk few-retries -e 's/^max_retries = 3$/max_retries = 2/'
expect "an events consumer that gives up before the policy does is found" 1 "max_retries is 2, under 3" parity "$BASE" "$T/few-retries.toml"

mk dlq-no-producer -e 's/binding = "EVENTS_DLQ"/binding = "EVENTS_DEAD"/'
expect "a Worker with no way to send to the dead-letter queue is found" 1 "both needed" parity "$BASE" "$T/dlq-no-producer.toml"

mk dlq-unread -e '/^queue = "__EVENTS_DLQ__"$/{N;s/^queue = "__EVENTS_DLQ__"\(.\)max_batch_size/queue = "__EVENTS_OTHER__"\1max_batch_size/;}'
expect "a dead-letter queue nobody reads is found" 1 "not exactly the events queue and its dead-letter queue" parity "$BASE" "$T/dlq-unread.toml"

sed 's/^THRUST_EVENTS_DLQ = "thrust-spike-events-dlq"/THRUST_EVENTS_DLQ = "thrust-spike-other"/' "$BASE" > "$T/base-dlq-var.toml"
if cmp -s "$BASE" "$T/base-dlq-var.toml"; then echo "BAD  the change for 'base-dlq-var' changed nothing, so its control proves nothing"; failn=$((failn + 1)); fi
expect "wrangler.toml naming another dead-letter queue is found" 1 "wrangler.toml: THRUST_EVENTS_DLQ is" parity "$T/base-dlq-var.toml" "$DEPLOY"

# the number comes from the policy's own constant: the same files, and a policy that allows more deliveries than the queue's retries
sed 's/pub const MAX_DELIVERIES: u32 = 4;/pub const MAX_DELIVERIES: u32 = 6;/' "$HERE/crates/core/ports/src/lib.rs" > "$T/ports-six.rs"
if cmp -s "$HERE/crates/core/ports/src/lib.rs" "$T/ports-six.rs"; then echo "BAD  the change for 'ports-six' changed nothing, so its control proves nothing"; failn=$((failn + 1)); fi
export DEPLOY_PARITY_PORTS="$T/ports-six.rs"
expect "a policy of more deliveries than the queue's retries is found" 1 "max_retries is 3, under 5" parity "$BASE" "$DEPLOY"
export DEPLOY_PARITY_PORTS="$T/no-such-ports-file.rs"
expect "a ports source that cannot be read is a usage error" 2 "cannot read" parity "$BASE" "$DEPLOY"
unset DEPLOY_PARITY_PORTS

mk real-name -e 's/__WORKER_NAME__/thrust-staging/'
expect "a real Worker name in the template is found" 1 "name is" parity "$BASE" "$T/real-name.toml"

mk no-var -e '/^THRUST_CSRF_ENFORCE/d'
expect "a variable the Worker reads, gone, is found" 1 "var_names" parity "$BASE" "$T/no-var.toml"

mk other-batch -e 's/max_batch_size = 10/max_batch_size = 11/'
expect "a changed queue consumer setting is found" 1 "queue_consumers" parity "$BASE" "$T/other-batch.toml"

mk other-class -e 's/new_sqlite_classes = \["RoomDo"\]/new_sqlite_classes = ["RoomDo", "Other"]/'
expect "a changed Durable Object migration is found" 1 "migrations" parity "$BASE" "$T/other-class.toml"

mk other-cron -e 's/^crons = \["\* \* \* \* \*", "0 0 \* \* \*"\]$/crons = ["*\/5 * * * *", "0 0 * * *"]/'
expect "a changed cron is found" 1 "crons" parity "$BASE" "$T/other-cron.toml"

# the triggers (issue #59): every wrangler file triggers exactly the schedules of crates/core/usecases/src/schedules.rs, which the Worker's
# `scheduled` handler runs the job of; a trigger the table lacks does nothing, and a schedule nothing triggers never runs
SCHED="$HERE/crates/core/usecases/src/schedules.rs"
STAGE2="$HERE/crates/worker/wrangler.stage2.toml"
PGSTAGE="$HERE/crates/worker/wrangler.pg.stage.toml"
parity_crons() { $PYRUN "$PARITY" --crons "$@"; }
mkfrom() { # mkfrom <source file> <name> <sed script...>: a copy of another file with the change
  src="$1"; name="$2"; shift 2
  sed "$@" "$src" > "$T/$name.toml"
  if cmp -s "$src" "$T/$name.toml"; then echo "BAD  the change for '$name' changed nothing, so its control proves nothing"; failn=$((failn + 1)); fi
}
TWO='^crons = \["\* \* \* \* \*", "0 0 \* \* \*"\]$'

expect "every wrangler file triggers exactly the schedules of the code" 0 "deploy_parity: crons ok (4 files" parity_crons "$BASE" "$DEPLOY" "$STAGE2" "$PGSTAGE"

mk no-hard-delete-cron -e "s/$TWO/crons = [\"* * * * *\"]/"
expect "a template that does not fire the hard delete is found" 1 "the deploy template: triggers.crons is" parity "$BASE" "$T/no-hard-delete-cron.toml"
mkfrom "$BASE" base-no-sweep-cron -e "s/$TWO/crons = [\"0 0 * * *\"]/"
expect "a wrangler.toml that does not fire the sweep is found" 1 "wrangler.toml: triggers.crons is" parity "$T/base-no-sweep-cron.toml" "$DEPLOY"
mkfrom "$STAGE2" stage2-one -e "s/$TWO/crons = [\"* * * * *\"]/"
expect "a stage file that does not fire the hard delete is found" 1 "stage2-one.toml: triggers.crons is" parity_crons "$BASE" "$DEPLOY" "$T/stage2-one.toml" "$PGSTAGE"
mkfrom "$PGSTAGE" pgstage-extra -e "s/$TWO/crons = [\"* * * * *\", \"0 0 * * *\", \"0 3 * * *\"]/"
expect "a stage file that fires a cron the code does not have is found" 1 "pgstage-extra.toml: triggers.crons is" parity_crons "$BASE" "$DEPLOY" "$STAGE2" "$T/pgstage-extra.toml"
mkfrom "$STAGE2" stage2-twice -e "s/$TWO/crons = [\"* * * * *\", \"0 0 * * *\", \"0 0 * * *\"]/"
expect "a cron triggered twice is found" 1 "stage2-twice.toml: triggers.crons is" parity_crons "$T/stage2-twice.toml"
expect "--crons with no file is a usage error" 2 "deploy_parity.py --crons" parity_crons

# a cron added to the two files together, and not to the code: the files agree with each other, so only the table can say it is wrong
mkfrom "$BASE" base-three -e "s/$TWO/crons = [\"* * * * *\", \"0 0 * * *\", \"0 3 * * *\"]/"
mkfrom "$DEPLOY" deploy-three -e "s/$TWO/crons = [\"* * * * *\", \"0 0 * * *\", \"0 3 * * *\"]/"
expect "a cron added to both files and not to the code is found" 1 "not the crons of crates/core/usecases/src/schedules.rs" parity "$T/base-three.toml" "$T/deploy-three.toml"
# ... and the table gaining that schedule is what makes it right, so the check follows the code and not a number of crons
awk '{ print } /id: "hard-delete"/ { print "    Schedule { id: \"third\", cron: \"0 3 * * *\", job: Job::HardDelete }," }' "$SCHED" > "$T/schedules-three.rs"
if cmp -s "$SCHED" "$T/schedules-three.rs"; then echo "BAD  the change for 'schedules-three' changed nothing, so its control proves nothing"; failn=$((failn + 1)); fi
export DEPLOY_PARITY_SCHEDULES="$T/schedules-three.rs"
expect "a schedule no wrangler file triggers is found" 1 "not the crons of crates/core/usecases/src/schedules.rs" parity "$BASE" "$DEPLOY"
expect "the same files agree with a table that has the third schedule" 0 "deploy_parity: ok" parity "$T/base-three.toml" "$T/deploy-three.toml"
# a cron quoted in a comment is not a schedule
awk '{ print } /^pub const SCHEDULES/ { print "    // cron: \"9 9 * * *\" is only a comment" }' "$SCHED" > "$T/schedules-comment.rs"
if cmp -s "$SCHED" "$T/schedules-comment.rs"; then echo "BAD  the change for 'schedules-comment' changed nothing, so its control proves nothing"; failn=$((failn + 1)); fi
export DEPLOY_PARITY_SCHEDULES="$T/schedules-comment.rs"
expect "a cron quoted in a comment of the table is not a schedule" 0 "deploy_parity: ok" parity "$BASE" "$DEPLOY"
export DEPLOY_PARITY_SCHEDULES="$T/no-such-schedules-file.rs"
expect "a schedules source that cannot be read is a usage error" 2 "cannot read" parity "$BASE" "$DEPLOY"
printf 'nothing here\n' > "$T/schedules-none.rs"
export DEPLOY_PARITY_SCHEDULES="$T/schedules-none.rs"
expect "a schedules source with no cron in it is a usage error" 2 "has no" parity "$BASE" "$DEPLOY"
unset DEPLOY_PARITY_SCHEDULES

mk other-limit -e 's/simple = { limit = 60, period = 60 }/simple = { limit = 61, period = 60 }/'
expect "a changed rate-limit allowance is found" 1 "ratelimits" parity "$BASE" "$T/other-limit.toml"

cp "$DEPLOY" "$T/probe.toml"; printf '\n[[ratelimits]]\nname = "RL"\nnamespace_id = "__RL_V1_ID__"\nsimple = { limit = 5, period = 60 }\n' >> "$T/probe.toml"
expect "a probe-only binding in the template is found" 1 "probe-only bindings" parity "$BASE" "$T/probe.toml"

# the mail service (E3): the EMAIL binding is in both and, in the template, restricted to the configured sender; the variables that give a
# Worker a mail service are the deployment's and not a development run's
mk no-sender-restriction -e '/^allowed_sender_addresses/d'
expect "a template whose EMAIL binding is not restricted to the sender is found" 1 "allowed_sender_addresses" parity "$BASE" "$T/no-sender-restriction.toml"
mk real-sender-restriction -e 's|allowed_sender_addresses = \["__EMAIL_FROM__"\]|allowed_sender_addresses = ["no-reply@notify.sow.foo"]|'
expect "a real sender address in the template's restriction is found" 1 "not the sender marker" parity "$BASE" "$T/real-sender-restriction.toml"
mk no-email-binding -e '/^\[\[send_email\]\]/,/^allowed_sender_addresses/d'
expect "a template with no EMAIL binding is found" 1 "send_email" parity "$BASE" "$T/no-email-binding.toml"
mk no-mail-name -e '/^THRUST_EMAIL_FROM_NAME/d'
expect "a mail variable dropped from the template is found" 1 "the deploy template sets" parity "$BASE" "$T/no-mail-name.toml"
mk no-mail-from -e '/^THRUST_EMAIL_FROM = /d'
expect "the sender variable dropped from the template is found" 1 "the deploy template sets" parity "$BASE" "$T/no-mail-from.toml"
mk real-origin -e 's|__PUBLIC_BASE_URL__|https://thrust-staging.sow.foo|'
expect "a real origin in the template is found" 1 "not the origin marker" parity "$BASE" "$T/real-origin.toml"
mk real-from -e 's|THRUST_EMAIL_FROM = "__EMAIL_FROM__"|THRUST_EMAIL_FROM = "no-reply@notify.sow.foo"|'
expect "a real sender in the template's variable is found" 1 "not the sender marker" parity "$BASE" "$T/real-from.toml"
awk '{ print } /^\[vars\]/ { print "THRUST_EMAIL_FROM = \"no-reply@thrust.test\"" }' "$BASE" > "$T/base-mail.toml"
expect "a mail variable in the shared wrangler.toml is found" 1 "a development run would have a mail service" parity "$T/base-mail.toml" "$DEPLOY"

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
