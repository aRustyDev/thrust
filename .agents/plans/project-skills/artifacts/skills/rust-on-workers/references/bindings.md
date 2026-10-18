# Bindings from Rust (workers-rs 0.8.7)

Evidence: `crates/thrust-adapters-cloudflare/src/lib.rs`, `crates/thrust-worker/src/{lib,durable,probes}.rs`,
`crates/thrust-worker/entry.mjs`, `crates/thrust-worker/wrangler.toml`,
`crates/thrust-contract-tests/tests/worker_only.rs`. All behaviours below are **[spike]** (recorded and
exercised by the contract suite on local workerd) unless tagged otherwise. Config semantics of each
binding are Cloudflare's: read their docs for those, not this file.

`crates/thrust-worker/wrangler.toml` carries placeholder all-zero ids and **no secrets**; real ids and
secrets are supplied at deploy time. The local Postgres URL reaches Hyperdrive through the
environment, never through `localConnectionString` in the file.

## Map

| Binding | Rust entry point | Where | Gotcha |
|---|---|---|---|
| Hyperdrive (Postgres) | `env.hyperdrive("HYPERDRIVE")` | `HyperdriveConnector` | per-request; see below |
| KV | `env.kv("SESSIONS")` | `KvSessionStore` | TTL minimum 60 s |
| R2 | `env.bucket("BLOBS")` | `R2BlobStore` | bytes via `worker::Data::Bytes` |
| Queues producer | `env.queue("EVENTS")` | `CfQueue` | send JSON **text**, not a struct |
| Queues consumer | `#[event(queue)]` | `thrust-worker/src/lib.rs` | at-least-once: consumer must be idempotent |
| Durable Objects (SQLite) | `#[durable_object]`, `env.durable_object(..)` | `thrust-worker/src/durable.rs` | classes must live in the Worker's wasm module |
| Cron | `#[event(scheduled)]` | `thrust-worker/src/lib.rs` | local trigger via `--test-scheduled` |
| Rate Limiting, Secrets Store, send_email, service binding | `env.rate_limiter`, `env.secret_store`, `env.send_email`, `env.service` | `probes.rs` | native wrappers exist in 0.8.7 |
| Workers AI | `env.ai("AI")` | `probes.rs` | compile-checked only; `wrangler dev --local` cannot run it |
| Workflows | trigger through `js_sys::Reflect`; the class cannot be defined in Rust | `entry.mjs` + `probes.rs` | JS shim, see below |

## Hyperdrive and Postgres

`hd.connect()` gives a `Socket`; `tokio_postgres::Config` is filled from `hd.host()/port()/user()/
password()/database()` and `connect_raw(socket, NoTls)` returns `(client, connection)`. The connection
driver is spawned with `wasm_bindgen_futures::spawn_local`. workerd ties a socket to the request that
opened it, so the connector (and the `App` holding it) is built per request. Reads share one
lazily-opened connection; a transaction asks for an `exclusive` checkout and gets its own.
Local dev: `CLOUDFLARE_HYPERDRIVE_LOCAL_CONNECTION_STRING_HYPERDRIVE` (suffix = binding name), set by
`scripts/worker-dev.sh` from `DATABASE_URL`.

## Queues

Producer: build `worker::RawMessageBuilder::new(JsValue::from_str(&text))`, optional `delay_seconds`
(clamped to 86 400), `build_with_content_type(QueueContentType::Text)`, `queue.send_raw(..)`.
Consumer signature: `async fn queue(batch: MessageBatch<String>, env: Env, _ctx: Context) -> Result<()>`.
Per message: decode the envelope, `msg.ack()` on success, `msg.retry_with_options(&QueueRetryOptionsBuilder::
new().with_delay_seconds(5).build())` on handler error, and `ack` an undecodable message so a poison
message cannot loop (a real deployment routes it to a dead-letter queue). The Worker consumer and the
native consumer call the same `process_event` body (ADR-0070). The contract suite injects the same
envelope twice through `/internal/inject-event` to prove redelivery is processed once.

## Durable Objects with SQLite

- `wrangler.toml`: `[[durable_objects.bindings]]` plus `[[migrations]] new_sqlite_classes = [...]`.
- `#[durable_object] pub struct X { state: State }`; `impl DurableObject for X { fn new(state, env) ... }`.
  The constructor is synchronous and so is SQLite DDL: `CREATE TABLE IF NOT EXISTS` keeps it idempotent.
- SQL: `state.storage().sql().exec(sql, Some(vec![v.into()]) | None)` then `.to_array()` / `.one()`.
- Stub: `env.durable_object("ROOM")?.get_by_name(name)?` then `stub.fetch_with_request(..)`.
- Rate limiter: one DO instance per key (ADR-0033, ADR-0082).
- WebSocket hibernation: `state.accept_web_socket(&pair.server)`, `serialize_attachment` /
  `deserialize_attachment`, and the `websocket_message` / `websocket_close` / `websocket_error` handlers.
  When the peer vanished, close codes 1005/1006/1015 are *reported* to you but must not be echoed:
  measured, echoing 1006 throws `InvalidAccessError`; answer with 1000.
- Hibernation is observed, not asserted: the test sleeps past the idle eviction (~10 s), then checks the
  SQLite sequence and socket attachment survived and prints whether the object was re-instantiated.
- DO classes must be exported from the same wasm module as `fetch`, so the spike keeps them in
  `crates/thrust-worker`; the file header records that the architecture reserves a dedicated crate.

## Cron

`[triggers] crons = [...]` in `wrangler.toml`; handler `async fn scheduled(event: ScheduledEvent, env:
Env, _ctx: ScheduleContext)`. Locally, `wrangler dev --test-scheduled` exposes `/__scheduled?cron=...`;
the contract test calls it, after proving the outbox event was *not* already delivered (otherwise the
test would be vacuous). Cron sweeper design: ADR-0046, ADR-0072.

## Workflows: the JS shim

workers-rs 0.8.7 provides no way to define a Workflow class (`WorkflowEntrypoint` is a JS class); a
Rust-exported class via wasm-bindgen is untested. `crates/thrust-worker/entry.mjs` re-exports the Rust
Worker unchanged
(`export * from "./build/worker/shim.mjs"; export default rust;`) and adds the one class. Contract
written in the file header:

- Rust triggers it by calling `env.WORKFLOW.create({ params })` through `js_sys::Reflect` (see
  `workflow()` in `probes.rs`); poll `status()` for completion.
- The class only sequences `step.do` calls and calls **back into** Rust through the `SELF` service
  binding. **No business logic in the shim.**
- Exit: replace it if workers-rs gains Workflow support, or if a Rust-exported class via wasm-bindgen
  is shown to work (untested).
- `wrangler.toml` points `main = "entry.mjs"`; the memory-only config (`wrangler.memory.toml`) points
  `main = "build/worker/shim.mjs"` and has no shim.

## Test routes

`/_test/room/*` and `/_test/bindings/*` are answered by the Worker root before the router, require
`THRUST_TEST_ROUTES` on **and** a constant-time-compared `x-admin-token`, and answer 404 otherwise.
They exist so the contract suite can say "this binding was exercised", not "this binding compiles".
Never enable them in a real deployment.
