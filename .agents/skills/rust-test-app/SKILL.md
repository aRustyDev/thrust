---
name: rust-test-app
description: >-
  Use when testing app layers above a Rust library: axum router tests with `tower::ServiceExt::oneshot` ("test axum handler", `no method named oneshot`), Dioxus 0.7 component tests with `dioxus-ssr` (`VirtualDom::new_with_props`, `rebuild_in_place`, `render_immediate`, `data-testid`), an in-process server on `TcpListener::bind("127.0.0.1:0")` serving the router and a web build (`E2E_WEB_DIST`), or "which layer should this test live in". NOT for Dioxus app code (dioxus-07), dx-components (dioxus-design-system), Blitz (blitz-native), porting a UI (clone-frontend), logic, fuzz or mutation tests (rust-test-logic, -fuzz, -mutation), JS tests, HTTP assertions shared across adapters, or browser and desktop/mobile tests (not covered yet).
license: AGPL-3.0-only
author: aRustyDev
version: 0.1.0
---

# rust-test-app

Pick the lowest layer that can catch the bug: many router and component tests, one browser test per critical flow. Dioxus pinned to 0.7.x; re-check on 0.8 (alphas).

| Changed | Test | Read |
|---|---|---|
| view or business logic | plain test of an extracted function | `rust-test-logic` |
| handler, extractor, status, JSON shape | router test, `oneshot`, no socket | `references/api-router-tests.md` |
| rules across API call sequences | model test | `rust-test-logic` |
| what a component renders per state | Dioxus SSR test, no browser | `references/dioxus-component-tests.md` |
| something needs a real URL (SDK client, static build) | in-process harness | `references/harness.md` |
| a user flow in a real browser | not covered here yet | none |
| desktop or mobile shell | not covered here yet | none |

Templates to copy: `references/examples/`.

Not covered yet: browser (playwright-rs) and desktop/mobile shells; prove them on your CI before relying on them.

## Ground rules
- Run with cargo-nextest in crates written for it; a suite that parses plain `cargo test` output stays on it; doctests: `cargo test --doc`.
- Harness tests live in package `e2e` (the routing key), named `e2e_`. Never pass `--retries`, `--flaky-result`, `NEXTEST_RETRIES` or `NEXTEST_FLAKY_RESULT`: `[re-run]` a per-test `flaky-result = "fail"` is only a default; `--flaky-result pass` overrides it, so a retried failure goes green.
- Select by `data-testid` written in `rsx!`, never class or text. Never sleep.
- Each test owns its data: fresh state or `TestApp`; never a fixed port.
- Shared router crates stay runtime-free (axum's defaults pull in tokio): server crates go in the harness crate (`references/api-router-tests.md`).
- Keep logic out of `rsx!`: macro bodies are not mutation-tested `[zip]`.
- A failing test is a finding: do not delete, weaken, `#[ignore]`, accept a snapshot unread or lengthen a timeout.
- `dx`, `cargo install` and toolchains are the user's call: propose and wait. Never claim a `dx` run you did not make.
- A check that cannot fail is not evidence: break the handler or markup once and watch it go red.

## Procedure
1. Pick the layer; move decisions out of handlers and `rsx!` first.
2. Router: `pub fn router(state) -> Router`, injected state, `oneshot`, status then JSON as `Value`.
3. Component: `render_with` flush loop, assert on `data-testid`, fakes via context.
4. Harness only if something must connect; the UI build is the user's `dx` step.
5. Run under nextest; show the control red; report.

## Output contract
Per test: layer, file, command, pass count, the control that failed, what was not run. Tags: `[re-run]` run and seen (evidence: `references/SOURCES.md`); `[zip]` recorded by the source's author, not re-run; `[derived]` reasoned from docs or code; `[unverified: <tool> not installed]`.

## Example
Q: "added GET /api/items/{id}, do i spin up the server on port 3000?" A: no socket.
```rust
let resp = router(state).oneshot(Request::builder().uri("/api/items/7").body(Body::empty()).unwrap()).await.unwrap();
assert_eq!(resp.status(), StatusCode::OK);
```
Needs `use tower::ServiceExt` and a `tower` dev-dependency. Control: break the handler, not the assertion; it must fail.
