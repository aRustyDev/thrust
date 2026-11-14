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

Planted mutants: features in `crates/thrust-adapters-memory/Cargo.toml` (`mutant_ignore_tenant`,
`mutant_dup_sequence`, `mutant_no_activity_dedupe`, `mutant_ignore_revoked`, which makes a revoked session and a revoked
API key resolve, and `mutant_set_password_every_user`, which writes a new password hash to every user), `crates/thrust-api/Cargo.toml`
(`mutant_operator_takes_admin_token`, which lets the test routes' token open the operator's routes, killed by the main pass, and
`mutant_operator_ignores_flag`, which ignores their switch, which no contract target can show because each is started with the
switch on: `crates/thrust-compat-plane/tests/operator.rs` kills it, run with `--features thrust-api/mutant_operator_ignores_flag`),
`crates/thrust-core/Cargo.toml` (`mutant_skip_csrf`, `mutant_unverified_email`, `mutant_operator_skips_strength` and
`mutant_operator_keeps_sessions`, which break the operator's password reset, and the two that break the CSRF enforcement switch,
`mutant_csrf_never`, ignored when it is on, killed by the main pass, and `mutant_csrf_always`, ignored when it is off, killed by the
`csrfoff` pass; and `mutant_change_password_skips_old`, `mutant_change_password_skips_strength` and
`mutant_change_password_keeps_sessions`, which break a person's change of their own password, killed by the main pass and by
`crates/thrust-core/tests/change_password.rs`; and `mutant_oidc_state_reusable`, which keeps an OIDC login's state after it is used, killed by the
main pass and by `crates/thrust-compat-plane/tests/oidc.rs`), `crates/thrust-compat-plane/Cargo.toml` (`mutant_change_password_allows_api_key`,
which lets an API key change the password, killed by the main pass; `mutant_oidc_failure_redirect_no_code`, which drops the `error_code` of a refused SSO
login's redirect, killed by the main pass and by `tests/oidc.rs`; and `mutant_oidc_flag_always_on`, which says the SSO button is on although no provider is
configured, which no contract target can show because each is started with a provider: `crates/thrust-compat-plane/tests/oidc.rs` kills it, run with
`--features mutant_oidc_flag_always_on`, and it has no `scripts/mutants.sh` row; and `mutant_smtp_flag_always_on` and `mutant_reset_redirect_without_code`, which say mail is on
although nothing can send it and drop the `error_code` of a refused reset's redirect, killed by the main pass and by `tests/reset.rs`), the password reset by email's
`crates/thrust-core/Cargo.toml` mutants (`mutant_reset_token_reusable`, `_ignores_expiry`, `_uid_unchecked`, `_takes_token_first`, `_skips_strength` and `_keeps_sessions`,
each killed by the main pass, with a `scripts/mutants.sh` row, and by `crates/thrust-core/tests/password_reset.rs`; and `_ignores_cooldown`, which no contract target can
show because the test seam makes a token that the cooldown does not see, so only that file kills it) and
`crates/thrust-adapters-selfhosted` (`mutant_discard_noop`),
gated with `#[cfg(feature = "mutant_…")]` at the line they break, passed through by `thrust-server`. Each is listed
in `scripts/mutants.sh`. cargo-mutants: configured by `.cargo/mutants.toml` and the `mutants` nextest profile;
`mutants.out*` is never committed.

Spike 0002 adds `crates/thrust-adapters-memory` `mutant_search_case_sensitive` (search without folding case; it is in the adapter's workspace search, which `/api/v1` and so the engine pass go through; the slice's per-project search, `search_work_items`, was a second place for it until step 5c-3 of #58 deleted it) and runs the engine pass in `scripts/mutants.sh` against it and against `mutant_dup_sequence` of the memory adapter. The SQLite adapter's own planted bugs (`mutant_no_gate`, `mutant_gate_leak`, `mutant_non_atomic_commit`, `mutant_broken_migration`) and their rows were archived with that adapter on the tag `spike/0002-worker-do`.

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
`crates/thrust-server/Cargo.toml`, break exactly one invariant under the `cfg`, and add it to the list in
`scripts/mutants.sh`.

## In-repo examples

- Planted: `mutant_ignore_tenant` in `crates/thrust-adapters-memory/src/lib.rs`, killed by the contract test
  `cross_tenant_access_is_rejected_and_has_no_effect`, and by 40 of `thrust-core`'s tests, `another_tenants_project_is_not_found` and
  `the_slug_decides_which_workspace_a_project_is_looked_up_in` among them (measured with `cargo nextest run -p thrust-core
  --no-fail-fast --features thrust-adapters-memory/mutant_ignore_tenant`, a dev-dependency feature).
- cargo-mutants: `cargo mutants -f crates/thrust-core/src/lib.rs` (run with `PROPTEST_DISABLE_FAILURE_PERSISTENCE=1`).
  The first run left 14 MISSED, all in code no test touched yet (outbox relay, attachments, the consumer entrypoint);
  `crates/thrust-core/tests/outbox.rs` and `attachments.rs` test that behaviour through the public API and the
  rerun reports 0 missed. `authenticate`'s only mutant is unviable, so its test has a hand-applied control instead.

## Target

cargo-mutants `--in-diff` on every PR and a full run on a schedule; other crates join once they have tests of their own
(G-3). The planted mutants already run in CI (the `mutants` job). Mutation testing of the permission seam is open question Q-0123.1 in ADR-0123.
