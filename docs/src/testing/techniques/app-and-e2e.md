# App and end-to-end

Status: **template-only** (axum router tests, Dioxus SSR, in-process harness) · **planned** (browser, desktop,
mobile) · Gate: none

## Catches

X18 (P, once a UI exists), X2 and X20 (B, router tests). Tests the application layers above the library: a route's
request/response mapping without a socket, a component's rendered output, a whole flow through an in-process
server.

## Use when / Not when

| Layer | Use | Test with |
|---|---|---|
| Logic extracted from a handler or component | always first | [example](example-and-parameterized.md) or [property](property.md) tests |
| One route, one request | status codes, extractors, error mapping | `tower::ServiceExt::oneshot` on the `thrust-api` router |
| A Dioxus component | rendered markup and state | `dioxus-ssr`, selecting by `data-testid` |
| A flow across the server and a web build | the wiring | an in-process server on `127.0.0.1:0` (package `e2e`, prefix `e2e_`) |
| A real browser, desktop or mobile shell | what only a browser shows | not covered yet |

Not for HTTP behaviour that must hold on every adapter: that is the [contract suite](contract-suite.md).

## Name and location

Router and component tests in the crate they test. In-process e2e in a package named `e2e`, tests prefixed `e2e_`;
neither the package nor the `e2e` nextest profile exists yet (`.config/nextest.toml` says to add them from the
template when the crate exists).

## Run

```sh
scripts/rs cargo nextest run -p thrust-api      # no tests yet [derived]
```

## Required controls

[Broken implementation](../evidence-and-controls.md#broken-implementation); for markup, assert on a `data-testid`
and a value, not on a class name that a broken render could still contain.

## Owning skill

`.agents/skills/rust-test-app/SKILL.md`. Dioxus application code is `dioxus-07`; visual comparison of a ported UI is
`clone-frontend` (`.agents/skills/clone-frontend/scripts/pixeldiff.py`).

## In-repo examples

None yet. Templates are under `.agents/skills/rust-test-app/references/`.

## Target

Playwright end-to-end and the UI parity smoke set: ADR-0123 and the parity matrix in
`.agents/plans/contrail/TRACEABILITY.md`; whether e2e runs on both adapter sets is Q-0123.2.
