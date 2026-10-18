# Chart layout, values design, values.schema.json

Behaviour is for Helm v4.2.4 on the example chart in `examples/chart/thrust/` (2026-10-18). Tags: `[derived]` =
from Helm documentation (helm-www `charts.mdx`, hooks page), not executed; `[unverified: not run]` = not
executed (for example anything needing a cluster).

## Layout

```text
thrust/
  Chart.yaml            apiVersion v2, name, version (SemVer 2), appVersion, kubeVersion
  values.yaml           every default, documented
  values.schema.json    optional in Helm, required by this skill
  templates/            _helpers.tpl, deployment, service, ingress, migrate-job, postgresql, NOTES.txt
  tests/                helm-unittest suites (excluded from the package by .helmignore)
  ci/                   *-values.yaml files that chart-testing lints/installs one by one
  .helmignore
```

Put charts under `charts/<name>/` (the `ct` default `chart-dirs`). Helm reserves `charts/`, `crds/`,
`templates/` and the listed file names; other files are left alone.

## Chart.yaml

- `version`: the chart's own SemVer 2 version; **bump it on every chart change**. Non-SemVer values are
  rejected `[derived]`.
- `appVersion`: the application's version, "needn't be SemVer", quote it `[derived]`. The example uses it as
  the default image tag: `image.tag: ""` -> `.Chart.AppVersion` (renders `...thrust-server:0.0.0`).
- `kubeVersion`: a SemVer range. The `-0` suffix (`">=1.27.0-0"`) is the folk fix for vendor builds such as
  `1.30.3-eks-1234` failing a plain `>=1.27.0`. On 4.2.4 `helm template --kube-version 1.30.3-eks-1234` passed
  with and without the suffix, so it is harmless but not shown to be needed; the install-time check against a
  real server version `[unverified: not run]`.
- `helm lint` prints `[INFO] Chart.yaml: icon is recommended`; informational, exit stays 0.

## Preconditions (assumed contract)

The chart assumes the image and server described by the sibling `dockerfile` skill: container port 8080,
`/healthz`, binaries at `/usr/local/bin/thrust-server` and `/usr/local/bin/migrate`, uid/gid 65532, env-only
`DATABASE_URL`/`THRUST_ADMIN_TOKEN`, `NoTls` database connections. Those come from
`crates/thrust-server`, `crates/thrust-api` and `crates/thrust-adapters-postgres`; re-read them when they change.

## values.yaml design

- Group by concern: `image`, `database`, `adminToken`, `postgresql`, `migrate`, `service`, `ingress`,
  `resources`, security contexts.
- Credentials are **references**: `database.existingSecret` + `existingSecretKey`. No `password`, `token`,
  `url` keys anywhere (`secrets-and-hooks.md`).
- **Database TLS:** the server and `migrate` connect without TLS today. A managed Postgres that requires TLS
  cannot be used until the server supports it; say so next to `database.existingSecret` in `values.yaml`.
- Make the safe thing the default: non-root, read-only root filesystem, `capabilities.drop: ["ALL"]`,
  `seccompProfile: RuntimeDefault`, `automountServiceAccountToken: false`. The user id must match the image's
  `USER` (65532).
- `image.digest` wins over `image.tag` (`name@sha256:...`), so a deployment can pin immutably.
- The optional in-chart database is **dev/evaluation only** and says so in `values.yaml`, `NOTES.txt` and the
  template header. Its StatefulSet runs as the alpine image's `postgres` uid 70 with `fsGroup`, drops all
  capabilities and sets resources `[unverified: not run on a cluster]`.
- Keep a `server.extraEnv` map for non-secret tuning, but let the schema refuse names the chart owns
  (`DATABASE_URL`, `THRUST_ADMIN_TOKEN`, `THRUST_LISTEN`) or that must never be set in production
  (`THRUST_TEST_ROUTES`).

## values.schema.json

Helm validates the **final merged `.Values`** (not just `values.yaml`) on `helm install`, `upgrade`, `lint` and
`template`, and also against every subchart schema `[derived]`. The example uses draft-07; other drafts were
not tried.

Techniques in the example and the message each produced on Helm 4.2.4:

| Technique | Input | Message |
|---|---|---|
| `additionalProperties: false` | `--set replicaCout=2` | `at '': additional properties 'replicaCout' not allowed` |
| `type` | `--set replicaCount=two` | `at '/replicaCount': got string, want integer` |
| `pattern` | `--set image.digest=sha256:xyz` | `... does not match pattern '^(sha256:[a-f0-9]{64})?$'` |
| `pattern` on a string | `--set migrate.hook=pre-delete` | `... does not match pattern '^(pre\|post)-...'` |
| `propertyNames` + `not`/`enum` | `--set-string server.extraEnv.THRUST_TEST_ROUTES=1` | `invalid propertyName 'THRUST_TEST_ROUTES'` (plain `--set ...=1` fails earlier: `got number, want string`) |
| `allOf` + `if/then/else` | defaults (no database Secret) or `postgresql.enabled=true` with no auth Secret | `'allOf' failed - at '/database/existingSecret': minLength: got 0, want 1` |
| `allOf` + `if/then` | `postgresql.enabled=true` with the default `pre-install,pre-upgrade` hook | `at '/migrate/hook': 'pre-install,pre-upgrade' does not match pattern '^post-(install\|upgrade)...'` |

The last row enforces the rule the skill teaches: the in-chart Postgres does not exist while a `pre-*` hook runs
(`secrets-and-hooks.md`), so the schema refuses the combination instead of rendering a Job aimed at a database
that is not there.

Without the schema file, `helm template --set replicaCout=2` exited 0 and rendered `replicas: 1`: the typo
vanished silently. That is the argument for the schema.

Guidance:

- Set `additionalProperties: false` on every object you own; leave `resources`, `affinity` etc. as free
  `"type": "object"`, their shape is Kubernetes' business.
- Use `required` for keys with no safe default and `minLength: 1` for "must be non-empty" (an empty string
  satisfies `required`).
- Encode cross-field rules with `if/then/else`. The messages are terse; also keep a `required "..."` call in
  the template so rendering with validation skipped still fails with a sentence (`--skip-schema-validation` ->
  `execution error ... database.existingSecret is required when postgresql.enabled=false`).
- No remote `$ref`s: validation must work offline; Helm documents `--skip-schema-validation` mainly for
  air-gapped installs whose schema has remote references `[derived]`.
- Mirror `values.yaml` and the schema in the same commit; a key added to one only is the usual drift.
- Generators that derive a schema from `values.yaml` exist (for example the `helm-values-schema-json` plugin);
  none was evaluated. Treat generated output as a first draft: inferred schemas are permissive and cannot know
  cross-field rules `[unverified: not run]`.

## .helmignore

Same idea as `.gitignore`, **but `**` is not supported**. A line `**/__snapshot__/` made `helm lint` fail on
every values file with `double-star (**) syntax is not supported` and `unable to load chart`, which reads like
a broken chart rather than a bad ignore pattern. Use `tests/` (snapshots live under it) or single-star globs.
Ignore `tests/` and `ci/` so they stay out of the package (`helm package` + `tar tzf` shows neither).

## Names and labels

- Truncate generated names at 63 characters (DNS label limit) and `trimSuffix "-"`; keep the standard
  `app.kubernetes.io/*` labels in a helper.
- **Give every component its own `app.kubernetes.io/component`** (`server`, `migrate`, `postgresql`) and put it
  in every selector and pod label. With selectors of only `{name, instance}`, the app Service and Deployment
  also matched the Postgres and migrate pods; that mis-routes traffic and the Deployment selector is immutable
  once created, so retrofitting it needs a delete/recreate. The example's Deployment and Service select
  `{name, instance, component: server}`; the migrate Job's pods carry `component: migrate`.
