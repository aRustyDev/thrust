# test-suites: plan

Facts below are from `main` at `958be99` (2026-11-16), with cargo-nextest 0.9.140; recount them when each pull request
starts.

## Decisions (owner, 2026-11-16)

| Question | Decision |
|---|---|
| The load suite's form | **nextest tests** in a new crate, `thrust-load-tests` at `crates/testing/suites/load`, named `load_…`, run by a `load` profile that the default filter excludes. As binaries they would need a normal dependency on the contract suite's library, which ADR-0009 rule 6 and `scripts/check-layout.sh` refuse; as tests they use it as a dev-dependency |
| The first profiling workload | **The real router, in-process**: `thrust-api-v1`'s routes over `thrust-core` and the memory adapters, driven through `tower::ServiceExt::oneshot`. A heap profile with dhat-rs (a crate dependency) and a CPU profile with samply (`cargo install samply`) through a nextest wrapper script |
| How this relates to the CI split | **Its own plan, changing scripts and crates now.** The CI-split phases wire the workflows later (Test, Bench, Prof, Perf). A workflow file changes here only where a command line in it changes |

## What exists today (measured)

**The contract suite's passes map one to one onto nextest filtersets.** For each of the 11 passes of
`scripts/contract.sh`, `cargo test -- --list <the pass's filters>` and `cargo nextest list -E <a filterset>` select the
same test names. libtest's `--skip x` and positional `x` match a substring anywhere in the name, and so does nextest's
`test(/x/)`.

| Pass | Binary | Filterset | Tests |
|---|---|---|---|
| `main` | `contract` | `not test(/sweeper_\|panic_\|blast_\|csrf_off_/)` | 71 |
| `sweeper` | `contract` | `test(/sweeper_/)` | 3 |
| `csrfoff` | `contract` | `test(/csrf_off_/)` | 6 |
| `panic` | `contract` | `test(/panic_/)` | 1 |
| `blast` | `contract` | `test(/blast_/)` | 1 |
| `budget` | `contract` | `test(/list_endpoints_stay_within_five_db_round_trips/)` | 1 |
| `worker` | `worker_only` | `test(/worker_room_/)` | 1 |
| `bindings` | `worker_only` | `test(/worker_bindings_/)` | 5 |
| `cron` | `worker_only` | `test(/worker_cron_/)` | 1 |
| `engine` | `engine` | `all()` | 4 |
| `mail` | `mail` | `all()` | 7 |

Control: the `main` filterset with `csrf_off_` dropped selects 77 tests where libtest selects 71.

**`main` on `native-memory`, run three ways against a real target:**

| Runner | Result | Time running tests |
|---|---|---|
| `cargo test` (today) | 71 passed | 5.1 s |
| `cargo nextest run --profile ci` | 71 passed, JUnit `tests="71" failures="0"` | 4.5 s |
| the same, with the OIDC tests in a test group of one | 71 passed | 6.4 s |

`contract.sh` exited 5 on both nextest runs, as it must: its count guard parses libtest's `test result:` line, which
nextest does not print. One of the grouped runs never reached the tests: the port `contract.sh` picked answered
`/_info` with `OK` instead of the target's JSON, and the script's own guard stopped it. That is the port-picking race
behind the known exit-4 flake, not nextest; a second run passed.

**What changes under nextest: one process per test.** `cargo test` runs a test binary's tests as threads of one process.
The suite has one piece of process-wide state: the mock identity provider (`src/mock_idp.rs`), a `OnceLock` that binds
the fixed port `THRUST_MOCK_IDP_PORT`, the issuer the target under test was started with. Under nextest each OIDC test
starts its own, so two that run at once collide on the port: the panic `the mock identity provider's port is free`. Only
the `compat_oidc_*` tests use it. Each provider also makes its own RSA key, which is safe because `thrust-adapters-oidc`
fetches the discovery document and the key set on every call (`OpenIdProvider::metadata`) and every flow runs inside one
test. A test group of one thread for `test(/compat_oidc_/)` serialises them.

**Who parses `cargo test`'s output today:**
- `scripts/contract.sh`: 11 passes run, the passed count read from `test result:`, exit 5 on a mismatch (ADR-0121's count
  guard).
- `scripts/stage2-contract.sh`: 13 passes against a deployed Worker, each with its own count.
- `scripts/check-contract-counts.sh`: each arm's filters through `cargo test -- --list`, the count compared.
- `scripts/mutants.sh`: a mutant is killed when the exit is 101 and a test reported `... FAILED`. It also runs the
  adapter suites `pg_adapter`, `smtp_*` and the Cloudflare email tests on `cargo test`.
- `.github/workflows/ci.yml`: the `pg-adapter` job's two `cargo test` lines (`pg_adapter`, `smtp_sink`).
- `scripts/verify-all.sh`: the same two suites.

The doctests stay on `cargo test --doc`; nextest cannot run them.

**The probes.** `crates/testing/suites/api/src/bin/bench.rs` (150 lines) and `bench_scale.rs` (292 lines) are
`[[bin]]`s of `thrust-contract-tests`, run by `contract.sh`'s `bench` and `scale` passes. They use the suite library's
`Target`, `User` and `Fixture` helpers, and succeed when they exit 0.

**In-process routers already exist.** `crates/api/v1/tests/common/mod.rs` builds the v1 router over the memory adapters
with an injected clock and ids and drives it with `oneshot`.

**nextest 0.9.140.** Stable: JUnit output (`[profile.<name>.junit]`), `--no-tests`, `--tracer`, test groups,
`cargo nextest list --message-format json`. Experimental: `--message-format libtest-json`, setup scripts, wrapper
scripts. nextest exits 100 when a test fails and 4 when no test ran.

## PR 1: the contract suite onto nextest

1. **`.config/nextest.toml`.** A `contract` profile with JUnit on, `fail-fast = false`, and the `mock-idp` test group
   (`max-threads = 1`, filter `test(/compat_oidc_/)`). The default profile keeps excluding the suite. Each pass's filter
   stays in its script, as a filterset passed with `-E`: one profile for the suite rather than one per pass, because
   `stage2-contract.sh` has passes of its own (`crontrigger`, `email`, `ratelimit`, a narrower `bindings`).
2. **`scripts/contract.sh`.** Each `cargo test … -- <filters>` becomes
   `cargo nextest run -p thrust-contract-tests --profile contract -E 'binary(<bin>) & (<filterset>)'`; `--test-threads=1`
   becomes `-j 1`, `--nocapture` becomes `--no-capture`. The passed count comes from the JUnit file's `<testsuites>`
   element (`tests` minus `failures` and `errors`), not from text. Exit codes stay the script's: nextest's 100 is passed
   through as the failure it is, and its 4 ("no test ran") becomes the count guard's 5, because 4 already means "the
   target never came up".
3. **`scripts/stage2-contract.sh`** the same way.
4. **`scripts/check-contract-counts.sh`** reads each arm's filterset and counts with
   `cargo nextest list --profile contract --message-format json -E …`. Its self-test is rewritten for the new arm shape.
5. **`scripts/mutants.sh`**: a mutant is killed when nextest exits 100 and the JUnit file reports at least one failure.
   The cause check (`kill_cause`, the text a failure must contain) reads the same log, which nextest prints for a failing
   test.
6. **AGENTS.md and the docs**: the paragraph that says the two integration suites "stay on `cargo test`", the commands
   table, `docs/src/testing/running.md`, `techniques/contract-suite.md`, and ADR-0121's Confirmation as cited by the
   docs (the ADR itself stays as accepted).

Verify:
- For every pass of both scripts, the nextest selection equals today's `cargo test` selection, name for name, with the
  dropped-alternative control (the measurement above, made a script).
- Every cell of `scripts/contract-all.sh` passes locally on the targets this machine can run, and in CI.
- The count guard still fails: a filterset that drops one test exits 5; one that selects nothing exits 5, not 4.
- The OIDC group: the three `compat_oidc_*` tests run with `-j 3` and no group, repeated, collide at least once; with the
  group, never.
- `scripts/mutants.sh`: the same rows killed as on `cargo test` (43 killed and the 4 guards refused, as of #221), and one
  row whose mutant is made harmless reads SURVIVED.
- `check-contract-counts.sh` and its `--self-test` under dash.

## PR 2: the adapter suites onto nextest

`pg_adapter`, `smtp_sink` and the other adapter test binaries that `mutants.sh` and the `pg-adapter` job run with
`cargo test`. A `pg` profile (or the default with `--ignore-default-filter`, as AGENTS.md's commands table already shows
for `pg_adapter`). The `pg-adapter` job's two lines in `ci.yml`, `verify-all.sh`, and `mutants.sh`'s adapter rows. AGENTS.md
then says that every Rust test runs on nextest except the doctests.

Verify: the same tests selected; the job's results the same in CI; `mutants.sh`'s adapter rows killed as before, and
`kill_cause` still tells a real kill from a setup failure (without Docker, every `pg_adapter` test fails in setup, which
must not read as a kill).

## PR 3: `crates/testing/suites/load` (`thrust-load-tests`)

- A new crate. `src/lib.rs` holds what the probes compute (percentiles, rates, the key rotation that keeps each key under
  its allowance), so that logic gets tests of its own. `tests/latency.rs` holds `load_latency_*` (today's `bench`) and
  `tests/scale.rs` holds `load_scale_*` (today's `bench_scale`).
- The probes' arguments become environment variables with today's defaults: iterations 200 at concurrency 1 and 8;
  `THRUST_SCALE_TOTAL` 100,000 and `THRUST_SCALE_PER_PROJECT` 1,000, which `contract.sh` already passes.
- A dev-dependency on `thrust-contract-tests` for `Target`, `User` and `Fixture`.
- A `load` profile: `default-filter = 'package(thrust-load-tests)'`, one thread, `success-output = "immediate"` so the
  numbers are printed, and a long slow-timeout. The `default` and `deep` profiles exclude the package.
- `contract.sh`'s `bench` and `scale` passes run
  `cargo nextest run --cargo-profile release --profile load -E 'test(/^load_latency_/)'` (and `load_scale_`). The two
  `[[bin]]`s leave `thrust-contract-tests`.
- AGENTS.md rule 4 gains the `load_` prefix. `crates/testing/suites/README.md` and `testing/README.md` name `load/`, and
  `docs/src/testing/techniques/performance.md` names the tests.

Verify:
- The numbers are of the same order as the binaries' on the same machine in the same session, run back to back. A
  measurement cannot be "equal", so the PR shows both.
- A load test fails when the target is down: `THRUST_BASE_URL` pointed at a closed port exits non-zero.
- `scripts/check-layout.sh`: the new crate is in a group whose README names it.

`stress`, `spike` and `soak` are not created: each joins when it has a workload (ADR-0009, no stubs).

As built (2026-11-17): the `default` profile leaves out only `package(thrust-load-tests) & test(/^load_/)`, not the whole
package, so the library's own hermetic tests (`tests/measure.rs`: percentiles, key rotation, the seeded items, `timed`) run in
the default loop. cargo-mutants on the crate: 38 mutants, 33 caught, 0 missed, 2 unviable, 3 timeouts (loops made endless). Its
first run missed 6; the fix was to test `report`'s output (it takes the writer), `env_usize`, and the index `timed` hands each
call, not to skip anything.

## PR 4: `crates/testing/profiling` (`thrust-profiling`), the heap profile

- `crates/testing/profiling/` is one crate, `thrust-profiling`: each workload is a test file with `prof_` tests. It
  becomes a group only if two workloads need dependency sets that conflict.
- The workload builds the v1 router over the memory adapters, with an injected clock and ids, seeds a workspace and runs
  a fixed mix through `oneshot`: creates, lists, a search and an update. It is deterministic.
- `tests/heap.rs`: `#[global_allocator] static ALLOC: dhat::Alloc` (its own test binary, so no other test runs under it).
  `prof_heap_router` writes `target/profiling/dhat-heap.json` (viewed with dhat's `dh_view.html`) and prints the totals:
  bytes, blocks, peak.
- A `prof` profile: `default-filter = 'package(thrust-profiling)'`, one thread. The `default` and `deep` profiles exclude
  the package. AGENTS.md rule 4 gains `prof_`.

Verify: a control workload that allocates a known extra amount reports a larger total by at least that amount, so the
profile measures the workload and not nothing.

As built (2026-11-17): the `default` profile leaves out only `package(thrust-profiling) & test(/^prof_/)`;
`tests/workload.rs` (every request of a generated mix answered) runs in the default loop. The router's profile: 320 requests,
128.8 MB in 1.03 M blocks, peak 1.58 MB, about 26 s under dhat (it records a backtrace per allocation), so the `prof` profile
kills nothing. The control runs a smaller mix on fresh worlds: two runs without ballast allocate the same bytes, and 4 MiB of
ballast adds exactly 4 MiB; with the ballast removed from the workload, the control fails ("added 0 bytes").

## PR 5: the CPU profile through a nextest wrapper script

- `samply` (`cargo install samply`, the owner's decision when the PR starts; it records on macOS and Linux). A wrapper
  script in `.config/nextest.toml` (`experimental = ["wrapper-scripts"]`) runs the `prof_cpu_*` tests under
  `samply record --save-only` and writes `target/profiling/<test>.json.gz`, which samply's viewer (or the Firefox
  Profiler) opens as a flame graph.
- The wrapper's syntax is checked against nextest 0.9.140's documentation when the PR starts. The feature is
  experimental, so the PR says what breaks if it changes.

Verify, the wrapper's own control: a wrapper that never runs the test binary makes the test pass without running it. So
a `prof_cpu_` test that must fail fails under the wrapper, and the profile file names the workload's functions.

## Left for later (not this plan)

- **The workflows.** CI-split Phase 2 (Test) runs the contract cells and `mutants` with whatever this plan leaves; Phase
  12 (Bench), 13 (Prof) and 14 (Perf) give the load suite and the profiles their workflows. `cargo nextest archive`
  (build once, fan out to the cells) belongs to Phase 2.
- **Budgets.** The load tests and the profiles measure; nothing gates on a number yet (X19, G-4). A budget is a later
  decision, with a baseline measured on the runner it gates.
- **Other profilers.** Network, storage and kernel profiles (`--tracer` gives strace or dtruss for one test) and a GPU
  profile (nothing runs on a GPU) wait for a question that needs them. Profiling the Worker inside workerd is not
  possible with native profilers.
- **The port-picking race** that fails a cell with exit 4 (and made one spike run above stop before its tests) is its
  own fix.

## Status (2026-11-16)

| Item | State |
|---|---|
| Plan | this pull request |
| PR 1, contract suite onto nextest | merged in #243 (`a5a11ff`): CI found cargo-nextest missing from the contract cells, fixed with a `nextest` input to `setup-rust` |
| PR 2, adapter suites onto nextest | merged in #252 (`b83ece3`): every Rust test on nextest except the doctests; merged with #248's mutants verdicts |
| PR 3, load suite | merged in #262 (`ab9c29b`): `thrust-load-tests`; a new crate also needs `release-plz.toml` entries |
| PR 4, profiling: heap | in review, branch `test/profiling-heap` |
| PR 5, profiling: CPU | not started |
