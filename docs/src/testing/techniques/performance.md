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
| `scripts/contract.sh <target> bench` | latency bench in `crates/thrust-contract-tests/src/bin/bench.rs`; succeeds if it exits 0 |
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
