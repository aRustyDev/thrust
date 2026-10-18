# The contract suite: targets, passes, guards

Evidence: `crates/thrust-contract-tests/{src/lib.rs,tests/contract.rs,tests/worker_only.rs}`,
`scripts/contract.sh`, `scripts/contract-all.sh`, `scripts/worker-dev.sh`. Tags are defined in
`SKILL.md`; provenance in `SOURCES.md`.

## Shape

One suite, identical assertions and fixtures on every target. It sees only two environment variables,
`THRUST_BASE_URL` and `THRUST_ADMIN_TOKEN` (`Target::from_env`), and creates fixtures through
`POST /internal/seed`, the same route on every target. Tenancy fixture: two workspaces; tenant A has
one project with an admin, member, guest and an in-workspace outsider; tenant B has its own.

`scripts/contract.sh <target> [pass]` builds/starts the target, waits for `/healthz`, runs the guards
below, runs `cargo test` for the pass, and exits with **`cargo test`'s own status** (no pipe, no
trailing `echo`). The admin token and the throwaway Postgres password are generated per run and never
written to a file. It picks a random free port and only signals processes it started.

## Targets and passes

| Target | What runs | Needs |
|---|---|---|
| `native-memory` | `thrust-server` + in-memory adapters | cargo |
| `native-pg` | `thrust-server` + Postgres pool; in-process stand-ins for queue/KV/blob/limiter | docker |
| `worker-memory` | Worker under `wrangler dev`, in-isolate adapters | wrangler, worker-build |
| `worker-cf` | Worker under `wrangler dev`, Cloudflare adapter set (Hyperdrive, Queues, KV, R2, DO, cron) | wrangler, worker-build, docker |

Runs that need docker (`native-pg`, `worker-cf`) start a Postgres container and pull its image on first
use, which needs network access; so do the testcontainers adapter tests.

The authoritative list of passes is the `case "$PASS"` block in `scripts/contract.sh`. In outline:

| Pass | Selects | Why separate |
|---|---|---|
| `main` (default) | `contract` tests, skipping the `sweeper_`, `panic_`, `blast_` prefixes | outbox fast path ON |
| `sweeper` | `sweeper_*` | fast path OFF: the sweeper must do the delivery |
| `panic` | `panic_*`, alone, fresh target | a Worker panic resets the isolate and killed parallel tests |
| `blast` | `blast_*` | observational (prints `PANIC-OBSERVATION`), not in `contract-all` |
| `cron` | `worker_cron_*` (worker-*) | `scheduled` handler via `--test-scheduled` |
| `worker` | `worker_room_*` (worker-cf) | Durable Object WebSocket hibernation + SQLite |
| `bindings` | `worker_bindings_*` (worker-cf) | one real operation per binding |
| `bench` | release-build latency client | success == bench exits 0; loopback numbers only |

`scripts/contract-all.sh` runs every cell listed as a `run <target> <pass>` line in the script, serially,
and writes `target/contract-logs/summary.txt`. Serial because cells share `crates/thrust-worker/build`.

Tag any cell you could not run `[unverified: <tool> not installed]` instead of reporting the matrix as green.

## Guards that exist so green means something

| Guard | Where | Failure |
|---|---|---|
| Target says what it really is: `/_info` must report the `target` kind and `adapters` set the cell claims | `contract.sh` | exit 5 `VACUITY GUARD: target kind / adapter set` |
| Pass count: exit 0 but `passed != EXPECT_*` | `contract.sh` | exit 5 `VACUITY GUARD: exit 0 but passed count != expected` |
| Infra failure is not a test failure: build failed, postgres not ready, never healthy | `contract.sh` | exit 3 (build, postgres) or 4 (never healthy) |
| Precondition asserted: "activity is empty before the sweeper/cron ran" | `sweeper_*`, `worker_cron_*` | assertion text says "test would be vacuous" |
| Load actually ran: an assertion on the success count of the load generator | `blast_radius_of_one_panic` | assertion |
| Async effects polled, not slept: `eventually(what, timeout, f)` | `src/lib.rs` | panics naming `what` on timeout |
| Duplicate delivery forced, not hoped for: the same envelope injected twice | `redelivered_event_is_processed_once` | an earlier test never redelivers, so it cannot tell an idempotent consumer from a called-once one |

Expected counts are the `EXPECT_*` variables at the top of `scripts/contract.sh`. They are
hand-maintained: adding or removing a test in a pass means editing the matching one, otherwise the
guard (correctly) fails. Read them from the script; do not copy numbers from documentation.

## Inline controls (the house style)

Each test that asserts "X is allowed" also asserts the neighbouring "Y is rejected" in the same test:
cross-tenant 404 next to the tenant's own 200; guest 403 next to member 201; duplicate 409 next to a
different id 201; over-limit 429 next to another user unaffected; oversize 413 followed by "the stored
object was not clobbered"; with-deleted 403 for a member next to the admin seeing the tombstone. A
server that accepts everything, or rejects everything, cannot pass.

## Panic and state

`THRUST_STATE_SURVIVES_PANIC=0` is set only for `worker-memory` (state lives in the isolate). Everywhere
else the row written before the panic must still be there. Observations (`PANIC-OBSERVATION`,
`HIBERNATION-OBSERVATION`) are printed, not asserted; the assertion is "the target recovers".

## Adding a test

Follow the one procedure in `SKILL.md`. Extras specific to this suite:

- Behaviour that exists on one target only goes in `tests/worker_only.rs`; everything else in `tests/contract.rs`.
- A test that needs the fast path off, panics, or a special target needs its own filter prefix, its own
  branch in the `case "$PASS"` block, and its own `EXPECT_*`.
- Run `scripts/contract-all.sh` before merging if you touched an adapter or the router.
