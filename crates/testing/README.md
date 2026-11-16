# crates/testing/

Crates that exist only for testing and never ship (ADR-0009 rules 6 and 7). No crate depends on one except as a
dev-dependency.

| Directory | Package | What it is |
|---|---|---|
| `suites/api/` | `thrust-contract-tests` | ADR-0121's HTTP contract suite: one black-box suite, run unchanged against every target by `scripts/contract.sh` |
| `fixtures/oracle/` | `thrust-oracle-fixture` | The v1 oracle's seed (`tools/oracle-v1/scripts/seed.py`) as a fixture for `/internal/seed` (#58) |

- **`suites/`** holds black-box suites that test a running target from outside. `system/`, `acceptance/`, `security/`
  and `load/` join `api/` when each has code.
- **`fixtures/`** holds test-data libraries.
- **`profiling/`** will hold in-process profiling workloads that span crates (CPU, memory, network, GPU, storage,
  kernel; flamegraphs), run under nextest wrapper scripts. It is created with its first workload.

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
