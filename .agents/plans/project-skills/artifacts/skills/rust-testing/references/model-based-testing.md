# Model-based testing: proptest + proptest-state-machine (and stateright)

> **NOT YET USED IN THIS REPO.** Source: docs.rs pages for **proptest 1.12.0**,
> **proptest-state-machine 0.9.0** and **stateright 0.31.0**, and the proptest book chapter "State
> Machine testing" (it states no crate version), read 2026-10-18. Snippets are quoted, not compiled.
> Anything not on those pages is tagged **[unverified]**; legend in `SKILL.md`.

## When it pays off

The proptest book: state-machine testing "automates the checking of properties of a system under test
(SUT) against an abstract reference state machine definition", finds a counter-example, and shrinks it
to a minimal sequence of transitions. Its fitting examples include "an API for a database" and
"interactions between a client(s) and a server". There is "some initial investment needed" and it
"usually takes a bit more time to run than simple prop tests".

In this repo that points at the stateful behaviours the example-based contract suite samples only a
few paths of: sequence numbers under create/delete/rollback, soft-delete visibility per role, the
outbox/activity projection under redelivery, tenancy across interleaved operations.

## The two traits and the macro (as documented)

`ReferenceStateMachine` drives generation and is the model the SUT is compared against:

| Item | Signature (from the book) |
|---|---|
| `type State`, `type Transition` | `Transition` is typically an enum whose variants carry the inputs |
| `init_state` | `fn init_state() -> BoxedStrategy<Self::State>` (use `Just(..)` for a constant) |
| `transitions` | `fn transitions(state: &Self::State) -> BoxedStrategy<Self::Transition>` (usually `prop_oneof!`) |
| `apply` | `fn apply(mut state: Self::State, transition: &Self::Transition) -> Self::State` |
| `preconditions` (optional) | `fn preconditions(state: &Self::State, transition: &Self::Transition) -> bool`, default `true` |

`StateMachineTest` wraps the SUT:

| Item | Signature (from the book) |
|---|---|
| `type SystemUnderTest`, `type Reference` | the SUT, and the type that implements `ReferenceStateMachine` |
| `init_test` | `fn init_test(ref_state: &Self::Reference::State) -> Self::SystemUnderTest` |
| `apply` | `fn apply(mut state: Self::SystemUnderTest, ref_state: &Self::Reference::State, transition: Transition) -> Self::SystemUnderTest`: post-conditions are `assert!`s here; `ref_state` already has the transition applied |
| `check_invariants` | `fn check_invariants(state: &Self::SystemUnderTest, ref_state: &Self::Reference::State)`: checked after every transition |
| `teardown` (optional) | `fn teardown(state: Self::SystemUnderTest, ref_state: Self::Reference::State)` |

Run it:

```rust
prop_state_machine! {
  #[test]
  fn name_of_the_test(sequential 1..20 => MyStateMachineTest);
}
```

`sequential` is the only mode supported ("currently, only the sequential is supported"); the number or
range is how many transitions to generate.

## Shrinking, and why preconditions matter

The sequence of transitions is itself a strategy, so a failure is shrunk: first delete transitions from
the back until the minimum size, then shrink individual transitions from the front, then shrink the
initial state. If a transition depends on current state (for example "remove one of the existing keys"),
shrinking can make it invalid, so you must override `preconditions` to re-check it. The book also warns
that calling `proptest::sample::select` on an empty collection fails "in a somewhat obscure way".

## Operating it

- A found failure is captured in a regressions file with a seed that replays it deterministically
  (book). The directory name is not on the pages read: **[unverified]**.
- `PROPTEST_CASES` (cases per run), `PROPTEST_VERBOSE=1` (print the transitions applied),
  `PROPTEST_TIMEOUT` are the documented environment overrides; or set `proptest_config` `cases`/`verbose`/`timeout`.
- Post-conditions live inside `StateMachineTest::apply`, not in their own function; there are no
  "symbolic" variables as in Erlang's eqc_statem.
- The crate-root pages are reference only ("refer to the Proptest Book"); `proptest` itself provides
  `prop_assert!`, `prop_assert_eq!`, `prop_assume!`, `prop_compose!`, `prop_oneof!`, `proptest!` and an
  attribute macro `property_test`.

## How it would fit this repo (derived, not tried)

- **Level:** run it at the adapter/port level (in process, fast), not over HTTP; the HTTP contract suite
  remains the cross-target oracle. Model = a small pure reference of work-item sequencing and visibility;
  SUT = a `Database` implementation. Run the same machine against the in-memory adapter and the Postgres
  adapter (testcontainers): that is exactly the drift check `adapters-and-flakes.md` shows is missing.
- **Async:** the documented `apply` / `check_invariants` signatures are synchronous while the ports are
  async, so the SUT wrapper needs a runtime handle inside `apply` (derived from the signatures; not tested).
- **Controls still apply.** A state-machine test that cannot fail is no better than any other: run it
  against the existing mutants (`mutant_dup_sequence`, `mutant_ignore_tenant`) and require a failure
  (`must-fail-controls.md`). Copy each shrunk counter-example into a deterministic contract test.
- **Not a Worker test:** it runs natively; wasm32 coverage still comes from the `worker-*` matrix cells.

## stateright (a different tool: model checking)

stateright "is a library for model checking systems, with an emphasis on distributed systems". You write
a `Model` (`State`, `Action`, `init_states`, `actions`, `next_state`, `properties`) with `always`
(invariant) and `sometimes` (reachability) properties; a `Checker` (for example `.checker().spawn_bfs()
.join()`) searches for a counter-example to every `always` property and an example for every
`sometimes` one, reported as a `Path` of steps. It checks **your model**, not your implementation (its
actor framework can also spawn actors on a real network). Use it for protocol-level questions such as
outbox delivery plus redelivery plus the sweeper; use proptest-state-machine to compare an
implementation with a model.
