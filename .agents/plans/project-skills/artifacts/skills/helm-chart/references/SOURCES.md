# Sources: helm-chart

External sources are cited for facts and short syntax snippets only; no third-party skill text is reproduced.
The example chart is original. External sources retrieved 2026-10-18. Repository files are cited by path.

## Repository files (assumed contract)

| Path | Supports |
|---|---|
| `crates/thrust-server/src/main.rs` | environment variables; `DATABASE_URL` and `THRUST_ADMIN_TOKEN` from the environment only; `THRUST_TEST_ROUTES` |
| `crates/thrust-adapters-postgres/src/bin/migrate.rs`, `src/lib.rs`, `migrations/0001_init.sql` | migrate binary, environment-only URL, idempotent `CREATE TABLE IF NOT EXISTS`, `NoTls` |
| `crates/thrust-adapters-selfhosted/src/pg.rs` | pool connects with `NoTls` |
| `crates/thrust-api/src/lib.rs` | `/healthz` returns the constant `ok`; `/_info`, `/_test/panic` and `/internal/*` gating |
| `scripts/contract.sh` | Postgres 16-alpine used by the `native-pg` cell (basis for the dev Postgres image) |

## External sources

| Source | Supports | Licence |
|---|---|---|
| https://github.com/helm/helm-www (`docs/topics/charts.mdx`) | layout, `version` / `appVersion`, `kubeVersion`, schema scope, `--skip-schema-validation` | MIT |
| https://github.com/helm/helm-www (`docs/topics/charts_hooks.md`) | hook lifecycle order, blocking Job hooks, `--wait` and post-install, delete policies, weights | MIT |
| https://github.com/helm/helm-www (`docs/chart_best_practices/values.md`) | naming and structure conventions | MIT |
| `helm install --help`, `helm template --help`, `helm package --help` (Helm v4.2.4; https://github.com/helm/helm) | `--wait` strategies, `--validate` deprecation, `--dry-run`, `--skip-schema-validation`, packaging flags | Apache-2.0 |
| https://github.com/helm-unittest/helm-unittest (README) | suite syntax, flags, `.helmignore` and snapshot note, install options | MIT |
| https://github.com/helm/chart-testing (README, `doc/ct_lint.md`, `doc/ct_install.md`) | what `ct lint` / `ct install` do, defaults, `ci/*-values.yaml`, prerequisites | Apache-2.0 |
| https://github.com/yannh/kubeconform | strict validation, file-extension and stdin behaviour | Apache-2.0 |

## Verification status

| Claim group | Status |
|---|---|
| `helm lint` (default, external-DB and dev-Postgres values; `--strict`), `helm template`, `helm package`, lint on the `.tgz`, `--set` vs `--set-string`, `--skip-schema-validation`, schema-less typo control, `--kube-version` with and without `-0` | executed with Helm v4.2.4 (`KUBECONFIG=/dev/null` for the later runs) |
| Schema rejection cases, including `postgresql.enabled` with a `pre-*` hook | executed; messages quoted in `chart-and-values.md` |
| helm-unittest suites; `failedTemplate` forms against a schema failure; must-fail controls (schema removed, changed expectation, dropped selector label) | executed with helm-unittest 1.1.1 |
| kubeconform `-strict` on both rendered sets, a must-fail control, the `.log` and stdin behaviours | executed with kubeconform v0.8.0 (downloads schemas on first use) |
| Secret-leak `grep` including a control line; env-order `awk` check including an inverted control | executed |
| `$(VAR)` expansion, hook ordering, `--wait` before post-install hooks, Postgres securityContext, kubelet behaviour | `[derived]` / `[unverified: not run]`: no cluster was used |
| `ct lint` / `ct install`, OCI `helm push`, chart signing, plugin install verification | `[unverified: not run]` |
