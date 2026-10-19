# Running the suites

Every cargo command goes through `scripts/rs`, which puts rustup's stable toolchain first on `PATH` and fixes the
`rust-lld` library path on the dev machine (its header says why). Bare `cargo` may pick up a toolchain with no
wasm32 standard library.

## The hermetic loop (nextest)

```sh
scripts/rs cargo nextest run --workspace        # [verified 2026-10-19: 12 tests, all thrust-editor-doc]
scripts/rs cargo test --doc --workspace         # [verified 2026-10-19 dd093fa: 0 doctests in 10 crates]
```

Profiles live in `.config/nextest.toml`; read the file for the exact values. In short:

| Profile | Selects | Use |
|---|---|---|
| `default` | everything except `oracle_`, package `thrust-contract-tests` and binary `pg_adapter` | the inner loop |
| `ci` | same as default, no fail-fast, JUnit report at `target/nextest/ci/junit.xml` | CI (none exists yet: G-5) |
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
  pass:   main | sweeper | panic | blast | bench | worker | cron | bindings
```

The header of `scripts/contract.sh` describes each pass and which targets accept it. Prerequisites: Docker (for
Postgres on `native-pg` and `worker-cf`), and `wrangler` for `worker-*` targets. The script generates a throwaway
admin token and database password per run and keeps them out of files and argv.

| Script | What it runs | Prerequisites |
|---|---|---|
| `scripts/contract-all.sh` | the 16-cell matrix serially, summary in `target/contract-logs/summary.txt` | Docker, wrangler |
| `scripts/mutants.sh` | the planted mutants; every one must be KILLED | Docker |
| `scripts/verify-all.sh` | the hermetic nextest run, matrix, mutants, `check-deps.sh`, `pg_adapter`, probes; summary in `target/verify/summary.txt` | Docker, wrangler |
| `scripts/flake-hunt.sh <target> <pass> <iterations>` | one cell N times, keeping every log | as the cell |
| `scripts/stage2-contract.sh [pass]` | the suite against a **deployed** Worker | an owner-provided Cloudflare sandbox only |

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

## Don't

- pass `--retries`, `--flaky-result`, `NEXTEST_RETRIES` or `NEXTEST_FLAKY_RESULT` ([Rule 2](rules.md));
- pipe a suite through `tee` or end the command with `; echo $?` when you will act on its status: the reported exit
  code becomes the last command's. Redirect instead: `cmd > log 2>&1`;
- `pkill wrangler` or `workerd`: other sessions run their own. The scripts kill only the processes they started.
