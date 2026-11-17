# crates/testing/

Crates that exist only for testing and never ship (ADR-0009 rules 6 and 7). No crate depends on one except as a
dev-dependency.

| Directory | Package | What it is |
|---|---|---|
| `suites/api/` | `thrust-contract-tests` | ADR-0121's HTTP contract suite: one black-box suite, run unchanged against every target by `scripts/contract.sh` |
| `suites/load/` | `thrust-load-tests` | The load tests (`load_latency_*`, `load_scale_*`): they measure a running target, which `scripts/contract.sh <target> bench` or `scale` starts, and pass when every request succeeded. A dev-dependency on `thrust-contract-tests` for its client |
| `profiling/` | `thrust-profiling` | In-process profiling workloads: the v1 router driven through `oneshot`, deterministic. `prof_heap_router` writes a dhat heap profile to `target/profiling/` (`cargo nextest run -p thrust-profiling --profile prof`); the workload's own hermetic tests run in the default loop |
| `fixtures/oracle/` | `thrust-oracle-fixture` | The v1 oracle's seed (`tools/oracle-v1/scripts/seed.py`) as a fixture for `/internal/seed` (#58) |

- **`suites/`** holds black-box suites that test a running target from outside. `system/`, `acceptance/`, `security/`
  join `api/` and `load/` when each has code.
- **`fixtures/`** holds test-data libraries.
- **`profiling/`** is one crate, `thrust-profiling`: in-process workloads that span crates, and `prof_` tests that profile
  them (CPU, memory, network, GPU, storage, kernel; flamegraphs), run with the `prof` profile. Today one workload, the v1
  router over the memory adapters, and its heap profile (dhat-rs). It becomes a group only if two workloads need
  dependency sets that conflict.

## What does not go here

Each kind of test has a home, here or elsewhere, so the kinds with no directory yet are not homeless:

| Kind | Where it lives |
|---|---|
| Unit tests | in the crate's `src/`, under `#[cfg(test)]` |
| A crate's integration, property, model, database and regression tests | the crate's own `tests/` (AGENTS.md rule 5: cargo-mutants runs only the mutated package's tests) |
| A crate's micro-benchmarks | the crate's own `benches/` |
| Mutation testing | a tool run over crates: `.cargo/mutants.toml`, `scripts/mutants.sh` |
| Test profiles, timeouts, retries | `.config/nextest.toml` |
| The browser journeys (end to end) | `e2e/` at the repository root, a pnpm project, not a crate |

Tests are routed by name, not by folder: a test's kind is its name prefix (`prop_`, `model_`, `parity_`, `oracle_`,
`fuzz_replay_`, `regression_`; AGENTS.md rule 4), and a group of crates is a package-name prefix.
