# Performance

Status: **live**, measurement only · Gate: none

## Catches

X19 (measured, not gated). Records cold-start, startup and request latency so a regression is visible in numbers.
Nothing fails a build on a threshold yet.

## Use when / Not when

Run before and after a change to startup code, the wasm bundle, the router hot path or a binding; put both numbers in
the PR. Not a test: a slower result does not go red, so never cite it as evidence that behaviour is correct.

## Name and location

| Tool | Measures |
|---|---|
| `scripts/startup-check.sh [config]` | Worker global-scope startup via `wrangler check startup` (local, writes a `.cpuprofile`) |
| `scripts/stage2-latency.sh [N]` | cold and warm `/healthz` latency against a **deployed** Worker (owner sandbox only) |
| `scripts/contract.sh <target> bench` | the latency load tests, `load_latency_*` in `crates/testing/suites/load/tests/latency.rs` (one request at a time, and eight at once), release-built with the `load` profile: a create and a list through `/api/v1`, the slice's activity feed, and a create's way to a visible activity row; succeeds if it exits 0. Not comparable to a number recorded before the vehicle tests moved (G-26), when the create and the list were the slice's |
| `scripts/contract.sh <target> scale` | the 100k-row scale probe, `load_scale_list_search_and_create` in `crates/testing/suites/load/tests/scale.rs`: the first page of `/api/v1`'s list, its workspace search and creates into a project of that size. The same caveat |
| `cargo nextest run -p thrust-profiling --profile prof` | the heap profile of the v1 router driven in-process (320 requests: creates, lists, searches, updates), with dhat-rs as the test binary's allocator: `prof_heap_router` prints the bytes and blocks allocated, the peak and what is still held, and writes `target/profiling/dhat-heap-router.json` for dhat's viewer. `prof_heap_counts_a_known_extra_allocation` is its control: two runs without ballast allocate the same, and 4 MiB of ballast adds 4 MiB |
| `crates/editor/doc/examples/schema_cost.rs` | schema construction time (`cargo run --release -p thrust-editor-doc --example schema_cost`) |

## Run

```sh
scripts/startup-check.sh                     # needs wrangler [unverified: not run]
scripts/contract.sh native-memory bench      # [unverified: not run]
```

## Required controls

None: a measurement cannot fail. Compare against a baseline measured on the same machine in the same session.

## Owning skill

None.

## In-repo examples

The scripts above.

## Target

Budgets enforced as gates have no owner yet (G-4).
