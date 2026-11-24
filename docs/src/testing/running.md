# Running the suites

Every cargo command goes through `scripts/rs`, which puts the rustup toolchain `rust-toolchain.toml` pins (the version CI
builds with; `rustup toolchain install <version>` if it says it is missing) first on `PATH` and fixes the
`rust-lld` library path on the dev machine (its header says why). Bare `cargo` may pick up a toolchain with no
wasm32 standard library.

## The hermetic loop (nextest)

```sh
scripts/rs cargo nextest run --workspace        # [verified 2026-10-19: 81 tests, thrust-core and thrust-editor-doc]
scripts/rs cargo test --doc --workspace         # [verified 2026-10-19 dd093fa: 0 doctests in 10 crates]
```

Profiles live in `.config/nextest.toml`; read the file for the exact values. In short:

| Profile | Selects | Use |
|---|---|---|
| `default` | everything except `oracle_`, package `thrust-contract-tests` and binary `pg_adapter` | the inner loop |
| `ci` | same as default, no fail-fast, JUnit report at `target/nextest/ci/junit.xml` | the `hermetic` job in CI |
| `deep` | only `prop_`, `model_`, `oracle_` tests, long timeouts | with `PROPTEST_CASES=4096` or more |
| `oracle` | only `oracle_` tests (need Node, or the oracle's generated output in `target/spike-0006/`) | comparison with the oracle |
| `mutants` | default filter, fail fast, no retries | selected by cargo-mutants via `.cargo/mutants.toml` |
| `workers` | package `thrust-worker`, no fail-fast, JUnit report at `target/nextest/workers/junit.xml`. Only with `--target wasm32-unknown-unknown`: the Worker's tests are compiled for wasm32 and nowhere else, so natively it selects nothing (exit 4) | the `workers` job in CI, through `scripts/wasm-tests.sh` (below) |

A `-E` filter is **intersected** with the profile's default filter: `-E 'test(/\boracle_/)'` under the default
profile selects nothing. To reach the excluded integration suites through nextest, add `--ignore-default-filter`
(`scripts/rs cargo nextest list --workspace --ignore-default-filter` lists 49 tests [verified 2026-10-19]).

### Exit codes

| Code | From | Meaning |
|---|---|---|
| 0 | either | all selected tests passed; check the count, see [count guard](evidence-and-controls.md#count-guard) |
| 4 | nextest | **no tests to run**: the filter or prefix selected nothing. Not red, not green |
| 100 | nextest | tests failed (the contract scripts pass it through, and `scripts/mutants.sh` reads it, with a failure in the row's JUnit report, as a kill) |
| 101 | nextest, `cargo test --doc` | a build failed (nextest), or a doctest failed. `scripts/mutants.sh` does not read a failed build as a kill |
| 2 / 3 / 4 | `cargo mutants` | MISSED mutants / timeouts / the baseline itself failed or selected nothing (see `rust-test-mutation`) |
| 5 | `scripts/contract.sh` | target guard or count guard tripped, or the pass's filterset selected nothing (nextest's 4, which would read as this script's "the target never came up") |

## The contract suite

```
scripts/contract.sh <target> [pass]
  target: native-memory | native-pg | worker-memory | worker-cf
  pass:   main | sweeper | panic | csrfoff | blast | bench | worker | cron | bindings
          mail                              (the password reset by email through a mail service. A native target: a Mailpit container is started beside it and it gets
                                             `THRUST_SMTP_*` (needs Docker and `nc`). A Worker target: it gets `THRUST_EMAIL_FROM`, and its `send_email` binding is wrangler's
                                             simulator, whose log is read (no container). Both get `THRUST_PUBLIC_BASE_URL`, and `--test mail` walks request, message, link
                                             and reset. In contract-all.sh as `native-memory`, `worker-memory` and `worker-cf mail`, so CI cells. Every other pass starts
                                             its target with no mail service)
          oracle                            (native-memory, native-pg, worker-cf: the v1 oracle's gate, corpus comparison and controls; needs `uv`.
                                             In contract-all.sh, so a CI cell. By hand: `scripts/oracle-v1.sh <target> [controls]`)
          e2e                               (worker-* only: the first browser journey, the stock app in Chromium against a Worker that serves it, with its must-fail
                                             control; needs Node and pnpm, and the app built. In contract-all.sh as `worker-cf e2e`, so a CI cell. By hand: `scripts/e2e.sh <target> [controls]`)
          e2e-mail                          (worker-* only: the browser journeys that need a mail service, e2e/journeys-mail/: the same Worker and app with the `mail` pass's mail
                                             service, and the journey reads the code it is mailed from the simulator's log. In contract-all.sh as `worker-cf e2e-mail`, so a CI
                                             cell. By hand: `scripts/e2e.sh <target> mail [controls]`)
          engine | scale                    (every target, opt-in: not in contract-all.sh. The engine-deciding tests; list, search and create at 100k items)
```

`csrfoff` runs the six `csrf_off_` tests against a target started with no CSRF setting at all, so that it shows the
composition roots' default (enforcement off, Q-0050.8); every other pass runs with it on, except `e2e`, which runs at that default
too (the stock app sends the token only on its auth posts), and the script refuses a pass whose
target's `/_info` says otherwise (exit 5).

Each pass runs `cargo nextest run -p thrust-contract-tests --profile contract -E 'binary(<bin>) & (<filterset>)'`, and the
script reads the passed count from the JUnit report it keeps beside the pass's log (`<log>.junit.xml`). The `contract` profile
runs the `compat_oidc_` tests one at a time: each starts the mock identity provider on the one port the target was given.

The header of `scripts/contract.sh` describes each pass and which targets accept it. Prerequisites: Docker (for
Postgres on `native-pg` and `worker-cf`), and `wrangler` for `worker-*` targets. The script generates a throwaway
admin token and database password per run and keeps them out of files and argv.

| Script | What it runs | Prerequisites |
|---|---|---|
| `scripts/contract-all.sh` | the 25-cell matrix serially (21 contract cells, the three `mail` cells among them, 3 `oracle` cells and the `e2e` journey), summary in `target/contract-logs/summary.txt` | Docker, wrangler, `uv`, Node, pnpm and `nc` |
| `scripts/e2e.sh <target> [controls]` | the browser journeys (one spec each in `e2e/journeys/`): the built app in Chromium against a Worker that serves it, then with `controls` their must-fail controls; builds the app when it is not built. `plane-oracle` runs them on the pinned Plane instead (local only; `ORACLE_DIR`, and `E2E_RECORD_HAR=1` writes `e2e/har/`; `tools/e2e-plane/README.md`) | Docker (`worker-cf`, `plane-oracle`), wrangler, Node, pnpm, `uv` and the fork checkout (`plane-oracle`) |
| `scripts/mutants.sh` | the planted mutants; every one must be KILLED. `MUTANTS_SHARDS=<n> MUTANTS_SHARD=<i>` runs only the rows at positions `i + 1`, `i + 1 + n`, ... (what one leg of the CI matrix runs), and `MUTANTS_LIST=1` lists every row with its shard and runs none | Docker |
| `scripts/wasm-tests.sh [nextest args]` | the Worker's wasm32 unit tests (`crates/worker/src/wasm_tests.rs`, G-6): nextest's `workers` profile, one wasm-bindgen runner process per test, with a count guard (every `#[wasm_bindgen_test]` the source declares must have run; a run with arguments, `-E 'test(core_config)'` say, skips only that comparison). The runner is `wasm-bindgen-test-runner` **of exactly the version `Cargo.lock` pins for `wasm-bindgen`**, found on `PATH` or in `WASM_BINDGEN_TEST_RUNNER`: `scripts/wasm-test-runner.sh` (the cargo runner `.cargo/config.toml` names for wasm32) refuses any other and prints both versions | Node 26 (the `workers` job pins 26.13.1; 24.12 and later were read in Node's source and not run, and every 22 and 23 fails: see the runbook), the `wasm32-unknown-unknown` target, the runner |
| `scripts/verify-all.sh` | the hermetic nextest run, matrix, mutants, `check-deps.sh`, `pg_adapter`, probes; summary in `target/verify/summary.txt` | Docker, wrangler |
| `scripts/flake-hunt.sh <target> <pass> <iterations>` | one cell N times, keeping every log | as the cell |
| `scripts/stage2-contract.sh [pass]` | the suite against a **deployed** Worker (run it with `sh`) | an owner-provided Cloudflare sandbox only |
| `scripts/spike-0002-pg.sh role\|migrate\|hyperdrive\|resources\|deploy\|sql\|status\|down` | creates, inspects and tears down the deployed `worker-pg` cell (a per-PR role and schema, two Hyperdrive configs, the Worker) and keeps a UTC ledger | `CLOUDFLARE_ACCOUNT_ID`, a Hyperdrive Write token, cluster access, wrangler's OAuth login |

Run matrix cells **serially**: parallel runs share `crates/worker/build` and a second wrangler rebuild reloads
the first one's Worker (the header of `scripts/contract-all.sh` records this). Contract and stage-2 commands above are
`[unverified: not run]` for this page; their behaviour is quoted from the script headers.

## Other suites

```sh
scripts/rs cargo nextest run --ignore-default-filter -E 'binary(pg_adapter)'   # needs Docker [unverified: not run]
scripts/check-deps.sh                                                         # [verified 2026-10-19 dd093fa: exit 0]
scripts/check-layout.sh                                                       # ADR-0009: every crates/ directory a crate or a named group; testing/, spikes/, core/ edges; stale paths
scripts/check-layout.sh --self-test                                           # the same rules on worlds it must refuse
scripts/check-topology.sh                                                     # one Worker: no [[workflows]], no [[services]], main = build/worker/shim.mjs (issue #43)
scripts/check-topology.sh --self-test                                         # the same check on fixtures it must refuse, so it cannot pass by looking at nothing
scripts/check-cargo-lock.sh                                                   # Cargo.lock is what cargo writes: a cargo run without --locked changes nothing (the file is put back either way)
scripts/check-cargo-lock.sh --self-test                                       # the same on workspaces it must refuse: two packages swapped, a member the lockfile has not heard of
```

The editor-doc differential inputs that are not committed are generated from the pinned Plane tree (tag
`oracle/v1-pin-20261018`): `tools/spike-0006/setup-oracle.sh <plane-tree> <workdir>` once (needs Node and pnpm),
then `tools/spike-0006/gen-all.sh <workdir>`, plus `tools/spike-0006/sanitize/run-all.sh <plane-tree>` (needs uv)
and the `convert-sanitised.ts` oracle for the sanitiser test. Then:

```sh
scripts/rs cargo nextest run --workspace --profile oracle    # the 4 oracle_ tests; each fails if its input is missing
```

`corrupted_binaries_are_detected` writes `corrupt.json` (evidence for `tools/spike-0006/oracle/verify-rust-out.ts`)
only when `SPIKE0006_OUT=<dir>` is set, relative to the workspace root.

## Format, lint and the dependency audit

The commands of the `lint` and `deny` jobs. The first two need the `clippy` and `rustfmt` components of the pinned
toolchain, and the last needs `cargo-deny` 0.20 on `PATH` (installing it is your decision); it fetches the RustSec
advisory database, so it needs the network.

```sh
scripts/rs cargo fmt --all --check                                       # [verified 2026-11-05 7d0074f: exit 0]
scripts/rs cargo clippy --workspace --all-targets --locked -- -D warnings   # [verified 2026-11-05 7d0074f: exit 0]
scripts/rs cargo clippy -p thrust-worker --no-default-features --target wasm32-unknown-unknown --locked -- -D warnings   # [verified 2026-11-05 7d0074f: exit 0]
scripts/rs cargo deny --locked check                                     # [verified 2026-11-05 7d0074f: advisories, bans, licenses, sources ok]
```

`scripts/rs cargo fmt --all` fixes what the first one finds. The panic lints that clippy denies, and the `#[allow]` that
says why a site keeps its own, are on [Standards and their gates](../contributing/standards.md).

## CI

`.github/workflows/ci.yml` runs on every pull request, merge-queue entry and push to `main`, and every night on
`main`. Each job runs the same script you would run locally, so a red job reproduces with the command in its step.
A pull request runs only the jobs and cells its change needs; how that is decided is in
[CI/CD, What runs when](../architecture/cicd.md#what-runs-when).

| Job | Runs |
|---|---|
| `changes` | derives the contract cells from the `run <target> <pass>` lines of `scripts/contract-all.sh`, then decides which jobs and cells the change needs (`.github/scripts/plan-ci.py`, over the changed paths and `cargo metadata`; fails open) |
| `test.yml` | the workflow `.github/workflows/test.yml`, called by `ci.yml` through its job `test`: it holds the jobs `ci-scripts`, `hermetic`, `pg-adapter`, `contract` (the cells), `mutants` and `workers` and a `verdict` job that judges them, and the cells' markers, against the plan (any of the six, or all) and which `gate` requires (the migration moved the first five jobs here, their names and commands unchanged; `workers` is new, from stage 8 of the Test phase) |
| `ci-scripts` | (in `test.yml`) the tests of the CI helper scripts themselves (`.github/scripts/tests/`), 47 commands run a core at a time under GNU parallel, the slowest first: every one runs, the job fails if any does, and its log ends with each command's time and exit status (and, on a failure, which commands failed) |
| `lint.yml` | the workflow `.github/workflows/lint.yml`, called by `ci.yml` through its job `lint`: it holds the jobs `deps`, `lint`, `bash`, `yaml` and `json` and a `verdict` job that judges them against the plan (any of the five, or all) and which `gate` requires (the migration moved the first two here, and their names and commands are unchanged; `bash`, `yaml` and `json` are the Lint phase's first three stages) |
| `deps` | (in `lint.yml`) `scripts/check-deps.sh`, then `scripts/check-layout.sh` (ADR-0009's layout of `crates/`) and the other guards, most with their `--self-test`: topology, edge configuration, Plane 500, acceptance list, placement, migration lint, the lockfile's form, contract counts, the wrangler render, release-plz, the workflow lint, `actionlint` (which reads every workflow's `run:` blocks with shellcheck 0.11.0, installed in the job: the version `bash` holds the scripts to) and the OpenAPI diff |
| `lint` | (in `lint.yml`) `cargo fmt --all --check`, then `cargo clippy --workspace --all-targets --locked -- -D warnings`, then the same clippy for `thrust-worker` with `--no-default-features --target wasm32-unknown-unknown` (the clippy steps run even if rustfmt failed) |
| `yaml` | (in `lint.yml`) `.github/scripts/check-yaml.sh`: yamllint 1.38.0 (exactly: installed with `uv tool install yamllint==1.38.0`, the runner image has none) over every tracked `*.yml` and `*.yaml`, workflows and composite actions included, less `web/`, the plans, the spikes, the probes, any `node_modules/` and pnpm's lockfile `e2e/pnpm-lock.yaml`. What fails is a defect and not a style: a file that does not parse, and a mapping with a key written twice (`.yamllint.yaml` turns on `key-duplicates` and nothing else; yamllint's own `default` finds 1,328 things here, 1,132 of them line length). It runs its control first (a key written twice and a file that does not parse must be refused, a file that is only unstyled must pass, which shows that the configuration is the one in effect), so a yamllint that is not looking fails the job. Run it before pushing a YAML change: `.github/scripts/check-yaml.sh`, with yamllint 1.38.0 on `PATH`, or named by `YAMLLINT=<the binary>` (`uv tool install yamllint==1.38.0` puts it there) |
| `json` | (in `lint.yml`) `.github/scripts/check-json.py`, run as `python3 -I .github/scripts/check-json.py` (Python's own `json` module, which every runner image has: nothing to install or pin) over every tracked `*.json`, less `web/`, the plans, the spikes, the probes and any `node_modules/`; recorded outputs and generated files (the oracle's results, the compat fixtures, `package-lock.json`) are read. What fails is a defect and not a style: a file that does not parse (stricter than Python's default: `NaN`, `Infinity`, a byte order mark, bytes that are not UTF-8, trailing text, a comment and a trailing comma are refused) and an object with a key written twice, once its escapes are read (`"a"` and `"\u0061"` are one key). It runs its control first (a key twice, nested in an array, and spelled with an escape, a file that does not parse and NaN must be refused; an unstyled file, the same key in two objects and a number of 5,000 digits must pass), so a check that is not looking fails the job. biome 2.5.15 was measured and not used: by default it reads 178 of the 180 files and exits 0 (a file over 1 MiB is skipped with a warning, a lockfile is never read) and misses the escaped key; `jq` and `yq` accept a key written twice. Run it before pushing a JSON change: `python3 -I .github/scripts/check-json.py` |
| `bash` | (in `lint.yml`) `.github/scripts/check-shell.sh`: shellcheck 0.11.0 (exactly: the runner image's is 0.9.0) over every shell script the repository maintains, that is every tracked `*.sh` and every executable with a shell shebang (`scripts/rs`), less `web/`, the plans' scripts, the spikes, the probes and the zxcvbn measurement; and every `run:` block of the six composite actions (`.github/actions/*/action.yml`, which actionlint cannot read), cut out with `yq` and `jq` and read in the dialect of its `shell:`. Every level fails, with `.shellcheckrc` (SC2016 and SC2015 off); a finding that is meant is annotated where it is, `# shellcheck disable=SCnnnn # why`. It runs its control first (an unquoted variable must be refused, the configuration must be the one in effect), so a shellcheck that is not looking fails the job. Run it before pushing a shell change: `.github/scripts/check-shell.sh` (`--list` shows what it reads), with shellcheck 0.11.0 on `PATH` |
| `security` | the workflow `.github/workflows/security.yml`, called by `ci.yml`: its job `deny`, and its `verdict` job, which judges `deny` against the plan and which `gate` requires (the migration moved `deny` here; its name and commands are unchanged) |
| `deny` | (in `security.yml`) `cargo deny --locked check`: advisories, licences, bans and sources, from `deny.toml` |
| `hermetic` | (in `test.yml`) `cargo nextest run --profile ci --workspace --locked`, the contract counts guard, `cargo test --doc --workspace --locked`, then the OpenAPI tests (`openapi` feature). Built without debug info (`CARGO_PROFILE_DEV_DEBUG: none`): a panic still names its file and line, a backtrace has no file:line frames |
| `pg-adapter` | (in `test.yml`) `cargo nextest run -p thrust-adapters-selfhosted --profile adapters --locked -E 'binary(pg_adapter)'`, then the same for `binary(smtp_sink)` |
| `contract` | (in `test.yml`) one job per cell (`.github/workflows/contract-cell.yml`, called by `test.yml`, which `ci.yml` calls with the planner's matrix and cells): `scripts/contract.sh <target> <pass>`, then a marker that exactly that cell passed |
| `mutants` | (in `test.yml`) `scripts/mutants.sh`, as a matrix of four runners (`strategy.job-total` and `strategy.job-index` tell the script how many shards there are and which it is): the 66 rows rebuild `thrust-server` with a mutant's feature into one `target/`, so they cannot share a checkout, and each runner runs the rows at its positions. The job id stays `mutants`, so `verdict` and `gate` read the aggregate of the four legs |
| `workers` | (in `test.yml`) `scripts/wasm-tests.sh`: the Worker's own code that exists only on wasm32 (reading `Env` and the retention windows, the composition root, the queue consumer and the fetch entry point; [status](status.md) G-6), built as the cargo profile `wasm-test` (no debug info: the runner reads the whole binary once per test, 20 s a test with it and 1.9 s without) and run on Node by `wasm-bindgen-test-runner`, which the job installs at the version `Cargo.lock` pins (Cargo says which: `cargo pkgid --locked wasm-bindgen`). Runs when `thrust-worker` is affected, that is `crates/worker` or a crate it depends on, or when one of the scripts it runs changed (`scripts/wasm-tests.sh`, `scripts/wasm-test-runner.sh`, `scripts/lib/workerd-stubs.cjs`), and on a full run (`.cargo/`, `.config/` and `Cargo.lock` plan one). About 90 s with a cold cache (92 s in run `44825424412`: 35 s of wasm build, 19 s of tests) and 68 s with the cache (run `45084134281`: the build 7 s). The cache (231 MB, key `wasm-workers`) is saved on `main` only; a pull request restores `main`'s, as GitHub scopes branch caches, though I have not yet seen one do it. The repository's cache is at its 10 GB limit, so an entry that is new evicts the least recently used. On no critical path: the cells set the wall clock |
| `documentation` | the workflow `.github/workflows/documentation.yml`, called by `ci.yml`: its job `docs`, and its `verdict` job, which judges `docs` against the plan and which `gate` requires (the migration moved `docs` here; its name and commands are unchanged) |
| `docs` | (in `documentation.yml`) when the book's inputs changed (`docs/`, `AGENTS.md`, `.config/branding/`): `scripts/check-book.sh` (`mdbook build`, failing on any `ERROR` line and on a page `SUMMARY.md` does not list), then `scripts/check-book.sh --self-test` (a copy broken four ways, each refused). mdBook 0.5.4 and mdbook-mermaid 0.17.1 |
| `web` | the workflow `.github/workflows/web.yml`, called by `ci.yml`, which holds the job `web` and a `verdict` job that judges it against the plan and which `gate` requires (the migration moved the job here; its name and commands are unchanged); `web` runs when `web/` or its gate changed: `scripts/check-web-diff.sh` (the Q-R2 gate: only the files in `web/ALLOWED-CHANGES` differ from the oracle pin), then `pnpm --dir web install --frozen-lockfile --ignore-scripts` and `pnpm --dir web exec turbo run build --filter=web` |
| `build` | the workflow `.github/workflows/build.yml`, called by `ci.yml`: its jobs `linux` and `wasm`, and its `verdict` job, which judges them against the plan (one of the two, or both) and which `gate` requires. Each job of it is a compile proof for one target: it uploads what it built for three days, and publishes, signs and attests nothing. The deploy bundle is not built here (`deploy.yml` and `release.yml` build and attest it) |
| `build-extended` | the workflow `.github/workflows/build-extended.yml`, **not called by `ci.yml` and not read by `gate`**: a red run blocks neither a pull request nor a release candidate. Its jobs `macos`, `windows` and `docker` (below) run every night (06:11 UTC), when a release candidate is pushed (`rc/*`: after every merge to `main`, as release-plz updates that branch each time; the `-tmp-` twin it makes along the way starts no run), by hand, and on a pull request that edits the workflow, the action `build-native` or `docker/`; no other pull request starts it (the owner's decision of 2026-11-23) |
| `linux`, `macos`, `windows` | (each job sets Rust up itself, with a cache key of its own (`build-<name>`, saved on `main` only), and then the steps are the action `build-native`'s, to which the job gives a name and a target: `linux`, in `build.yml`, on `ubuntu-24.04` for `x86_64-unknown-linux-gnu`; `macos`, in `build-extended.yml`, on `macos-15` for `aarch64-apple-darwin`; `windows`, in `build-extended.yml`, on `windows-2025` for `x86_64-pc-windows-msvc`, where the binaries end in `.exe`) a check that the compiler's host is that target, then `cargo build --release --locked -p thrust-server`, then `cargo build --release --locked -p thrust-adapters-postgres --features native --bin migrate` (the command the deploy bundle builds `migrate` with), a check that both binaries exist, and an upload of them as `build-<name>-<target>` (`build-linux-x86_64-unknown-linux-gnu`). `linux` runs when `thrust-server` or `thrust-adapters-postgres` is affected and is a required check (through `gate`); `macos` and `windows` run on the schedule above and are not. "Release" is the repository's `[profile.release]`, which is the Worker's (`opt-level = "s"`, fat LTO, one codegen unit) |
| `docker` | (in `build-extended.yml`, on `ubuntu-24.04`, with the runner's own Docker: no action sets it up) `docker build --file docker/Dockerfile --tag thrust-server:ci .`, which builds `thrust-server` and `migrate` (the two commands of `build-native`) on musl and puts them in `distroless/static` as uid 65532, and fails on a lint violation (`# check=error=true`); then **starts the image once**: it publishes port 8080 on the loopback, requires `/healthz` to answer `ok` (curl waits for the port), requires the server's process to run as uid 65532 (read from `/proc`), and requires `docker stop` to end it with exit 0 inside five seconds; then runs `migrate` in the image and requires its refusal for want of `DATABASE_URL`. It pushes, logs in to and uploads nothing. On a hosted runner it takes 236 to 324 s, cold (two runs; the build is all but 6 s of it, the two checks a second each; the same Dockerfile on glibc took 149 s in one run: the musl compile is about twice as long). Locally: `docker build -f docker/Dockerfile -t thrust-server .` from the repository root (on Apple Silicon add `--platform linux/amd64`: emulated, about 4 minutes), then `docker run --rm -p 8080:8080 thrust-server`. What the image is: [Infrastructure](../architecture/infra.md#the-image-of-the-server) |
| `wasm` | (in `build.yml`) `worker-build --release . --no-default-features` in `crates/worker` (the Worker as it ships: without the in-memory adapter set the contract cells use), then `worker-build --release .` in `crates/edge` (the edge Worker): the commands `build-bundle` builds the deploy bundle's two Workers with. A check that each wrote `worker/shim.mjs`, `index.js` and `index_bg.wasm`, and an upload of both as `build-wasm-wasm32-unknown-unknown`. It runs when `thrust-worker` or `thrust-edge` is affected. Its cache key, `deploy-release`, is `build-bundle`'s, and `main` saves it: the bundle of a release candidate or a tag restores it |
| `gate` | `.github/scripts/assert-gate.sh`: every called workflow `changes` named succeeded **and** says its `verdict` was `pass`, every other was skipped. The cells are judged one level down: `test`'s own `verdict` job requires that every planned cell uploaded its pass marker |

`gate` is the one check to require: it fails on a named job that was skipped, on a job that ran though it was not
named, and on a planned cell that never reported, so a pipeline that silently ran less cannot read as green. `.github/workflows/flake-hunt.yml` repeats one
cell on demand and weekly, and `.github/workflows/deep.yml` runs the randomized tests (the `deep` profile, less the `oracle_` tests) at 4096 cases every night, and `.github/workflows/build-extended.yml` builds the server for macOS and Windows, and the server's image, every night and when a release candidate is pushed; none of the three is gated. Not in CI: the `oracle_` tests (they need the pinned Plane oracle's output) and stage 2.

### When `ci-scripts` is red

- **Read the end of the job's log.** It prints each of the 47 commands with its seconds and exit status, slowest first, and when any failed, `--- FAILED (n):` and each failed command with its exit status. The failed command's own output is above, kept together.
  Every command runs even when one fails, so a red job can name several: fix them all before you push.
- **Reproduce one command** from the repository root, as the job runs it: `sh .github/scripts/tests/<name>.sh` (under `dash` if your `sh` is bash: the runner's is dash), `python3 -I .github/scripts/tests/<name>.py`, with `RUSTUP_TOOLCHAIN=stable` for `test_plan_ci.py`.
  The commands do not depend on one another, so any one can be run alone, or all of them at once by any parallel runner.
- **A job stopped after ten minutes** has a command that hung, and prints no table (it was killed before). A command's lines appear only when it finishes, so the hung one is among the commands of the list in `test.yml` whose output is missing from the log.
- **A command that fails once and passes on a rerun** is a finding to file, not to retry away ([Rule 2](rules.md)). Rerun the failed job once to see whether it repeats.

### When `hermetic` is red

- **Read the nextest summary** (`Summary [...] 4780 tests run: ...`) and the `FAIL` lines above it: a failing test is a finding ([Rule 3](rules.md)), not something to retry away.
- **`prop_decode_never_panics_on_mutated_updates` (thrust-editor-doc) is issue #114**: a panic inside the yrs decoder on a one-byte mutation, which fails this job at random (1 of 22 probe runs on 2026-11-19, with every other test passing). It is open and not fixed; a red job whose only failure is that test is this issue, and one with any other failure is not.
- **The profile `deep`** (`PROPTEST_CASES=4096`) runs nightly in `.github/workflows/deep.yml`, not gated, and is red at random for the same reason (#114): see [status](status.md) G-48.

### When `workers` is red

- **Read the nextest summary** (`Summary [...] 15 tests run: ...`) and the `FAIL` lines: a failing test is a finding ([Rule 3](rules.md)). A panic prints its file and line and the JavaScript stack above it; the wasm function names in that stack are the Worker's own.
- **`… is '<found>', and Cargo.lock pins wasm-bindgen <wanted>: the runner must be <wanted> exactly`** (or `cannot run <runner>`, or `no single version of wasm-bindgen to match`) is `scripts/wasm-test-runner.sh` refusing a runner of another version: the runner and the `wasm-bindgen` the test binary was built with must agree exactly (wasm-bindgen's own requirement), and the wrapper names a mismatch here instead of leaving it to be found inside the runner. In CI the job installs the runner at the locked version, so this means that step broke; `no single version of wasm-bindgen to match` is `cargo pkgid --locked wasm-bindgen` failing, which it does when the lock has no `wasm-bindgen` or two versions of it (its own message is above, and says which); by hand, get that version's runner (a prebuilt binary of the release, or `cargo install wasm-bindgen-cli --version =<wanted> --locked`, which is your decision) and point `WASM_BINDGEN_TEST_RUNNER` at it.
- **`Node <version> cannot load the Worker's wasm32 tests`** (or, from a runner that skipped the wrapper, `ERR_INTERNAL_ASSERTION … a bug in Node.js` at `ModuleLoader.getModuleJobForRequire`) is the Node: every 22 and 23, 24 before 24.12 and 25 before 25.2 assert on the `cloudflare:` imports of the Worker's inline JavaScript, which the runner's CommonJS glue `require()`s (the web jobs' `web/.node-version`, 22.22.0, is one of them). Use Node 26. The job's pin is in `test.yml`, and `test_workflow_wiring.py` holds it to 26 or later.
- **`Cannot find module 'cloudflare:…'`, `… is a workerd module the wasm32 tests have no stand-in for` or `ReferenceError: <Name> is not defined`** is a new `worker` or `worker-sys` version importing a workerd module, or using a workerd global, that Node does not have. Add the stand-in to `scripts/lib/workerd-stubs.cjs`, making it do what workerd does and no more, and add its check and control to `.github/scripts/tests/test-wasm-tests.sh` (the file's header says how each existing one was found).
- **`declares N tests and the report has M`** is the count guard: a `#[wasm_bindgen_test]` the source declares did not run. The guard counts the attributes that start a line of `crates/worker/src/wasm_tests.rs`; a test behind a `cfg`, or a build without the default `memory` feature (`--no-default-features`, which this job does not use), drops tests this way.
- **Reproduce** from the repository root: `WASM_BINDGEN_TEST_RUNNER=<runner> scripts/wasm-tests.sh`, or one test with `scripts/wasm-tests.sh -E 'test(the_queue_consumer)'` (a run with arguments skips only the count). Nothing here needs Docker, wrangler or a running target.

### When `linux`, `macos` or `windows` is red

- **`this runner's Rust host is '<host>', and the job is a proof for '<target>'`** means the job ran on a runner for another target (the image behind a label moved, or the label was edited): it is the check that stops Linux binaries being uploaded under the name `build-macos-…`. Fix the label in the workflow (`build.yml` or `build-extended.yml`) and the target the job passes, which must agree.
- **A red `windows`** is read like a red `macos` (below): a red night or a red release candidate, blocking nothing. Its steps run under Git Bash (`shell: bash`) on a checkout that has no `.gitattributes`: nothing failed on that account in the first run, but if a script of the build ever fails on Windows with a `\r` in an error message or a `command not found` for a name that exists, the checkout's line endings are the first thing to look at. `ring` 0.17.14 compiled in about 6 s in the first run, and the job installs no assembler (whether the runner's image has one was not checked: if `ring` ever fails there, look at the image's release notes first). **`cargo built nothing at target/release/<name>.exe`** names the suffix the job passed in `exe`: a Windows job must say `exe: .exe`, and any other must not.
- **The cache is not saved, in silence** (the job's post step of `rust-cache` prints nothing, no `Cache up-to-date.` and no `Saving cache`, and `gh cache list` shows no entry for `build-<name>`): `setup-rust` was called from inside another composite action. Call it from the job itself, as the three jobs do; the header of `.github/actions/build-native/action.yml` has the measurement and the runner issues.
- **A red `macos`** is a red night, or a red run of a release candidate's branch, in the Actions tab: it blocks nothing and no check on a pull request shows it, so someone has to read it. A compiler or linker error on `macos` only is a break of that target: a crate that needs an API Linux has and macOS has not, or a C dependency (`ring`, the one crate of the server's tree with C and assembly in it) that Apple's compiler refuses. Reproduce it on a Mac with the commands below; `macos` runs on `macos-15`, three Apple M1 cores.
- **A compiler or linker error** is a finding: the contract cells build `thrust-server` in debug, so a break that only `linux` sees is a break of the release profile (an item behind `cfg(debug_assertions)`, a link-time error under LTO). Reproduce it from the repository root with `scripts/rs cargo build --release --locked -p thrust-server`, or `scripts/rs cargo build --release --locked -p thrust-adapters-postgres --features native --bin migrate` for `migrate`.
- **`cargo built nothing at target/release/<name>`** means cargo exited 0 and no binary is there: the package or the `[[bin]]` was renamed. The check exists so that a build that makes nothing cannot pass.
- **`--locked` fails** (`the lock file … needs to be updated but --locked was passed`) because `Cargo.lock` is not what the manifests need: update it with cargo and commit it (`scripts/check-cargo-lock.sh` says whether the file is in the form cargo writes).
- **A run of 30 minutes** is the job's timeout, and a build that long is a finding. Measured 2026-11-23 (the details are in `.agents/plans/ci-workflows/phases/0004-build.md`), with no cache and then with the cache `main` saves: `linux` about 134 s and 100 s (the server's release build 87 s cold); `macos` 198 to 249 s and 134 s; `windows` 219 to 290 s and 133 s. Cold times vary by about 1.3 times between runs of the same job, so a run is a finding only well outside those. A job whose log says `No cache found.` on `main` twice running has lost its cache: the entries are `v0-rust-build-<name>-…` in `gh cache list`, about 210 to 240 MiB each, and the repository's cache is over its 10 GB limit, so the least recently used go first (the night's run keeps `macos` and `windows` from the seven days an unread entry lives).

### When `docker` is red

It is a red night or a red release candidate, blocking nothing, like `macos` and `windows`; someone has to read it. By the line the log ends on:

- **`lint violation found for rules: <Rule>`**: BuildKit's own lint, which the first line of the Dockerfile (`# check=error=true`) makes an error. The rule's name says what (`JSONArgsRecommended`: a shell-form `ENTRYPOINT`; `SecretsUsedInArgOrEnv`; `UndefinedVar`). `docker build --check -f docker/Dockerfile .` runs the lint alone.
- **A base that cannot be pulled** (`manifest unknown`, `not found`): the `FROM` lines are pinned by the digest of their index, and nothing bumps them. Resolve the tag again with `docker buildx imagetools inspect <ref> --format '{{.Manifest.Digest}}'` and put the new digest in the line.
- **`COPY failed`, or a cargo error naming a file that is not there**: `docker/Dockerfile.dockerignore` is an allow list (`Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `.cargo`, `crates`) and the build read something else. Allow that path; keep the `*` line.
- **A cargo or linker error** is a finding, and it may be musl's: the image builds on musl, and `linux` (glibc) is green, so a crate that needs glibc, or a C dependency whose build script does not know musl (`ring` builds), breaks only here. Reproduce it with the `docker build` above.
- **`info: downloading` lines from `rustup`** are `rust-toolchain.toml` having moved past the compiler the builder image ships (Dependabot's weekly bump): expected, a few seconds, and gone when the base's digest is re-resolved to an image that ships the new one.
- **One `curl: (56)` line above a green step** is the wait for the port (Docker's userland proxy resets the early connections), not a failure.
- **`/healthz answered '<x>', not 'ok'`, or a curl error after twenty tries** (the step prints the container's log): the server did not start, or does not listen where the port is published (`THRUST_LISTEN` must stay `0.0.0.0:8080`).
- **`the server runs as uids <x>, not as 65532`**: the Dockerfile's `USER` line.
- **`the server exited with 137 after docker stop`**: the server ignored the stop signal for five seconds and was killed. The image's `STOPSIGNAL` must be SIGINT while the server handles only SIGINT (`crates/server/src/main.rs`).
- **`migrate did not run in the image`**: `migrate` is not in it, cannot run in it, or no longer says `DATABASE_URL must be set` when it has none.

### When `wasm` is red

- **A compiler error** is a finding. The ten `worker-cf` cells build the Worker as it ships with the same command, so a break that only `wasm` sees is in the edge Worker, which nothing else builds in release before a release candidate. Reproduce it from `crates/worker` with `../../scripts/rs worker-build --release . --no-default-features`, or from `crates/edge` with `../../scripts/rs worker-build --release .`; the `wasm32-unknown-unknown` target and `worker-build` at the version `setup-rust` pins (`0.9.0`) must be installed.
- **`worker-build wrote no <dir>/<file>`** means `worker-build` exited 0 and left out one of the three files the shim needs (`worker/shim.mjs` imports `../index.js`, which imports `index_bg.wasm`), or wrote it empty.
- **A slow run** is a cache that was not restored: read the `rust-cache` step, which says "No cache found" or "Cache hit". A cold run compiles `worker-build` from source (110 s of a job that takes 238 s); a restored one finds it already installed and takes 79 s (measured 2026-11-23). `main` saves the cache, and the entries of a repository over its limit are evicted oldest first.

### When a `mutants` leg is red

- **Each leg judges 16 or 17 of the 66 rows.** Its log names each row as it starts (`row 6: native-memory main + skip-csrf`; a row's own output goes to a file, so nothing else appears), and ends with the summary: one line per row (`KILLED`, `SURVIVED`, `UNCLEAR`, `DID NOT RUN`, `REFUSED`, `NOT REFUSED`), `shard <i> of <n>: <rows> of the <total> rows`, and the counts. Legs do not cancel one another, so a red job can have several red legs: read all of them.
- **`SURVIVED`** is the finding the job exists to make: the suite passed against a build broken on purpose, so an assertion is vacuous. **`DID NOT RUN`** says why (a build, a server that did not come up, Docker); it is not evidence about any assertion.
- **A leg that says `VACUITY GUARD: shard i of n selected no row`** has more shards than rows or a wrong index; **`... selected k rows and recorded m verdicts`** means a row in `scripts/mutants.sh` does not start with `want "<label>"`, so it runs on every leg. `.github/scripts/tests/test-mutants-shards.sh` finds that before a push.
- **Reproduce one leg** from the repository root with Docker running: `MUTANTS_SHARDS=4 MUTANTS_SHARD=<i> scripts/mutants.sh`, or one row alone by finding its number in `MUTANTS_SHARDS=4 MUTANTS_SHARD=0 MUTANTS_LIST=1 scripts/mutants.sh` and picking the shard it falls on. Never two at once in one checkout: they share `target/`.
- **`postgres adapter + discard-noop` ends at its first failure** (`--max-fail=1:immediate`): the mutant makes its model test wait on a poisoned connection for about 125 s in about half of the runs, and the two other tests of the row kill the mutant in about 3 s. Its verdict reads `KILLED (3 test(s) failed)`, but one test failed and two were terminated (nextest's report counts a terminated test as failed), and each terminated test leaves its Postgres container running: nothing on a runner, which is discarded with the job; locally, remove the containers labelled `org.testcontainers.managed-by=testcontainers` when no other run is using them.
- **A leg stopped after twenty minutes** has a row that hung, and prints no summary (it was killed before). The last `row <k>: <label>` line of its log is the row it was in.

## Don't

- pass `--retries`, `--flaky-result`, `NEXTEST_RETRIES` or `NEXTEST_FLAKY_RESULT` ([Rule 2](rules.md));
- pipe a suite through `tee` or end the command with `; echo $?` when you will act on its status: the reported exit
  code becomes the last command's. Redirect instead: `cmd > log 2>&1`;
- `pkill wrangler` or `workerd`: other sessions run their own. The scripts kill only the processes they started.
