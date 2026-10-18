# Secrets, credentials, hooks and migrations

Assumed contract (re-read the sources when they change): `crates/thrust-server/src/main.rs` reads
`DATABASE_URL` and `THRUST_ADMIN_TOKEN` from the **environment only**;
`crates/thrust-adapters-postgres/src/bin/migrate.rs` does the same (its header: the URL "contains the
password, never from argv"). The image layout (`/usr/local/bin/migrate`, uid 65532, port 8080) is the one the
sibling `dockerfile` skill defines. The chart's job is to deliver the values as Secret-backed env vars and
nothing else. Tags: `[derived]` = from documentation, not executed; `[unverified: not run]` = not executed.

## The pattern: reference a Secret, never carry one

```yaml
# values.yaml
database:
  existingSecret: ""            # name only
  existingSecretKey: DATABASE_URL
```

```yaml
# rendered container env: only a secretKeyRef, no password text anywhere in the output
- name: DATABASE_URL
  valueFrom:
    secretKeyRef:
      name: "thrust-database"
      key: "DATABASE_URL"
```

Rules:

1. No `password`, `token`, `url`, `apiKey` key in `values.yaml` or the schema; `additionalProperties: false`
   then rejects anyone who tries to add one via `--set`.
2. Do **not** template a `kind: Secret` from values. Create the Secret out of band with a secret manager
   (External Secrets, Sealed Secrets, SOPS) or `kubectl create secret generic ... --from-file` (not
   `--from-literal`: it lands in shell history and `ps`) `[unverified: not run]`.
3. Helm keeps the release record, **including the supplied values**, in a Secret in the release namespace
   (`sh.helm.release.v1.*`); anything in values is readable by whoever can read Secrets there `[derived]`.
4. A schema `if/then` plus a template `required "..."` call make "forgot the Secret name" a render-time error
   instead of a pod stuck in `CreateContainerConfigError`.
5. Check the output, not the intent:

```sh
env KUBECONFIG=/dev/null helm template rel charts/thrust -f charts/thrust/ci/external-db-values.yaml > rendered.yaml
grep -n -i -E 'password|postgres://[^$ ]*:[^$@ ]+@' rendered.yaml
grep -c 'kind: Secret' rendered.yaml
```

   Expected: no hits on the external-DB values; on the dev-Postgres values only the `POSTGRES_PASSWORD` /
   `postgres-password` names and the `$(POSTGRES_PASSWORD)` reference; `0` Secrets. The URL branch of the
   pattern matches a control line `postgres://app:hunter2@db.internal:5432/thrust`, so a silent `grep` means
   something.

## Composing a URL from a password Secret

The in-chart dev Postgres exposes only the superuser password, but the server wants a full URL. Kubernetes
expands `$(NAME)` inside `env[].value` from **earlier entries of the same container's env list**:

```yaml
- name: POSTGRES_PASSWORD            # must come first
  valueFrom: { secretKeyRef: { name: thrust-postgres, key: postgres-password } }
- name: DATABASE_URL
  value: "postgres://postgres:$(POSTGRES_PASSWORD)@rel-thrust-postgresql:5432/thrust"
```

The chart renders this order in both the Deployment and the migrate Job (checked by the `awk` order test in
`testing-and-release.md`); the expansion itself is Kubernetes behaviour `[derived]`. Consequences: the password
must be URL-safe (percent-encode otherwise), and the `$(...)` text is visible in `kubectl describe` but the
secret value is not. Beyond dev, skip this: point `database.existingSecret` at a Secret that already holds the
full connection URL.

## Hooks and the migrate Job

Documented lifecycle (Helm hooks page): CRDs install, templates render, **`pre-install` hooks are loaded and
awaited**, only then are the release's regular resources loaded, then `post-install` hooks run. A hook `Job`
is a *blocking* operation: Helm waits for it to complete and the **release fails if it fails** `[derived]`.

| Consequence | Design response |
|---|---|
| A `pre-*` hook cannot use anything the chart itself creates (Secret, ServiceAccount, ConfigMap, Postgres) | the Secret is `existingSecret`, created beforehand; with the in-chart Postgres use `migrate.hook: "post-install,post-upgrade"`, which `values.schema.json` **enforces** |
| Hook resources are not tracked by the release | `helm.sh/hook-delete-policy: before-hook-creation,hook-succeeded` (the example) or a Job `ttlSecondsAfterFinished` |
| Hook order is by weight, then kind, then name | always set `helm.sh/hook-weight` (the example sets `"0"`) |
| A failed hook fails `helm upgrade` | `backoffLimit` bounds retries; keep the failed Job (`hook-succeeded` only) so its logs survive |

### The post-hook race and `--wait`

`migrate` makes **one** connection attempt and exits non-zero if the database does not answer; the Job's
`backoffLimit` (default 3 in the example) gives a short retry window, not a wait loop. With the in-chart
Postgres the StatefulSet starts at the same time as the post-hook is queued, so the hook can lose the race
unless Helm waits for readiness first. The hooks page documents that with `--wait` Helm "will not run the
post-install hook until [all resources] are ready" `[derived]`. In Helm 4 `--wait` alone selects the `watcher`
strategy (wait for all resources to be ready); omitting the flag defaults to `hookOnly`. So:

```sh
helm install thrust charts/thrust -f ci/dev-postgres-values.yaml --wait   # run by the user; touches a cluster
```

Postgres' readiness probe (`pg_isready`) then gates the migrate hook. `[unverified: not run - no cluster]`.
The same applies to `helm upgrade`. A `pg_isready` init container on the Job would remove the dependency on
`--wait`, at the cost of a second image in the migrate pod (the server image has no shell or client tools).

The Job overrides the image entrypoint: `command: ["/usr/local/bin/migrate"]` (Kubernetes `command` replaces
the image `ENTRYPOINT`; `args` would replace `CMD`). The binary applies embedded SQL that is currently
`CREATE TABLE IF NOT EXISTS`, so re-running on every upgrade is safe **today**; when versioned migrations land,
re-check idempotency and concurrency before adding replicas that migrate on start.

A migrate **init container** in the Deployment is the alternative: no hook ordering (it simply retries until
the database answers) but it runs once per pod, so N replicas race the same SQL. Prefer the Job.

## Admin token and test routes

`THRUST_ADMIN_TOKEN` is optional in the chart (`adminToken.existingSecret`), same Secret pattern.
`THRUST_TEST_ROUTES` turns on `/_info` and `/_test/panic` (answered for **any** caller) and `/internal/*`
(which also needs the `x-admin-token` header) for the contract suite. It must not be settable from values: the
example schema forbids it in `server.extraEnv`.

## Probes

`/healthz` returns the constant `ok` (`thrust-api`), so liveness and readiness probes prove only that the HTTP
listener answers. If readiness must reflect database reachability, that needs a new endpoint in the server, not
a chart change. Kubernetes does not read the image's Dockerfile `HEALTHCHECK`; the Deployment's probes are the
only ones that count.
