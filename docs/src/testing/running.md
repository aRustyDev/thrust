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
| `scripts/mutants.sh` | the planted mutants; every one must be KILLED | Docker |
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
| `test.yml` | the workflow `.github/workflows/test.yml`, called by `ci.yml` through its job `test`: it holds the jobs `ci-scripts`, `hermetic`, `pg-adapter`, `contract` (the cells) and `mutants` and a `verdict` job that judges them, and the cells' markers, against the plan (any of the five, or all) and which `gate` requires (the migration moved the five jobs here; their names and commands are unchanged) |
| `ci-scripts` | (in `test.yml`) the tests of the CI helper scripts themselves (`.github/scripts/tests/`) |
| `lint.yml` | the workflow `.github/workflows/lint.yml`, called by `ci.yml` through its job `lint`: it holds the jobs `deps` and `lint` and a `verdict` job that judges them against the plan (one of the two, or both) and which `gate` requires (the migration moved both jobs here; their names and commands are unchanged) |
| `deps` | (in `lint.yml`) `scripts/check-deps.sh`, then `scripts/check-layout.sh` (ADR-0009's layout of `crates/`) and the other guards, most with their `--self-test`: topology, edge configuration, Plane 500, acceptance list, placement, migration lint, the lockfile's form, contract counts, the wrangler render, release-plz, the workflow lint and the OpenAPI diff |
| `lint` | (in `lint.yml`) `cargo fmt --all --check`, then `cargo clippy --workspace --all-targets --locked -- -D warnings`, then the same clippy for `thrust-worker` with `--no-default-features --target wasm32-unknown-unknown` (the clippy steps run even if rustfmt failed) |
| `security` | the workflow `.github/workflows/security.yml`, called by `ci.yml`: its job `deny`, and its `verdict` job, which judges `deny` against the plan and which `gate` requires (the migration moved `deny` here; its name and commands are unchanged) |
| `deny` | (in `security.yml`) `cargo deny --locked check`: advisories, licences, bans and sources, from `deny.toml` |
| `hermetic` | (in `test.yml`) `cargo nextest run --profile ci --workspace --locked`, then `cargo test --doc --workspace --locked` |
| `pg-adapter` | (in `test.yml`) `cargo nextest run -p thrust-adapters-selfhosted --profile adapters --locked -E 'binary(pg_adapter)'`, then the same for `binary(smtp_sink)` |
| `contract` | (in `test.yml`) one job per cell (`.github/workflows/contract-cell.yml`, called by `test.yml`, which `ci.yml` calls with the planner's matrix and cells): `scripts/contract.sh <target> <pass>`, then a marker that exactly that cell passed |
| `mutants` | (in `test.yml`) `scripts/mutants.sh` |
| `documentation` | the workflow `.github/workflows/documentation.yml`, called by `ci.yml`: its job `docs`, and its `verdict` job, which judges `docs` against the plan and which `gate` requires (the migration moved `docs` here; its name and commands are unchanged) |
| `docs` | (in `documentation.yml`) when the book's inputs changed (`docs/`, `AGENTS.md`, `.config/branding/`): `scripts/check-book.sh` (`mdbook build`, failing on any `ERROR` line and on a page `SUMMARY.md` does not list), then `scripts/check-book.sh --self-test` (a copy broken four ways, each refused). mdBook 0.5.4 and mdbook-mermaid 0.17.1 |
| `web` | the workflow `.github/workflows/web.yml`, called by `ci.yml`, which holds the job `web` and a `verdict` job that judges it against the plan and which `gate` requires (the migration moved the job here; its name and commands are unchanged); `web` runs when `web/` or its gate changed: `scripts/check-web-diff.sh` (the Q-R2 gate: only the files in `web/ALLOWED-CHANGES` differ from the oracle pin), then `pnpm --dir web install --frozen-lockfile --ignore-scripts` and `pnpm --dir web exec turbo run build --filter=web` |
| `gate` | `.github/scripts/assert-gate.sh`: every called workflow `changes` named succeeded **and** says its `verdict` was `pass`, every other was skipped. The cells are judged one level down: `test`'s own `verdict` job requires that every planned cell uploaded its pass marker |

`gate` is the one check to require: it fails on a named job that was skipped, on a job that ran though it was not
named, and on a planned cell that never reported, so a pipeline that silently ran less cannot read as green. `.github/workflows/flake-hunt.yml` repeats one
cell on demand and weekly. Not in CI: the `oracle_` tests (they need the pinned Plane oracle's output) and stage 2.

## Don't

- pass `--retries`, `--flaky-result`, `NEXTEST_RETRIES` or `NEXTEST_FLAKY_RESULT` ([Rule 2](rules.md));
- pipe a suite through `tee` or end the command with `; echo $?` when you will act on its status: the reported exit
  code becomes the last command's. Redirect instead: `cmd > log 2>&1`;
- `pkill wrangler` or `workerd`: other sessions run their own. The scripts kill only the processes they started.
