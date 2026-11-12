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
| `scripts/contract.sh <target> bench` | latency bench in `crates/thrust-contract-tests/src/bin/bench.rs`: a create and a list through `/api/v1`, the slice's activity feed, and a create's way to a visible activity row; succeeds if it exits 0. Not comparable to a number recorded before the vehicle tests moved (G-26), when the create and the list were the slice's |
| `scripts/contract.sh <target> scale` | the 100k-row scale probe, `src/bin/bench_scale.rs`: the first page of `/api/v1`'s list, its workspace search and creates into a project of that size. The same caveat |
| `crates/thrust-editor-doc/examples/schema_cost.rs` | schema construction time (`cargo run --release -p thrust-editor-doc --example schema_cost`) |

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
