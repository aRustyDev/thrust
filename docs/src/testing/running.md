# Running the suites

Every cargo command goes through `scripts/rs`, which puts rustup's stable toolchain first on `PATH` and fixes the
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

A `-E` filter is **intersected** with the profile's default filter: `-E 'test(/\boracle_/)'` under the default
profile selects nothing. To reach the excluded integration suites through nextest, add `--ignore-default-filter`
(`scripts/rs cargo nextest list --workspace --ignore-default-filter` lists 49 tests [verified 2026-10-19]).

### Exit codes

| Code | From | Meaning |
|---|---|---|
| 0 | either | all selected tests passed; check the count, see [count guard](evidence-and-controls.md#count-guard) |
| 4 | nextest | **no tests to run**: the filter or prefix selected nothing. Not red, not green |
| 100 | nextest | tests failed |
| 101 | `cargo test` | tests failed (the contract scripts and `scripts/mutants.sh` rely on this) |
| 2 / 3 / 4 | `cargo mutants` | MISSED mutants / timeouts / the baseline itself failed or selected nothing (see `rust-test-mutation`) |
| 5 | `scripts/contract.sh` | target guard or count guard tripped |

## The contract suite

```
scripts/contract.sh <target> [pass]
  target: native-memory | native-pg | worker-memory | worker-cf
          native-sqlite | worker-do          (opt-in, spike 0002: not in contract-all.sh)
  pass:   main | sweeper | panic | csrfoff | blast | bench | worker | cron | bindings
          engine | scale                    (every target: the engine-deciding tests; list, search and create at 100k items)
          do                                (worker-do only: the object's own checks on its real SQLite)
```

`csrfoff` runs the six `csrf_off_` tests against a target started with no CSRF setting at all, so that it shows the
composition roots' default (enforcement off, Q-0050.8); every other pass runs with it on, and the script refuses a pass whose
target's `/_info` says otherwise (exit 5).

The header of `scripts/contract.sh` describes each pass and which targets accept it. Prerequisites: Docker (for
Postgres on `native-pg` and `worker-cf`), and `wrangler` for `worker-*` targets. The script generates a throwaway
admin token and database password per run and keeps them out of files and argv.

| Script | What it runs | Prerequisites |
|---|---|---|
| `scripts/contract-all.sh` | the 16-cell matrix serially, summary in `target/contract-logs/summary.txt` | Docker, wrangler |
| `scripts/mutants.sh` | the planted mutants; every one must be KILLED | Docker |
| `scripts/verify-all.sh` | the hermetic nextest run, matrix, mutants, `check-deps.sh`, `pg_adapter`, probes; summary in `target/verify/summary.txt` | Docker, wrangler |
| `scripts/flake-hunt.sh <target> <pass> <iterations>` | one cell N times, keeping every log | as the cell |
| `scripts/stage2-contract.sh [pass]` | the suite against a **deployed** Worker (`THRUST_EXPECT_ADAPTERS=durable` for a deployed `worker-do`; run it with `sh`) | an owner-provided Cloudflare sandbox only |
| `scripts/spike-0002-do.sh up\|deploy\|status\|url\|down` | creates, inspects and tears down the deployed `worker-do` cell and keeps a UTC ledger | `CLOUDFLARE_ACCOUNT_ID` in the environment, wrangler's OAuth login |

Run matrix cells **serially**: parallel runs share `crates/thrust-worker/build` and a second wrangler rebuild reloads
the first one's Worker (the header of `scripts/contract-all.sh` records this). Contract and stage-2 commands above are
`[unverified: not run]` for this page; their behaviour is quoted from the script headers.

## Other suites

```sh
scripts/rs cargo nextest run --ignore-default-filter -E 'binary(pg_adapter)'   # needs Docker [unverified: not run]
scripts/check-deps.sh                                                         # [verified 2026-10-19 dd093fa: exit 0]
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

## CI

`.github/workflows/ci.yml` runs on every pull request, merge-queue entry and push to `main`. Each job runs the
same script you would run locally, so a red job reproduces with the command in its step.

| Job | Runs |
|---|---|
| `changes` | decides whether anything Rust-relevant changed (`.github/scripts/classify-changes.sh`; fails open) |
| `plan` | derives the contract cells from the `run <target> <pass>` lines of `scripts/contract-all.sh` |
| `ci-scripts` | the tests of the CI helper scripts themselves (`.github/scripts/tests/`) |
| `deps` | `scripts/check-deps.sh` |
| `hermetic` | `cargo nextest run --profile ci --workspace`, then `cargo test --doc --workspace` |
| `pg-adapter` | `cargo test -p thrust-adapters-selfhosted --test pg_adapter` |
| `contract` | one job per cell (`.github/workflows/contract-cell.yml`): `scripts/contract.sh <target> <pass>` |
| `mutants` | `scripts/mutants.sh` |
| `gate` | `.github/scripts/assert-gate.sh`: every job succeeded **and** every planned cell uploaded its pass marker |

`gate` is the one check to require: it fails on a skipped job when Rust changed and on a planned cell that never
reported, so a pipeline that silently ran less cannot read as green. `.github/workflows/flake-hunt.yml` repeats one
cell on demand and weekly. Not in CI: the `oracle_` tests (they need the pinned Plane oracle's output) and stage 2.

## Don't

- pass `--retries`, `--flaky-result`, `NEXTEST_RETRIES` or `NEXTEST_FLAKY_RESULT` ([Rule 2](rules.md));
- pipe a suite through `tee` or end the command with `; echo $?` when you will act on its status: the reported exit
  code becomes the last command's. Redirect instead: `cmd > log 2>&1`;
- `pkill wrangler` or `workerd`: other sessions run their own. The scripts kill only the processes they started.
