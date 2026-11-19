# Per-PR previews (#57)

Thrust owns reconciliation and the Access-protected provisioner; GitHub Actions uses no
Kubernetes credential. The development database bootstrap, dedicated Hyperdrive, Worker,
Access policy and service token are live. Reconciliation creates per-PR roles, schemas and
Hyperdrives, repairs interrupted credential updates, and sweeps closed PRs. PR #298's
creation and cleanup were observed through Actions and the provisioner inventory. The
provisioner checks are part of the required CI gate (#298). A synthetic direct live test
exercised password-update recovery; the interrupted-update path still needs an Actions test.

## Remaining work

1. Define and implement a trusted migration execution and credential contract. Migrate the
   exact PR revision as `pr_<n>` through a bounded, authenticated path; never expose the
   provisioner, deployment or shared database credentials to PR-controlled code. Demonstrate
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
   a trusted source run and must match its inputs to the actual commit. These
   point-in-time checks can go stale: a future consumer must validate the artifact, its trusted
   source run and the current PR head again before any privileged action. The triggering PR run
   is not proof of provisioned resources or migration authorization; independently verify both.
2. Derive an isolated Preview configuration from trusted inputs: the reconciled per-PR
   Hyperdrive, isolated bucket and queue, dev rate limits, and Preview-owned Durable Objects.
   The Worker currently requires a queue producer binding even though Previews do not consume
   queues. Define provisioning and cleanup for the extra resources. Do not copy production
   resources or secrets. `scripts/preview-config.py` now renders this profile with explicitly
   supplied resource identifiers, distinct production data resources, Preview-owned rooms and
   disabled email. It does not establish ownership or origin of a supplied ID; its caller must
   verify those against trusted live inventory. It is not yet wired to deployment. Configure
   Access before making Preview URLs reachable.
3. Deploy the exact built revision, check the actual applied binding identities and smoke the
   Preview, then publish its Flagship target. `scripts/preview-result.py` currently checks
   Hyperdrive, R2 and queue binding names only: establish the live API response shape and add
   fail-closed identity checks or a separately verified binding contract before relying on it.
   A stale run must not publish over a newer commit or a closed PR.
4. Reconcile Preview, Flagship, database, Hyperdrive, bucket and queue deletion, including
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
