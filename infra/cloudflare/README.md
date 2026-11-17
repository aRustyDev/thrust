# `infra/cloudflare`

The Cloudflare half of Thrust's database path: a **dedicated remotely-managed tunnel** and **one Workers
VPC TCP service per CNPG cluster**. This root, the Hyperdrive lifecycle scripts, and the scheduled PR
schema/Hyperdrive reconciler are owned by this repository. The durable CNPG clusters and their in-cluster
`cloudflared` connector remain in the infrastructure repository.

State remains on R2 at `products/thrust/cloudflare/646535719915/terraform.tfstate`. The historical key is
intentional: changing it would make OpenTofu propose replacement of live Cloudflare resources.

## Owns

| Resource | What |
|---|---|
| `cloudflare_zero_trust_tunnel_cloudflared.thrust_pg` | tunnel `thrust-pg`, `config_src = "cloudflare"`. The in-cluster connector remains `products/thrust/kube/cloudflared.tf` in the infrastructure repository |
| `cloudflare_connectivity_directory_service.postgres["thrust-pg"]` | VPC service, `tcp`/5432, `app_protocol postgresql`, host `thrust-pg-rw.thrust.svc.cluster.local`, tunnel as resolver |
| `cloudflare_connectivity_directory_service.postgres["thrust-pg-dev"]` | the same for `thrust-pg-dev-rw.thrust.svc.cluster.local` |

A service pins host **and** port, which is what contains SSRF: a Worker bound to it can reach exactly one
Postgres. A Workers VPC *network* binding (which would bind the whole tunnel) is deliberately not used.

## What the provider supports, and what stays out-of-band

Checked with `tofu providers schema -json` against `cloudflare/cloudflare` 5.28.0 (lockfile):

| Need | Provider resource | Decision |
|---|---|---|
| Tunnel | `cloudflare_zero_trust_tunnel_cloudflared` | **declared** |
| Workers VPC service | `cloudflare_connectivity_directory_service` (>= 5.13.0, beta) | **declared** |
| Hyperdrive config | `cloudflare_hyperdrive_config` (has `origin.service_id`) | **NOT declared**: `origin.password` is `required, sensitive`, not write-only, so it would be written into R2 state. Created by `hyperdrive.sh` |
| Connector token | data source exists | **NOT used**: it would persist the credential in state. Fetched once by `store-tunnel-token.sh` |

## GitHub environments and Terraform input

Run [cloudflare-infrastructure.yml](../../.github/workflows/cloudflare-infrastructure.yml) from the protected
`cloudflare-infrastructure` GitHub environment. It passes Terraform inputs only as process environment:

| GitHub environment entry | Workflow environment variable | Purpose |
|---|---|---|
| variable `CLOUDFLARE_ACCOUNT_ID` | `TF_VAR_cloudflare_account_id` | required Cloudflare provider/resource input |
| secret `CLOUDFLARE_API_TOKEN` | `CLOUDFLARE_API_TOKEN` | provider authentication; never a Terraform variable |
| variable `TOFU_R2_S3_ENDPOINT` | `AWS_ENDPOINT_URL_S3` | existing R2 backend endpoint |
| secrets `TOFU_R2_ACCESS_KEY_ID`, `TOFU_R2_SECRET_ACCESS_KEY` | `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` | existing R2 backend authentication |

Optional Terraform variables keep the checked-in safe defaults. If an operator deliberately overrides one,
set its `TF_VAR_<variable_name>` value in the protected environment rather than a `.tfvars` file. The workflow
uses the existing state key and has separate plan and apply dispatches; an `apply` dispatch creates and applies a
fresh plan in its protected run. Review a current plan dispatch before approving or starting an apply.

The `preview-control-plane` environment holds the independent runtime credentials: variable
`CLOUDFLARE_ACCOUNT_ID` and `THRUST_PREVIEW_PROVISIONER_URL`, and secrets `HYPERDRIVE_API_TOKEN`,
`PREVIEW_PROVISIONER_ACCESS_CLIENT_ID`, and `PREVIEW_PROVISIONER_ACCESS_CLIENT_SECRET`. The scheduled
[preview-reconcile.yml](../../.github/workflows/preview-reconcile.yml) consumes those values to reconcile only
same-repository previews.

For a local, read-only validation only:

```bash
tofu -chdir=infra/cloudflare init -backend=false
tofu -chdir=infra/cloudflare validate
tofu -chdir=infra/cloudflare test
```

Expect **3 to add** (tunnel + 2 VPC services), 0 change, 0 destroy; stop on anything else.
**Applied 2026-10-18** (ids and the `verify_ca` -> `disabled` change are recorded in the runbook, step 3).

**Credentials.** `CLOUDFLARE_API_TOKEN`: Account > Cloudflare Tunnel > Edit, plus the Workers VPC write
permission, **and the identity behind the token must hold the account role *Connectivity Directory Admin***
(Cloudflare: "if you authenticate with an API token, ensure the token belongs to a user with the required
roles"). If that cannot be arranged, create the two services once with the wrangler OAuth login (which
carries `connectivity (admin)`) and `tofu import` them: the runbook has the exact commands. The R2 backend
uses the `woven-r2-tofu` AWS profile. **Hyperdrive needs a separate token** (Account > Hyperdrive > Write);
the wrangler OAuth login does not carry it.

## TLS (Q-0094.2) — TESTED: `verify_ca` failed, `disabled` works, the server is not authenticated

> **Plainly: the Hyperdrive-to-Postgres hop is ENCRYPTED, and the database server is NOT AUTHENTICATED
> against our CA.** **Tested live, 2026-10-18, against CNPG's private cluster CA: with the VPC services on
> `cert_verification_mode = verify_ca`, `hyperdrive.sh ensure thrust-pg` FAILED ("Failed to connect to the
> provided database: Internal error."); after changing both services to `disabled` the same command
> SUCCEEDED.** The origin backend reports `TLSv1.3` in `pg_stat_ssl` (the smoke test in the runbook, step 8),
> so the hop is encrypted — but nothing checks the server's certificate, so an attacker who could win the
> `thrust-pg-rw` Service IP inside the cluster could terminate TLS between `cloudflared` and Postgres.
> **`var.cert_verification_mode` therefore defaults to `disabled`, which is the live state** (a plain
> `tofu plan` proposes nothing); `verify_ca` and `verify_full` stay allowed values.
> **Hardening is a recorded follow-up, not done.**

* CNPG serves TLS unconditionally and `pg_hba` rejects non-TLS (`hostnossl all all all reject`); that is
  **not** weakened. Hyperdrive always uses TLS.
* Workers VPC defaults to `verify_full` and trusts only public CAs and Cloudflare Origin CA. CNPG signs with a
  cluster-internal CA, and **uploading a custom CA to Workers VPC is not supported yet**, so `verify_full` also
  fails the handshake (documented, not separately tested; `verify_ca`, the setting Cloudflare suggests for a
  private CA, was tested and failed).
* Hyperdrive's own CA upload and `--sslmode verify-*` live in the config's `mtls` block, which the API
  reference says **"cannot be used with VPC Service origins; TLS must be managed on the VPC Service"**.
* The ways to authenticate the server, all **recorded follow-ups, none done**: CNPG serving a certificate that
  chains to a Cloudflare Origin CA or a public CA (needs the generic chart to expose `spec.certificates`, a
  hostname Origin CA can actually issue for — not `*.svc.cluster.local` — used as the VPC service host and
  resolvable in-cluster, and the services moved back to `verify_full`); Workers VPC gaining custom-CA upload; or
  ADR-0094's Option 2 (Tunnel + Access), whose Hyperdrive config does accept a CA. The same gap is recorded for
  every in-cluster client in `docs/src/dev/specs/datastore-tls-requirements.md` (D-TLS-4).

## Changing a VPC service — a provider bug to expect

An in-place update of `cloudflare_connectivity_directory_service` (a `cert_verification_mode` or hostname
change) **fails in OpenTofu although it succeeds remotely.** Observed 2026-10-18 applying `disabled`: the API
returned a new `created_at` and the provider failed with *"Provider produced inconsistent result after apply
… .created_at: was 2026-10-18T19:44:18Z, but now 2026-10-18T20:08:00Z"*; the change had taken effect (verified
through the API) and the next plan showed no changes. **Do not retry blindly: verify through the API or the
dashboard, then re-plan and expect "No changes".** `lifecycle { ignore_changes = [created_at] }` does **not**
help and is not in the code — OpenTofu warns that it is a *"Redundant ignore_changes element … `created_at` is
decided by the provider alone and therefore there can be no configured value to compare with"* (tested with
a scratch copy), and the failure is an apply-time consistency check inside the provider, not configuration
drift. Only an upstream provider fix removes it. Details and the verification steps: runbook, step 3.

## How a Worker gets its connection

1. Production's Hyperdrive config `thrust-pg` (VPC service `thrust-pg`, role `thrust`) exists —
   `infra/cloudflare/hyperdrive.sh ensure`, then `check`.
2. The Worker declares `"hyperdrive": [{"binding": "HYPERDRIVE", "id": "<config id>"}]` and
   `compatibility_flags: ["nodejs_compat"]`, and connects with `env.HYPERDRIVE.connectionString` (`pg`
   >= 8.16.3). It never sees the database hostname or password. The runbook has a throwaway smoke Worker.
3. Hyperdrive does not support advisory locks, `LISTEN`/`NOTIFY`, SQL-level `PREPARE` or per-session state.
4. **Query caching is DISABLED** on the configs this repo creates (owner decision: strict read-after-write
   for the system of record). `hyperdrive.sh ensure` defaults to it for `thrust-pg`, `thrust-pg-dev` and
   per-PR configs alike (dev for parity), and `check` fails on a standard config with caching on.
   `--caching-enabled` opts one config in to Cloudflare's default caching — spike 0002 should measure
   caching on a **separate** config (e.g. `ensure thrust-pg-cache --vpc-service thrust-pg --user thrust
   --caching-enabled --origin-connection-limit 10`; every config opens its own pool against the same
   `max_connections`, so keep its limit small), never by flipping the production one.

## Scripts

| Script | Does |
|---|---|
| `hyperdrive.sh check` | read-only, idempotent: both standard configs exist, route through the expected VPC service **and have caching disabled**. Exit 0 ok, 1 missing/drift, 3 could not evaluate |
| `hyperdrive.sh ensure <name> …` | create-if-absent (also for per-PR configs), **caching disabled unless `--caching-enabled`**; for `--user pr_<digits>` the **origin connection limit defaults to the per-PR role's `CONNECTION LIMIT` (8, the shared constant in `infra/preview/pr-role-limits.sh`) and a larger value is refused**; any limit below Hyperdrive's documented minimum of 5 is refused; `--dry-run` prints the redacted request and makes no call |
| `hyperdrive.sh rotate-password <name>` | replace only the origin password of an existing config; carries the live `caching` block over, because a PUT would otherwise reset it to enabled |
| `store-tunnel-token.sh` | API -> 0600 temp file -> Secrets Manager; prints the token's length only |
| `test-scripts.sh` | hermetic tests (fake `curl`/`aws`), with controls that must fail |

None puts a secret on a command line: bodies go on `curl`'s stdin, the API token in a 0600 header file.
`wrangler hyperdrive create --password` would put the password in `ps`.

## Rollback

Reverse order of the runbook. Delete the Hyperdrive configs first (`wrangler hyperdrive delete <id>`), then
scale the connector to 0 (a tunnel with live connectors cannot be deleted), then `tofu destroy` here —
which removes both VPC services and the tunnel. Bound Workers lose their database path at once.
