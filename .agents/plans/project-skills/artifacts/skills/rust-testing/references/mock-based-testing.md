# Mocks: mockall and wiremock, and when to prefer fakes

> **NOT YET USED IN THIS REPO.** The repo's method is in-memory adapters behind the port traits
> (`adapters-and-flakes.md`). Source: docs.rs pages for **mockall 0.15.0** and **wiremock 0.6.5**, read
> 2026-10-18; snippets are quoted, not compiled. Anything beyond those pages is tagged **[derived]**
> (reasoning from this repo's code) or **[unverified]**; legend in `SKILL.md`.

## Fake or mock: the decision

| | In-memory adapter (this repo) | Mock (`mockall`) |
|---|---|---|
| Verifies | observable behaviour through the real code path | that specific calls happened, in a specific order, with specific arguments |
| Breaks when | behaviour changes | the implementation is refactored, even if behaviour is identical |
| Drift risk | fake diverges from the real adapter (see the rollback-gap example in `adapters-and-flakes.md`) | the mock encodes the test author's *assumption* about the dependency, and nothing compares it with reality |
| Fits | cross-target contract tests, anything stateful | an interaction that **is** the behaviour (a notification was sent exactly once; a retry used the right delay); a boundary with no usable fake |

Default here: fake first, because the contract suite asserts behaviour and runs the same assertions on
every target. Reach for a mock only when the interaction itself is the thing under test. **[derived]**

## mockall 0.15.0

`#[automock]` mocks most traits and structs with a single `impl` block; `mock!` covers what it cannot
(multiple/inherited traits, external traits, multiple impl blocks). The generated struct is the
original name with `Mock` prepended.

```rust
use mockall::*;
use mockall::predicate::*;
#[automock]
trait MyTrait { fn foo(&self, x: u32) -> u32; }

let mut mock = MockMyTrait::new();
mock.expect_foo().with(predicate::eq(4)).times(1).returning(|x| x + 1);
```

Documented behaviour that matters:

- Each expectation needs a **return value** (`return_const`, `returning`, `return_once` for non-`Clone`).
  Only with the `nightly` feature do `Default` returns become implicit.
- **Matching is FIFO:** all expectations on a method are evaluated in order and the first match wins;
  if none match, mockall panics. Put specific expectations before a fallback.
- **Call counts:** by default an expectation may be called any number of times. Use `.times(n)` /
  `.never()` explicitly if the count is part of the assertion.
- **Sequences:** `Sequence::new()` + `.in_sequence(&mut seq)` enforces order across expectations and
  across mock objects. **Checkpoints** (`mock.checkpoint()`) validate and clear expectations mid-test.
- **Static methods and module functions have global expectations**; you must synchronise tests yourself.
- **Mock objects are always `Send`.** For non-`Send` argument or return types there are `withf_st`,
  `return_const_st`, `returning_st`, `return_once_st`, which add runtime checks and panic if used from
  another thread.
- **Async traits:** compatible with native async-fn-in-trait and with `async_trait`, with two limits:
  `#[automock]` must appear **before** `#[async_trait]`, and `#[async_trait]` must be imported by its
  canonical name. Methods returning `impl Future` are mocked with `Box::pin(..)` return values.
- Mocking a concrete struct needs the `mockall_double` crate (`#[double]`) to swap the import path in tests.

### Against this repo's ports **[derived, not tried]**

The ports are `#[async_trait]` traits with `Send + Sync` supertraits, which the documented async rules
accommodate. Keep mockall out of `thrust-ports` itself (the crate must stay free of anything beyond
`thrust-contracts`, `async-trait`, `thiserror`, `futures-util`; `scripts/check-deps.sh` guards runtime
and platform crates): put `mock!` blocks in the test crate that needs them.

## wiremock 0.6.5 (HTTP)

For black-box tests of code that calls a third-party HTTP API (the workspace has no such caller today).

```rust
let mock_server = MockServer::start().await;          // random local port
Mock::given(method("GET")).and(path("/hello"))
    .respond_with(ResponseTemplate::new(200))
    .mount(&mock_server).await;
```

- A request that matches no mounted `Mock` gets a **404**, which is a free negative control.
- **Spying:** `Mock::expect(n)` sets a call-count expectation, verified when the `MockServer` is dropped
  at the end of the test; a failed verification panics. By default no expectations are set.
- **Isolation:** one `MockServer` per test; do not share them. It is shut down when it goes out of scope.
- Custom matching with the `Match` trait or `Fn` closures; dynamic replies with the `Respond` trait.
- Works with `async_std` and `tokio`. Its own comparison table: mockito is sequential and mocks one API;
  httpmock is parallel, unbounded, sync or async, and has a standalone mode; wiremock is parallel,
  unbounded, async only, no standalone mode.
- The docs page's install snippet still says `wiremock = "0.5"` while the crate on the same page is
  0.6.5: use `cargo add wiremock --dev`, not the pasted version.
- The server runs inside the test process. A Worker under `wrangler dev` calling a localhost mock was
  **not tried** here: **[unverified]**.

## The control still applies

A mock-based test can pass vacuously (an expectation never reached, a `.times` left unbounded). Make the
count part of the assertion, and prove the test fails with the behaviour removed
(`must-fail-controls.md`).
