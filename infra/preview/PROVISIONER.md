# Preview provisioner database bootstrap

`provisioner-bootstrap.sql` installs the restricted login and its database functions. It
must be run once, by a trusted superuser on the `thrust` database of `thrust-pg-dev`. The caller
must pass `THRUST_PREVIEW_PROVISIONER_PASSWORD` and `THRUST_PR_ROLE_CONNECTION_LIMIT` in the psql
process environment; source `pr-role-limits.sh` for the latter. Do not place the password in psql
arguments or logs. Ensure SQL statement logging and audit extensions do not record passwords.

The operator wrapper `scripts/bootstrap-provisioner.sh` reads the existing Secrets Manager secret
`/thrust/preview-provisioner-origin-password`, resolves the primary of the fixed
`thrust-pg-dev` cluster, and streams the password and SQL to psql over `kubectl exec` stdin. It
requires the trusted operator's existing Kubernetes access and AWS Secrets Manager read access;
GitHub Actions does not use it. Set the AWS account and region using the operator's normal profile.
To create a new secret once, with the target AWS account and region already selected:

```bash
password_file="$(mktemp)"
chmod 600 "$password_file"
openssl rand -hex 24 | tr -d '\n' > "$password_file"
if aws secretsmanager create-secret --name /thrust/preview-provisioner-origin-password --secret-string "file://$password_file" > /dev/null; then
  rm -f "$password_file"
else
  printf 'Secret creation failed; inspect the private file at %s and the AWS secret before retrying.\n' "$password_file" >&2
fi
```

Do not recreate or overwrite an existing secret on retry. Check the selected AWS account and the
database's `PUBLIC` grants, then run the bootstrap once:

```bash
bash infra/preview/scripts/bootstrap-provisioner.sh
env -u HYPERDRIVE_ORIGIN_PASSWORD infra/cloudflare/hyperdrive.sh ensure thrust-preview-provisioner \
  --vpc-service thrust-pg-dev --user thrust_preview_provisioner --origin-connection-limit 5 \
  --password-sm-key /thrust/preview-provisioner-origin-password
```

The second command requires `CLOUDFLARE_ACCOUNT_ID`, `HYPERDRIVE_API_TOKEN` and access to the dev
VPC service ID through `THRUST_DEV_VPC_SERVICE_ID` or the Cloudflare OpenTofu outputs. Record the
returned Hyperdrive configuration ID in the protected deployment environment. If it fails after
the database bootstrap commits, retry only the Hyperdrive command; the SQL deliberately refuses
to overwrite an existing provisioner. The Hyperdrive `ensure` command validates an existing
configuration's origin and caching, but cannot verify its stored password.

The bootstrap refuses to run if PUBLIC has database CREATE, or if the role or administration
schema already exists. Review the effect on other clients before revoking any existing PUBLIC
privilege. The login inherits PostgreSQL's PUBLIC privileges; review those on every database the
role can connect to. The bootstrap does not change unrelated grants.

The registry records object OIDs for previews created by these functions. Existing `pr_*`
objects are not automatically adopted: a collision fails closed and requires a trusted operator
to reconcile or migrate it. Inventory fails closed while unmanaged `pr_<digits>` resources exist.
Schema removal uses
`DROP SCHEMA ... CASCADE`, so dependencies on objects in that schema can be removed too; do not
create external dependencies on preview objects. Role deletion fails and rolls back if the role
owns other objects or has outstanding grants. Bootstrap reapplication and credential rotation
require a separately reviewed operator procedure.

Before cutover, inventory existing preview resources and compare them with the desired GitHub PR
set. A trusted superuser can run this read-only query on the dev primary (the query does not
include passwords):

```sql
SELECT 'role' AS kind, rolname AS name FROM pg_catalog.pg_roles WHERE rolname ~ '^pr_[0-9]+$'
UNION ALL
SELECT 'schema', nspname FROM pg_catalog.pg_namespace WHERE nspname ~ '^pr_[0-9]+$'
ORDER BY kind, name;
```

Also run `infra/cloudflare/hyperdrive.sh list-pr`. Do not start reconciliation until each existing
resource has been reviewed and migrated or removed by a trusted operator. Removal of a preview
schema can destroy its data and dependent objects; this bootstrap does not perform that cleanup
or automatically adopt an existing role. After cleanup, reconcile will create fresh resources for
the desired PRs.

Run the disposable Postgres test with `bash infra/preview/scripts/test-provisioner-bootstrap.sh`.
This does not configure Hyperdrive, Access, or the Worker. The reconciliation workflow is already
wired to the provisioner and cannot succeed until the cutover is configured.

The `deploy-preview-provisioner.yml` manual workflow deploys a commit reachable from main with
`workers_dev` and preview URLs disabled. The protected `preview-provisioner` environment needs
`CLOUDFLARE_ACCOUNT_ID` and `THRUST_PREVIEW_PROVISIONER_HYPERDRIVE_ID` variables and a
`CLOUDFLARE_API_TOKEN` secret. The token used by the route workflow must have Worker write,
Worker metadata read, Access applications/policies read and service-token read permissions.
The closed deploy workflow does not enable the route; every closed deployment disables it again.

After the closed deployment, find the immutable ID for `thrust-preview-provisioner` in the
Cloudflare Workers dashboard or the `GET /accounts/{account_id}/workers/workers` API. With an
operator API token that can write Access applications and service tokens, run:

```bash
python3 infra/cloudflare/preview-access.py --worker-id "$WORKER_ID" --credentials-file "$PRIVATE_CREDENTIAL_FILE"
```

The file contains the only copy of the newly generated service-token secret. Store it securely;
the script reuses it on subsequent runs. If a token creation fails before the file is populated,
inspect Cloudflare for an orphaned token before retrying. Do not print the file. The provisioner
environment also needs variables `THRUST_PREVIEW_PROVISIONER_WORKER_ID`,
`THRUST_PREVIEW_PROVISIONER_URL`, `PREVIEW_PROVISIONER_ACCESS_TOKEN_ID` and secrets
`PREVIEW_PROVISIONER_ACCESS_CLIENT_ID`, `PREVIEW_PROVISIONER_ACCESS_CLIENT_SECRET`. Copy the
matching JSON fields into those entries and the reconciliation environment using secret-safe
stdin, for example:

```bash
python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["client_secret"], end="")' "$PRIVATE_CREDENTIAL_FILE" | gh secret set PREVIEW_PROVISIONER_ACCESS_CLIENT_SECRET --env preview-control-plane
```

Run `enable-preview-provisioner.yml` on main after configuration. It verifies the exact Access
policy, enables the route, tests anonymous denial and authenticated inventory, and attempts to
disable the route if either check fails. A failure to confirm disable is reported as critical and
requires immediate operator action. Cloudflare's policy and service-token API are described in
the [Worker Access documentation](https://developers.cloudflare.com/workers/configuration/cloudflare-access/)
and [service-token documentation](https://developers.cloudflare.com/cloudflare-one/access-controls/service-credentials/service-tokens/).

The reconcile environment needs `THRUST_PREVIEW_PROVISIONER_URL`,
`PREVIEW_PROVISIONER_ACCESS_CLIENT_ID` and `PREVIEW_PROVISIONER_ACCESS_CLIENT_SECRET` in addition
to the existing Hyperdrive credentials. Once the Access-protected route is enabled and verified,
set the repository variable `THRUST_PREVIEW_PROVISIONER_ENABLED` to `true` to activate reconciliation.
Until then its job is skipped, including on the PR that introduces it. A pending marker persists
across a database-password update until the corresponding Hyperdrive update succeeds; the next
run retries any pending preview.
