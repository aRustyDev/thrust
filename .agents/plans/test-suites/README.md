# test-suites

Three pieces of test work that the crates-layout plan (ADR-0009) left for later:

1. **The contract suite onto nextest.** It is the last Rust suite that runs on `cargo test`, through scripts that parse
   libtest's text: `scripts/contract.sh`, `scripts/stage2-contract.sh`, `scripts/check-contract-counts.sh` and
   `scripts/mutants.sh`. The Postgres and SMTP adapter suites (`pg_adapter`, `smtp_sink`) follow. AGENTS.md says to move
   them on purpose, not by accident; this is that move.
2. **A load suite** at `crates/testing/suites/load`. It takes the latency probe (`bench`) and the 100k-row scale probe
   (`bench_scale`) out of `thrust-contract-tests`, where they are binaries, and makes them nextest tests.
3. **`crates/testing/profiling/`**, created with its first workload: the real router driven in-process, with a heap
   profile (dhat-rs) and a CPU profile (samply, through a nextest wrapper script).

Done looks like this:
- every Rust test runs on nextest, and only the doctests stay on `cargo test --doc` (nextest cannot run them);
- the two probes are `load_` tests in their own crate;
- `crates/testing/profiling/` holds a `prof_` workload whose heap and CPU profiles are written by one command, each with
  a control that shows it can fail.

**Scope.** This plan changes crates, scripts and `.config/nextest.toml`. The workflows that run these suites belong to
the CI-split plan ([ci-workflows](../ci-workflows/README.md)), whose phases 2 (Test), 12 (Bench), 13 (Prof) and 14
(Perf: load, soak, spike) wire them. A workflow file is touched here only where a script's command line changes.

## Map

| Path | Contents |
|---|---|
| `PLAN.md` | the owner's decisions; what exists today, measured; the five pull requests, each with what it changes and how it is verified; what is left for later; the status table |
