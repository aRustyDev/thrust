# Research 0004 — Rust on Workers: spike 0001 results and the binding-coverage matrix

Date: 2026-10-18. Spike: `spikes/0001-rust-on-workers-feasibility.md` (bead `plane-byd.1`). Code: thrust repo worktree
`/Users/asmith/repos/arustydev/thrust-wt/spike-0001-rust-on-workers`, branch `feat/spike-0001-rust-on-workers`
(merged into `aRustyDev/thrust` `main` as PR [#17](https://github.com/aRustyDev/thrust/pull/17)). Commits, all signed: `f037986` skeleton + memory adapters + native server + shared suite · `621115c` wasm32 Worker root ·
`564baa9` shared Postgres adapter + send-shape probe · `d32df20` Cloudflare adapter set · `620e2a1` redelivery test, mutants, binding probes, bench ·
`c7ff487` sqlx probe, `verify-all.sh`, harness `/_info` retry · `abbd1b1` Stage 2 on the real platform (single-pipeline statements, harness for a deployed Worker, probes). Method: everything below was **executed locally** (native + `wrangler dev` local workerd +
Docker Postgres) unless tagged otherwise. **Stage 2 (a real Cloudflare account) WAS run on 2026-10-18** (§10, tag **[tested-cf]**).

Verification tags: **[tested]** I ran it and the command is given · **[doc]** a vendor page I read (URL given) ·
**[snippet]** seen in a source file or search snippet, not executed · **[unverified]** inference, not observed.
`[tested]` always means "on local workerd / local Postgres", never "on Cloudflare's network". **[tested-cf]** means executed against the deployed Worker on Cloudflare's network (Stage 2, §10).

## 1. Verdict: **Conditional pass** (Stage 1 and Stage 2 both run; the conditions are listed below)

| Exit criterion (spike file) | Result | Evidence |
|---|---|---|
| Slice green on both targets with ONE shared suite | **Met** | 16 matrix cells green, §4. Same `contract.rs`, same fixtures, 4 target combinations. |
| Bundle < 20 MiB (limit 64 MiB) | **Met**: 1.58 MiB locally; **1.67 MiB deployed (1,709 KiB, 568 KiB gzip)** [tested-cf] | §6.1, §10 |
| Startup < 300 ms (limit 1 s) | **Met**: 17.7 ms profile window locally; **2–3 ms "Worker Startup Time" at deploy** [tested-cf] | §6.2, §10 |
| Every P0 binding (KV, R2, Queues, DO, Hyperdrive, Cron) native or thin shim | **Met**: all six are native in workers-rs 0.8.7 and exercised | §5 |
| Panic handling policy defined | **Met** | §7 (abort + reset; blast radius measured) |
| *Conditional*: Workflows/Email need a shim Worker, record the contract | **Workflows needs a JS shim (contract recorded and tested); Email does not** | §5, `crates/thrust-worker/entry.mjs` |
| *Fail*: a P0 binding unusable from Rust, or the dual build forces divergent business logic | **Not triggered** | all business logic (core, ports, contracts, router, SQL) is the same source on both targets; §3, §4 |
| Stage 2: the same suite green against the REAL platform (Hyperdrive, KV, R2, Queues, DO, cron) | **Met, after one adapter change (§10.4 F1)**: 15/15, repeated 8/8 | §10 [tested-cf] |
| Stage 2: Hyperdrive usable from Rust | **Met only through the single-pipeline statement flow**; tokio-postgres' default flow fails | §10.4 F1 [tested-cf] |

**Evaluated in Stage 2 (§10) [tested-cf]:** real Hyperdrive behaviour (pooling, `BEGIN…COMMIT` on a real Hyperdrive socket, latency), Queue retry/DLQ,
cold start, p95 over a real network, real Durable Object hibernation, real cron firing, a real Secrets Store, deploy-time validation (size, startup), CPU per invocation.
**Still not evaluated:** a real Email send, AI Gateway, KV eventual consistency, Durable Object billing, multi-region latency, a 24-hour cost reconciliation,
Hyperdrive caching on versus off, the undecodable-queue-message branch.

**Conditions on the pass.** (1) Workflows needs the JS shim (unchanged). (2) **Every SQL statement must go through the single-pipeline typed flow**
(`query_typed` / `execute_typed`, one round trip on the unnamed statement): the default `tokio-postgres` flow (a named prepared statement, then a second round trip)
failed through Hyperdrive on every statement without result columns and intermittently on the rest (§10.4 F1). (3) Email was not exercised beyond "the binding resolves
and rejects a sender domain the account does not own".

**Precise trigger for the TypeScript-core fallback (reopen ADR-0006).** I do not conclude Fail. Reopen if Stage 2 shows any one of:
1. a `BEGIN … COMMIT` over one Hyperdrive `Socket` is not atomic or fails (the sequence-allocation and outbox design depend on it);
2. the real Queues consumer cannot ack/retry individual messages from Rust, or text-encoded JSON bodies do not arrive intact;
3. the deployed bundle is > 20 MiB, or `startup_time_ms` from `wrangler versions upload` is ≥ 300;
4. any use-case needs `#[cfg(target_arch = "wasm32")]` or a second implementation to pass the shared suite.

**Stage 2 outcome of the reopen triggers.** (1) **Hit by the default driver flow, cleared by the single-pipeline flow**: transactions (BEGIN, statements, COMMIT) on a Hyperdrive socket
work and the suite passes 15/15 repeated 8/8; with `prepare`-then-`execute` they failed. That is a constraint on how Rust code talks to Hyperdrive, not a reason to leave Rust;
whether it should be recorded in ADR-0006/ADR-0002 is the owner's call. (2) Not triggered: individual `ack` and `retry` with a delay worked, a failing message was retried 3 times and dead-lettered, and JSON bodies arrived intact.
(3) Not triggered: 1.67 MiB deployed, 2–3 ms startup. (4) Not triggered: the shared adapter changed for BOTH targets (every statement now goes through one wrapper), no `cfg(target_arch)`, no second implementation.

## 2. What was built (maps to ARCHITECTURE §2a)

```
crates/thrust-contracts         ids, WorkItem, event Envelope (ADR-0046), API DTOs        230 lines  shared
crates/thrust-ports             port traits, all futures `Send`                            218        shared
crates/thrust-core              authz seam (Scope proof type), use-cases, outbox relay,
                                sweeper, queue consumer, attachments                       533        shared
crates/thrust-api               ONE axum router, default-features=false                    293        shared (compiled into BOTH roots)
crates/thrust-adapters-postgres the SQL, at tokio_postgres::Client level (+ 0001_init.sql)  436 + 75   shared
crates/thrust-adapters-memory   in-memory adapter for every port                           422        shared
crates/thrust-adapters-selfhosted  deadpool pool connector, clock, ids, tokio Background    89        native only
crates/thrust-adapters-cloudflare  KV, R2, Queues, DO limiter, Hyperdrive connector, ...   280        wasm32 only
crates/thrust-server            native root: axum + tokio + CatchPanicLayer                137        native only
crates/thrust-worker            wasm32 root: fetch/queue/scheduled, DO classes, probes     581 + 26 (entry.mjs)   wasm32 only
crates/thrust-contract-tests    the ONE HTTP suite + bench + worker-only tests            ~1,080
```
About 2,200 lines (+ the SQL) are the *same source* on both targets; the target-specific code is ~230 lines native and
~890 lines Worker (the Worker figure includes the two DO classes and the binding probes, which have no native twin).

The slice: **create work item** = rate limit → authz (`Scope`) → tx{ `allocate` (counter-row upsert, ADR-0043 B1, also
`sort_order`) → insert → outbox row } → after-commit publish via the `Background` port → queue → consumer writes an
`activity` row (idempotent on `event_id`); plus **list with soft-delete visibility** (`visible()` default scope,
admin-only `with_deleted`), soft delete, attachments (BlobStore), and a cron/`POST /internal/sweep` outbox sweeper.

## 3. First design call: port trait shape

**Decision: ports return `Send` futures (plain `#[async_trait]`, *not* `#[async_trait(?Send)]`); the Cloudflare adapters wrap
their JS-backed futures in `worker::send::SendFuture`. No per-target executor wrapper, no `cfg` switch on the trait.**
(ADR-0070's Rust sketch says `#[async_trait(?Send)]` — it must change.)

Why (evidence, `probes/send-shape/run.sh`, one port + one axum handler, compiled in every shape on both targets) **[tested]**:

| port shape | native | wasm32 |
|---|---|---|
| `#[async_trait]` (Send) | compiles | compiles |
| `#[async_trait(?Send)]` | **E0277** `Handler<_, _>` not implemented | **E0277** |
| `?Send` on wasm32 only (`cfg_attr`) | compiles | **E0277** |

axum requires every handler future to be `Send` on *every* target it is compiled for, so the moment the HTTP layer is one
shared router (ARCHITECTURE §2a `thrust-api-core`) the ports must be `Send` on wasm32 too. `?Send` would force either two
routers (the divergent-logic failure mode) or an executor shim that marshals every use-case onto a `LocalSet`.
Making ports `Send` costs one wrapper per JS-touching adapter method (`SendFuture::new(async move { … }).await`), which is
sound because a Worker isolate is single-threaded (workers-rs relies on the same argument for its own
`unsafe impl Send for KvStore/Queue/Hyperdrive/…`). The `unsafe` lives inside `worker`: Thrust's own crates contain no `unsafe`
(`grep -rn unsafe crates/` matches one comment) **[tested]**.

**Executor abstraction.** Core never calls `tokio::spawn`; it asks the `Background` port (`fn spawn(BoxFuture<()>)`):
native = `tokio::spawn`, Worker = `ctx.waitUntil`, tests = inline. `Clock` and `IdGen` are ports too: chrono's `clock` feature
is off in `thrust-contracts`, so `Utc::now()` literally does not exist in core, and uuid has no RNG feature there.

**No Tokio in core/ports** — `scripts/check-deps.sh`, `cargo tree -i <crate>` for each of 17 runtime/platform crates against
`thrust-contracts`, `thrust-ports`, `thrust-core`, `thrust-api` (the shared router) and `thrust-adapters-memory`, `--target all`:
**none present** **[tested]**. Their full dependency sets are serde/serde_json/uuid/chrono/thiserror/async-trait/futures-core+task+util/
slab/pin-project-lite (and axum/http/tower/bytes/matchit for `thrust-api`). Cross-checks: `deadpool-postgres`, `hyper`, `mio`,
`socket2`, `tower-http`, `thrust-adapters-selfhosted` are **absent** from the Worker's wasm32 graph; `worker`, `worker-sys`,
`wasm-bindgen`, `js-sys`, `web-sys`, `thrust-adapters-cloudflare` are **absent** from the native server's graph.
**Honest caveat:** `tokio` *is* in the Worker bundle's graph, pulled by `worker` itself and by `tokio-postgres`, with features
`bytes, io-util, sync` only (no `rt`, `net`, `time`, `macros`) — i.e. trait/channel code, not a runtime.

## 4. Dual-target evidence: one suite, four target combinations

`scripts/contract.sh <target> <pass>`; `scripts/contract-all.sh` runs the matrix serially (parallel `wrangler dev` runs
share `crates/thrust-worker/build` and reload each other: measured). Targets: `native-memory`, `native-pg` (self-hosted set:
Postgres + in-process stand-ins), `worker-memory` (in-isolate adapters), `worker-cf` (Cloudflare set: Hyperdrive→Postgres,
KV, R2, Queues producer+consumer, DO SQLite, cron). The suite is HTTP-only and never learns which target it is on.

Final serial run (`scripts/verify-all.sh` → `scripts/contract-all.sh`), all **PASS**, passed-count = expected-count in every cell **[tested]**:

| target | main (15) | sweeper (1) | panic (1) | cron (1) | worker: DO hibernation (1) | bindings (6) |
|---|---|---|---|---|---|---|
| `native-memory` | PASS | PASS | PASS | n/a | n/a | n/a |
| `native-pg` | PASS | PASS | PASS | n/a | n/a | n/a |
| `worker-memory` | PASS | PASS | PASS | PASS | n/a | n/a |
| `worker-cf` | PASS | PASS | PASS | PASS | PASS | PASS |

16 cells, 77 test executions (60 main-pass: the same 15 tests × 4 targets); the main-pass tests are byte-for-byte the same file on all four targets. The same run also passed the 4 mutation controls (all killed),
the dependency-graph check (no forbidden crate), the 6 testcontainers adapter tests, and the port-shape probe. Earlier in the session the `worker-memory` cells were flaky (the
body-cancel bug below); they are green after the fix.

**What the suite asserts** (15 tests in the main pass; every "allowed" assertion is paired with a neighbouring must-fail control):
unauthenticated 401 · admin routes need the admin token · monotonic `sequence_id`/`sort_order` and per-project counters ·
visible-scope list ordering · soft delete hides rows, `with_deleted` is 403 for a member and 200 for an admin, double delete 404,
**numbers never reused** · **cross-tenant create/list/with_deleted → 404 and no row leaks** · guest/outsider forbidden to write ·
live-duplicate `external_id` → 409 but re-creatable after soft delete (partial unique index) · **20 concurrent creates give unique
contiguous sequence numbers** · outbox → queue → consumer writes exactly one activity row, deletion adds a second ·
**a redelivered (duplicate) event id yields one activity row, a different id still yields one** · per-user rate limit 429 with
`Retry-After` while another user is unaffected · attachments (put/get/delete, binary-safe, content-type, cross-tenant 404,
traversal rejected, oversize 413) · input validation 400. Plus passes: `sweeper` (fast path off; asserts nothing delivered
*before* the sweep, then the sweep delivers), `panic`, and the Worker-only `worker` (DO hibernation) / `cron` / `bindings`.

**Non-vacuity.** A check that cannot observe the thing it claims to test is not evidence, so:
- `contract.sh` fails if exit is 0 but the passed-test count ≠ the expected count, and asks the target `/_info` which adapter
  set it is *really* running before testing (during development this guard fired twice: once on a bug in its own key-order matching,
  once on an empty reply right after startup; it now retries the probe and still fails closed).
- **Mutation controls** (`scripts/mutants.sh`): deliberately broken builds that the suite must fail **[tested]**:

| mutant | killed by |
|---|---|
| memory adapter ignores the tenant predicate | `cross_tenant_access_is_rejected_and_has_no_effect`, `attachments_roundtrip_with_authz`, `outbox_event_reaches_the_queue_consumer_exactly_once` |
| memory allocator hands out the same sequence number | `create_allocates_monotonic_sequence_and_sort_order`, `list_returns_visible_items…`, `soft_delete_hides_rows…`, `concurrent_creates_never_duplicate_a_sequence_id` |
| consumer has no idempotency on `event_id` | **only** `redelivered_event_is_processed_once` |
| Postgres connector recycles a connection still inside a transaction | `an_abandoned_transaction_does_not_poison_the_pool` |

  The third row is a finding about my own suite: the original "exactly once" test could not have caught a missing dedupe
  (nothing redelivers), which is why `redelivered_event_is_processed_once` exists.
- Adapter-level tests against a real Postgres via **testcontainers** (6, all pass): membership vs other tenant, 24 concurrent
  allocations unique+contiguous, rollback atomicity + unique violation → `Conflict` + no counter gap, abandoned-tx discard,
  soft-delete frees the external key but not the sequence number, outbox atomicity + idempotent activity insert.

**A real bug found by flake-hunting, not by a red test (workers-rs/workerd portability).** `worker-memory main` failed in 2 of 8
runs (3 of 8 with debug logging) with 500s and `IncompleteMessage`. Debug log: `TypeError: Can't read from request stream after
response has been sent` → `Error: Network connection lost`. Cause: `worker::Body` holds a reader on the JS request stream and
cancels it on drop; handlers that answer early (401/403/404/409 before the JSON extractor runs) drop the body *after* the
response, workerd rejects the cancel, and `wrangler dev` resets other in-flight connections. Fix: drain (bounded, 8 MiB) the
request body in the Worker root before routing. After the fix: **0 failures in 16 consecutive runs** (`scripts/flake-hunt.sh
worker-memory main 16`). Cost: the Worker root has no streaming uploads (R2 streaming PUT would need a separate path; ADR-0081).
Whether production workerd tolerates the unfixed shape is **[unverified]**; do not rely on it.

## 5. Binding-coverage matrix (workers-rs 0.8.7, `worker-build` 0.8.7, wrangler 4.123.0)

Exactly one category per row: **native** = typed API in `worker` · **shim** = via wasm-bindgen / a JS shim ·
**REST** = REST fallback · **no** = not possible.

| Binding | Category | Tag | Evidence |
|---|---|---|---|
| KV | native | **[tested]** | `KvSessionStore` (`get().json`, `put().expiration_ttl`) is the session store of every `worker-cf` cell. Local KV is strongly consistent, so KV's eventual consistency is **[unverified]**. https://developers.cloudflare.com/kv/ |
| R2 | native | **[tested]** put/get/delete via the attachments tests; `list` is compiled, not exercised | `R2BlobStore`; streaming bodies not exercised (body is buffered). https://developers.cloudflare.com/r2/ |
| Queues, producer | native (feature `queue`, marked beta upstream) | **[tested]** | `CfQueue::send_raw` with a JSON **text** message; `wrangler dev` logs `QUEUE thrust-spike-events 10/10` batches. https://developers.cloudflare.com/queues/ |
| Queues, consumer | native (`#[event(queue)]`) | **[tested]** ack path; `retry_with_options`/DLQ path **[unverified]** | consumer calls `process_event`, the same body as the native consumer. Redelivery test injects duplicates through the real queue. |
| Durable Objects, SQLite storage | native (`#[durable_object]`, `state.storage().sql()`) | **[tested]** | `RateLimiterDo` (fixed-window counter) behind the shared 429 test; `RoomDo` history table. `new_sqlite_classes` migration. https://developers.cloudflare.com/durable-objects/ |
| Durable Objects, WebSocket hibernation | native (`accept_web_socket`, `websocket_message/close/error`, `serialize_attachment`) | **[tested]** | `worker_room_do_websocket_hibernation_api_and_sqlite_storage`: after 14 s idle the object was **re-instantiated** (its `instance` field changed) while the socket stayed open; the SQLite sequence and the socket attachment survived. Local workerd timing only. |
| Hyperdrive | native (`Hyperdrive::connect()` → `Socket`) + `tokio-postgres` `Config::connect_raw` over it | **[tested]** against wrangler's local Hyperdrive emulation | the whole `worker-cf` suite incl. transactions, `ON CONFLICT`, partial unique index. Real Hyperdrive (pooling, TLS, caching) is **[unverified]**. https://developers.cloudflare.com/hyperdrive/ |
| Cron (`scheduled`) | native (`#[event(scheduled)]`) | **[tested]** through wrangler's `--test-scheduled` `/__scheduled` endpoint | `worker_cron_scheduled_handler_runs_the_sweeper` (nothing delivered before the call, delivered after). An actual cron trigger firing is **[unverified]**. https://developers.cloudflare.com/workers/configuration/cron-triggers/ |
| Workflows | **shim** | **[tested]** | workers-rs 0.8.7 has **no** Workflow API (the crate source contains no occurrence of "workflow") and cannot define a Workflow class (`WorkflowEntrypoint` is JS). Rust creates and polls an instance through raw `js_sys::Reflect` on `env.WORKFLOW`; the class lives in the JS shim `entry.mjs`; the workflow ran two `step.do` steps and called back into the Rust Worker through a service binding; status `complete`, output verified (`worker_bindings_workflow_via_wasm_bindgen_and_js_shim`). https://developers.cloudflare.com/workflows/build/workers-api/ |
| Email Service (send) | native (`env.send_email`, `SendEmailBuilder`) | **[tested]** local *simulation* only (nothing leaves the machine) | real delivery needs an onboarded domain + Workers Paid **[doc]** https://developers.cloudflare.com/email-service/api/send-emails/workers-api/ ; local simulation https://developers.cloudflare.com/email-service/observability/local-development/sending/ ; REST and SMTP alternatives exist **[doc]** https://developers.cloudflare.com/email-service/api/send-emails/rest-api/ . Inbound `#[event(email)]` exists in the macro **[snippet]**, not exercised. |
| AI Gateway | **REST** | **[doc]**; `worker::Ai::run` compiles but is not run | `Ai::run(model, input)` in 0.8.7 takes no options argument, so the documented `env.AI.run(model, input, { gateway: { id } })` cannot be expressed natively (source read). Use the REST endpoints with a `cf-aig-gateway-id` header, or a 3-argument raw `Reflect` call. `wrangler dev --local` cannot run the AI binding. https://developers.cloudflare.com/ai-gateway/usage/worker-binding-methods/ · https://developers.cloudflare.com/ai-gateway/usage/rest-api/ |
| Rate Limiting | native (`env.rate_limiter`) | **[tested]** local | limit 3/10 s: outcomes `[true,true,true,false,false]`. https://developers.cloudflare.com/workers/runtime-apis/bindings/rate-limit/ |
| Secrets Store | native (`env.secret_store`, `.get()`) | **[tested]** against the *local* store | value length asserted, value never echoed. Real store + scopes **[unverified]**. https://developers.cloudflare.com/secrets-store/integrations/workers/ |
| Service bindings | native (`env.service` → `Fetcher::fetch`) | **[tested]** (self-binding) | also the path the Workflow shim uses to call Rust. RPC (`Fetcher::into_rpc`) **[snippet]**. https://developers.cloudflare.com/workers/runtime-apis/bindings/service-bindings/ |

**Counts (14 rows: Queues and DO each split in two):** native **12** · shim **1** (Workflows) · REST **1** (AI Gateway) · not possible **0**.
Tags: `[tested]` 13 (locally; Email is a simulation) · `[doc]` 1.

**P0 gaps that matter** (KV, R2, Queues, DO, Hyperdrive, Cron are all native and exercised, but):
1. **Queues**: consumer retry/DLQ/`max_retries` untested; no ordering primitive (ADR-0046's per-key ordering needs the DO or sequence-check design);
   measured create→activity delivery was ≈ 1.05 s with `max_batch_timeout = 1` (§6.3); I believe a partially filled batch waits for that timeout **[unverified]**.
   Messages are sent as JSON *text*: sending the struct directly goes through serde-wasm-bindgen, whose default turns
   `serde_json::Value` objects into JS `Map`s that `JSON.stringify` renders as `{}` — that failure mode is **[unverified]** (derived
   from reading the code, not reproduced); the text encoding avoids it regardless.
2. **Hyperdrive**: only the local emulation was used. Each request opens its own socket (a socket belongs to one request in workerd);
   per-request connect cost on the real service is the biggest unknown for p95.
3. **R2**: request bodies are fully buffered (needed to avoid the body-cancel bug above); large uploads need a streaming design.
4. **KV**: eventual consistency cannot show up locally; sessions are fine, anything needing read-your-writes must not use KV (ADR-0082 already says DO for strong counters).
5. **DO**: hibernation observed locally only; ADR-0043 B3 (DO as the sequence allocator) was not tried — B1 (counter row) works.
6. **Cron**: real trigger firing not observed.

Not in the requested list but seen: Analytics Engine, Workers AI (`Ai`), D1 (feature `d1`, "alpha"), Containers, dynamic dispatch, static
assets (`env.assets`) have wrappers in `worker` 0.8.7 **[snippet]** — none exercised. **sqlx**: see `probes/sqlx-wasm` (§8).

## 6. Measurements (all LOCAL, Apple-silicon macOS, loopback; not Cloudflare latency)

### 6.1 Bundle size (limit 64 MiB uncompressed; pass target < 20 MiB)
Command: `scripts/rs worker-build --release --out-dir target/wb-cf crates/thrust-worker --no-default-features` (Cloudflare set only) and the same without
`--no-default-features` (`wb-memory`, both adapter sets), then `wc -c` / `gzip -9 -c`. Release profile: `opt-level="s"`, `lto=true`, `codegen-units=1`,
`strip="debuginfo"`, then worker-build's `wasm-opt` (binaryen 132).

| bundle | `index_bg.wasm` | `index.js` | total uncompressed | gzip(wasm) |
|---|---|---|---|---|
| Cloudflare set only | 1,620,726 B (1.55 MiB) | 38,731 B | **1.58 MiB** | 551,395 B |
| memory + Cloudflare | 1,690,994 B | 38,731 B | 1.65 MiB | 574,783 B |
| first memory-only slice (axum + core, before any binding crate) | 689,924 B | — | 0.66 MiB | — |

`wrangler check startup` reports the bundle as 1647.53 KiB / gzip 551.78 KiB. (Cloudflare checks only the uncompressed size, 64 MiB **[doc]**
https://developers.cloudflare.com/workers/platform/limits/ .)

### 6.2 Startup time (limit 1 s; pass target < 300 ms)
Command: `scripts/startup-check.sh wrangler.toml` = `wrangler check startup` (local V8 profile of global-scope execution, no login, nothing deployed):
profile window **17.7 ms**, active **10.0 ms**, GC 0.0 ms, 4 samples. wrangler itself warns this is a local-CPU number, not Cloudflare's.
Cold-start latency on Cloudflare is Stage 2. (Gotcha: `wrangler check startup` reuses the existing `build/` output; it does not re-run `[build]`.)

### 6.3 Latency of the slice (`scripts/contract.sh <target> bench`: 200 creates, 100 lists, 100 activity reads, 40 create→activity deliveries; loopback)
Native server built `--release`; Worker via `wrangler dev` (includes miniflare's proxy hop). Other agents were using this machine; treat as order-of-magnitude.

| target | create p50 / **p95** (c=1) | create p95 (c=8) | list p95 | activity p95 | create→activity visible p50 |
|---|---|---|---|---|---|
| native-memory | 0.10 / **0.11 ms** | 0.25 ms | 0.32 ms | 0.12 ms | 0.15 ms |
| native-pg | 2.63 / **5.35 ms** | 18.07 ms | 1.38 ms | 0.95 ms | 17.1 ms |
| worker-memory | 1.11 / **1.76 ms** | 7.93 ms | 2.01 ms | 1.53 ms | 2.06 ms |
| worker-cf | 21.51 / **27.44 ms** | 139.66 ms | 13.02 ms | 14.02 ms | **1,048.6 ms** |

Reading it: `worker-cf` create does a KV read, a DO round trip, a membership query, a 5-statement transaction (BEGIN, allocate, insert, outbox, COMMIT) and a queue send in
`waitUntil`, versus a single in-process call on `worker-memory`. My reading of the c=8 create p95 (140 ms) is the hot counter row (ADR-0043 B1) serialising concurrent creates in one
project plus a single local isolate — I did not isolate the cause **[unverified]**, so treat it as a prediction to check in Stage 2, not a production figure. The 1.05 s delivery
is consistent with `max_batch_timeout = 1` in the consumer config acting as a batching floor (I did not vary it **[unverified]**).

### 6.4 Panic behaviour (see §7 for the policy)
- `panic=abort` is the only mode on stable `wasm32-unknown-unknown`. worker 0.8.7 installs a panic hook (message visible in the log) and, via
  wasm-bindgen's `--experimental-reset-state-function`, **resets the Wasm instance** after a panic (default since workers-rs 0.6.5,
  https://developers.cloudflare.com/changelog/post/2025-09-19-workers-rs-panic-recovery/ **[doc]**).
- **[tested]** the panicking request returns HTTP 500; the next requests are served; durable state survives (`worker-cf`) and isolate-local state does not
  (`worker-memory`: sessions gone, 401).
- **[tested]** blast radius (`scripts/contract.sh <target> blast`, observational, not part of the matrix): 16 closed-loop clients doing an authenticated list (it awaits
  I/O) around ONE panic. **native-pg: 0 of 7,228 and 0 of 10,775** other requests failed (native-memory: 0 of 33,564). **worker-cf, 5 runs: 14/284, 15/231, 14/316, 13/330,
  12/331 failed (4–6 %)** — roughly the set of requests suspended on the isolate at that moment — and one request per run hung until the 30 s client timeout. The dev-server
  log shows the mechanism: suspended tasks of the old instance throw `Invalid stale object from previous Wasm instance` / `memory access out of bounds` right after
  `Critical RuntimeError: unreachable`, and the panic message itself is logged (`panicked at crates/thrust-api/src/lib.rs:147:5`).
  The Cloudflare-set dev server kept serving in all 6 runs I made (2 earlier + 4). On the in-isolate memory set (all state lost, so every later request is a 401) the dev server stopped
  answering within 3 s in **2 of 4** runs (small n; whether that is only a `wrangler dev` artefact is **[unverified]**). That is why the `panic` pass in the matrix checks only the single
  deterministic request-level behaviour and the blast-radius pass is separate.
- `--panic-unwind` (nightly + `-Zbuild-std`) would narrow the blast radius to the panicking request **[doc]** (worker README, "Panic Recovery with
  `--panic-unwind`"). **Not built or run**: it needs a nightly toolchain that is not among the installs this spike was allowed to make. **[unverified]** for Thrust code
  (the README also notes closures passed to JS must be `UnwindSafe`).

## 7. Panic policy (defined)

1. **Worker (production shape): `panic=abort` + workers-rs instance reset.** Treat a panic as "this isolate dies now": it fails the panicking request with 500 and
   can fail or hang the other requests suspended on that isolate (measured ≈ 5 % of 16 concurrent clients). Therefore: (a) **no correctness-bearing state in isolate
   globals** — the in-memory adapters are test-only and are not in the Cloudflare bundle (`--no-default-features`); (b) core/ports/adapters deny
   `clippy::unwrap_used`, `expect_used`, `panic`, `indexing_slicing` (not yet enforced in the spike — TODO for P0) so request-derived data can never panic;
   (c) every write is a transaction and every event goes through the outbox, so an abort loses at most an uncommitted transaction and an undispatched row
   that the sweeper re-sends to idempotent consumers; (d) queue consumers are idempotent on `event_id` (tested) so an aborted batch is safe to redeliver (the
   retry path itself is **[unverified]**); (e) `/_test/panic` exists only behind `THRUST_TEST_ROUTES`.
2. **Native server: `panic=unwind` + `CatchPanicLayer`** (500, logged); no collateral failures measured.
3. Revisit `--panic-unwind` at P0 planning if Stage 2 shows the abort blast radius matters at the sizing target (≤ 50 users it likely does not).

## 8. Findings that change or sharpen the plan

**Environment (this machine, would bite any agent):**
- `cargo`/`rustc` on PATH are **Homebrew 1.98.1, which has no `wasm32-unknown-unknown` std**; the target exists only in **rustup stable 1.99.0**. `rustup run stable cargo`
  does not fix it (cargo resolves a bare `rustc` through PATH). `scripts/rs` puts the rustup toolchain first.
- rustup 1.99.0's `rust-lld` aborts with `dyld: Library not loaded: @rpath/libLLVM.dylib`; fixed in `scripts/rs` with `DYLD_FALLBACK_LIBRARY_PATH` (set inside the script because
  SIP strips `DYLD_*` from `/bin/sh`'s environment).
- `strip = true` in the release profile drops the wasm `target_features` section; wasm-bindgen then does not enable externref and dies with
  "externref table required for catch wrappers". Use `strip = "debuginfo"`.
- wrangler 4.123.0's workerd accepts `compatibility_date` ≤ 2026-08-18. `worker-build` downloads its own wasm-bindgen (0.2.129) and binaryen/`wasm-opt` (132), so no brew install was needed.
- `CLOUDFLARE_INCLUDE_PROCESS_ENV=true` turns the *whole* process environment into Worker bindings; the harness starts wrangler under `env -i` with an explicit allow-list.
- Fixed ports collide (OrbStack publishes 18080; other sessions run wrangler on 8787): harness picks random free ports and only kills its own process tree.

**workers-rs 0.8.7 / workerd:** request-body drain (above); `websocket_close` must not echo reserved close codes 1005/1006/1015 (`InvalidAccessError`); `uuid` needs a feature-gated RNG on wasm32 (compile_error otherwise) and
`IdGen` as a port keeps core free of it; DO classes must be exported from the same Wasm module as `fetch` (so `thrust-collab-do` cannot be a separate crate in the same Worker; ADR-0090 needs a separate Worker or a re-export);
`worker`'s `Queue` name collides with `thrust_ports::Queue` in glob imports.

**Data layer (Q-0044.1):** `tokio-postgres` 0.7.18 with feature `js` builds for wasm32 and runs over `worker::Socket` via `connect_raw`; **one SQL crate serves both targets**. The Postgres adapter is written at
`tokio_postgres::Client` level with a `Connector` trait (native pool / Hyperdrive socket).
**sqlx (Postgres driver) does not build for `wasm32-unknown-unknown`** **[tested]** (`probes/sqlx-wasm/run.sh`): with no runtime feature the `home` crate (which sqlx-postgres uses to find `~/.pgpass`)
fails with `cannot find function home_dir_inner`; with `runtime-tokio`, `mio` rejects the target ("This wasm target is unsupported by mio. If using Tokio, disable the net feature");
the very first blocker, `getrandom` needing its `js` feature, is trivial and only unmasks these. So sqlx cannot be the shared query layer for the Worker; it remains usable native-only behind the port.
ORMs on top of tokio-postgres (sea-orm, diesel-async) were **not** tried **[unverified]**. **Stage 2 settled the "two round trips" question and found something worse (§10.4 F1) [tested-cf]:**
`Client::query(&str, …)` / `execute(&str, …)` prepares a NAMED statement (`Parse`, `Describe`, `Sync`) and then binds and executes it in a second round trip, and through Hyperdrive that flow
failed on every statement without result columns and intermittently on the rest. The adapter now uses `query_typed` / `execute_typed` (one pipeline on the unnamed statement) for every
statement on both targets; the cost is no server-side statement caching and an explicit Postgres type on every parameter. Libraries built on named prepared statements (sqlx, sea-orm,
diesel-async) are likely to hit the same failure **[unverified]**.

**ADR/QUESTIONS edits for the coordinator (I changed none):**
- ADR-0070: replace `#[async_trait(?Send)]` by `#[async_trait]` (Send futures) and note the Cloudflare adapters wrap with `SendFuture`; note messages are sent as text-encoded JSON.
- ADR-0006 *Confirmation*: add the table in §1, the verdict "conditional", and the reopen triggers.
- ADR-0007 / Q-0007.1 (CF-only bindings in the self-hosted bundle): answered — they are simply not compiled in. `thrust-server`'s dependency graph contains no `worker*`, `wasm-bindgen`, `js-sys` or `web-sys` crate and DO classes exist only in `thrust-worker` (`scripts/check-deps.sh` **[tested]**).
- Q-0006.1: portability is a compile-target property — confirmed for core/ports/router/SQL.
- Q-0044.1: see data-layer paragraph. Q-0070.1: the 128 KB message ceiling was **not** tested here.
- ADR-0046: the sweeper + after-commit fast path both work on both targets; ordering per key is not provided by Queues (unchanged).
- ADR-0081: Worker-root request bodies are buffered (see §4).

## 9. Projected monthly cost (Cloudflare-side pieces exercised; sizing target ≤ 50 users, ≤ 100k work items; envelope < $100/mo)

Prices from the vendor pages read on 2026-10-18 **[doc]**: Workers https://developers.cloudflare.com/workers/platform/pricing/ ·
R2 https://developers.cloudflare.com/r2/pricing/ · Durable Objects https://developers.cloudflare.com/durable-objects/platform/pricing/ ·
Hyperdrive https://developers.cloudflare.com/hyperdrive/platform/pricing/ · Email https://developers.cloudflare.com/email-service/platform/pricing/.

**Assumed workload (mine, stated so it can be challenged):** 50 users × 22 days × ~4,500 API requests/user/day ≈ **5 M requests/month** (the SPA is chatty); 10 ms CPU per request;
the DO rate limiter called on *every* request (pessimistic; the spike only calls it on create); 1 KV session read per request; 500 k queue messages/month; 100 k items × ~200 KB = 20 GB of attachments; cron every minute.

| line | arithmetic | $/month |
|---|---|---|
| Workers Paid plan | base | 5.00 |
| Workers requests | 5.0 M + 43 k cron + 50 k queue batches ≈ 5.1 M < 10 M included | 0.00 |
| Workers CPU | (5 M × 10 ms + 0.9 M + 0.75 M) = 51.7 M ms − 30 M included = 21.7 M × $0.02/M | 0.43 |
| KV | 5 M reads < 10 M included; ~1.1 k writes | 0.00 |
| Queues | 500 k × 3 ops = 1.5 M − 1 M included = 0.5 M × $0.40/M | 0.20 |
| Durable Objects (limiter) | requests 5 M − 1 M included = 4 M × $0.15/M = 0.60; duration ≈ 3 k GB-s ≪ 400 k included; SQLite rows ≈ 10 M written ≪ 50 M included | 0.60 |
| R2 | 20 GB − 10 GB free = 10 GB-mo × $0.015 = 0.15; Class A 100 k < 1 M, Class B 1 M < 10 M free | 0.15 |
| Hyperdrive | included on Workers Paid, queries unlimited | 0.00 |
| Workers Logs | ~10 M events < 20 M included | 0.00 |
| **Total (Cloudflare side, exercised pieces)** | | **≈ $6.38** |

**10× load check** (50 M requests, 5 M queue messages, 200 GB R2): requests (50.5 M − 10 M) × $0.30 = 12.15; CPU (517 M − 30 M) × $0.02 = 9.74; DO requests (50 M − 1 M) × $0.15 = 7.35;
KV reads (50 M − 10 M) × $0.50 = 20.00; Queues (15 M − 1 M) × $0.40 = 5.60; R2 190 GB × $0.015 = 2.85; plan 5.00 → **$62.69**; Workers Logs would add (100 M − 20 M) × $0.60 = $48 unless sampled
(Workers Logs moves to Observability pricing on 2026-12-01 **[doc]**). So the Cloudflare side stays inside the $100 envelope with ~10× headroom if logs are sampled.

**Not priced / not exercised:** the Postgres itself (Hyperdrive needs a reachable origin; this is likely the dominant cost and I have no quote — **[unverified]**), Zitadel, Email Sending (3,000/month included, then $0.35 per 1,000; a digest of 50 users × 22 days × 2 stays inside the included amount; only simulated here),
Workflows (500 k steps/month included; not used at P0), Containers/Workers AI. Real consumption must be reconciled in Stage 2 step S2-7.

## 10. Stage 2 — RAN on 2026-10-18 (real Cloudflare network, Woven account)

Tag **[tested-cf]** = executed against the deployed Worker on Cloudflare's network. Everything tagged **[tested]** elsewhere in this file stays "local workerd / local Postgres".
Every latency figure comes from **one vantage point** (a laptop on the US east coast, one client region); no distant-region run was made. Code: thrust worktree,
`scripts/stage2-contract.sh`, `scripts/stage2-latency.sh`, `crates/thrust-worker/wrangler.stage2.toml`.

### 10.1 Setup used, and where it departed from the runbook

- **Endpoint.** Woven account `<cloudflare-account-id>`, `https://thrust-spike-worker.woven-4d9.workers.dev`. Every resource is `thrust-spike-`-prefixed; the ledger is §10.7.
- **Credentials.** The `thrust-spikes` API token ran Workers, Hyperdrive, Queues, Workflows and Durable Objects. **Its KV reads returned 401 and its R2 and Secrets Store reads 403, probed twice, also after the owner updated it**,
  so KV, R2 and the Secrets Store secret were created and deleted with the owner's wrangler OAuth login (the split the earlier smoke test used). The owner should know the token as issued cannot do those three.
- **Database.** `thrust-pg-dev` (CNPG on woven-o11y, ADR-0094) through a NEW Hyperdrive config `thrust-spike-pg`: the dev VPC service, role `pr_1001` created by the repo's `pr-schema.sh` (connection limit 8),
  schema `pr_1001`, pool 8, caching disabled. The spike's own `0001_init.sql` ran as that role (6 tables). Production `thrust-pg` and the `public` schema were not touched.
- **Worker.** Stage 1's bindings with real ids: Hyperdrive, KV, R2, queue `thrust-spike-events` with a DLQ, the two SQLite Durable Object classes, cron `* * * * *`, Rate Limiting namespace 91001, Secrets Store
  secret `thrust-spike-probe` in the account's default store (empty before), Email, a self service binding, Workflow `thrust-spike-probe`. Vars: `THRUST_TEST_ROUTES=1`, `THRUST_ADAPTERS=cloudflare`,
  `THRUST_CREATE_RATE_LIMIT=30`, `THRUST_RATE_WINDOW_SECS=3600`; `THRUST_OUTBOX_FASTPATH` on, and off for S2-4.
- `THRUST_ADMIN_TOKEN` was generated into a 0600 file and sent as a Worker secret over stdin; the test routes fail closed without it (404 or 401).

### 10.2 Results by runbook step

| Step | Result | Evidence |
|---|---|---|
| S2-0 resources | created: Hyperdrive config, KV namespace, R2 bucket, queue + DLQ, Secrets Store secret, Worker secret | account inventory diff |
| S2-1 schema | applied as role `pr_1001`: 6 tables in schema `pr_1001`, owner `pr_1001` | `information_schema.tables` count and `pg_tables.tableowner` |
| S2-2 deploy | **1,709 KiB raw / 568 KiB gzip** (target < 20 MiB); **Worker Startup Time 2–3 ms** (target < 300 ms) | `wrangler deploy` output, every deploy |
| S2-3 the shared suite | **15/15, then 8/8 on repeat**, after three fixes (F1 adapter, test client TLS, F3 window). Before the window fix: 5 of 7 runs green, both failures the rate-limit test | `stage2-contract.sh main` |
| S2-4 sweeper and cron | sweeper test 1/1; the REAL cron trigger delivered an outbox row **53 s** after the create, with nothing sweeping on demand | `sweeper`, `crontrigger` passes |
| S2-5 Durable Objects | WebSocket + SQLite 1/1; **hibernation observed** (object instance changed across a 14 s idle, sequence and socket attachment survived); the two refusal controls now require a real HTTP 4xx | `worker` pass |
| S2-6 bindings | **5/5**: Rate Limiting (F2), Secrets Store (the real store was read: 25 characters, not the local dummy's 28), service binding, Workflow on the real engine via the JS shim, admin gate. **Email NOT exercised** (F9) | `bindings` pass |
| S2-7 latency | §10.3 | `bench`, `stage2-latency.sh` |
| S2-8 pooling | origin saw a **constant 5** connections of role `pr_1001` in 150 samples across c=1 and c=8, at most 3 active; caching on/off comparison **not done** | `pg_stat_activity` sampled once a second |
| S2-9 poison messages | decodable event with no `project_id`: **4 deliveries (1 + `max_retries` 3), 5–6 s apart, then dead-lettered** (pulled from the DLQ, event id matched). Undecodable body: **not observable**, the producer refuses it with a 422 | `wrangler tail`, DLQ pull |
| S2-10 24 h cost | **not done**; CPU per invocation measured instead (F8) | GraphQL analytics |
| panic policy | panic request gets HTTP 500 and the Worker keeps serving 1/1; blast radius: **1 of 328** concurrent in-flight requests failed around one panic | `panic`, `blast` passes |
| S2-11 teardown | **done and verified**: the account inventory is identical to the one taken before; R2, the Secrets Store and Postgres were checked by name or catalog (after a first pass that failed on a dependency cycle) | §10.7 |
| Stage 1 regression after the adapter change | **16 of 16 cells green** (two re-run after a test rename) | §10.7 |

### 10.3 Numbers [tested-cf]

Client: one US-east laptop. `n` = requests. Provisional thresholds (proposed in the pre-run runbook, owner to confirm): create p95 < 300 ms and list p95 < 150 ms from the origin's region, delivery p95 < 3 s.

| Measure | c = 1 | c = 8 |
|---|---|---|
| POST create work item (n=200) | p50 143 ms · p95 **296** · p99 424 · max 483 | p50 282 · p95 **559** · p99 1,244 · max 1,304 |
| GET list work items (n=100) | p50 68 · p95 86 · p99 132 | p50 56 · p95 75 · p99 87 |
| GET activity (n=100) | p50 62 · p95 89 · p99 160 | p50 47 · p95 65 · p99 148 |
| create → activity visible (n=40, fast path on) | p50 1,392 · p95 1,842 · max 4,877 | p50 1,359 · p95 2,270 · max 5,018 |
| first DB-backed request after a fresh isolate | 615 ms | 595 ms |

- `/healthz` (no database): first request after a redeploy 92 ms; warm n=29 p50 87 ms, p95 139 ms, max 168 ms. The first request is not distinguishable from the warm spread from this vantage, which fits the 2–3 ms startup.
  An idle ≥ 15 minute repeat was not taken: the 1-minute cron keeps the Worker invoked.
- Against the thresholds: create p95 at c=1 is **296 ms, a pass by 4 ms and measured from the laptop, not from the origin's region**; list passes; delivery p95 passes (one outlier of 4.9–5.0 s in each run).
  At c=8 create p95 is 559 ms: list and activity did not move, so the cost is in the create path, most likely waiting on the single `project_sequences` row all creates in one project update **[inferred, not isolated]**.
- **CPU per invocation:** 6,852 invocations in the window: mean **8.35 ms**; successful ones p50 5.75 ms, p99 54.2 ms; 2 `scriptThrewException` (the two deliberate panics). §9 assumed about 10.3 ms (517 M CPU-ms per 50 M requests): in line.

### 10.4 Findings

**F1. Through Hyperdrive, tokio-postgres' default statement flow is unreliable; the single-pipeline typed flow is not.** The first suite run failed 13 of 15: every `/internal/seed` returned a masked 500, and `wrangler tail` showed the
real errors, `db: unexpected message from server` ten times and `db: connection closed` three. A probe route (`/_test/bindings/pgshapes`, kept in `probes.rs`; a fresh connection per case) run 8 times over 18 statement shapes gave:

| Statement flow | Result |
|---|---|
| simple protocol (`SELECT 1`; `BEGIN` / `SELECT` / `COMMIT`; `INSERT` then `ROLLBACK`) | 24 of 24 |
| `execute_typed`: `Parse`, `Bind`, `Describe`, `Execute`, `Sync` in one pipeline on the unnamed statement (an INSERT with a text parameter, a zero-row UPDATE, a zero-row INSERT) | **24 of 24** |
| default `query` / `execute(&str, …)`: a named statement (`Parse`, `Describe`, `Sync`), then `Bind`, `Execute`, `Sync`; the statement returns rows | **4 to 6 of 8 per case**; the rest `UnexpectedMessage`, also for a parameterless `SELECT` (5 of 8) |
| the same default flow, a statement with no result columns (INSERT or UPDATE without `RETURNING`, including zero-row and temp-table cases) | **0 of 40**: `Closed` 34, `UnexpectedMessage` 6 |

Postgres logged no error for any of it (only checkpoints), so the connection was ended upstream of the database **[observed]**. The earlier JS smoke Worker had passed because it wrote to a temp table and `pg` sends a statement with no parameters over the
simple protocol **[inferred]**, so it never exercised the failing flow. The mechanism is **not established [unverified]** and no JS-driver differential was run: whether this is a Hyperdrive defect or an interaction with tokio-postgres is open, and
the probe is a ready repro for a Cloudflare support case. **Adapter change:** every statement goes through a small `Pg` wrapper over `query_typed` / `execute_typed` (one round trip, unnamed statement), each parameter carrying its Postgres type
(`PgType` / `PgParam`); the source is shared by both targets and the local native-pg cell stays green. **Cost:** no server-side statement caching, and an explicit type at every call site. Libraries built on named prepared statements
(sqlx, sea-orm, diesel-async) are likely to hit the same failure **[unverified]**.

**F2. The Rate Limiting binding is approximate.** Limit 3 per 10 s allowed **4** calls before the first refusal, in both runs. Keep the Durable Object limiter wherever the limit must be exact (ADR-0033 / 0082 chose it for that reason); the binding is a coarse abuse guard.

**F3. A fixed window straddled by a slow test.** The window is aligned to the wall clock (60 s by default). The rate-limit test takes about 8 s over real latency, so it crossed a minute boundary some of the time and the 31st create was allowed (201, not 429).
A controlled test confirmed it: started at second :02, 4 of 4 passed; started at :56 so it must cross the boundary, 4 of 4 failed with the same assertion. A one-hour window (`THRUST_RATE_WINDOW_SECS`, already configurable) makes it about 0.2%; 8 of 8 then passed.
A fixed window also allows up to twice the limit across a boundary, which ADR-0033 / 0082 should say.

**F4. Poison messages.** A decodable event the handler rejects permanently (`Invalid`, no `project_id`) is retried `max_retries` times for nothing, then dead-lettered: the consumer does not tell permanent from transient errors. An undecodable message is logged and
acked (dropped, never dead-lettered); that branch could not be driven. A pull consumer on the DLQ read the dead letter back (`attempts` is reset to 0 there).

**F5. The cron sweeper is a one-minute fallback.** With the fast path off an event was delivered 53 s after the create, so the outbox's worst case on the real platform is about a minute plus the queue.

**F6. The origin pool is bounded.** 5 connections for the role at both c=1 and c=8, at most 3 active, against the role limit of 8. The constant 5 is probably Hyperdrive's minimum pool **[unverified]**.

**F7. Worker cold start is not the cost; the first database-backed request is.** About 0.6 s for the first request on a fresh isolate (it opens a Hyperdrive connection, which reaches the origin through the VPC service, the tunnel and `cloudflared`),
against 92 ms for a first `/healthz`; after that a simple read is 56–68 ms (p50) from this laptop. How the 0.6 s splits between the isolate, Hyperdrive's own connect and the tunnel was not measured.

**F8. Cost model holds.** 8.35 ms mean CPU per invocation against 10.3 assumed (§9). The stage's own cost is expected to be negligible: about 7,000 invocations, a handful of queue messages, a small bucket and a Hyperdrive config for roughly two hours **[inferred from the counts; no bill or usage report was read]**.

**F9. Email was not exercised.** The deployed binding rejects a sender domain the account does not own (`email from example.com not allowed because domain is not owned by the same account`), so it is live and enforcing; a real send needs an owned
domain and a verified destination, which were not provided. The local test `…email_send_binding_simulated_locally` is skipped on the deployed target and never counted as a pass.

### 10.5 Harness corrections made during the stage

The test client had no TLS (`reqwest` and `tokio-tungstenite` now use rustls); `ws_connect` mapped only `http://` to `ws://` (now `https` to `wss`); **the two "must fail" WebSocket controls passed vacuously on that scheme error** (they now require an HTTP 4xx
from the Worker); the rate-limit binding test asserts what holds on both targets (first three allowed, a refusal within ten calls) instead of the exact local shape; the Secrets Store test takes its expected length from `THRUST_EXPECT_SECRET_LEN`; a `crontrigger`
pass waits for the real trigger (a deployed Worker has no `/__scheduled`); its test is named `stage2_…` because a `worker_cron_…` name was picked up by the local `cron` pass, which then waited 180 s and failed (found by the regression matrix).

### 10.6 Not done

Hyperdrive caching on versus off (spike 0002's scope); the 24-hour traffic run and cost reconciliation (S2-10); a distant-region bench; an idle ≥ 15 minute cold start; a real Email send; the undecodable-message branch; KV eventual consistency; Durable Object billing;
unwind panic mode; AI Gateway; the root cause of F1.

### 10.7 Resource ledger, teardown and regression

Everything created, and how its removal was verified (an inventory of the account was taken before anything was created and compared afterwards; the final diff was empty):

| Kind | Created | Removed, and checked by |
|---|---|---|
| Postgres | role and schema `pr_1001` on `thrust-pg-dev` | `pr-schema.sh drop 1001` (cascaded to the 6 tables); catalog query: 0 roles, 0 schemas, 0 role-level settings like `pr_%` |
| Hyperdrive | `thrust-spike-pg` | inventory |
| KV | `thrust-spike-sessions` | inventory; the pre-existing `SESSIONS` namespace was not touched |
| R2 | `thrust-spike-blobs` (0 objects at the end) | asking for it by name fails. (`wrangler r2 bucket list` is capped at 20 entries, so a list diff cannot prove an R2 delete.) |
| Queues | `thrust-spike-events`, `thrust-spike-events-dlq`, plus an HTTP pull consumer on the DLQ | inventory |
| Secrets Store | secret `thrust-spike-probe` in `default_secrets_store` (empty before) | the store lists "no secrets" again, as it did before |
| Worker | `thrust-spike-worker`, with its Worker secret, Durable Object storage, cron and Workflow `thrust-spike-probe` | inventory; its URL answers 404. The secret, DO storage and cron went with the Worker **[inferred]** |

**A teardown ordering mistake, corrected.** The first pass deleted the other resources before breaking a cycle: a Worker cannot be deleted while it is a queue consumer, and a queue cannot be deleted while a Worker binds it. The Worker and both queues survived that
pass (exit 1, which was a real failure there), and for a few minutes the Worker ran with dangling bindings and a failing cron. The second pass removed the consumer first. **Order that works: remove the consumer, delete the Worker, delete the queues, then the rest.**

**Stage 1 regression after the adapter change.** The local 16-cell matrix (`scripts/contract-all.sh`) was green in 14 cells; the two `cron` cells failed because the new Stage 2 test matched their `worker_cron_` filter (it waits 180 s for a cron that a local `wrangler dev` never fires).
After renaming it `stage2_…`, both re-ran green (2–3 s). Two additive edits (the `r2purge` probe and the rename) landed while the matrix was running; neither changes an existing route.

## 11. Reproduce (everything is a script in the thrust worktree)

```
scripts/verify-all.sh                          # everything below, serially -> target/verify/summary.txt (needs Docker; ~25 min)
scripts/contract-all.sh                        # the 16-cell matrix (~15 min)
scripts/contract.sh <native-memory|native-pg|worker-memory|worker-cf> <main|sweeper|panic|blast|cron|worker|bindings|bench>
scripts/mutants.sh                             # mutation controls
scripts/check-deps.sh                          # dependency-graph evidence
probes/send-shape/run.sh                       # port-trait shape matrix
probes/sqlx-wasm/run.sh                        # sqlx on wasm32
scripts/rs cargo test -p thrust-adapters-selfhosted --test pg_adapter   # testcontainers
scripts/startup-check.sh wrangler.toml         # wrangler check startup
scripts/flake-hunt.sh <target> <pass> <n>      # repeat a cell
# Stage 2 (needs a deployed Worker built from crates/thrust-worker/wrangler.stage2.toml; see §10):
THRUST_BASE_URL=https://<worker>.<sub>.workers.dev THRUST_ADMIN_TOKEN_FILE=<0600 file> THRUST_EXPECT_SECRET_LEN=<n> \
  scripts/stage2-contract.sh <main|worker|bindings|crontrigger|sweeper|panic|blast|ratelimit>
THRUST_BASE_URL=... scripts/stage2-latency.sh 30          # healthz cold/warm
GET /_test/bindings/pgshapes (admin token)                # the Hyperdrive statement-shape probe behind F1
```

## 12. What is NOT proven

Stage 2 did not cover: a real Email send (needs a sender domain the account owns and a verified destination); AI Gateway; KV eventual consistency; Durable Object billing; any client region other than one
US-east laptop; a 24-hour traffic run and cost reconciliation; Hyperdrive caching on versus off; the undecodable-queue-message branch (the producer API refuses such a body with a 422, so it cannot be injected);
`--panic-unwind`; and the root cause of F1 (no JS-driver differential was run, so it is not known whether the defect is Hyperdrive's or an interaction with tokio-postgres). Also still open from Stage 1:
the production behaviour of the body-cancel bug; ORMs on tokio-postgres (sqlx itself was shown NOT to build, §8); D1; `thrust-collab-do` as a separate crate; the 128 KB queue message ceiling; Valkey/RabbitMQ/S3 adapters
on the native side (in-process stand-ins were used for queue, cache, blob and rate limiter); Dioxus/UI; BetterAuth. The native p95 numbers are for an in-process queue, not RabbitMQ. The `worker-cf` numbers in §6 are
`wrangler dev` on a shared laptop; §10 has the real-platform ones. Statistical weight is small where stated (blast radius: 5 local Worker runs plus one deployed run; F1: 8 probe runs; the dev-server death on the memory set: 2 of 4).
