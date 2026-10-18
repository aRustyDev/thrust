# In-process harness: `TestApp`

For tests that need a real URL: an SDK's HTTP client, a deep-link check against the static web build,
and (not covered yet) a browser. A handler test does not: use `oneshot`
(`references/api-router-tests.md`). Code: `examples/harness_lib.rs`, its test
`examples/e2e_harness_test.rs`, the env block `examples/cargo-config-env.toml`. Example names
(`api`, `e2e`) are placeholders. `[re-run]` on the archive's original files:
`e2e_harness_serves_ui_and_api` passes under `--profile e2e` with `E2E_WEB_DIST` pointing at the stub
dist. Environment and logs: `SOURCES.md`. `harness_lib.rs` is the original plus a one-line header;
`e2e_harness_test.rs` differs by one renamed deep-link path (`/todos/42` to `/entries/42`) and a header.
The shipped copies, with `api_example.rs` and `ui_components_example.rs`, were also compiled and run
over the archive workspace's crates: default profile 31 passed and 1 skipped, `--profile e2e` 1 passed
`[re-run]`.

## What it does

`TestApp::start().await` serves **the real router and a built static web UI** from one in-process axum
server:

- binds `TcpListener::bind("127.0.0.1:0")`, so the OS picks a free port: no fixed ports, safe under
  nextest's parallel processes;
- `api::router(state).fallback_service(ServeDir::new(&dist).fallback(ServeFile::new(dist.join("index.html"))))`:
  API routes first, then files, then `index.html` for any other path so client-side routes deep-link;
- `axum::serve` runs on a spawned task; `Drop` aborts it, so each test's server dies with the test;
- exposes `base_url` (for example `http://127.0.0.1:41873`) and `state` for seeding.

```rust
let app = e2e::TestApp::start().await;                       // fresh server, fresh state
app.state.items.write().unwrap().insert(1, "seeded".into()); // seed directly, not by clicking
// point the SDK client, or reqwest, at app.base_url
```

Each test starts its own `TestApp` and seeds through `state` or API calls, so no test depends on
another's data.

## Where it lives

In its own harness crate (`e2e` in the source, `publish = false`), not in the router crate. Normal
dependencies of the harness crate: the router crate, `axum`, `tokio` (`macros`, `rt-multi-thread`,
`net`), `tower-http` with `fs`; dev-dependency `reqwest` with `default-features = false`. These resolved
and built as `tower-http` 0.7.1 and `reqwest` 0.13.5 in the re-run `[re-run]`; confirm current versions
when you add the dependency. This is the dependency-hygiene rule from
`references/api-router-tests.md` applied: a router crate that must run in a Worker takes none of these.

## Pointing it at the web build

The harness reads `E2E_WEB_DIST` and asserts `index.html` exists there, with messages
`E2E_WEB_DIST must point at the built web UI` and `web UI not built at <path>`. Set it for every test
with `.cargo/config.toml`:

```toml
[env]
E2E_WEB_DIST = { value = "target/dx/<app>/debug/web/public", relative = true }
```

`relative = true` resolves against the directory that holds `.cargo/`. nextest passes `[env]`
variables to tests `[zip]`. The block above does not use `force`, so **a variable already set in the
environment beats it**: with `E2E_WEB_DIST=/nonexistent` exported, the harness test failed (exit 100)
even though `.cargo/config.toml` pointed at a good stub `[re-run]`. That is useful (CI can point
elsewhere) and a trap (a stale exported value silently wins): when the harness cannot find `index.html`,
print the variable before editing the config.

**Point it only at the build output directory.** The harness serves the whole subtree under
`E2E_WEB_DIST` (`ServeDir`), and an exported variable silently wins over the config, so a path set too
high (a repository root, a home directory) serves every file beneath it on `127.0.0.1` for the length of
the test `[derived]`.

**Building the UI is the user's call.** The source builds with `dx build --platform web` and finds the
debug output under `target/dx/<app>/debug/web/public` (release under `.../release/...`) `[zip]`
`[unverified: dx not installed]`. `dx` can be Deno's npx-style shim on a machine with Deno: run
`dioxus-07`'s preflight (`dx --version` must print `dioxus 0.7.x`) before anyone runs it, propose the
command and wait, and never report a `dx` command as run unless it was.

Before any web build exists, the harness can be proved against a one-line stub (`[zip]`,
`crates/e2e/test-dist/index.html` in the source):

```html
<!doctype html><html><body><div id="main"></div></body></html>
```

A stub proves the harness. It says nothing about the real UI.

## Gotchas

- **The SPA fallback hides API 404s.** An unknown `/api/...` path that matches no route falls through
  to the static service and returns `index.html` with status 200 `[derived]`. Assert error statuses on
  routes that exist, and test unknown-route behaviour with `oneshot` on `router(state)`, where it is a
  real 404.
- **A profile can select zero tests, and then nextest exits 4.** The source's nextest config keeps
  `package(e2e)` out of the default profile (`default-filter`) and runs it under `--profile e2e`: the
  default profile ran 31 tests and skipped 1, the `e2e_` harness test `[re-run]`. A `-E` filter is
  intersected with the profile's default filter, so `cargo nextest run -E 'package(e2e)'` under the
  default profile printed `no tests to run` and exited 4 `[re-run]`; `-p e2e` should behave the same
  `[derived]`. Remedy: run the profile that selects it, `--profile e2e` (it passed, 1 test) `[re-run]`;
  `--ignore-default-filter` also exists `[derived: --help]`, but naming the profile is clearer. A run of
  zero tests is not a pass: check the reported count.
- **Retries.** The source's config retries `package(e2e)` twice, which was meant for browser tests. Under
  it, a deterministic failure of the hermetic harness test was attempted three times (`TRY 1 FAIL`,
  `TRY 2 FAIL`, `TRY 3 FAIL`) `[re-run]`: a retry only delays a real failure and would hide a flaky pass.
  Retries, if any, belong to browser tests only; none exist yet. Never pass `--retries`,
  `--flaky-result`, `NEXTEST_RETRIES` or `NEXTEST_FLAKY_RESULT`: a per-test `flaky-result = "fail"` in
  `nextest.toml` is only a default, and `--flaky-result pass` (or the environment variable) overrides it,
  so a retried failure goes green `[re-run]`. Treat a FLAKY report on a harness test as a bug in the test.
- **Routing is by package, not by prefix.** The profiles select the harness test with `package(e2e)`
  (`--profile e2e` runs nothing else): keep the harness crate named `e2e`, or change the filters when you
  rename it. The `e2e_` test-name prefix (`e2e_harness_serves_ui_and_api`) is a naming convention that
  matches the package; the `e2e` filters in the source's config do not read it `[zip]`.

## Control for the harness test

`examples/e2e_harness_test.rs` checks the UI, an API route and the deep-link fallback. To show it can
fail, export `E2E_WEB_DIST=/nonexistent` for one run: the test must fail, and in the re-run it did, with
exit 100 `[re-run]`. The panic was the harness's `index.html` assertion, `web UI not built at
/nonexistent` `[re-run]`. Changing the deep-link expectation in the test to 404 also made it fail `[re-run]`. A
harness test that still passes there is not testing the harness. Unset the variable afterwards.
