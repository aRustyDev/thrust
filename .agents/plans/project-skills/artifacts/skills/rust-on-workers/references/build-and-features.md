# Build, features per target, release profile, panic policy

Evidence: root `Cargo.toml`, per-crate `Cargo.toml` files, `scripts/check-deps.sh`,
`crates/thrust-server/src/main.rs`, `crates/thrust-worker/src/lib.rs`. Tags: **[re-run]** reproduced
locally from a copy of the spike tree, offline; **[spike]** recorded by the spike, not re-run;
**[derived]** reasoned from this repo's code, not tried. Detail: `SOURCES.md`.

## 1. Who may depend on what

`scripts/check-deps.sh` enforces it (ADR-0002, ADR-0007). It checks, with `cargo tree -i` on all targets:

- `thrust-contracts`, `thrust-ports`, `thrust-core`, `thrust-api`, `thrust-adapters-memory` contain none of:
  tokio, tokio-util, tokio-postgres, mio, socket2, hyper, hyper-util, tower-http, worker, worker-sys,
  wasm-bindgen(-futures), js-sys, web-sys, deadpool-postgres, lapin, sqlx.
- The wasm32 graph of `thrust-worker --no-default-features` lacks deadpool-postgres, hyper, mio,
  socket2, tower-http, `thrust-adapters-selfhosted`, `thrust-server`.
- The native `thrust-server` graph lacks worker, worker-sys, wasm-bindgen, js-sys, web-sys,
  `thrust-adapters-cloudflare`, `thrust-worker`.

**[re-run]** The guard exits 0, and prints that `tokio` *is* in the Worker graph, pulled in by
`tokio-postgres` and `worker` itself. That is not a violation: the invariant is "no runtime in the clean
crates", not "no tokio anywhere in wasm". Do not "fix" that line.

**Guard gap:** the script treats `did not match any packages` as "absent". A misspelled name in its
`FORBIDDEN` list therefore passes forever. **[re-run]** Positive control: `scripts/rs cargo tree -p
thrust-server -e normal --target all -i tokio` must print a tree; if it prints the "did not match" error
the query shape, not the code, is broken. A mistyped `-p` subject is *not* silent: with `-i`, cargo
ignores the unknown package and prints the whole-workspace tree, so the guard reports a hit.

## 2. Features per target

| Concern | Where | Rule |
|---|---|---|
| uuid generators | root `[workspace.dependencies]`: `default-features = false, features = ["serde","std"]` | No generator features at the root: id generation is the `IdGen` port. The Cloudflare crate adds `v7` + `js` (RNG via `crypto.getRandomValues`); selfhosted adds `v7`. |
| chrono clock | `thrust-contracts`: no `clock` feature | `Utc::now()` does not exist there; time comes from the `Clock` port. `thrust-adapters-selfhosted` turns `clock` on. |
| tokio-postgres | `thrust-adapters-postgres`: `default-features = false, features = ["js", "with-uuid-1", "with-chrono-0_4", "with-serde_json-1"]` | `js` makes it build for wasm32. `runtime` (TCP) comes only from the `native` feature, never enabled for wasm32. |
| axum | `thrust-api` and `thrust-worker`: `default-features = false, features = ["json","query","matched-path"]` | Drops tokio/http1/tracing/form. Only `thrust-server` adds `tokio` + `http1`. |
| worker | `worker = { version = "0.8", features = ["http", "queue"] }` | `http` gives `worker::request_from_wasm` / `response_to_wasm`; `queue` gives the consumer types. |
| adapter set | `thrust-worker`: `default = ["memory"]` | `--no-default-features` builds the Cloudflare-only bundle; that is the one to size and ship. |
| crate type | `thrust-worker`: `crate-type = ["cdylib"]` | Required for the wasm module. |

The Worker's `THRUST_ADAPTERS` var only selects between sets that were compiled in; asking for one
that was not compiled returns an error naming it.

**sqlx does not build for wasm32.** **[re-run]** The spike's sqlx probe (`probes/sqlx-wasm`), postgres
feature and no runtime feature (sqlx 0.8.6), fails in `home 0.5.12` (`cannot find function
home_dir_inner`). The `--features tokio` configuration was not run. Use `tokio-postgres` with `js` over
Hyperdrive.

**Build and check commands:**

```sh
scripts/rs cargo check -p thrust-worker --no-default-features --target wasm32-unknown-unknown
scripts/rs worker-build --release . --no-default-features    # from crates/thrust-worker
```

**[re-run]** Both exit 0 from a copy of the spike tree (rustc 1.99.0, worker 0.8.7, worker-build 0.8.7,
wasm-bindgen CLI 0.2.129). `cargo check` does not exercise wasm-bindgen output; the bundle command does.

## 3. Release profile (root `Cargo.toml`)

```toml
[profile.release]
opt-level = "s"
lto = true
codegen-units = 1
strip = "debuginfo"   # NOT `strip = true`
# no `panic = ...` on purpose
```

- **`strip = true` breaks the bundle.** It drops the wasm `target_features` section, so wasm-bindgen does
  not enable externref (the explanation is the spike's **[spike]**). **[re-run]** With the profile as
  shipped, `worker-build` exits 0; changing only `strip` to `true` makes it exit 1:

  ```text
  error: failed to generate catch wrappers

  Caused by:
      externref table required for catch wrappers
  Error: Running the wasm-bindgen CLI
  ```

- **No `panic` key.** **[re-run]** `rustc --print target-spec-json` for `wasm32-unknown-unknown`
  (rustc 1.99.0, via `RUSTC_BOOTSTRAP=1 -Zunstable-options`) reports `"panic-strategy": "abort"`, so the
  Worker aborts on panic whatever the profile says. A workspace-wide `panic = "abort"` would also
  remove unwinding from the native server and silently disable `CatchPanicLayer` (`tower-http` feature
  `catch-panic`, wired in `crates/thrust-server/src/main.rs`).

### What a panic does on each target

- Native: caught per request, answers 500 with the standard error body.
- Worker: `console_error_panic_hook::set_once()` logs it (called in the `fetch` and `scheduled`
  handlers); the isolate is reset. State that lives in the isolate (the `memory` adapter set) is lost;
  durable state (Postgres, KV, R2, DO) survives. **[spike]** The contract suite's panic test runs alone
  against a fresh target because a panic reset killed parallel tests.

## 4. Behaviours of the Worker root that cost time (all **[spike]**)

- **Drain the request body before routing.** `Limited::new(body, 8 MiB).collect()` then rebuild the
  request. Without it, handlers answering early (401/403/404/409) drop the body after the response is
  sent and workerd raises `TypeError: Can't read from request stream after response has been sent`;
  `wrangler dev` then resets other in-flight connections (roughly 1 in 4 suite runs failed). Cost:
  bodies are buffered, so the Worker root has no streaming uploads.
- **Queue messages go as JSON text** (`RawMessageBuilder` + `QueueContentType::Text`), via
  `encode_envelope`. Sending the struct directly passes through serde-wasm-bindgen, which turns
  `serde_json::Value` objects into JS `Map`s that `JSON.stringify` renders as `{}`: silent payload loss.
- **KV rejects `expirationTtl < 60`**; the adapter clamps with `ttl_secs.max(60)`.
- **`Date` is frozen during pure CPU work in workerd**, which `JsClock` accepts.
- **Secrets and vars:** read through `env.secret(name)` then `env.var(name)`; never from a file.
