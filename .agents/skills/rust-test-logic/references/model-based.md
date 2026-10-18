# Model-based tests (`model_`)

Tags: `[re-run]` reproduced on this machine; `[zip]` recorded by the source's author, not re-run; `[derived]` read
from crate docs or source, not run.

Generate a sequence of operations, run it against the system under test and against a model that is obviously
correct, and compare observable results after every step. Name the tests `model_*`: the shipped config gives them
`retries = 0`, `flaky-result = "fail"` and longer timeouts (`nextest-profiles.md`).

Complete templates: `examples/kv_store_model.rs` (both styles on a key/value store) and `examples/api_model.rs`
(HTTP call sequences against an axum router). `KvStore`, `your_crate` and `your_api` are placeholders for your own
types.

## Choose the model

| System under test | Model |
|---|---|
| Data structure | `BTreeMap`, `Vec`, `HashSet` from std |
| Business rules | a deliberately naive Rust implementation inside the test |
| Ported TypeScript logic | the legacy code through the oracle bridge (`parity-harness.md`) |
| CRUD API | an in-memory map plus the documented rules (id allocation, 404s, validation) |

If the model needs its own tests, it is too clever. The model encodes the **spec**: when it and the code disagree,
check the spec (or the TypeScript behaviour) before deciding which one to change.

## Style A: lockstep loop (the default)

Compares every return value. Use it whenever any operation is valid in any state.

```rust
#[derive(Clone, Debug)]
enum Op { Insert(String, i64), Get(String), Remove(String) }

fn key() -> impl Strategy<Value = String> { "[a-d]" }   // tiny key space: operations collide

fn op() -> impl Strategy<Value = Op> {
    prop_oneof![
        (key(), any::<i64>()).prop_map(|(k, v)| Op::Insert(k, v)),
        key().prop_map(Op::Get),
        key().prop_map(Op::Remove),
    ]
}

proptest! {
    #![proptest_config(common::proptest_config(256))]

    #[test]
    fn model_kv_store_lockstep(ops in prop::collection::vec(op(), 0..64)) {
        let mut real = KvStore::new();
        let mut model = BTreeMap::new();
        for op in ops {
            match op {
                Op::Insert(k, v) => prop_assert_eq!(real.insert(k.clone(), v), model.insert(k, v)),
                Op::Get(k) => prop_assert_eq!(real.get(&k), model.get(&k).copied()),
                Op::Remove(k) => prop_assert_eq!(real.remove(&k), model.remove(&k)),
            }
            prop_assert_eq!(real.len(), model.len());
            prop_assert_eq!(real.is_empty(), model.is_empty());
        }
    }
}
```

## Style B: proptest-state-machine

Use it when which operations are valid depends on the state: remove an **existing** key, log out only when logged
in, page only after a cursor exists. Dependency: `proptest-state-machine = "0.9"` (0.9.0 resolved `[re-run]`).

- `ReferenceStateMachine`: `init_state`, `transitions(&state)` (may pick from the current state with
  `proptest::sample::select`), `apply`, and **`preconditions`**. Shrinking removes earlier steps, so a transition
  that depends on state must be re-validated: always implement `preconditions` when `transitions` looks at the
  state. Preconditions use proptest's global rejection filtering, so a hard-to-satisfy one can slow the test or
  exceed the rejection limit `[derived]`.
- `StateMachineTest`: `init_test`; `apply` receives the reference state **after** the transition, so assert
  post-conditions there; `check_invariants` runs after `init_test` and after every step `[derived]`.
- Run it with `prop_state_machine! { #![proptest_config(common::proptest_config(128))] #[test] fn model_name(sequential
  1..40 => YourMachine); }`.

## API-sequence model tests

The most valuable model test for a service. Generate HTTP call sequences, send them to the real router in-process
with `tower::ServiceExt::oneshot` (no sockets), and compare status codes and JSON against an in-memory model.

- Choose target ids mostly from ids the model has issued (live and deleted), plus an occasional id that never
  existed.
- Run async code inside the proptest body with a current-thread tokio runtime
  (`rt.block_on(async { ..; Ok::<(), TestCaseError>(()) })?`). tokio stays a dev-dependency: some crates forbid it
  as a normal dependency, so check the crate's own dependency policy before adding it.
- Encode every documented rule in the model: id allocation, 404 vs 410, validation errors, idempotency of PUT and
  DELETE.

The zip's reference workspace reports that this test found a real bug in seconds: ids were allocated as max+1, so
deleting the newest item and creating another reused its id. The model said ids are never reused, and proptest
shrank it to `Create, Create, Create, Create, Delete(last), Create` `[zip]`.

This is not a per-route handler test. One request, one assertion on status and body belongs to `rust-test-app`; a
`model_` test earns its cost only where state carries across calls.

## When a sequence fails

1. Read the shrunk `minimal failing input` (the operation list).
2. The seed is saved beside the test (`tests/<file>.proptest-regressions`); commit it, or after a CI failure copy
   the printed `cc <hash>` line into the committed file (`property.md`).
3. Turn the shrunk sequence into a plain test that replays the operations with hard-coded values: a readable
   regression that survives generator changes. Keep it in the same file so it shares the `send` helper:

```rust
// Replay of the shrunk sequence: Create x4, Delete(last), Create. Example names and ids.
#[tokio::test]
async fn ids_are_never_reused_after_delete() {
    let app = router(AppState::default());
    for name in ["a", "b", "c", "d"] {
        send(&app, Method::POST, "/api/items", Some(json!({ "name": name }))).await;
    }
    send(&app, Method::DELETE, "/api/items/4", None).await;
    let (status, body) = send(&app, Method::POST, "/api/items", Some(json!({ "name": "e" }))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(body, Some(json!({ "id": 5, "name": "e" })));
}
```

4. Decide whether the code or the model is wrong, using the spec or the TypeScript behaviour. Do not change the
   model to make the test pass without saying which rule it encodes.

## Rules

- Compare observable behaviour (return values, status codes, JSON), never private fields.
- Check invariants after every step, including the cheap queries (`len`, `is_empty`): in the zip's mutation run two
  missed mutants on `is_empty` were closed by one model invariant `[zip]`.
- Sequence length `1..40` is a good default; raise it in `deep` runs.
- These tests are **sequential**. Concurrency bugs (races, linearizability) need loom or shuttle; raise that as a
  separate task.
- Do not use the `model` crate: its last release was in 2019 and it pins an old proptest `[zip]`.
- Files under `tests/` declare `mod common;` and use `common::proptest_config`, for a stable location and no
  warning noise (`property.md`).
