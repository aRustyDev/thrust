# Workflows

One row per file in this directory. How each works, and why, is in [CI/CD](../../docs/src/architecture/cicd.md); this is the
index, and `.github/scripts/check-ci-index.sh` fails (in the `deps` job) when a workflow has no row here or a row names a file
that is not here.

The one required check on `main` is `gate`, the last job of `ci.yml`. **`gate` waits for** `ci.yml`'s own jobs and for every
workflow that `ci.yml` calls, through the calling job. A workflow with triggers of its own, that `ci.yml` does not call, is not
waited for: nothing in `gate` reads it.

| File | Triggers | `gate` waits | Does |
|---|---|---|---|
| `ci.yml` | `pull_request`, `merge_group`, `push` to `main`, `schedule` (every night, 03:23 UTC), `workflow_dispatch` | it is the gate | The pipeline: `changes` plans which jobs and contract cells a change needs, the jobs run, and `gate` proves that the planned set ran and passed |
| `contract-cell.yml` | `workflow_call` | yes, through `ci.yml`'s `contract` job | Runs one cell of the contract matrix on its own runner. Also called by `e2e-schedule.yml` |
| `deploy.yml` | `workflow_run` of `ci` (a release candidate, or a tagged push to `main`), `workflow_dispatch`, `pull_request` that edits its files (a dry run) | no | The deploy pipeline: plan, build or fetch the bundle, deploy it to an environment, prove it with a smoke, roll back by itself when the smoke fails |
| `e2e-schedule.yml` | `schedule` (every day, 04:41 UTC), `workflow_dispatch`, `pull_request` that edits the file | no | Runs the `worker-cf` browser-journey cells every day, through `contract-cell.yml`, to catch drift in the runner, the browser and the packages |
| `flagship.yml` | `push` to `main` that changes the policy, `schedule` (every night, 05:23 UTC), `workflow_dispatch` | no | The edge's routing flags as code: makes every flag what the policy says, and checks them every night. Off until the repository variable `FLAGSHIP_APP_ID` is set |
| `flake-hunt.yml` | `workflow_dispatch`, `schedule` (Mondays, 03:17 UTC) | no | Repeats one contract cell 1 to 50 times to catch a flake |
| `release-plz.yml` | `push` to `main` | no | The release flow: keeps the release-candidate pull request up to date, and on its merge tags the release, creates it as a draft and starts `release.yml` |
| `release.yml` | `workflow_dispatch` (on the release tag), `pull_request` that edits it (a dry run) | no | Builds the release bundle at its tag, attests it and its SBOM, attaches them and publishes the release |

A workflow that other workflows call (its `on:` has `workflow_call`) follows the rules of
[ADR-0102](../../.agents/plans/contrail/adrs/ci/0102-one-workflow-per-concern-and-the-verdict-job.md): it routes by an input, sets
no workflow-level `concurrency`, and asks for no permission its caller does not grant. `lint-workflow.sh` enforces the first two.
