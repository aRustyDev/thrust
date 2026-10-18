---
name: helm-chart
description: Use when packaging the thrust server (plus an optional Postgres) as a Helm 4 chart or fixing one - `values don't meet the specifications of the schema`, `double-star (**) syntax is not supported`, `values.schema.json` / `Chart.yaml` / `.helmignore`, `helm unittest` or `ct lint` failures, `got number, want string` from `--set`, a migrate hook that runs before the database exists, passing `DATABASE_URL` without putting a password in values. NOT for raw Kubernetes manifests, Kustomize, Argo CD or Flux config, the image Dockerfile (see dockerfile), GitHub Actions workflows (see gha-workflow-design), or other applications' charts.
allowed-tools:
  - Edit(charts/**)
  - Bash(helm version --short)
author: aRustyDev
version: 0.1.0
---

# Helm chart for the self-hosted server

The `Edit` rule covers all file writes and pre-approves `charts/**` only. Worked example:
`references/examples/chart/thrust/`.

**Preconditions (assumed contract):** the image and server described by `dockerfile` (port 8080, `/healthz`,
`/usr/local/bin/migrate`, uid 65532, env-only `DATABASE_URL`, no database TLS). Re-read
`crates/thrust-server` and `crates/thrust-adapters-postgres` when they change.

Cluster safety: `helm template --validate`, `--dry-run=server`, `helm install`, `helm upgrade` and `ct install`
act on the current kube-context. Run every agent-run helm command as `env KUBECONFIG=/dev/null helm ...`, never
run those flags, and leave installs to the user. Installing helm-unittest or `ct` is the user's decision.

| Need | Read |
|---|---|
| Layout, values, `values.schema.json`, `.helmignore`, labels | `references/chart-and-values.md` |
| Secrets, `$(VAR)`, hooks, `--wait`, migrations | `references/secrets-and-hooks.md` |
| Lint, render, kubeconform, helm-unittest, `ct`, release | `references/testing-and-release.md` |

## Procedure

0. `helm version --short` must report v4.
1. Values name Secrets (`existingSecret` + key); never carry a credential.
2. `values.schema.json` with `additionalProperties: false` and cross-field `if/then`.
3. Keep `ci/*-values.yaml` so lint and `ct` have values that satisfy the schema.
4. A `pre-*` hook runs before the chart's resources exist: the in-chart Postgres needs `post-*` (schema-enforced).
5. Check: `helm lint --strict` per ci file, `helm template` per ci file, one negative `--set` or
   `failedTemplate` case per schema rule, a grep for password material, `helm unittest` if the plugin exists.
6. Bump `Chart.yaml` `version` on every change.

## Output contract

The chart, its `ci/` values, and the commands run with results. Tags: `[unverified: not run]` = command not
executed; `[derived]` = from documentation, not executed.

## Example

Q: "`helm lint` says double-star (**) syntax is not supported." A: `.helmignore` rejects `**`; ignore `tests/`
or use single-star globs (`references/chart-and-values.md`).
