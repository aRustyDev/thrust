# Testing, linting, versioning and release

Behaviour is for Helm v4.2.4, helm-unittest plugin 1.1.1 and kubeconform v0.8.0 on `examples/chart/thrust/`
(2026-10-18). Tags: `[derived]` = from documentation, not executed; `[unverified: not run]` = not executed.
Installing helm-unittest, `ct` or `kind` is the user's decision: propose it and wait.

## Cluster safety first

`helm template`, `helm lint`, `helm package` and `helm unittest` are client-side. These are **not**:
`helm template --validate` (deprecated; prints "use '--dry-run=server' instead"), `--dry-run=server`,
`helm install`, `helm upgrade`, `ct install`. Such flags act on whatever kube-context is current. An agent runs
each helm call as `env KUBECONFIG=/dev/null helm ...` so that nothing can reach a cluster, and never runs the
flags above; a disposable cluster is addressed explicitly with `--kube-context`.

## The local loop

```sh
export KUBECONFIG=/dev/null            # an agent instead prefixes `env KUBECONFIG=/dev/null` to each call
CH=charts/thrust
helm lint --strict $CH -f $CH/ci/external-db-values.yaml
helm lint --strict $CH -f $CH/ci/dev-postgres-values.yaml
helm template rel $CH -f $CH/ci/external-db-values.yaml > rendered-ext.yaml
helm template rel $CH -f $CH/ci/dev-postgres-values.yaml > rendered-pg.yaml
helm unittest $CH
```

Plain `helm lint $CH` with default values **fails by design** (no database Secret named): the schema doing its
job, which is why `ci/*-values.yaml` exist.

The example chart's templates are not valid YAML until rendered. Exclude `references/examples/chart/` from
repository-wide YAML linters and secret scanners (its `ci/` values name Secrets only and hold no credential).

Environment order for the `$(POSTGRES_PASSWORD)` interpolation (needs exit 0; an inverted copy exits 1):

```sh
helm template rel $CH -f $CH/ci/dev-postgres-values.yaml -s templates/deployment.yaml > order.yaml
awk '/name: POSTGRES_PASSWORD/{p=NR} /name: DATABASE_URL/{d=NR} END{exit !(p && d && p<d)}' order.yaml
```

Negative cases that must exit non-zero, kept as a cross-check next to the unit suite:

```sh
helm template rel $CH -f $CH/ci/external-db-values.yaml --set replicaCout=2                     # unknown key
helm template rel $CH -f $CH/ci/external-db-values.yaml --set replicaCount=two                  # wrong type
helm template rel $CH -f $CH/ci/external-db-values.yaml --set image.digest=sha256:xyz           # bad digest
helm template rel $CH -f $CH/ci/external-db-values.yaml --set-string server.extraEnv.THRUST_TEST_ROUTES=1
helm template rel $CH --set postgresql.enabled=true --set postgresql.auth.existingSecret=s      # pre-* hook with dev Postgres
```

Assert on the exit status **and** a fragment of the message: a command that fails for the wrong reason (a typo
in the chart path) is not a passing negative test. Redirect to a file and test `$?` of the helm command itself,
never through a pipe.

## Manifest validation with kubeconform (optional; downloads schemas)

```sh
kubeconform -strict -summary -kubernetes-version 1.30.0 rendered-ext.yaml rendered-pg.yaml
```

Both rendered sets validate (`Valid: 8, Invalid: 0` for the example). A rendered copy with `readinessProbe`
mis-spelled gives `Invalid: 1`, so the check can fail. Behaviours to know:

- It reads only files ending `.yaml`, `.yml` or `.json`. Given a file named `out.log` it prints
  `0 resource found in 0 file` and exits 0, a vacuous pass; check that the `Summary` count is what the chart
  renders. (Given a stream on stdin it parses that: `3 resources found parsing stdin` for the external-DB render.)
- By default it downloads schemas from a public GitHub raw URL. For offline CI point `-schema-location` at a
  local checkout of the schema repository `[unverified: not run]`.
- `-strict` rejects unknown fields; keep it on.

## helm-unittest (plugin 1.1.1)

Suite files live in `tests/*_test.yaml` (default glob) and are listed in `.helmignore` so they are not
packaged. Syntax used: `suite`, `templates`, `release`, `set`, `tests[].it`, `asserts[]` with `isKind`,
`equal`, `contains`, `hasDocuments`, `isNotEmpty`, `failedTemplate`, `documentIndex`.

```yaml
suite: deployment
templates: [deployment.yaml]
set: { database.existingSecret: thrust-database }
tests:
  - it: pins by digest when one is given, and the digest wins over the tag
    set:
      image.tag: "1.2.3"
      image.digest: "sha256:0000000000000000000000000000000000000000000000000000000000000000"
    asserts:
      - equal:
          path: spec.template.spec.containers[0].image
          value: "registry.example.com/thrust/thrust-server@sha256:0000000000000000000000000000000000000000000000000000000000000000"
```

### Asserting on a values-schema failure

A schema failure is a failed render, and `failedTemplate` asserts it (`tests/schema_test.yaml`):

```yaml
  - it: rejects an unknown top-level key
    set:
      replicaCout: 2
      database.existingSecret: thrust-database
    asserts:
      - failedTemplate:
          errorPattern: "additional properties 'replicaCout' not allowed"
```

Behaviour of each form against a schema failure (plugin 1.1.1):

| Assertion | Result |
|---|---|
| `failedTemplate: {}` (bare) | passes on **any** failed render, including one caused by a typo in the test's own values |
| `failedTemplate` with a matching `errorPattern` | passes |
| `failedTemplate` with a non-matching `errorPattern`, or an `errorMessage` that is not the schema text | the test **errors** |
| `equal`, `notFailedTemplate` etc. on the failed render | the test **errors** |
| valid values plus `failedTemplate` (control) | **fails** |

So always give an `errorPattern` that names the offending path or key, and keep a control that the suite can
fail (valid values plus `failedTemplate`, or the schema file removed: all six schema cases then fail).

Other behaviours:

1. A suite must be shown able to fail: changing one expected value in a copy produces a failure with an
   expected/actual diff; keep such a control when adding suites.
2. Avoid positional assertions (`env[2]`): they break when an env var is added. Use `contains` with the whole
   entry, and check ordering with the `awk` test above.
3. A run creates an empty `tests/__snapshot__/` directory; if `matchSnapshot` is used, commit the snapshots and
   keep them out of the package (`tests/` in `.helmignore` covers them; do not write `**/__snapshot__/`).
4. Plugin install (README): `helm plugin install <url>` from a **pinned release tarball** with provenance
   verification; not `--verify=false`, not `:latest`. `helm plugin list` shows a PROVENANCE column; `unknown`
   means the install was not verified `[unverified: not run]`.
5. Useful flags (README): `-f <glob>`, `--strict` (strict suite parsing), `-u` (update snapshots; review the
   diff), `-o <file> -t JUnit` for CI reports.

## chart-testing (`ct`) `[unverified: not run]`

What `ct lint` does (`doc/ct_lint.md`): `helm lint`, a **version-increment check**, Yamale schema validation of
`Chart.yaml`, yamllint of `Chart.yaml` and values files, and maintainer validation, on the **changed** charts.
For every `ci/*-values.yaml` the chart is linted once; with none, once with defaults.

Settings that matter (documented defaults):

| Flag / key | Default | Note |
|---|---|---|
| `chart-dirs` | `charts` | keep the chart at `charts/thrust/` |
| `target-branch` | `main` | changed-chart detection diffs against it |
| `check-version-increment` | `true` | a chart change without a `Chart.yaml` `version` bump **fails lint**; `--all` / `--charts` switch this check off |
| `validate-maintainers` | `true` | looks up accounts on GitHub/GitLab/Bitbucket; set `false` unless `maintainers:` hold real accounts |
| `additional-commands` | none | for example `helm unittest {{ .Path }}` runs per chart (templated) |

```yaml
# ct.yaml
chart-dirs: [charts]
target-branch: main
check-version-increment: true
validate-maintainers: false
additional-commands:
  - helm unittest {{ .Path }}
```

Prerequisites (README): Helm, Git >= 2.17, yamllint, Yamale, kubectl; `ct lint` also needs `lintconf.yaml` and
`chart_schema.yaml`, searched in `.`, `.ct/`, `$HOME/.ct`, `/etc/ct`. `ct install` installs each chart into a
real cluster (`helm install` + `helm test`): give it only a disposable `kind` cluster's kubeconfig, never a
shared one. The upstream README's Docker example mounts a kubeconfig into the container; use a throwaway
cluster's own kubeconfig for that.

**`ct install` and `existingSecret` charts:** by default each release goes into its own randomly named
namespace (`ct_install.md`), where the Secrets named in `ci/*-values.yaml` do not exist: the migrate hook (or
the pod) fails on a missing Secret and the install times out. Create a namespace and the throwaway Secrets
first, then run `ct install --namespace <that namespace>` `[derived]`.

## Versioning and release

- `version` (chart) and `appVersion` (app) are independent. Bump `version` for every chart change, even
  template-only; bump `appVersion` when the image changes. Consumers pin `version`.
- Package with overrides rather than editing files in CI: `helm package charts/thrust --version X.Y.Z
  --app-version A.B.C --destination dist/` (flags in `helm package --help`; the example packages to
  `thrust-0.1.0.tgz` with `tests/` and `ci/` absent).
- Publish as OCI (`helm push dist/thrust-X.Y.Z.tgz oci://<registry>/<owner>/charts`) `[unverified: not run]`;
  sign with `helm package --sign` (PGP) or sign the OCI artifact with cosign `[unverified: not run]`.
- The image the chart deploys should come from `appVersion` or an explicit `image.digest`; never `latest`.
- If a release tool rewrites `Chart.yaml`, it must update both fields in the same commit as the chart change so
  `ct`'s increment check passes `[unverified: depends on the repository's release tooling]`.
