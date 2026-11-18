# Workflows

One row per file in this directory. How each works, and why, is in [CI/CD](../../docs/src/architecture/cicd.md); this is the
index, and `.github/scripts/check-ci-index.sh` fails (in the `deps` job) when a workflow has no row here or a row names a file
that is not here.

The one required check on `main` is `gate`, the last job of `ci.yml`. **`gate` waits for** `ci.yml`'s own jobs and for every
workflow that `ci.yml` calls, through the calling job. A workflow with triggers of its own, that `ci.yml` does not call, is not
waited for: nothing in `gate` reads it.

| File | Triggers | `gate` waits | Does |
|---|---|---|---|
| `ci.yml` | `pull_request`, `merge_group`, `push` to `main`, `schedule` (every night, 03:23 UTC), `workflow_dispatch` | it is the gate | The pipeline: `changes` plans which jobs and contract cells a change needs, the workflows it calls run them, and `gate` proves that every called workflow that was planned ran and passed. It holds only `changes`, those calls and `gate` |
| `contract-cell.yml` | `workflow_call` | yes, through `test.yml`'s `contract` job, which `ci.yml`'s `test` job calls | Runs one cell of the contract matrix on its own runner, and uploads a marker that exactly that cell passed. Also called by `e2e.yml` |
| `documentation.yml` | `workflow_call` (from `ci.yml`'s `documentation` job), `workflow_dispatch` (runs the workflow alone) | yes, through `ci.yml`'s `documentation` job and its own `verdict` job | The book's check: `docs` builds the book and fails on any `ERROR` line or a page missing from `SUMMARY.md`; `verdict` judges it against the plan it was given (ADR-0102) |
| `security.yml` | `workflow_call` (from `ci.yml`'s `security` job), `workflow_dispatch` (runs the workflow alone) | yes, through `ci.yml`'s `security` job and its own `verdict` job | The supply-chain check: `deny` runs `cargo deny` over `deny.toml` (advisories, licences, bans, sources); `verdict` judges it against the plan it was given (ADR-0102) |
| `web.yml` | `workflow_call` (from `ci.yml`'s `web` job), `workflow_dispatch` (runs the workflow alone) | yes, through `ci.yml`'s `web` job and its own `verdict` job | The stock app's build check: `web` runs the Q-R2 gate (only the allowed files differ from the oracle pin), installs `web/` at its frozen lockfile and builds the app; `verdict` judges it against the plan it was given (ADR-0102) |
| `lint.yml` | `workflow_call` (from `ci.yml`'s `lint` job), `workflow_dispatch` (runs the workflow alone) | yes, through `ci.yml`'s `lint` job and its own `verdict` job | The repository's guards and the Rust lint: `deps` runs the dependency, layout, topology, edge, Plane 500, acceptance-list, placement, migration, contract-count, deploy-configuration, release and Flagship guards, the workflow lint, the index guard, `actionlint` and the OpenAPI diff; `lint` runs `rustfmt` and `clippy`; `verdict` judges the jobs against the plan it was given (ADR-0102) |
| `test.yml` | `workflow_call` (from `ci.yml`'s `test` job), `workflow_dispatch` (runs the workflow alone) | yes, through `ci.yml`'s `test` job and its own `verdict` job | The repository's tests: `ci-scripts` tests the CI's own scripts (the planner, the gate, the lint, the mutant driver, the deploy and release helpers); `hermetic` runs the hermetic nextest loop, the contract counts, the doctests and the OpenAPI tests; `pg-adapter` runs the Postgres and SMTP adapters against containers; `contract` runs the contract cells, one call of `contract-cell.yml` per cell of the matrix `ci.yml` gives it; `mutants` runs the mutation controls; `verdict` judges the jobs against the plan it was given and proves that every planned cell reported its marker (ADR-0102) |
| `deploy.yml` | `workflow_run` of `ci` (a release candidate, or a tagged push to `main`), `workflow_dispatch`, `pull_request` that edits its files (a dry run) | no | The deploy pipeline: plan, build or fetch the bundle, deploy it to an environment, prove it with a smoke, roll back by itself when the smoke fails |
| `cloudflare-infrastructure.yml` | `workflow_dispatch` | no | Plans or applies the Thrust Cloudflare infrastructure |
| `deploy-preview-provisioner.yml` | `workflow_dispatch` | no | Deploys the provisioner without a public route |
| `enable-preview-provisioner.yml` | `workflow_dispatch` | no | Verifies Access before enabling the provisioner route |
| `preview-control-tests.yml` | `pull_request`, `merge_group`, `push` to `main`, `workflow_dispatch` | no | Runs the preview control-plane tests separately; relevant tests also run in `test.yml` |
| `preview-reconcile.yml` | `pull_request`, `schedule`, `workflow_dispatch` | no | Reconciles per-PR database and Hyperdrive resources |
| `e2e.yml` | `schedule` (every day, 04:41 UTC), `workflow_dispatch`, `pull_request` that edits the file | no | Runs the `worker-cf` browser-journey cells every day, through `contract-cell.yml`, to catch drift in the runner, the browser and the packages |
| `flagship.yml` | `push` to `main` that changes the policy, `schedule` (every night, 05:23 UTC), `workflow_dispatch` | no | The edge's routing flags as code: makes every flag what the policy says, and checks them every night. Off until the repository variable `FLAGSHIP_APP_ID` is set |
| `flake-hunt.yml` | `workflow_dispatch`, `schedule` (Mondays, 03:17 UTC) | no | Repeats one contract cell 1 to 50 times to catch a flake |
| `release-plz.yml` | `push` to `main` | no | The release flow: keeps the release-candidate pull request up to date, and on its merge tags the release, creates it as a draft and starts `release.yml` |
| `release.yml` | `workflow_dispatch` (on the release tag), `pull_request` that edits it (a dry run) | no | Builds the release bundle at its tag, attests it and its SBOM, attaches them and publishes the release |

A workflow that other workflows call (its `on:` has `workflow_call`) follows the rules of
[ADR-0102](../../.agents/plans/contrail/adrs/ci/0102-one-workflow-per-concern-and-the-verdict-job.md): it routes by an input, sets
no workflow-level `concurrency`, and asks for no permission its caller does not grant. `lint-workflow.sh` enforces the first two.
