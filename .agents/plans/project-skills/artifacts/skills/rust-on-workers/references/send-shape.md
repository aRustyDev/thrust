# `Send` futures: how one axum router serves native and workers-rs

Evidence: `probes/send-shape/` (standalone crate, `run.sh` compiles every shape x target cell),
`crates/thrust-ports/src/lib.rs` (the rule), `crates/thrust-adapters-cloudflare/src/lib.rs` (the wrap).

## The measured matrix

One port trait, one axum handler; only the trait declaration varies.

| Port declaration | native | wasm32-unknown-unknown |
|---|---|---|
| `#[async_trait]` (futures `Send`) | compiles | compiles |
| `#[async_trait(?Send)]` | **fails** | **fails** |
| `?Send` on wasm32 only (`cfg_attr` switch) | compiles | **fails** |

**[re-run]** From a copy of the probe, offline, rustc 1.99.0 (rustup stable), axum 0.8.9 (the probe's
lockfile): all six cells match the probe's own logs.

The compile error, for grep-ability:

```text
error[E0277]: the trait bound `fn(Extension<Arc<dyn Port>>) -> _ {handler}: Handler<_, _>` is not satisfied
```

Why **[spike]** (the probe's header comment): axum's `Router` stores `Box<dyn ... + Send>`, so every handler
future must be `Send` on **every** target the router is compiled for. If the HTTP layer is shared by
both composition roots, port futures must be `Send` on wasm32 too. A `cfg` switch therefore cannot help.

## The resolution in this repo

1. **Ports are `Send`.** `thrust-ports` documents it as a crate rule (ADR-0002): plain `#[async_trait]`,
   returned futures `Send`, `BoxFuture` is `Pin<Box<dyn Future + Send + 'static>>`, no `worker`/`tokio`.
2. **Adapters that touch JS values wrap the JS-backed part in `worker::send::SendFuture`.** Sound
   because a Worker isolate is single-threaded. Pattern (from the KV adapter):

   ```rust
   async fn get(&self, token: &str) -> Result<Option<Principal>, CacheError> {
       let key = format!("sess:{token}");
       SendFuture::new(async move { self.0.get(&key).json::<Principal>().await.map_err(kv_err) }).await
   }
   ```

3. **The wrap lives in `thrust-adapters-cloudflare` only.** workers-rs also ships a `#[worker::send]`
   attribute macro (documented for axum handlers; present in worker-macros 0.8.7, read from source).
   It cannot be used in the shared router because `thrust-api` may not depend on `worker`
   (`scripts/check-deps.sh` forbids it). The repo does not use the macro anywhere; that follows from the
   dependency rule **[derived]**.

`SendFuture` is `unsafe impl Send` for any `F` (read in worker 0.8.7 `src/send.rs`): it silences the
compiler, it does not check anything. Keep it at the narrowest scope (the JS-touching `async move`
block), not around whole business functions.

## Symptoms when it goes wrong

| Symptom | Cause | Fix |
|---|---|---|
| `Handler<_, _>` not satisfied after adding a port method | port declared `?Send` (or cfg-switched) | declare it plain `#[async_trait]` |
| `error: future cannot be sent between threads safely ... the trait `Send` is not implemented for ...` inside an adapter `impl` | a `!Send` value (JS handle, `Rc`) is held across an `.await` in a `Send`-declared port | wrap the JS-touching block in `SendFuture::new(async move { ... })` |

**[re-run]** Second row: a minimal `Rc`-across-await adapter against a `#[async_trait]` port (scratch
crate, rustc 1.99.0) fails with "future cannot be sent between threads safely ... `Rc<u32>`".

## Per-request construction (related, not a Send issue)

**[spike]** workerd ties a socket and `ctx.waitUntil` to one request, so the Worker root builds the `App` per
request (`build_app` in `crates/thrust-worker/src/lib.rs`) and `HyperdriveConnector` is created per
request. Only the router (`ROUTER: OnceLock<axum::Router>`) and the in-memory adapter state are
process-lifetime. Do not cache a Hyperdrive-backed `App` in a `static`.

## Checklist: adding a port or an adapter

1. Port trait in `thrust-ports`: plain `#[async_trait]`, `Send + Sync` supertraits, no platform import.
2. Native adapter in `thrust-adapters-selfhosted` (or memory adapter in `thrust-adapters-memory`).
3. Worker adapter in `thrust-adapters-cloudflare`, JS-touching blocks inside `SendFuture`.
4. `scripts/check-deps.sh`, then the wasm32 `cargo check` listed under Verify in `SKILL.md`.
5. A contract-suite assertion with a must-fail control (skill `rust-testing`).
