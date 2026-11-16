#!/bin/sh
# Mutation controls: prove the contract suite can FAIL, i.e. that its "X is rejected" assertions
# observe the thing they claim to test. Each mutant is a deliberately broken build; the suite must
# fail on it. A row of the contract suite (scripts/contract.sh, on nextest) is killed when nextest exits 100 and the pass's
# JUnit report counts a test that did not pass; a row of an adapter's own tests (still on `cargo test`) when cargo exits 101
# and a test reported FAILED. A mutant that survives means some assertion is vacuous.
#
#   scripts/mutants.sh
set -u
# A counterexample found against a deliberately broken build is not a regression of the real code: do not let the
# generated tests (crates/testing/suites/api/tests/model) save it into the source tree.
export PROPTEST_DISABLE_FAILURE_PERSISTENCE=1
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/target/mutants"
mkdir -p "$OUT"
SUMMARY="$OUT/summary.txt"
: >"$SUMMARY"
SURVIVED=0

# A row that was neither killed nor refused has not said why, and on a runner its output is thrown away with the job: the one thing the job log
# then lacks is the reason. `contract.sh` says it when it stops (a server that exited early, one that never became healthy, a mail server that did not
# come up, a build that failed), in the row's `$OUT/<name>.out`, so print the end of that file to the job log. $1 the file, $2 the label.
show_why() {
  if [ -n "${1:-}" ] && [ -f "$1" ]; then
    echo "--- $2: not killed, and this is the end of what it said ($1)" >&2
    tail -n 40 "$1" >&2
    echo "--- end" >&2
  fi
  return 0
}

kill_check() {
  # $1 label, $2 exit status, $3 log file with test output, $4 (optional) the row's own output file, for `show_why`
  label="$1"; status="$2"; log="$3"
  failed="$(grep -c '\.\.\. FAILED$' "$log")"
  # A pass run with --nocapture (the engine pass) prints `test x ... ` and the panic text, and only then `FAILED` on a
  # line of its own, so its failures are counted from cargo's own `test result: FAILED. N passed; M failed` line.
  if [ "$failed" -eq 0 ]; then
    failed="$(sed -n 's/^test result: FAILED\. [0-9]* passed; \([0-9]*\) failed.*/\1/p' "$log" | tail -1)"
    failed="${failed:-0}"
  fi
  if [ "$status" -eq 101 ] && [ "$failed" -ge 1 ]; then
    printf '%-34s KILLED   (%s test(s) failed)\n' "$label" "$failed" >>"$SUMMARY"
    grep -E '\.\.\. FAILED$|^test .* \.\.\. $' "$log" | sed 's/^/    /' >>"$SUMMARY"
  else
    printf '%-34s SURVIVED or did not run (exit=%s, failed tests=%s)\n' "$label" "$status" "$failed" >>"$SUMMARY"
    show_why "${4:-}" "$label"
    SURVIVED=1
  fi
}

# A kill that also names its cause. `kill_check` counts any failure with exit 101 as a kill, and a run that never got as far as
# the thing under test fails too: without Docker every `pg_adapter` test panics in its setup, which reads as KILLED. $4 is text
# only the assertion the mutant is aimed at prints, so a log without it is not a kill.
kill_check_msg() {
  # $1 label, $2 exit status, $3 log file with test output, $4 text the failure must contain, $5 (optional) the row's own output file
  if grep -qF -- "$4" "$3"; then
    kill_check "$1" "$2" "$3" "${5:-}"
  else
    printf '%-34s SURVIVED or did not run (the log never says: %s)\n' "$1" "$4" >>"$SUMMARY"
    show_why "${5:-}" "$1"
    SURVIVED=1
  fi
}

# A row of the contract suite: `scripts/contract.sh` runs the pass on nextest and keeps its JUnit report beside the pass's log
# (`<log>.junit.xml` for `<log>.test.log`). Killed when nextest exited 100 (a test failed; 101 is a build that failed, and 4 or 5
# a pass that ran nothing) and the report counts at least one test that did not pass.
junit_attr() { sed -n "s/.*<testsuites[^>]* $2=\"\([0-9][0-9]*\)\".*/\1/p" "$1" 2>/dev/null | head -n 1; }
kill_check_suite() {
  # $1 label, $2 exit status, $3 the pass's log, $4 (optional) the row's own output file, for `show_why`
  label="$1"; status="$2"; log="$3"
  junit="${log%.test.log}.junit.xml"
  failures="$(junit_attr "$junit" failures)"
  errors="$(junit_attr "$junit" errors)"
  failed=$(( ${failures:-0} + ${errors:-0} ))
  if [ "$status" -eq 100 ] && [ "$failed" -ge 1 ]; then
    printf '%-34s KILLED   (%s test(s) failed)\n' "$label" "$failed" >>"$SUMMARY"
    grep -E '^ +FAIL \[' "$log" | sort -u | sed 's/^ */    /' >>"$SUMMARY"
  else
    printf '%-34s SURVIVED or did not run (exit=%s, failed tests=%s)\n' "$label" "$status" "$failed" >>"$SUMMARY"
    show_why "${4:-}" "$label"
    SURVIVED=1
  fi
}

# kill_check_msg for a row of the contract suite.
kill_check_suite_msg() {
  # $1 label, $2 exit status, $3 the pass's log, $4 text the failure must contain, $5 (optional) the row's own output file
  if grep -qF -- "$4" "$3"; then
    kill_check_suite "$1" "$2" "$3" "${5:-}"
  else
    printf '%-34s SURVIVED or did not run (the log never says: %s)\n' "$1" "$4" >>"$SUMMARY"
    show_why "${5:-}" "$1"
    SURVIVED=1
  fi
}

# name:feature:pass. The CSRF switch (Q-0050.8) has a mutant for each direction: ignored when on (the main pass, which runs
# with enforcement on, must fail) and ignored when off (the `csrfoff` pass must fail).
for triple in \
  "ignore-tenant:mutant_ignore_tenant:main" \
  "duplicate-sequence-number:mutant_dup_sequence:main" \
  "no-activity-dedupe:mutant_no_activity_dedupe:main" \
  "ignore-revoked:mutant_ignore_revoked:main" \
  "skip-csrf:mutant_skip_csrf:main" \
  "csrf-switch-ignored-when-on:mutant_csrf_never:main" \
  "csrf-switch-ignored-when-off:mutant_csrf_always:csrfoff" \
  "unverified-email:mutant_unverified_email:main" \
  "operator-reset-skips-strength:mutant_operator_skips_strength:main" \
  "operator-reset-keeps-sessions:mutant_operator_keeps_sessions:main" \
  "operator-reset-hits-every-user:mutant_set_password_every_user:main" \
  "operator-door-takes-admin-token:mutant_operator_takes_admin_token:main" \
  "change-password-skips-old:mutant_change_password_skips_old:main" \
  "change-password-skips-strength:mutant_change_password_skips_strength:main" \
  "change-password-keeps-sessions:mutant_change_password_keeps_sessions:main" \
  "change-password-allows-api-key:mutant_change_password_allows_api_key:main" \
  "oidc-state-reusable:mutant_oidc_state_reusable:main" \
  "oidc-refusal-without-error-code:mutant_oidc_failure_redirect_no_code:main" \
  "reset-token-reusable:mutant_reset_token_reusable:main" \
  "reset-ignores-expiry:mutant_reset_ignores_expiry:main" \
  "reset-uid-unchecked:mutant_reset_uid_unchecked:main" \
  "reset-takes-token-first:mutant_reset_takes_token_first:main" \
  "reset-skips-strength:mutant_reset_skips_strength:main" \
  "reset-keeps-sessions:mutant_reset_keeps_sessions:main" \
  "reset-refusal-without-error-code:mutant_reset_redirect_without_code:main" \
  "mail-reset-ignores-cooldown:mutant_reset_ignores_cooldown:mail" \
  "mail-reset-token-reusable:mutant_reset_token_reusable:mail" \
  "magic-refusal-without-error-code:mutant_magic_redirect_without_code:main" \
  "magic-form-without-csrf-token:mutant_magic_csrf_unchecked:main" \
  "mail-code-cap-ignored:mutant_code_cap_ignored:mail" \
  "mail-code-attempts-unbounded:mutant_code_attempts_unbounded:mail" \
  "mail-code-deactivated-signs-in:mutant_code_deactivated_signs_in:mail" \
  "mail-code-mails-unknown:mutant_code_mails_unknown:mail"; do
  name="${triple%%:*}"
  rest="${triple#*:}"
  feature="${rest%%:*}"
  pass="${rest##*:}"
  THRUST_SERVER_FEATURES="$feature" THRUST_LOGDIR="$OUT/$name" "$ROOT/scripts/contract.sh" native-memory "$pass" >"$OUT/$name.out" 2>&1
  status=$?
  kill_check_suite "native-memory $pass + $name" "$status" "$OUT/$name/native-memory-$pass.test.log" "$OUT/$name.out"
done

# The engine pass (spike 0002, `contract.sh <target> engine`): the 50-writer, 10k-create test against the duplicate-sequence
# mutant of the memory adapter, and the search tests against its case-sensitive-search mutant. Each must fail it: that is the
# control the issue names for the 50-writer test. (The SQLite adapter's own rows are archived with it, on the tag
# `spike/0002-worker-do`.)
for spec in \
  "native-memory:duplicate-sequence-number:mutant_dup_sequence" \
  "native-memory:case-sensitive-search:mutant_search_case_sensitive"; do
  target="${spec%%:*}"
  rest="${spec#*:}"
  name="${rest%%:*}"
  feature="${rest##*:}"
  THRUST_SERVER_FEATURES="$feature" THRUST_LOGDIR="$OUT/engine-$target-$name" "$ROOT/scripts/contract.sh" "$target" engine >"$OUT/engine-$target-$name.out" 2>&1
  status=$?
  kill_check_suite "$target engine + $name" "$status" "$OUT/engine-$target-$name/$target-engine.test.log" "$OUT/engine-$target-$name.out"
done
# The vacuity guard of the CSRF switch (Q-0050.8): a pass that is started against a server with the OTHER enforcement
# setting must be refused before a test runs (exit 5, and the message says csrf_enforce). Not a build mutant: the server
# is the real one, started with THRUST_SERVER_CSRF_ENFORCE, the knob only this script sets.
guard_check() {
  # $1 label, $2 exit status, $3 output file
  label="$1"; status="$2"; out="$3"
  if [ "$status" -eq 5 ] && grep -q 'VACUITY GUARD: csrf_enforce' "$out"; then
    printf '%-34s REFUSED  (exit 5, no test ran)\n' "$label" >>"$SUMMARY"
  else
    printf '%-34s NOT REFUSED (exit=%s)\n' "$label" "$status" >>"$SUMMARY" && show_why "$out" "$label"
    SURVIVED=1
  fi
}
THRUST_SERVER_CSRF_ENFORCE=off THRUST_LOGDIR="$OUT/guard-main-vs-off" "$ROOT/scripts/contract.sh" native-memory main >"$OUT/guard-main-vs-off.out" 2>&1
status=$?
guard_check "guard: main pass, server off" "$status" "$OUT/guard-main-vs-off.out"
THRUST_SERVER_CSRF_ENFORCE=on THRUST_LOGDIR="$OUT/guard-off-vs-on" "$ROOT/scripts/contract.sh" native-memory csrfoff >"$OUT/guard-off-vs-on.out" 2>&1
status=$?
guard_check "guard: csrfoff pass, server on" "$status" "$OUT/guard-off-vs-on.out"

# The vacuity guard of mail (E2): `scripts/contract.sh` asks the app's own question, `/api/instances/`, and refuses to run (exit 5, no test ran) a pass
# whose target gives the wrong answer: `true` for the `mail` pass, `false` for every other. Two builds are refused by it, and each is a row here
# rather than in the kill loop above, because the verdict is the refusal and not a failed test (`kill_check_suite` wants exit 100):
#   - a server whose composition root reads the relay and does not hand it to the app tells the app mail is off, and the `mail` pass must not pass
#     for want of a test that looks;
#   - a server that says mail is on though nothing can send it (`mutant_smtp_flag_always_on`, which the main pass used to kill with the dark test
#     and still does in `crates/api/compat-plane/tests/reset.rs` and the recorded S01) is refused by the main pass before its first test.
refusal_check() {
  # $1 label, $2 feature, $3 pass, $4 the guard's message
  label="$1"; feature="$2"; pass="$3"; msg="$4"
  name="guard-$pass-$feature"
  THRUST_SERVER_FEATURES="$feature" THRUST_LOGDIR="$OUT/$name" "$ROOT/scripts/contract.sh" native-memory "$pass" >"$OUT/$name.out" 2>&1
  status=$?
  if [ "$status" -eq 5 ] && grep -q "$msg" "$OUT/$name.out"; then
    printf '%-34s REFUSED  (exit 5, no test ran)\n' "$label" >>"$SUMMARY"
  else
    printf '%-34s NOT REFUSED (exit=%s)\n' "$label" "$status" >>"$SUMMARY" && show_why "$OUT/$name.out" "$label"
    SURVIVED=1
  fi
}
refusal_check "guard: mail pass, relay not wired" mutant_smtp_unwired mail 'VACUITY GUARD: is_smtp_configured is not true'
refusal_check "guard: main pass, mail flag on" mutant_smtp_flag_always_on main 'VACUITY GUARD: is_smtp_configured is not false'

# Postgres adapter mutant: recycle a connection that is still inside an abandoned transaction.
"$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-adapters-selfhosted \
  --features mutant_discard_noop --test pg_adapter -- an_abandoned_transaction model_postgres_adapter_matches >"$OUT/pg-discard.out" 2>&1
status=$?
kill_check "postgres adapter + discard-noop" "$status" "$OUT/pg-discard.out"

# Postgres adapter mutant (issue #39): the round-trip counter counts nothing. The counting tests must notice, and the failure
# must be a wrong count: "statements, the model says" is in the generated test's message and nowhere else, so a pool that never
# started (no Docker) is not a kill.
"$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-adapters-selfhosted \
  --features mutant_count_nothing --test pg_adapter -- counting >"$OUT/pg-count-nothing.out" 2>&1
status=$?
kill_check_msg "postgres adapter + count-nothing" "$status" "$OUT/pg-count-nothing.out" "statements, the model says"

# The same counter's mutants at the server, on native-pg (issue #39): the budget test, alone (the `budget` pass), must fail on
# each, and for the reason the mutant is for. `kill_check` alone would also count a run that never reached the test (Postgres
# did not start, the build failed), so the log has to say what the mutant breaks: a page that sends a statement per row is
# "over budget", a counter that counts nothing is "below floor".
for triple in \
  "extra-statement-per-row:mutant_extra_stmt_per_row:over budget" \
  "count-nothing:mutant_count_nothing:below floor"; do
  name="${triple%%:*}"
  rest="${triple#*:}"
  feature="${rest%%:*}"
  needle="${rest#*:}"
  THRUST_SERVER_FEATURES="$feature" THRUST_LOGDIR="$OUT/$name" "$ROOT/scripts/contract.sh" native-pg budget >"$OUT/$name.out" 2>&1
  status=$?
  kill_check_suite_msg "native-pg budget + $name" "$status" "$OUT/$name/native-pg-budget.test.log" "$needle" "$OUT/$name.out"
done

# The SMTP adapter's mutants (E2 of the reset by email, D-041): each is a deliberately broken build of thrust-adapters-selfhosted, run against the
# suite aimed at it. `kill_check_msg`, so the log has to say what the mutant breaks and a run that never got that far (no Docker for `smtp_sink`,
# a build that failed) is not a kill. binary:needle: a refusal read as an outage and the reverse (all 600 reply codes at the 5 steps); the sender's
# address taken from the recipient; an id returned that the message does not carry; a password allowed over a connection in the clear; STARTTLS made
# a preference, so a relay that cannot do TLS is used in the clear; a relay's certificate trusted whoever signed it (the three TLS tests, against Mailpit).
for quad in \
  "swap-refusal-and-outage:mutant_smtp_swap_refusal_and_outage:smtp_wire:were read the wrong way" \
  "from-is-recipient:mutant_smtp_from_is_recipient:smtp_wire:the envelope sender" \
  "wrong-message-id:mutant_smtp_wrong_message_id:smtp_wire:holds a message under the returned id" \
  "password-in-clear:mutant_smtp_password_in_clear:smtp_config:combinations disagree with the reference" \
  "starttls-opportunistic:mutant_smtp_starttls_opportunistic:smtp_wire:a message went to a relay that cannot do TLS" \
  "trust-any-certificate:mutant_smtp_trust_any_certificate:smtp_sink:UNTRUSTED CERTIFICATE ACCEPTED"; do
  name="${quad%%:*}"
  rest="${quad#*:}"
  feature="${rest%%:*}"
  rest="${rest#*:}"
  bin="${rest%%:*}"
  needle="${rest#*:}"
  "$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-adapters-selfhosted --features "$feature" --test "$bin" \
    >"$OUT/smtp-$name.out" 2>&1
  status=$?
  kill_check_msg "smtp adapter + $name" "$status" "$OUT/smtp-$name.out" "$needle"
done

# The Cloudflare email adapter's mutants (E3 of the reset by email, D-041): each is a deliberately broken build of thrust-adapters-cloudflare, run
# against its own test file (`tests/email.rs`, which runs natively: only the binding glue, `CfEmail`, needs workerd). Same shape as the SMTP rows: the
# log has to say what the mutant breaks. A refusal read as an outage and the reverse (the 21 codes of Cloudflare's own table); an unknown code read as a
# refusal; the sender of the mail taken from the recipient; a recipient that is not one plain address handed to the binding. (The Worker's wiring has
# one more, `mutant_worker_email_unwired`, which needs the wasm build: it is a hand run of `scripts/contract.sh worker-memory mail`, G-39.)
for quad in \
  "swap-refusal-and-outage:mutant_cf_email_swap_refusal_and_outage:email:documented codes were read the wrong way" \
  "unknown-code-is-refused:mutant_cf_email_unknown_code_is_refused:email:an unknown code was refused" \
  "from-is-recipient:mutant_cf_email_from_is_recipient:email:the sender of the mail is not the configured one" \
  "recipient-unchecked:mutant_cf_email_recipient_unchecked:email:a recipient that is not one address was not refused"; do
  name="${quad%%:*}"
  rest="${quad#*:}"
  feature="${rest%%:*}"
  rest="${rest#*:}"
  bin="${rest%%:*}"
  needle="${rest#*:}"
  "$ROOT/scripts/rs" cargo test --manifest-path "$ROOT/Cargo.toml" -p thrust-adapters-cloudflare --features "$feature" --test "$bin" \
    >"$OUT/cf-email-$name.out" 2>&1
  status=$?
  kill_check_msg "cloudflare email adapter + $name" "$status" "$OUT/cf-email-$name.out" "$needle"
done

cat "$SUMMARY"
exit "$SURVIVED"
