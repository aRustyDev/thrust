---
name: rust-on-workers
description: >-
  Use when Rust must run on Cloudflare Workers (workers-rs, wasm32-unknown-unknown, worker-build,
  wrangler dev) AND natively from the same crates. Fires on "future cannot be sent between threads
  safely" or "Handler<_, _> is not satisfied" (axum + workers-rs), "externref table required for
  catch wrappers", "newest date supported by this server binary" in a Rust Worker, uuid/tokio-postgres
  features on wasm32, Hyperdrive/KV/R2/Queues/Durable Object/cron bindings from Rust, panic=abort vs
  CatchPanicLayer; and, for ANY wasm32 build on macOS where Homebrew rustc shadows rustup, "can't
  find crate for `std`" or "dyld: Library not loaded: @rpath/libLLVM.dylib". NOT for JS/TS Workers,
  generic native Rust, Dioxus app code, or deploying.
allowed-tools:
  - Bash(scripts/check-deps.sh)
  - Bash(scripts/rs cargo check -p thrust-worker --no-default-features --target wasm32-unknown-unknown)
author: aRustyDev
version: 0.1.0
---

# rust-on-workers

Scope: `crates/thrust-worker` and the shared crates behind it. Prerequisites: macOS with rustup stable
and its wasm32 target (Homebrew rustc may also be present), wrangler 4.x, worker-build; this skill
never installs tools. Reference tags (`[re-run]`, `[spike]`, ...) are defined in `references/SOURCES.md`.

## Route by symptom

| You see or need | Read |
|---|---|
| axum `Handler<_, _>` not satisfied; "cannot be sent between threads safely"; adding a port or adapter | `references/send-shape.md` |
| Crate dependency limits; features per target; release profile; externref; panic policy; body drain | `references/build-and-features.md` |
| Hyperdrive, KV, R2, Queues, Durable Objects, cron, Workflow shim | `references/bindings.md` |
| `wrangler dev`, build command, `compatibility_date`, rustc / rust-lld errors | `references/dev-loop-and-toolchain.md` |

## Rules

1. **Port futures are `Send` on every target:** plain `#[async_trait]`, never `?Send`, never a `cfg`
   switch. Wrap JS-backed futures in `worker::send::SendFuture` in `thrust-adapters-cloudflare` only.
2. **Core, ports, contracts, api, adapters-memory carry no runtime or platform crate.**
   `scripts/check-deps.sh` is the gate (ADR-0002, ADR-0007); run it after any `Cargo.toml` edit there.
3. **Platform features live in platform crates** (uuid `v7` + `js`, tokio-postgres `js`), not in
   `[workspace.dependencies]`.
4. **Release profile, panic policy, request-body draining:** `references/build-and-features.md`.
5. **Build wasm through `scripts/rs`** on macOS where Homebrew's rustc shadows rustup (two faults).
6. **workers-rs 0.8.7 provides no way to define a Workflow class;** a Rust-exported class via
   wasm-bindgen is untested. Use the JS shim `crates/thrust-worker/entry.mjs`, no logic in it.

## Verify and report

```sh
scripts/check-deps.sh
scripts/rs cargo check -p thrust-worker --no-default-features --target wasm32-unknown-unknown
```

This proves it compiles, not that it runs under workerd; that is the `worker-cf` cell of the contract
suite (skill `rust-testing`). Report each check with its exit status and what was not run:

```text
wasm32 cargo check -p thrust-worker --no-default-features: exit 0
scripts/check-deps.sh: exit 0
Not run: worker-cf contract cell (needs docker)
```

JS-side Worker concerns: Cloudflare's Workers docs and, if your agent has one, a Workers
best-practices skill.
