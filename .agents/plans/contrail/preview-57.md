# Per-PR previews (#57)

Thrust owns reconciliation and the Access-protected provisioner; GitHub Actions uses no
Kubernetes credential. The development database bootstrap, dedicated Hyperdrive, Worker,
Access policy and service token are live. The existing reconciliation path creates per-PR roles,
schemas and Hyperdrives on `thrust-pg-dev`, repairs interrupted credential updates, and sweeps
closed PRs. PR #298's creation and cleanup were observed through Actions and the provisioner
inventory. The provisioner checks are part of the required CI gate (#298). A synthetic direct
live test exercised password-update recovery; the interrupted-update path still needs an Actions
test.

## Snapshot-plus-per-PR-cluster target (2026-11-23)

The shared-schema path is a legacy transition, not the isolation model that will enable Preview
migrations. The target reuses a **sealed base snapshot** for equal trusted inputs and gives each
active PR a different mutable CNPG cluster. Equal schema names do not establish equality; the
snapshot fingerprint is the tagged SHA-256 of the exact authorized migration artifact, PostgreSQL
major version, and immutable bootstrap SHA-256. An equal fingerprint may reuse a base snapshot,
but it never shares a live database, Cluster, PVC, credential, VPC service, Hyperdrive config,
or Worker binding across PRs.

`thrust-preview-provisioner::preview_cluster_plan` now makes the snapshot, cluster and
idempotency-key naming contract deterministic and rejects non-canonical digests or unsupported
PostgreSQL majors. Its generative tests prove that equal trusted inputs reuse only the snapshot,
while distinct PRs get distinct mutable cluster identities; they also prove that changing the
migration input or PostgreSQL major changes the fingerprint. `infra/preview/CLUSTER_CONTROL_PLANE.md`
defines the enforcement and activation boundary. This is deliberately not wired into the live
schema reconciler yet: no in-cluster controller, snapshot-capable storage validation, dynamic VPC
service lifecycle, or end-to-end receipt exists, so switching it on would only remove working
previews without establishing the new isolation.

The first controller component is now executable: `PreviewClusterLease` and
`reconcile_cluster_lease` model a durable per-PR lease. A retry advances only snapshot -> cluster
-> connectivity -> ready receipt, rejects contradictory partial inventory, and deletes only
connectivity -> cluster -> lease. It never deletes a fingerprint-keyed snapshot as part of one
PR's close. A changed fingerprint creates a candidate lease while the existing ready lease stays
attached. Persistence, Kubernetes/CNPG actions, receipts and atomic active-candidate switching
remain to be implemented.

The durable request/persistence format is now a bounded, versioned JSON document. It holds only
the PR lease and the trusted artifact/bootstrap digests needed to recompute its identity; it
refuses unknown fields, forged derived names and malformed bytes, and contains neither database
credentials nor Kubernetes/Cloudflare tokens. The existing fuzz target replays both a valid lease
seed and malformed request inputs. It is not an authenticated network endpoint yet.

The matching ready-only receipt format now names the derived per-cluster VPC service and
Hyperdrive configuration and returns only their bounded opaque IDs plus the CNPG cluster UID.
It validates the lease again and cannot carry a connection string or password. The controller
must still verify those IDs against Cloudflare/CNPG before emitting it, and the deployment path
must independently verify the receipt before binding a Preview.

The active PR cluster must have controller-owned CPU/memory limits, `storage` and `walStorage`
PVC capacities, a connection ceiling, and a dedicated namespace aggregate `ResourceQuota`. The
last protects the preview fleet; the individual Cluster/PVC and Pod limits protect each Preview.
Kubernetes RBAC cannot restrict writes by label, so the Kubernetes-side controller must be scoped
to a dedicated namespace and validate names and durable lease state before every write. GitHub
Actions and the Cloudflare Worker stay without Kubernetes credentials.

## Remaining work

1. Implement and activate the Kubernetes-side cluster controller. It must persist a PR lease,
   build/verify/seal the base snapshot, restore a fresh one-instance CNPG Cluster for each PR,
   create unique non-exported credentials, enforce the controller-owned quota profile, and return
   a non-secret receipt. Validate the exact CNPG recovery API against the deployed operator and
   prove that the CSI driver supports the selected snapshot/clone path, that PVC capacities are
   hard limits, and that WAL recovery is correct. Model crash/retry at each lease phase. The
   controller service account must be namespace-scoped; GitHub Actions has no kubeconfig and the
   Cloudflare Worker has no Kubernetes credential.
2. Implement dynamic Cloudflare connectivity for the new clusters. Each cluster needs an exact
   `*-rw` VPC TCP service, a unique Hyperdrive config and a receipt tying their IDs to the lease.
   Extend the Cloudflare resource controller with idempotent create/verify/delete and recovery
   for interrupted updates. Do not use a tunnel-wide VPC network binding. Upgrade the existing
   VPC service TLS configuration to private-CA `verify_full` before trusting new routes; the
   current development services record an unauthenticated encrypted hop.
   The Cloudflare root now accepts an additive, protected `preview_vpc_services` map and validates
   every Preview service name and exact `-rw` hostname; the controller/reconciler still needs to
   materialize the active lease set into that input and consume the resulting service IDs.
   `render-preview-vpc-services.py` now materializes the map deterministically from bounded,
   recomputed provisioning/ready lease documents while omitting deleting leases; it is read-only
   and refuses malformed or duplicate ownership rather than rendering an empty desired state.
   `hyperdrive.sh ensure-preview-cluster`, `verify-preview-cluster` and
   `drop-preview-cluster` now provide the corresponding dedicated-origin lifecycle: they reject
   the shared development service, bind the matching `pr_<n>` login, verify a non-secret
   `{id,name,vpc_service_id}` result and delete only a canonical cluster config. The Kubernetes
   controller still needs to call them (or an equivalent API client), combine the verified ID
   with its CNPG UID into the ready receipt, and recover a partially applied VPC/Terraform update.
   `receipt_from_verified_connectivity` now enforces that combination in the provisioner contract:
   it admits only a ready lease and a verified Hyperdrive name equal to that lease's derived
   identity, so a valid result for another PR/candidate cannot be persisted as this Preview.
3. Define and implement a trusted migration execution and credential contract. Migrate the
   exact PR revision as `pr_<n>` through a bounded, authenticated path; never expose the
   provisioner, deployment or cluster database credentials to PR-controlled code. Demonstrate
   that a migration attempting to write to `public` fails. Order migration before deployment.
   The existing native runner uses a session advisory lock, which Hyperdrive transaction pooling
   does not support. Use a transaction-scoped lock on the Hyperdrive path. SQL transaction-control
   statements can end the outer transaction and release that lock; this must be prevented or the
   migration source must be trusted before the runner accepts PR-provided SQL. The atomic runner
   alone does not validate SQL. `migrate_guarded_with` executes SQL inside a PostgreSQL exception
   block to prevent transaction termination; real-Postgres tests cover escape attempts and a
   restricted preview role. This protects the transaction boundary, not authorization or resource
   isolation, and it has not yet been exercised through live Hyperdrive. Two historical migrations
   search constraints across schemas; when their exact original SQL runs as a preview role, a
   schema-scoped catalog view corrects those lookups without changing recorded migration checksums.
   The view is created and removed in the migration transaction, and existing objects are never
   replaced. `migration-artifact.py` extracts a
   bounded, checksummed list from an exact Git commit without running PR code. Its caller must
   separately prove that the commit is the authorized current PR head; the artifact is untrusted
   data and it does not validate SQL, authorize a run, or establish an execution boundary. The
   `preview-migration-source.yml` workflow prepares that artifact after a PR reconciliation run,
   resolving exactly one eligible open same-repository PR from the run's head SHA and rechecking
   the PR after extraction. It runs trusted scripts from main and never checks out PR code.
   `preview_artifact::Artifact` validates bounded JSON, ordered unique file names and versions,
   SQL digests, and the expected commit before offering borrowed migrations to an executor. A
   digest within an untrusted artifact is not evidence of provenance: the executor still needs
   a trusted source run and must match its inputs to the actual commit. `authorize-migration-artifact.py`
   checks freshly fetched GitHub run, artifact and open-PR metadata, then independently extracts
   the exact commit and emits only the regenerated bytes if the entire downloaded artifact matches.
   The caller must fetch the metadata from GitHub and recheck the PR immediately before use; this
   point-in-time check cannot prevent a push or close after it runs. The read-only
   `preview-migration-verify.yml` workflow exercises those checks after source preparation or
   by source run ID; it does not authorize a later deployment or publish an artifact.
   `migrate_preview_with`
   checks the configured database, login and current role, schema ownership and search path,
   role attributes and memberships, and schema/database CREATE privileges inside the locked
   transaction before applying an artifact. This is not a complete audit of table, function,
   or other grants; the restricted-role bootstrap and its live privileges remain independent
   security boundaries. The preview identity gate also refuses inherited access to relations or
   sequences outside the preview schema (including `public`) and executable non-system
   `SECURITY DEFINER` routines, before any migration SQL runs. A closed-by-default
   `preview-migration-executor` Worker now connects a configured per-PR Hyperdrive to this entry
   point and checks bounded artifact data. Its local
   Worker-runtime test covers disabled configuration, a mismatched binding, malformed data and
   an unavailable database; it is part of the PostgreSQL CI job. The template has no route and
   no deployment workflow. Before enabling it, a trusted caller must prove artifact provenance,
   authorize and recheck the PR head, verify the Hyperdrive origin and grants, and enforce Access.
   In particular, assess the preview login's ability to alter its own role or password and any
   inherited/public table or function grants before accepting untrusted SQL.
   A read-only audit of `thrust-pg-dev` on 2026-11-20 found `pr_338` and `pr_343` had no elevated
   role attributes or memberships, no database/public-schema CREATE, and connection limits of 8.
   Neither had accessible cross-schema table privileges or executable non-system security-definer
   functions. Their role settings contained only their search paths, and the database session's
   statement, lock, and idle-transaction timeout defaults were all zero. This is a point-in-time
   observation, not a complete grant or future-role audit. [Hyperdrive's published limits](https://developers.cloudflare.com/hyperdrive/platform/limits/)
   cap individual query duration at 60 seconds; this does not bound the complete migration run or
   its storage use. Enforce appropriate resource limits before enabling the executor, and account
   for PostgreSQL allowing an ordinary role to change its own password
   ([PostgreSQL 16 ALTER ROLE](https://www.postgresql.org/docs/16/sql-alterrole.html)).
   Test the complete path through a real Hyperdrive binding. These
   point-in-time checks can go stale: a future consumer must validate the artifact, its trusted
   source run and the current PR head again before any privileged action. The triggering PR run
   is not proof of provisioned resources or migration authorization; independently verify both.
4. Derive an isolated Preview configuration from trusted inputs: the reconciled per-PR
   Hyperdrive, isolated bucket and queue, dev rate limits, and Preview-owned Durable Objects.
   The Worker currently requires a queue producer binding even though Previews do not consume
   queues. Define provisioning and cleanup for the extra resources. Do not copy production
   resources or secrets. `scripts/preview-config.py` now renders this profile with explicitly
   supplied resource identifiers, distinct production data resources, Preview-owned rooms and
   disabled email. It does not establish ownership or origin of a supplied ID; its caller must
   verify those against trusted live inventory. It is not yet wired to deployment. Configure
   Access before making Preview URLs reachable. `infra/cloudflare/preview-data-resources.sh` now inventories,
   creates and verifies the reserved R2/Queue pairs, and stages a one-day R2 expiry rule only after a detached
   Preview closes; its generated hermetic controls cover creation, partial-pair refusal, bound-queue refusal and
   delayed cleanup. It is intentionally not wired to Actions: first clear/migrate the legacy prefix and configure
   the separate narrowly scoped R2/Queues token. Cloudflare has no provisionable rate-limit namespace object, so
   a trusted allocator and an audited non-production namespace range are still required before this is a complete
   isolated configuration. Its hermetic controls run in the existing `ci-scripts` job; this does not make the
   control-plane test workflow itself a required repository gate.
5. Deploy the exact built revision, check the actual applied binding identities and smoke the
   Preview, then publish its Flagship target. `scripts/preview-result.py` currently checks
   Hyperdrive, R2 and queue binding names only: establish the live API response shape and add
   fail-closed identity checks or a separately verified binding contract before relying on it.
   A stale run must not publish over a newer commit or a closed PR.
6. Reconcile Preview, Flagship, database cluster, VPC service, Hyperdrive, bucket and queue deletion, including
   missed close events and interrupted updates. Exercise a real PR through open, migrate,
   connect, deploy, close and cleanup; run interrupted Hyperdrive recovery through Actions.

## Deployment action assessment (2026-11-18)

The [official Wrangler action](https://github.com/marketplace/actions/deploy-to-cloudflare-workers-with-wrangler)
can deploy a named Preview and return its URL and IDs. Evaluate it for the trusted upload and
GitHub Deployment reporting; compare with the existing pinned Wrangler invocation in
`scripts/deploy-target.sh`. An action does not replace exact-revision, binding, smoke, Access
or Flagship checks. Pin any adopted action to a reviewed full commit and its Wrangler version.

The [W3Dev deploy action](https://github.com/W3Dev/cloudflare-worker-deploy) is a reference for
native Preview creation, outputs and close-event deletion; assess its handling of prebuilt
artifacts and permissions before adoption. The [Preview URL lookup](https://github.com/marketplace/actions/cloudflare-preview-url)
can help a separate read-only test workflow, subject to its naming support; the deploy itself
already returns the URL. The [shidil](https://github.com/marketplace/actions/cloudflare-workers-preview)
and [mertselimb](https://github.com/marketplace/actions/cloudflare-workers-pr-preview) actions
are oriented around PR-specific Worker deployments rather than this native Preview lifecycle.
None supplies the Postgres provisioner, resource isolation or reconciliation contract.

[Cloudflare's resource matrix](https://developers.cloudflare.com/workers/previews/resources/)
states that Hyperdrive, R2 and queues are shared when their identifiers are shared; a Preview
name by itself does not isolate them.
