# Mutation

Status: **live** (planted mutants, `scripts/mutants.sh`; cargo-mutants on `thrust-core`) · Gate: `scripts/mutants.sh` in CI

## Catches

X16 (P): tests that pass without checking what they claim. Mutation testing changes the code and asks whether any
test notices. A surviving mutant is a missing or vacuous assertion.

## Use when / Not when

Use at [step 7 of the loop](../tdd-loop.md#7-mutation) for every change, and when reviewing whether a module's tests
are strong enough. Two mechanisms, for two situations:

| Mechanism | When | How |
|---|---|---|
| **Planted mutants** | the code is only tested through the contract suite or `pg_adapter` (most crates today) | a cargo feature `mutant_<what>` compiles a deliberately broken variant; `scripts/mutants.sh` requires the suite to fail on it |
| **cargo-mutants** | the crate has tests of its own ([Rule 5](../rules.md)) | generates mutants automatically; fix each MISSED one through the public API |

Not a replacement for line coverage reports, and never a reason to write a test aimed at one mutant: strengthen the
invariant the mutant broke.

## Name and location

Planted mutants: features in `crates/adapters/memory/Cargo.toml` (`mutant_ignore_tenant`,
`mutant_dup_sequence`, `mutant_no_activity_dedupe`, `mutant_ignore_revoked`, which makes a revoked session and a revoked
API key resolve, and `mutant_set_password_every_user`, which writes a new password hash to every user), `crates/api/core/Cargo.toml`
(`mutant_operator_takes_admin_token`, which lets the test routes' token open the operator's routes, killed by the main pass, and
`mutant_operator_ignores_flag`, which ignores their switch, which no contract target can show because each is started with the
switch on: `crates/api/compat-plane/tests/operator.rs` kills it, run with `--features thrust-api/mutant_operator_ignores_flag`),
`crates/core/usecases/Cargo.toml` (`mutant_skip_csrf`, `mutant_unverified_email`, `mutant_operator_skips_strength` and
`mutant_operator_keeps_sessions`, which break the operator's password reset, and the two that break the CSRF enforcement switch,
`mutant_csrf_never`, ignored when it is on, killed by the main pass, and `mutant_csrf_always`, ignored when it is off, killed by the
`csrfoff` pass; and `mutant_change_password_skips_old`, `mutant_change_password_skips_strength` and
`mutant_change_password_keeps_sessions`, which break a person's change of their own password, killed by the main pass and by
`crates/core/usecases/tests/change_password.rs`; and `mutant_oidc_state_reusable`, which keeps an OIDC login's state after it is used, killed by the
main pass and by `crates/api/compat-plane/tests/oidc.rs`), `crates/api/compat-plane/Cargo.toml` (`mutant_change_password_allows_api_key`,
which lets an API key change the password, killed by the main pass; `mutant_oidc_failure_redirect_no_code`, which drops the `error_code` of a refused SSO
login's redirect, killed by the main pass and by `tests/oidc.rs`; and `mutant_oidc_flag_always_on`, which says the SSO button is on although no provider is
configured, which no contract target can show because each is started with a provider: `crates/api/compat-plane/tests/oidc.rs` kills it, run with
`--features mutant_oidc_flag_always_on`, and it has no `scripts/mutants.sh` row; and `mutant_smtp_flag_always_on` and `mutant_reset_redirect_without_code`, which say mail is on
although nothing can send it and drop the `error_code` of a refused reset's redirect: the second is killed by the main pass and by `tests/reset.rs`; the first is
killed by `tests/reset.rs` and the recorded S01, and since E2 the main pass REFUSES it before a test runs (the pass's vacuity guard asks `/api/instances/` and
wants `false`: exit 5, so its `scripts/mutants.sh` row is a refusal row and not a kill row)), the password reset by email's
`crates/core/usecases/Cargo.toml` mutants (`mutant_reset_token_reusable`, `_ignores_expiry`, `_uid_unchecked`, `_takes_token_first`, `_skips_strength` and `_keeps_sessions`,
each killed by the main pass, with a `scripts/mutants.sh` row, and by `crates/core/usecases/tests/password_reset.rs`; and `_ignores_cooldown`, which the main pass cannot
show because the test seam makes a token that the cooldown does not see, so only that file killed it until the `mail` pass, whose second test asks twice and counts the
messages at a mail server: it has a row on that pass, as `_token_reusable` has too), the sign-in with a code's (D-042: `thrust-core`'s `mutant_code_cap_ignored`, `_attempts_unbounded`, `_deactivated_signs_in` and `_mails_unknown`, each with a row on the `mail` pass, and `thrust-compat-plane`'s `mutant_magic_redirect_without_code` and `mutant_magic_csrf_unchecked`, each with a row on the main pass; the core's are killed by the generated model of `crates/core/usecases/tests/login_code.rs` and by the contract test aimed at each, the pair by `crates/api/compat-plane/tests/magic.rs`; the six rows were run by hand on 2026-11-16, each exit 101 with one contract test failed) and
`crates/adapters/selfhosted` (`mutant_discard_noop`; and the SMTP adapter's six, each with a row against the suite aimed at it:
`mutant_smtp_swap_refusal_and_outage`, which reads a relay's 5xx as an outage and its 4xx as a refusal, killed by `tests/smtp_wire` at every one of its 600 reply-code cases;
`mutant_smtp_from_is_recipient` and `mutant_smtp_wrong_message_id`, killed by the same file's generated `prop_a_message_arrives_as_it_was_given`;
`mutant_smtp_password_in_clear`, which lets a password go over a connection with no TLS, killed by `tests/smtp_config`'s 54,000-combination matrix;
`mutant_smtp_starttls_opportunistic`, which makes STARTTLS a preference so that a relay that cannot upgrade is used in the clear, killed by `tests/smtp_wire`'s
`a_relay_that_cannot_do_tls_is_not_used_when_tls_is_asked_for`; and
`mutant_smtp_trust_any_certificate`, which trusts a relay's certificate whoever signed it and whatever it names, killed by the three TLS tests of `tests/smtp_sink` against
Mailpit; and `thrust-server`'s `mutant_smtp_unwired`, which reads the relay and does not hand it to the app, which the `mail` pass refuses to run against: a vacuity-guard row, exit 5), and
`crates/adapters/cloudflare` (the email adapter's four, each with a row against `tests/email.rs`, which runs natively:
`mutant_cf_email_swap_refusal_and_outage`, which reads the binding's refusal as an outage and the reverse, killed by the test that pins all 21 codes of Cloudflare's own table
and by the one that reads a refusal that has no code; `mutant_cf_email_unknown_code_is_refused`, which refuses a code nobody has met instead of sending the message again later,
killed by `prop_a_code_the_page_does_not_list_is_an_outage`; `mutant_cf_email_from_is_recipient`, which sends as the person it is writing to, killed by
`prop_a_good_message_is_handed_over_as_it_is`; and `mutant_cf_email_recipient_unchecked`, which hands the binding a recipient that is not one plain address, killed by
`prop_a_message_to_something_that_is_not_one_address_is_refused`; and `thrust-worker`'s `mutant_worker_email_unwired`, which reads the sender and the binding and does
not hand the mail service to the app, which the Worker cells' `mail` pass refuses to run against (exit 5). That one has **no `scripts/mutants.sh` row**: it needs the wasm
build, which that script and its CI job do not make, so it was run by hand on 2026-11-15 (`worker-memory mail`, build command edited to pass
`--features mutant_worker_email_unwired`: `VACUITY GUARD: is_smtp_configured is not true`, no test run), as was `thrust-adapters-cloudflare/mutant_cf_email_from_is_recipient`
through the Worker, which the simulator's `allowed_sender_addresses` refuses), and `crates/core/usecases`'s `mutant_invalid_is_transient` (issue #53),
which treats an event the consumer rejects as invalid like a transient failure, so that it is delivered again with the backoff and not set aside at once: killed
by the contract suite's `permanent_consumer_error_is_dead_lettered_without_retry`, which waits 15 s for a row with a count of one, and by nothing else
(its `scripts/mutants.sh` row runs on native-memory; on 2026-11-18 it was also run by hand on `worker-cf main`, 73 passed and that test failed, and so was
restoring the acknowledge-and-drop of a message that is not an event in the Worker's consumer, `worker-cf main` and `bindings`, where the two new tests failed and
nothing else did),
gated with `#[cfg(feature = "mutant_…")]` at the line they break, passed through by `thrust-server`. Each is listed
in `scripts/mutants.sh`. cargo-mutants: configured by `.cargo/mutants.toml` and the `mutants` nextest profile;
`mutants.out*` is never committed.

Spike 0002 adds `crates/adapters/memory` `mutant_search_case_sensitive` (search without folding case; it is in the adapter's workspace search, which `/api/v1` and so the engine pass go through; the slice's per-project search, `search_work_items`, was a second place for it until step 5c-3 of #58 deleted it) and runs the engine pass in `scripts/mutants.sh` against it and against `mutant_dup_sequence` of the memory adapter. The SQLite adapter's own planted bugs (`mutant_no_gate`, `mutant_gate_leak`, `mutant_non_atomic_commit`, `mutant_broken_migration`) and their rows were archived with that adapter on the tag `spike/0002-worker-do`.

## Run

```sh
scripts/mutants.sh                                   # needs Docker [verified 2026-10-24: 9 KILLED, 2 guard refusals, 0 SocketNotFoundError]
scripts/rs cargo mutants -f <path/to/file.rs>        # [derived]; exit 4 if the crate has no tests of its own
```

## Required controls

The mechanism is itself a control. Check the baseline first: cargo-mutants exit 4 means the unmutated tree failed or
selected nothing, which is not a mutation result.

## Owning skill

`.agents/skills/rust-test-mutation/SKILL.md` for cargo-mutants. Planted mutants have **no owning skill** (G-2); add
one by copying an existing `mutant_*` feature: declare it in the adapter's `Cargo.toml`, pass it through in
`crates/server/Cargo.toml`, break exactly one invariant under the `cfg`, and add it to the list in
`scripts/mutants.sh`.

## In-repo examples

- Planted: `mutant_ignore_tenant` in `crates/adapters/memory/src/lib.rs`, killed by the contract test
  `cross_tenant_access_is_rejected_and_has_no_effect`, and by 40 of `thrust-core`'s tests, `another_tenants_project_is_not_found` and
  `the_slug_decides_which_workspace_a_project_is_looked_up_in` among them (measured with `cargo nextest run -p thrust-core
  --no-fail-fast --features thrust-adapters-memory/mutant_ignore_tenant`, a dev-dependency feature).
- cargo-mutants: `cargo mutants -f crates/core/usecases/src/lib.rs` (run with `PROPTEST_DISABLE_FAILURE_PERSISTENCE=1`).
  The first run left 14 MISSED, all in code no test touched yet (outbox relay, attachments, the consumer entrypoint);
  `crates/core/usecases/tests/outbox.rs` and `attachments.rs` test that behaviour through the public API and the
  rerun reports 0 missed. `authenticate`'s only mutant is unviable, so its test has a hand-applied control instead.

## Target

cargo-mutants `--in-diff` on every PR and a full run on a schedule; other crates join once they have tests of their own
(G-3). The planted mutants already run in CI (the `mutants` job). Mutation testing of the permission seam is open question Q-0123.1 in ADR-0123.
