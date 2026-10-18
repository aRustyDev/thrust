# Router-level tests (axum)

Pinned to axum 0.8, tokio 1, tower 0.5. `[re-run]` on the archive's original files and on the shipped
`examples/api_example.rs` (the original plus a one-line header, which also holds the full set of rstest
cases and a fixture): the router tests below pass, including `oneshot`, the status-code tables, rstest
cases and fixtures. Environment and logs: `SOURCES.md`. Example names (`Item`, `AppState`) are
placeholders.

## Setup

```toml
[dev-dependencies]
tower = { version = "0.5", features = ["util"] }      # ServiceExt::oneshot [re-run]
serde_json = "1"
rstest = "0.26"                                        # status-code tables; optional [re-run]
tokio = { version = "1", features = ["macros", "rt"] } # #[tokio::test] [derived]
```

The source's own `Cargo.toml` listed `tokio` as a normal dependency, not the dev-dependency above; see
"Dependency hygiene". The `tokio` line here is the minimal dev-dependency form and was not itself built.

The error ``no method named `oneshot` found for struct `Router` `` (E0599) means `use tower::ServiceExt;`
is missing. axum 0.8.9 already enables tower's `util` feature, so a missing feature would show up as an
unresolved import (E0432) instead, and the `features = ["util"]` above is explicit but redundant
`[derived]` (read from axum 0.8.9's `Cargo.toml`, not built).

Expose the app as `pub fn router(state: AppState) -> Router` so a test builds it with injected state:
fake clocks, in-memory stores, test doubles for outbound clients. Anything added with `.layer(..)` in
`main` is invisible to a router test: move it into `router()` if a test should cover it `[derived]`.

## No socket: `oneshot`

```rust
use axum::body::Body;
use axum::http::{Request, StatusCode};
use tower::ServiceExt; // for `oneshot`

async fn get(app: Router, uri: &str) -> (StatusCode, String) {
    let resp = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8(bytes.to_vec()).unwrap())
}

#[tokio::test]
async fn get_item_returns_json() {
    let state = AppState::default();
    state.items.write().unwrap().insert(7, "widget".into());
    let (status, body) = get(router(state), "/api/items/7").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, r#"{"id":7,"name":"widget"}"#);
}
```

`oneshot` consumes the service: `app.clone().oneshot(req)` for a second call on the same router
(`Router` is `Clone`) `[derived]`. Route paths in axum 0.8 are written `/api/items/{id}`: the sample's
`{id}` routes served requests in the re-run on axum 0.8.9 `[re-run]`.

## JSON requests and bodies

```rust
let req = Request::builder()
    .method(Method::POST)
    .uri("/api/items")
    .header("content-type", "application/json")
    .body(Body::from(serde_json::json!({ "name": "a" }).to_string()))
    .unwrap();
let resp = app.clone().oneshot(req).await.unwrap();
```

Compare response bodies as `serde_json::Value` (`serde_json::from_slice(&bytes)`) unless the exact
bytes are the contract (a signature over the body, a documented wire format). The string comparison in
`examples/api_example.rs` pins bytes, including field order: fine for a sample, stricter than most
endpoints need.

## Status-code tables

rstest cases give one test per row with a readable name (`#[case::missing(8, StatusCode::NOT_FOUND)]`),
`#[values(..)]` runs a matrix, `#[fixture]` supplies fresh state. Put `#[rstest]` above
`#[tokio::test]`, as in the example. A table whose cases all expect `OK` cannot fail: include the
rejected neighbour in every table (`7` found, `8` and `6` not).

## Sockets: only when something must connect

A browser or an SDK's real HTTP client needs a URL; a handler test does not. For those, use the
in-process harness (`references/harness.md`): bind `127.0.0.1:0`, serve on a spawned task, abort on
drop. **Never use fixed ports**: nextest runs tests in parallel processes, so a fixed port collides
`[zip]`.

## Dependency hygiene

The source's sample `api` crate lists `tokio` as a normal dependency: treat it as a pattern, not a
model.

- `tokio`, `tower-http` (with `fs`), `axum::serve` and other server crates belong in the **test or
  harness crate**.
- axum's own default `tokio` feature enables tokio's `net`, `rt` and `macros` `[derived]` (read from
  axum 0.8.9's `Cargo.toml`, not built), so a router crate that takes axum with default features already
  depends on a runtime. A shared router crate that must also run in a Worker, or in any runtime-free
  context, takes `axum = { version = "0.8", default-features = false, features = [..] }` with only the
  features the router uses; the harness crate takes axum with its defaults. Inspect the effect with
  `cargo tree -p <router-crate> -e normal` and look for `tokio` and `tower-http`. Build the Worker target
  with `-p <router-crate>`: a workspace-wide build unifies the harness crate's features into it
  `[derived]`.
- **The dependency guard** means a CI step or script that fails when a runtime crate such as `tokio`
  appears in `cargo tree -e normal` for the shared router crate. Read it before adding any dependency,
  and never relax it to make a test compile: a failing guard is a finding. If the repo has none, run the
  `cargo tree` command above by hand before and after the change and propose adding the check; do not
  invent one unasked.
- Dev-dependencies are fine (the block above), unless the guard also counts them: read it.

## Not here

- Rules that must hold across sequences of API calls (ids never reused after a delete): a `model_`
  test, owned by `rust-test-logic`; `oneshot` is only how it calls the router.
- Replaying exchanges recorded from a TypeScript or Python reference API: `ts-to-rust` owns freezing,
  normalising ids and timestamps, and the exception register. The source replays through `oneshot`
  from a `parity_` test `[zip]`, which avoids a socket.
- A suite that runs the same HTTP assertions against every adapter: a different job and a different
  skill; do not grow router tests into it.
