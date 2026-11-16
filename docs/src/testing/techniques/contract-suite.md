# Contract suite

Status: **live** · Gate: the `contract` jobs in CI (one per cell), or `scripts/contract-all.sh` locally

## Catches

X1, X2, X3, X4, X5, X8, X9, X20 (P); X13 (B, panic containment); X16 (its own guards); X21 to X24 (P, identity). This is the backbone of the
repo's evidence: **one** suite of HTTP assertions, run unchanged against every adapter set and both runtimes.

## Use when / Not when

Use for any behaviour a client can observe over HTTP that must be the same on every target: authorization and
tenancy, sequencing, soft delete, outbox delivery, rate limits, attachment authz, error mapping. Not for behaviour
that only one adapter has (`worker_only.rs` holds the Worker-only tests, and adapter internals go in
[real-backend](real-backends.md) tests), and not for pure logic (too slow and too indirect).

## Name and location

`crates/testing/suites/api`. The suite only sees `THRUST_BASE_URL` and `THRUST_ADMIN_TOKEN`; it never learns
which target it runs on. Fixtures are created through `POST /internal/seed`, the same route on every target
(`crates/testing/suites/api/src/lib.rs`), which is mounted only when `thrust-api` is built with `test_routes`.
The suite never calls `POST /internal/reset` (issue #58), which deletes every row: its tests run in parallel against one
target and each owns only the fixture it made. It checks the route's gate (`reset_requires_the_admin_token_and_a_refused_call_deletes_nothing`,
with a fixture made first as the control) and nothing more; the positive path is tested in the adapters and by
`scripts/oracle-v1-reset.sh`.

- `crates/testing/suites/api/tests/contract.rs`: the portable suite. Its work-item tests (sequence and `sort_order`,
  the list, soft delete, tenancy, the external key, concurrent creates, invalid input, the missing and the invalid key)
  run over `/api/v1` and answer as Plane does: the fixture plants an API key for every user (`User.key`, sent as
  `X-Api-Key`), a stranger is a 403, a list is newest first, and a member deletes only what they made. The tests whose
  subject is a slice-only behaviour (the slice's activity feed, attachments, the session routes that no Plane route
  replaces) still run over `/api/_slice`, until the routes that replace them exist (G-26 in [status](../status.md)). The
  tests that only need a work item as a vehicle (the outbox, redelivery, sweeper, cron, panic and blast-radius tests, the
  slice model's creates and deletes, the key's allowance, the bench) make it through `/api/v1`, or through Plane's internal routes where the credential is a
  session (the two session tests, the blast radius); what the queue consumer wrote is still read from the slice's feed.
  The allowance's test, `v1_requests_are_rate_limited_per_key`, holds only what every limiter promises (the first `limit`
  creates with a key pass, a 429 with a `Retry-After` comes within `2 x limit + 1`, a refused create makes nothing, and the
  limit is the key's: another user's key and a second key of the same user are not limited), because the Cloudflare
  Worker's limiter is a binding that counts approximately (D-007, [status](../status.md) D-18). The exact rule is tested
  where a limiter promises it, on `MemoryRateLimiter`. On `worker-cf` the target's limit is the `RL_V1` binding's, which
  `scripts/worker-dev.sh` writes into a throwaway copy of the config from `THRUST_V1_RATE_LIMIT`; `scripts/contract.sh`
  tells the suite the limit it started the target with (`THRUST_EXPECT_V1_LIMIT`), so a target started with another one
  fails the test: that is its control.
- `crates/testing/suites/api/tests/model/mod.rs`: its generated part, compiled into the same binary and run in the
  `main` pass: random operation sequences, events included, against a reference [model](model-based.md), every
  caller × project × route against the access table (252 requests: the activity feed and the three attachment routes),
  and the generator coverage control. Its
  `sweeper_model_outbox_delivers_what_was_swept_and_nothing_more` runs the same model in the `sweeper` pass, with
  the outbox fast path off. It is the model of what no Plane route replaces: attachments, the activity feed and what the
  queue consumer does with an event. Its work items are made and deleted through `/api/v1` with the actors' API keys, as a
  vehicle: `/api/v1`'s own rules (names, external keys, sequence ids, who may change what, the list) are
  `tests/v1_model/`'s and are not repeated.
- `crates/testing/suites/api/tests/v1_model/mod.rs` and `tests/v1_model/matrix.rs`: the same kind of test for `/api/v1`'s
  work items, over API keys, in the `main` pass: random sequences of create, delete, detail and list (a page of one to three
  and the second page by its cursor) by every kind of caller, in two tenants, a project that does not exist and, now and
  then, a slug that is not the project's workspace's, against a reference model written from the pin's rules (a stranger
  and a guest who writes are one 403, a member deletes only what they made, `sort_order` is 65535 and then 10000 after the
  greatest live one, a list is newest first, a 409 names the work item that has the key); the generator coverage control
  (`model_v1_strategy_reaches_every_step_class`); and every caller × slug × project × route (315 requests) against the
  access table. `sweeper_model_v1_outbox_holds_one_event_per_change_and_nothing_more` runs the sequences in the `sweeper`
  pass and checks the outbox depth after every request, so a request that writes an event it should not, or none it
  should, shows. `/api/v1` has no activity feed, so what the consumer does with an event stays with the slice's model.
- `crates/testing/suites/api/tests/auth/mod.rs` and `tests/auth/matrix.rs`: identity, in the `main` pass (issue #38):
  sign-in and the cookie attributes, the CSRF double-submit rule, sign-out and sign-out-everywhere, deactivation, API
  keys (`X-Api-Key`, shown once, revoked, expired, inactive owner), plus the generated matrix of every
  credential kind × state × method × CSRF-token position against a status table (126 requests). Sign-in, the CSRF token
  and sign-out are Plane's routes (`POST /auth/sign-in/`, `GET /auth/get-csrf-token/`, `POST /auth/sign-out/`), a session
  is read back through `GET /api/users/me/`, and a cookie mutation is a work item made (S48) or changed (X03) through
  Plane's routes, which authenticate through the extractor the slice's do, so the matrix is the credential layer's table
  and not a route's; sign-out-everywhere and the API-key routes are still the slice's, for want of a
  Plane route (G-26). The CSRF rule is a
  switch (`THRUST_CSRF_ENFORCE`, Q-0050.8): the main pass runs with it on, and the `csrfoff` pass (the six `csrf_off_`
  tests, in these two files) runs the same table with it off, a cookie mutation with no token, and the identity routes
  with the cookie alone. The OIDC login is Plane's route too (`GET /auth/oidc/` and `/auth/oidc/callback/`, D-040), so
  its tests are the three `compat_oidc_*` ones in `tests/compat/mod.rs`; the slice's own six `oidc_` tests in `auth` were
  deleted with its routes (step 5d of #58, after the five things they held that the compat tests did not were ported).
  They run against a mock identity provider that the suite serves on `127.0.0.1`
  (`crates/testing/suites/api/src/mock_idp.rs`, RS256 signed with the `rsa` crate, so it shares no code with the
  `openidconnect` crate that verifies its tokens), registered with the app's callback as its redirect URI
  (`scripts/contract.sh`); a deployed Worker cannot reach it, so `scripts/stage2-contract.sh`
  skips them (64 tests there: the `main` pass less the `oidc_` and `operator_` tests).
- `crates/testing/suites/api/tests/budget/mod.rs`: the round-trip budget (issue #39), in the `main` pass.
  `list_endpoints_stay_within_five_db_round_trips` reads the `x-thrust-db-round-trips` header (see
  [the test routes](../../flows/app.md)) of eight requests, each list seeded with six or more rows so that a statement per row
  would show: Plane's projects (S17) and issues (S47) asked by an admin, S47 asked by a limited
  guest who made six of the items and by one who made none, and `/api/v1` projects, states, labels and work items. On a Postgres target (`native-pg`, `worker-cf`) each must send 3 to 5 statements; the floor is what stops a counter
  that counts nothing from passing. On a `memory` target it asserts the header is **absent**, which is what keeps the test from
  passing there without looking. It measures the second request of each list, after the API key's background first-use stamp
  has landed. Each route names its caller (an admin, an API key or a session), and a limited guest costs what an admin
  costs: 3 with rows, 4 on an empty page. One case found over the budget is not covered (G-24 in
  [status](../status.md)): `/api/v1` work items with `?expand=assignees`, which send two statements per distinct
  assignee. The `budget` pass runs this test alone, for `scripts/mutants.sh`; it is not in
  `contract-all.sh`.
- `crates/testing/suites/api/tests/worker_only.rs`: Durable Object, cron and binding tests for `worker-*` targets.

A pass is selected by **test-name prefix**: `sweeper_`, `panic_`, `blast_`, `csrf_off_`, `worker_room_`, `worker_cron_`,
`worker_bindings_`, `stage2_cron_real_trigger_`. Unprefixed tests in `contract.rs` form the `main` pass. A new test therefore
picks its pass by its name. Each arm of `scripts/contract.sh` (and of `scripts/stage2-contract.sh`) is one call,
`suite_pass <bin> '<filterset>' [nextest args]`: the prefixes as a nextest filterset over one test binary, run with the
`contract` profile of `.config/nextest.toml`. `main`'s is `not test(/sweeper_|panic_|blast_|csrf_off_/)`. Under nextest
each test is a process of its own, so the profile puts the `compat_oidc_` tests in a test group of one thread: each starts
the mock identity provider on the one port the target was given as its issuer.

The `mail` pass is the exception: it is its own test binary (`tests/mail.rs`, run with `--test mail`), because it needs a target that has a mail
service and every other pass needs one that has none. It refuses to run when `/api/instances/` does not say `is_smtp_configured: true`. Every other
pass refuses to run when it says anything but `false`, and unsets `THRUST_SMTP_*`, `THRUST_EMAIL_FROM*` and `THRUST_PUBLIC_BASE_URL` first, so a
developer's shell cannot turn the dark test into a test of something else. What the mail service is depends on the kind of target, and the seven tests
(three of the password reset, four of the sign-in with a code) are the same on both, because the mail is read into one shape (`Subject`, `From`, `To`, `Text`):

- **A native target** has the SMTP adapter. `scripts/contract.sh` starts a Mailpit container (a mail server written by someone else, so that what the
  server sends is read by a server that parses it for itself) and the server with `THRUST_SMTP_*` pointing at it; the mail is read from Mailpit's API.
- **A Worker target** has the `send_email` binding (E3), which under `wrangler dev` without `remote = true` is a simulator: it sends nothing, and writes
  each message it is asked to send to the dev server's log (`From:`, `To:`, `Subject:`) and the text and html to files whose paths the log gives.
  `scripts/contract.sh` starts the Worker with `THRUST_EMAIL_FROM` and reads that log, so the adapter that runs is the Worker's own, built for wasm32,
  against the binding's real JS shape, and wrangler's account of what it was given is read from outside the adapter. No container: both Worker cells
  (`worker-memory mail`, `worker-cf mail`) run in CI. The simulator's lines reach the log a few milliseconds **after** the response that caused them, so
  a read straight after a request can miss its message and a "nothing was mailed" check would pass for want of waiting (measured: the first run read an
  empty log). The tests therefore read after a marker: one more reset for an account of their own, waited for in the log, which is in order, so
  everything sent before it is in it too. The simulator does not validate addresses and lets a line break in a subject through, so what refuses those is
  the adapter's own check (`tests/email.rs`), not the binding. What the pass cannot show is the service behind the binding: a real send is a deployed
  Worker's (G-39).

## Run

```sh
scripts/contract.sh native-memory           # main pass, 37 tests [verified 2026-10-24: 37 passed]
scripts/contract.sh native-memory csrfoff   # CSRF enforcement off, 6 tests [verified 2026-10-24: 6 passed]
scripts/contract.sh native-memory mail      # the password reset by email and the sign-in with a code through a real mail server (Mailpit), 7 tests [verified 2026-11-16: 7 passed]
scripts/contract.sh worker-cf bindings      # [unverified: not run]
scripts/contract.sh native-memory oracle    # the v1 oracle: scoped gate, corpus comparison, controls (not a test count: VERDICT: PASS)
scripts/e2e.sh worker-cf controls           # the first browser journey and its control (not a test count: VERDICT: PASS), `e2e` in contract-all.sh
scripts/contract-all.sh                     # the 23-cell matrix, serially [verified 2026-10-24: 18 of 18 PASS, before the oracle, e2e and mail cells]
```

See [Running](../running.md#the-contract-suite) for targets, passes and prerequisites.

## The engine pass and the scale pass

Spike 0002 (issue #37) adds two passes that are **not** in `contract-all.sh`, so they are not required CI checks:
- `engine` (own test binary, `crates/testing/suites/api/tests/engine.rs`, tests named `engine_`, so the main pass and `EXPECT_MAIN` are untouched; the target starts with a very high `/api/v1` limit, because the tests go through `/api/v1` with an API key per writer): the 50-writer, 10k-create test (each writer deletes the work items it made, as the pin's rule says, and the whole list is read back by cursor), search parity with `icontains` (the pin's workspace search: a name, a project identifier or a number says the query; a description is not searched), search freshness and scope (a stranger and a member of no project are answered with nothing, as the pin does), and the bulk seed. 4 tests on every target; `scripts/mutants.sh` runs it against the memory adapter's `mutant_dup_sequence` and `mutant_search_case_sensitive`.
- `scale` (a binary, like `bench`): list, search and create latency with 100,000 items seeded through `/internal/seed-work-items`.

`scripts/stage2-contract.sh` runs `main` (64 tests: the `oidc_` tests need the mock provider on 127.0.0.1, which a deployed Worker cannot reach, and the `operator_` tests need routes and a token a deployed Worker is started without), `csrfoff` and `engine` against a deployed `worker-pg`, and an `email` pass that sends one real message each time it is run. Numbers and conditions: `research/0011-database-engine-cells.md`.

The spike also built `native-sqlite` and `worker-do` targets (the same suite against a SQLite adapter on rusqlite and inside one Durable Object) and a `do` pass for the object's own checks. They are archived on the tag `spike/0002-worker-do`, not in the tree.

## Required controls

[Paired allow/deny](../evidence-and-controls.md#paired-allow-deny) in every test,
[target guard](../evidence-and-controls.md#target-guard) and [count guard](../evidence-and-controls.md#count-guard)
in every run, and a [planted mutant](../evidence-and-controls.md#planted-mutant) for each security or uniqueness
invariant. Adding a test means updating the pass's `EXPECT_*` count (and its `EXPECT=` in `stage2-contract.sh`): `scripts/check-contract-counts.sh` fails CI when one is stale, for the passes that no CI cell runs and for stage2's too. The CSRF switch has its own [target guard](../evidence-and-controls.md#target-guard):
`scripts/contract.sh` refuses a main pass whose server does not enforce the token and a `csrfoff` pass whose server does, and
`scripts/mutants.sh` shows both refusals. The generated tests add
[generator coverage](../evidence-and-controls.md#generator-coverage) and fail when they sent no requests; a
counterexample is saved in `crates/testing/suites/api/tests/model/mod.proptest-regressions`, committed
([Rule 7](../rules.md)). `scripts/mutants.sh` sets `PROPTEST_DISABLE_FAILURE_PERSISTENCE`, so the counterexamples
found against its broken builds are not saved.

## Owning skill

**None** (G-1). `rust-test-app` explicitly excludes "HTTP assertions shared across adapters". Until a skill exists,
copy the shape of an existing test in `crates/testing/suites/api/tests/contract.rs`: seed with `Fixture::seed`,
assert the allowed case, assert the neighbouring rejected case and its lack of effect, and use `eventually()` for
asynchronous effects. For a new operation or route, extend the generated part first ([Rule 9](../rules.md)): an `Op`
and its rule in the model, or a `Route` in the matrix.

## In-repo examples

`cross_tenant_access_is_rejected_and_has_no_effect`, `concurrent_creates_never_duplicate_a_sequence_id`,
`redelivered_event_is_processed_once`, `worker_bindings_rate_limiting_binding`; generated:
`model_work_items_over_http_match_the_reference_model`,
`every_actor_project_and_route_gets_the_status_of_the_access_table`.

## Target

Every port × adapter green as an acceptance criterion (SPEC §9); capability flags for legitimate adapter
differences (ADR-0121, `.agents/plans/contrail/adrs/test/patterns/0121-port-contract-tests-one-suite-n-adapters.md`).
