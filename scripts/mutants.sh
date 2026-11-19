#!/bin/sh
# Mutation controls: prove the contract suite can FAIL, i.e. that its "X is rejected" assertions
# observe the thing they claim to test. Each mutant is a deliberately broken build; the suite must
# fail on it. Every row runs on nextest (the contract suite's through scripts/contract.sh, an adapter's own tests through
# `adapter_row` below) and is killed when nextest exits 100 and the row's JUnit report counts a test that did not pass. A mutant
# that survives means some assertion is vacuous.
#
#   scripts/mutants.sh
#   MUTANTS_SHARDS=4 MUTANTS_SHARD=1 scripts/mutants.sh   only the rows at positions 2, 6, 10, ... (what one runner of the matrix runs)
#   MUTANTS_LIST=1 scripts/mutants.sh                     list every row and the shard it falls on, run nothing
# scripts/lib/mutants-shard.sh says how rows are shared out, and why a row starts with `want`.
set -u
# A counterexample found against a deliberately broken build is not a regression of the real code: do not let the
# generated tests (crates/testing/suites/api/tests/model) save it into the source tree.
export PROPTEST_DISABLE_FAILURE_PERSISTENCE=1
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="$ROOT/target/mutants"
mkdir -p "$OUT"
SUMMARY="$OUT/summary.txt"
: >"$SUMMARY"
# What a row's run says (KILLED, SURVIVED, UNCLEAR, DID NOT RUN): scripts/lib/mutants-verdict.sh, which says what each means.
# shellcheck source=scripts/lib/mutants-verdict.sh
. "$ROOT/scripts/lib/mutants-verdict.sh"
# Which rows this run is responsible for (the job is a matrix of shards, each on a runner with a `target/` of its own).
# shellcheck source=scripts/lib/mutants-shard.sh
. "$ROOT/scripts/lib/mutants-shard.sh"
shard_init || exit 2
# Any row that is not KILLED (or REFUSED, for the guard rows) fails the job: a row with no verdict has not shown the suite can fail.
NOT_KILLED=0
KILLED_N=0; SURVIVED_N=0; UNCLEAR_N=0; NOT_RUN_N=0; REFUSED_N=0; NOT_REFUSED_N=0

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

# record <label> <verdict line> <log> [the row's own output file]: one summary line, then for a kill the tests that failed (they are
# the assertions that guard the rule), and for anything else the end of what the row said.
record() {
  r_label="$1"; r_verdict="${2%%|*}"; r_detail="${2#*|}"; r_log="$3"; r_out="${4:-$3}"
  printf '%-34s %-11s (%s)\n' "$r_label" "$r_verdict" "$r_detail" >>"$SUMMARY"
  case "$r_verdict" in
    KILLED)
      KILLED_N=$((KILLED_N + 1))
      grep -E '^ +FAIL \[' "$r_log" | sort -u | sed 's/^ */    /' >>"$SUMMARY"
      return 0 ;;
    SURVIVED) SURVIVED_N=$((SURVIVED_N + 1)) ;;
    UNCLEAR) UNCLEAR_N=$((UNCLEAR_N + 1)) ;;
    *) NOT_RUN_N=$((NOT_RUN_N + 1)) ;;
  esac
  show_why "$r_out" "$r_label"
  NOT_KILLED=1
}

# A row run on nextest: a pass of the contract suite (`scripts/contract.sh`), or an adapter's own tests (`adapter_row` below). Either
# keeps its JUnit report beside its log (`<log>.junit.xml` for `<log>.test.log`). Killed when nextest exited 100 and the report counts
# at least one test that did not pass. $1 label, $2 exit status, $3 the row's log, $4 (optional) the row's own output file, for `show_why`.
kill_check_suite() { record "$1" "$(verdict_suite "$2" "$3")" "$3" "${4:-}"; }

# A kill that also names its cause: without Docker every `pg_adapter` test panics in its setup, which would read as a kill. $4 is text only
# the assertion the mutant is aimed at prints, so failures without it are UNCLEAR, not KILLED. $5 (optional) the row's own output file.
kill_check_suite_msg() { record "$1" "$(verdict_suite "$2" "$3" "$4")" "$3" "${5:-}"; }

# adapter_row NAME PACKAGE FEATURE FILTERSET [NEXTEST ARGUMENT...]: an adapter's own tests, built with the mutant's feature, on nextest with the
# `adapters` profile (.config/nextest.toml: a JUnit report and no timeout kill). `--ignore-default-filter` keeps those and drops the profile's
# selection (the Docker suites), so a hermetic suite can be a row too. Its log is $OUT/NAME.test.log and its report
# $OUT/NAME.junit.xml (a stale one is removed first, so a run that wrote none is read as none). Sets `status`: nextest's own, except
# that its 4 (no test ran: the filterset selects nothing) becomes 5, as scripts/contract.sh makes it, which the verdict reads as the
# vacuity guard and not as a target that never came up. Any further arguments go to nextest as they are (see the discard-noop row).
adapter_row() {
  ar_name="$1"; ar_package="$2"; ar_feature="$3"; ar_set="$4"
  shift 4
  ar_junit="${CARGO_TARGET_DIR:-$ROOT/target}/nextest/adapters/junit.xml"
  rm -f "$ar_junit" "$OUT/$ar_name.junit.xml"
  "$ROOT/scripts/rs" cargo nextest run --manifest-path "$ROOT/Cargo.toml" -p "$ar_package" --features "$ar_feature" \
    --profile adapters --ignore-default-filter -E "$ar_set" "$@" >"$OUT/$ar_name.test.log" 2>&1
  status=$?
  if [ -f "$ar_junit" ]; then mv "$ar_junit" "$OUT/$ar_name.junit.xml"; fi
  if [ "$status" -eq 4 ]; then status=5; fi
}

# name:feature:pass. The CSRF switch (Q-0050.8) has a mutant for each direction: ignored when on (the main pass, which runs
# with enforcement on, must fail) and ignored when off (the `csrfoff` pass must fail).
for triple in \
  "ignore-tenant:mutant_ignore_tenant:main" \
  "duplicate-sequence-number:mutant_dup_sequence:main" \
  "no-activity-dedupe:mutant_no_activity_dedupe:main" \
  "invalid-is-transient:mutant_invalid_is_transient:main" \
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
  want "native-memory $pass + $name" || continue
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
  want "$target engine + $name" || continue
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
    REFUSED_N=$((REFUSED_N + 1))
  else
    printf '%-34s NOT REFUSED (exit=%s)\n' "$label" "$status" >>"$SUMMARY" && show_why "$out" "$label"
    NOT_REFUSED_N=$((NOT_REFUSED_N + 1))
    NOT_KILLED=1
  fi
}
if want "guard: main pass, server off"; then
  THRUST_SERVER_CSRF_ENFORCE=off THRUST_LOGDIR="$OUT/guard-main-vs-off" "$ROOT/scripts/contract.sh" native-memory main >"$OUT/guard-main-vs-off.out" 2>&1
  status=$?
  guard_check "guard: main pass, server off" "$status" "$OUT/guard-main-vs-off.out"
fi
if want "guard: csrfoff pass, server on"; then
  THRUST_SERVER_CSRF_ENFORCE=on THRUST_LOGDIR="$OUT/guard-off-vs-on" "$ROOT/scripts/contract.sh" native-memory csrfoff >"$OUT/guard-off-vs-on.out" 2>&1
  status=$?
  guard_check "guard: csrfoff pass, server on" "$status" "$OUT/guard-off-vs-on.out"
fi

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
  want "$label" || return 0
  name="guard-$pass-$feature"
  THRUST_SERVER_FEATURES="$feature" THRUST_LOGDIR="$OUT/$name" "$ROOT/scripts/contract.sh" native-memory "$pass" >"$OUT/$name.out" 2>&1
  status=$?
  if [ "$status" -eq 5 ] && grep -q "$msg" "$OUT/$name.out"; then
    printf '%-34s REFUSED  (exit 5, no test ran)\n' "$label" >>"$SUMMARY"
    REFUSED_N=$((REFUSED_N + 1))
  else
    printf '%-34s NOT REFUSED (exit=%s)\n' "$label" "$status" >>"$SUMMARY" && show_why "$OUT/$name.out" "$label"
    NOT_REFUSED_N=$((NOT_REFUSED_N + 1))
    NOT_KILLED=1
  fi
}
refusal_check "guard: mail pass, relay not wired" mutant_smtp_unwired mail 'VACUITY GUARD: is_smtp_configured is not true'
refusal_check "guard: main pass, mail flag on" mutant_smtp_flag_always_on main 'VACUITY GUARD: is_smtp_configured is not false'

# Postgres adapter mutant: recycle a connection that is still inside an abandoned transaction. The two `an_abandoned_transaction` tests fail on it in
# about 3 s; the model test fails on it too, but takes either about 5 s or about 125 s (it waits on the connection the mutant poisoned), at random.
# One failure is the kill, so the first one ends the row: `--max-fail=1:immediate` terminates the tests still running (plain `--fail-fast` would
# wait for the model test). The model test stays in the filterset as a second witness: it ends the row itself, as before, whenever the other two do
# not fail. A terminated test counts as failed in nextest's JUnit report, so the verdict's "3 test(s) failed" is one real failure and two
# terminated, and the Postgres container of a terminated test is left running (the runner is discarded with the job; locally, remove the containers
# labelled org.testcontainers.managed-by=testcontainers).
if want "postgres adapter + discard-noop"; then
  adapter_row pg-discard thrust-adapters-selfhosted mutant_discard_noop \
    'binary(pg_adapter) & test(/an_abandoned_transaction|model_postgres_adapter_matches/)' --max-fail=1:immediate
  kill_check_suite "postgres adapter + discard-noop" "$status" "$OUT/pg-discard.test.log"
fi

# Postgres adapter mutant (issue #39): the round-trip counter counts nothing. The counting tests must notice, and the failure
# must be a wrong count: "statements, the model says" is in the generated test's message and nowhere else, so a pool that never
# started (no Docker) is not a kill.
if want "postgres adapter + count-nothing"; then
  adapter_row pg-count-nothing thrust-adapters-selfhosted mutant_count_nothing 'binary(pg_adapter) & test(/counting/)'
  kill_check_suite_msg "postgres adapter + count-nothing" "$status" "$OUT/pg-count-nothing.test.log" "statements, the model says"
fi

# The same counter's mutants at the server, on native-pg (issue #39): the budget test, alone (the `budget` pass), must fail on
# each, and for the reason the mutant is for. `kill_check_suite` alone would also count a run that never reached the test (Postgres
# did not start, the build failed), so the log has to say what the mutant breaks: a page that sends a statement per row is
# "over budget", a counter that counts nothing is "below floor".
for triple in \
  "extra-statement-per-row:mutant_extra_stmt_per_row:over budget" \
  "count-nothing:mutant_count_nothing:below floor"; do
  name="${triple%%:*}"
  rest="${triple#*:}"
  feature="${rest%%:*}"
  needle="${rest#*:}"
  want "native-pg budget + $name" || continue
  THRUST_SERVER_FEATURES="$feature" THRUST_LOGDIR="$OUT/$name" "$ROOT/scripts/contract.sh" native-pg budget >"$OUT/$name.out" 2>&1
  status=$?
  kill_check_suite_msg "native-pg budget + $name" "$status" "$OUT/$name/native-pg-budget.test.log" "$needle" "$OUT/$name.out"
done

# The SMTP adapter's mutants (E2 of the reset by email, D-041): each is a deliberately broken build of thrust-adapters-selfhosted, run against the
# suite aimed at it. `kill_check_suite_msg`, so the log has to say what the mutant breaks and a run that never got that far (no Docker for `smtp_sink`,
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
  want "smtp adapter + $name" || continue
  adapter_row "smtp-$name" thrust-adapters-selfhosted "$feature" "binary($bin)"
  kill_check_suite_msg "smtp adapter + $name" "$status" "$OUT/smtp-$name.test.log" "$needle"
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
  want "cloudflare email adapter + $name" || continue
  adapter_row "cf-email-$name" thrust-adapters-cloudflare "$feature" "binary($bin)"
  kill_check_suite_msg "cloudflare email adapter + $name" "$status" "$OUT/cf-email-$name.test.log" "$needle"
done

# A list run ends here. Any other run must have judged exactly the rows it chose: a row outside `want` ran everywhere, a shard with no rows ran
# nothing (scripts/lib/mutants-shard.sh).
shard_finish $((KILLED_N + SURVIVED_N + UNCLEAR_N + NOT_RUN_N + REFUSED_N + NOT_REFUSED_N)) || NOT_KILLED=1
printf 'shard %s of %s: %s of the %s rows\n' "$SHARD_I" "$SHARD_N" "$ROWS_SELECTED" "$ROW_N" >>"$SUMMARY"
printf 'killed=%s survived=%s unclear=%s did-not-run=%s refused=%s not-refused=%s\n' \
  "$KILLED_N" "$SURVIVED_N" "$UNCLEAR_N" "$NOT_RUN_N" "$REFUSED_N" "$NOT_REFUSED_N" >>"$SUMMARY"
cat "$SUMMARY"
exit "$NOT_KILLED"
