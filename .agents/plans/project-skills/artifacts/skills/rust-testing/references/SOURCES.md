# Sources and provenance: rust-testing

All prose is original; short API names and one-line quotes from the cited documentation are used for
accuracy and attributed inline. Dates are the day a source was read or a command was run.

## Repo sources

The spike branch `feat/spike-0001-rust-on-workers`, read 2026-10-18; cited paths are repo-relative:
`crates/thrust-contract-tests/{Cargo.toml,src/lib.rs,src/bin/bench.rs,tests/contract.rs,tests/worker_only.rs}`;
`scripts/{contract.sh,contract-all.sh,mutants.sh,flake-hunt.sh,check-deps.sh,rs,worker-dev.sh}`;
`crates/thrust-adapters-memory/{Cargo.toml,src/lib.rs}`;
`crates/thrust-adapters-selfhosted/{Cargo.toml,src/lib.rs,src/pg.rs,tests/pg_adapter.rs}`;
`crates/thrust-adapters-postgres/{Cargo.toml,src/lib.rs}`; `crates/thrust-server/{Cargo.toml,src/main.rs}`;
`crates/thrust-ports/{Cargo.toml,src/lib.rs}`; `Cargo.lock`; `.gitignore`.
ADRs are cited by id only: ADR-0002, 0007, 0043.

## External documentation (read 2026-10-18)

| Reference | Source | Version stated by the source |
|---|---|---|
| `doc-tests.md` | rustdoc book "Documentation tests", `rust-docs` component | rustc 1.99.0 |
| `model-based-testing.md` | docs.rs pages for proptest, proptest-state-machine, stateright; proptest book "State Machine testing" | proptest 1.12.0; proptest-state-machine 0.9.0; stateright 0.31.0; book page states none |
| `mock-based-testing.md` | docs.rs pages for mockall and wiremock | mockall 0.15.0; wiremock 0.6.5 |
| `nextest-and-mutants.md` | nexte.st root, "Retries and flaky tests", "Stress tests", "Why process-per-test?", "Mutation testing with cargo-mutants"; mutants.rs single-page book | crates.io: cargo-nextest 0.9.148, cargo-mutants 27.1.0; pages state no version except per-feature "0.9.x" markers |

Not read: GitHub READMEs, proptest-state-machine trait/macro item pages, mockall `examples`, wiremock
`matchers`. Anything depending on them is tagged `[unverified]` where it appears.

## Reproduced locally (`[re-run]`), 2026-10-18

Offline, from a scratch copy of the spike tree or a scratch crate. These are observations as of that
date, not operating numbers; the suite will change.

| Claim | Result |
|---|---|
| `scripts/contract.sh native-memory main` baseline | exit 0, all expected tests passed |
| The three in-memory mutants | each exit 101; killed by 3, 4 and 1 tests respectively, matching the spike's own summary |
| `scripts/check-deps.sh` passes; positive control (`-p thrust-server -i tokio` prints a tree) | reproduced |
| A mistyped `-p` subject is reported as a violation, not skipped | reproduced |
| `mutants.sh` `kill_check` output when no test log exists vs when zero tests failed | `failed tests=)` (empty) vs `failed tests=0)` |
| Doc-test forms: `?` trailer, hidden lines, `no_run`, `should_panic`, `compile_fail`; `compile_fail` passes on a typo; plain `cargo test` includes doc-tests | reproduced on rustc 1.99.0 |
| nextest 0.9.140 vs libtest, using the scripts' own parsers in a scratch crate | exit status 100 vs 101; pass-count parse `?` vs `2`; `... FAILED` count equal |
| nextest 0.9.140 lists `--retries`, `--flaky-result`, `--stress-count`, `--stress-duration` | confirmed in `--help` |
| Fake vs real `allocate` rollback semantics | read from source (memory comment and no-op `rollback`; Postgres in-transaction `UPDATE`; `pg_adapter.rs` assertion); not executed |

## Not run

- The Postgres adapter mutant and the testcontainers tests (image pull); the other contract-matrix
  cells; `scripts/flake-hunt.sh`; any Worker cell.
- Every cargo-mutants command: `[unverified: cargo-mutants not installed]`.
- The proptest, mockall, wiremock and stateright snippets are quoted from their docs, not compiled.

## Reference lifecycle

| File | Status | Last reviewed | Upstream version read |
|---|---|---|---|
| `contract-suite.md` | live | 2026-10-18 | n/a (repo) |
| `must-fail-controls.md` | live | 2026-10-18 | n/a (repo) |
| `adapters-and-flakes.md` | live | 2026-10-18 | n/a (repo) |
| `doc-tests.md` | not yet used | 2026-10-18 | rustc 1.99.0 |
| `model-based-testing.md` | not yet used | 2026-10-18 | proptest 1.12.0 / proptest-state-machine 0.9.0 / stateright 0.31.0 |
| `mock-based-testing.md` | not yet used | 2026-10-18 | mockall 0.15.0 / wiremock 0.6.5 |
| `nextest-and-mutants.md` | not yet used | 2026-10-18 | cargo-nextest 0.9.148 / cargo-mutants 27.1.0 |

**Promotion and removal rule.** When a tool is adopted in this repo, move the repo-specific method
(how it is wired, which crates, which scripts) into the live references (`contract-suite.md`,
`must-fail-controls.md`, `adapters-and-flakes.md`) and drop the upstream digest, keeping only a link to
the upstream docs. Delete a not-yet-used reference when the tool is explicitly rejected, or when it has
gone a year without review. Update the "last reviewed" cell whenever a file is re-read against its
upstream source.
