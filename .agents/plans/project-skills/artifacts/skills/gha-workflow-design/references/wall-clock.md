# Wall-clock: caching, fan-out, sharding

Tags: "documented" = docs or README (retrieved 2026-10-18); `[derived]` = inferred, not executed;
`[unverified: not run]` = not executed. No timing numbers are claimed: none were measured on GitHub-hosted
runners. Measure first.

## Measure before changing anything

Per-job timings of a finished run (the `jq` form was run against a public repository):

```sh
gh run view <run-id> --json jobs --jq '.jobs[] | "\(.name)\t\(.startedAt)\t\(.completedAt)\t\(.conclusion)"'
```

Wall-clock of a workflow is the **longest path** through `needs:`, not the sum of jobs. Runner-minutes (cost)
are the sum. Fan-out lowers the first and raises the second.

## Cache layers, cheapest first

| Layer | What it caches | Notes |
|---|---|---|
| `Swatinem/rust-cache` | `~/.cargo` (installed binaries, registry, git) and `./target` dependencies | keyed by job id (or `shared-key`), rustc release/host/hash, `Cargo.lock`/`Cargo.toml`, `rust-toolchain*`, `.cargo/config.toml` (documented) |
| `actions/cache` | anything with a key you choose | for artifacts rust-cache does not cover; key = tool + version + OS |
| sccache (`mozilla-actions/sccache-action`) | per-compilation-unit results across jobs and branches | needs `SCCACHE_GHA_ENABLED=true` and `RUSTC_WRAPPER=sccache` (documented); finer-grained than a `target/` snapshot |

`Swatinem/rust-cache` behaviours that matter (documented): it sets `CARGO_INCREMENTAL=0`; before saving it
removes files in `~/.cargo/bin` that existed before the action ran, unused dependencies and anything that is
not a dependency; base-branch caches are restorable by PRs; GitHub's cache budget for a repository is 10 GB with
eviction of the oldest entries. `cache-bin` (default `true`) includes `~/.cargo/bin`, so a binary installed by
`cargo install` **after** the action runs is saved and restored with the rest. The example relies on that for
`worker-build` and keeps no separate `actions/cache` entry: one save policy, one invariant.

**Save only from the default branch.** The composite's `save-cache` input maps to rust-cache's `save-if`
(`github.ref == 'refs/heads/main'`): every PR job restores, none saves, including the `worker-build` binary.
Otherwise N parallel cells each write a near-identical multi-hundred-MB entry and evict useful ones.

**Key by what shares build output.** The example uses `contract-native` and `contract-wasm` (cells of one family
compile the same crates), and `flake-hunt.yml` reuses those keys restore-only. A `shared-key` that is too broad
makes every job restore a cache that mostly does not apply; too narrow means nothing is ever warm.

Choosing between rust-cache and sccache is an experiment, not a default: change one, compare job times over
several runs, keep the winner `[unverified: not run]`. Do not stack them without measuring.

## Fan-out: the matrix

- Fan out over **independent units**. The contract suite's unit is the *cell* (a target plus a pass, listed in
  `scripts/contract-all.sh`), because each cell needs its own server process and its own environment.
- That script runs cells serially on one machine because parallel runs there share `crates/thrust-worker/build`
  and a second `wrangler` rebuild reloads the first one's worker (its header says so). On GitHub-hosted runners
  each job has its own workspace, so that hazard does not apply; on a **self-hosted runner reusing one
  workspace** it does, so cap `max-parallel` to 1 there.
- `fail-fast: false` so one red cell does not hide the others; the gate asserts the whole set anyway.
- Derive the matrix from the script (`plan-cells.sh`) instead of listing cells in YAML: no second list to drift,
  and `plan` fails if it parses zero cells or if the number of `run ...` lines differs from what it parsed.

Granularity trade-off for the cells:

| Layout | Wall-clock | Runner-minutes | When |
|---|---|---|---|
| One job per cell (example) | shortest | highest (setup repeated) | cells are slow compared with setup, or the cache is warm |
| One job per target, cells serial inside | medium | lower | setup dominates and cells are fast |
| One job, `scripts/contract-all.sh` | longest | lowest | small repos, or a laptop |

## Sharding inside one test binary: nextest

Commands below were run with cargo-nextest 0.9.140 on a minimal 8-test crate:

```sh
cargo nextest run --partition hash:1/2     # a subset runs, the rest is skipped; sizes depend on the hash
cargo nextest run --partition slice:1/2    # round-robin: shards differ in size by at most one
cargo nextest run --partition count:1/2    # accepted (the docs call it the worse option)
cargo nextest archive --archive-file nx.tar.zst
cargo nextest run --archive-file nx.tar.zst --partition hash:1/2   # build once, test elsewhere
```

- `hash:M/N` is deterministic and stable when tests are added or removed (documented); `slice:M/N` splits
  evenly; `count:` is the older form. Partitioning is per test, **not** by duration, so one slow test can still
  dominate a shard (documented).
- **Empty selections are errors by default.** With 64 hash shards of an 8-test crate, shards that received no
  test exited **4** (`no tests to run`) and shards that received one passed. A filter selecting nothing behaves
  the same, while `cargo test <nomatch>` exits **0** printing `0 passed`. Choose a shard count that cannot
  exceed the number of tests, pass the policy **explicitly** (`--no-tests=fail`; the default has differed across
  nextest releases, 0.9.140 fails), and never add `--no-tests=pass`/`warn` to a required job: that switches the
  vacuity guard off.
- Build once, test many: one job runs `cargo nextest archive`, uploads the archive, shards download it and run
  with `--archive-file` (plus `--workspace-remap` when paths differ) `[derived]`.
- The contract suite is **not** sharded by nextest: its tests share one live server and run with
  `--test-threads=1`. Shard by cell.

## Order of wins for a Rust workspace

1. Warm dependency cache restored on every PR job, saved on `main` only.
2. `changes` job so docs-only PRs skip the heavy jobs (`triggers-and-concurrency.md`).
3. Fan out cells; keep cheap guards (`deps`, lint) off the critical path with no `needs:` on the slow jobs.
4. `timeout-minutes` on every job (the platform default is 360 minutes `[derived]`, far longer than any healthy
   run, so a hung job otherwise burns hours).
5. Only then sccache or build-once archives, each measured.
