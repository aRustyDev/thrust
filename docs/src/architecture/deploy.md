# Deploying the Worker

How to set an environment up, deploy to it, and get back out when a deploy goes wrong. How the pipeline is built, and what
each control guarantees, is on [CI/CD](cicd.md#the-deploy-pipeline); this page is what a person does. The decisions behind
it are ADR-0110 (the pipeline and its round-29 and round-30 updates), ADR-0101 (the token that stands in for OIDC) and
ADR-0011 (why old web files are kept).

**No environment has had a deploy through this pipeline yet.** Everything below that says what a run does is what the
scripts do against a fake Cloudflare, and what `wrangler deploy --dry-run` and two `workflow_run` events have shown. The
first staging deploy is the check, and the list at the end says what it settles.

## A run ends in one of five ways

| The run | It means | What to do |
|---|---|---|
| `deploy` green, `deploy-do` skipped | The commit is live, the smoke passed, its control refused a version nobody deployed, and (when there was a previous release) a script only that release had is still served | Nothing |
| `deploy` green, `deploy-do` waiting | `deploy` deferred: the first deploy to this environment, a change to `[[migrations]]`, a live state it cannot read, or a previous release whose web files cannot be found. **Nothing was deployed.** The job summary says which | Read the summary. Approve `deploy-do` if you want it deployed, knowing a Durable Object lifecycle change cannot be rolled back |
| `deploy` red before "deployed" | A check refused it and nothing changed: a missing variable (named in the error), a bundle that failed its attestation or is for another commit, an expand migration that failed, `wrangler` that could not list what is live | Read the failing step. Every migration is safe to run again, so re-run |
| `deploy` red after "deployed", `rolled_back: yes` | The smoke, its control or the asset control failed, and the previous version is serving again (a probe showed it answer) | The release is not live. Find out why from the run, and fix forward |
| `deploy` red after "deployed", `rolled_back` anything else | No rollback was possible (a lifecycle change, or the first deploy), it failed, or it could not be verified. **A bad version may be serving** | [Roll back by hand](#rolling-back-by-hand), now |

## Setting an environment up

An environment is a GitHub environment with the same set of variables and secrets in each of its two copies: `staging` and
`staging-do-lifecycle`, `production` and `production-do-lifecycle`. (`deploy` runs in the first, `deploy-do` in the second;
a copy that is missing a value fails its job with the name of the value.)

- **Reviewers.** Required reviewers on `staging-do-lifecycle`, `production` and `production-do-lifecycle`. `plan` refuses to
  start a deploy if they are missing, because GitHub creates an unprotected environment when a job names one that does not
  exist, which would silently open the manual gate. Restricting deployment branches to `main` is worth setting too.
- **Variables** (none is secret): `CLOUDFLARE_ACCOUNT_ID`, `THRUST_WORKER_NAME`, `THRUST_BASE_URL` (`https://<hostname>`),
  `THRUST_HYPERDRIVE_ID`, `THRUST_R2_BUCKET`, `THRUST_QUEUE`, `THRUST_QUEUE_DLQ`,
  `THRUST_RL_SIGNIN_ID`, `THRUST_RL_V1_ID`, `THRUST_CSRF_ENFORCE` (`on` or `off`, no default, so turning it on for
  production is a decision), `DB_ACCESS_HOSTNAME`, `SMOKE_EMAIL`, `SMOKE_WORKSPACE`, and optionally `SMOKE_PROJECT`
  (default `smoke`). `scripts/render-wrangler.sh` checks each one's shape and refuses an all-zero id and a `thrust-spike`
  name.
- **Secrets**: `CLOUDFLARE_API_TOKEN`, `TUNNEL_SERVICE_TOKEN_ID`, `TUNNEL_SERVICE_TOKEN_SECRET`, `DATABASE_URL`
  (`postgres://<role>:<password>@127.0.0.1:15432/thrust`: the host and port are fixed, they are the local end of the tunnel)
  and `SMOKE_PASSWORD`.
- **The Cloudflare token** is scoped to Workers scripts (Account > Workers Scripts > Edit), and most likely Queues (a deploy
  attaches the queue consumer) `[unverified: the first deploy shows the narrowest set that works]`. It has no zone
  permission: the Custom Domain is attached by the infrastructure repository. It deviates from ADR-0101's OIDC because
  Cloudflare documents no OIDC federation for wrangler.
- **The repository variable `THRUST_DEPLOY_ENABLED`** (a repository variable, not an environment's) switches deploys after a
  push to `main` on when it is `true`. Until then a push plans `skip` and nothing else runs. Manual runs do not need it.
- **What is behind the variables**, in the infrastructure repository: for staging, a schema and a login role on
  `thrust-pg-dev` (with `pg_trgm` installed in `public` by an administrator, which migration `0006` needs and a schema-only
  role cannot do), a Hyperdrive configuration for that role, an R2 bucket, a queue and its dead-letter queue, two
  rate-limit namespace ids (sign-in and `/api/v1`) that differ from each other and are not `1001` to `1004`, the Custom Domain, and the tunnel, Access application and service
  token that `scripts/migrate-ci.sh` uses. The CA that signs `thrust-pg`'s certificate is in the repository
  (`.github/deploy/`, expires 2029-08-15).
- **The smoke account**, once per environment and by hand, after the hostname answers:
  `SMOKE_EMAIL=… SMOKE_PASSWORD=… SMOKE_WORKSPACE=… scripts/smoke-bootstrap.sh https://<hostname>`. It signs up the account,
  makes a workspace and a project, and is safe to run again. Keep the password in the password manager and give the same
  three values to both copies of the environment.

## The first deploy to an environment

The Worker does not exist yet, so nothing can be attached to a hostname and the smoke has nowhere to look.

1. Run **deploy** by hand: environment, `ref` `main`, and `first_deploy_skip_smoke` ticked.
2. `deploy` finds no live version, defers, and deploys nothing. Approve `deploy-do`. It runs the expand migration (the whole
   schema), creates the Worker with its Durable Object migration `v1`, and skips the smoke. There is no automatic rollback and
   nothing proves the version works yet; the run says so.
3. The infrastructure repository attaches the Custom Domain to the Worker.
4. Run `smoke-bootstrap.sh` (above).
5. Run **deploy** again, for the same commit, without the tick. The live version is now that commit, so nothing is deferred:
   the smoke runs, and so do its control and (there being no older release) nothing else.

`first_deploy_skip_smoke` is refused for an environment that already has a live version, before anyone is asked to approve
anything, and together with `inject_smoke_failure`.

## An ordinary deploy

A push to `main` after `ci` passed, once `THRUST_DEPLOY_ENABLED` is `true`; or **deploy** by hand with a commit of `main`.
`plan` first checks that the commit's own `gate` passed (asked for by name on that commit) and that the environments are
gated. Then `build` makes the bundle with no secret anywhere in the job, and attests it. `deploy` verifies the attestation,
reads what is live, and runs the expand migration only if a file under `crates/thrust-adapters-postgres/migrations/` differs
from the live version's commit. It puts the previous release's web files under this release's (below), deploys at 100 %, and
proves it. The job summary has a line for each step.

## Web files and open tabs

A web build has hashed file names. Workers static assets answer a missing hashed file with the single-page fallback, `200
text/html`, so a browser tab still running the last release would be given HTML where it asked for a script, and the app
breaks until the tab is reloaded. The deploy therefore keeps one release back: it fetches the previous release's bundle
(the artifact `deploy-bundle-<its commit>`, kept 90 days), verifies its attestation as it does the new one, and puts its
files under the new release's. Afterwards it asks the live hostname for a script only the previous release had, and requires
it to be served as that script, not as HTML.

If the previous release's bundle cannot be found (expired, or never made by this pipeline), `deploy` defers to the manual
gate: approving it deploys without keeping the old files, and open tabs may break until they reload. If it is found and fails
a check (a forged or damaged one), the run fails, and there is no way past that at the gate.

## Deploying a Durable Object lifecycle change

A change to `[[migrations]]` in `crates/thrust-worker/wrangler.deploy.toml` cannot be rolled back across and is not gradual
(ADR-0110, ADR-0090). `deploy` defers it, you approve `deploy-do`, and it deploys with no automatic rollback. Plan to fix
forward: if the new version is bad, deploying the next commit is the way out, because Cloudflare refuses a rollback past the
migration. A lifecycle change that has to ship together with an API change reopens ADR-0090.

## The rollback drill

Run **deploy** by hand on `staging` with `inject_smoke_failure` ticked, after a normal deploy. Every real check passes and
then the smoke fails on purpose. Expect a red run, a `wrangler rollback` to the previous version, and a line saying the
previous version answers. It is refused for production, and together with `first_deploy_skip_smoke`. No drill has been run yet;
when one has, its run is linked here.

## Rolling back by hand

`wrangler rollback <version-id> --name <worker> --message '<why>'`, with the account's credentials. Find the version with
`wrangler deployments list --name <worker>` (the live one is the latest deployment) and `wrangler versions list --name
<worker>`. A version carries the commit it was built from as its tag. Know what it does not do: it does not change the
database, the bucket or the queue (the schema stays expanded, which the previous code runs on, by the expand/contract rule);
it works only for the last 100 versions; Cloudflare refuses it across a Durable Object lifecycle change or when the version
it goes back to binds an R2 bucket, KV namespace or queue that no longer exists `[verified 2026-11-14 against Cloudflare's
Rollbacks page]`. Afterwards run the smoke by hand against the previous commit
(`scripts/smoke-deploy.sh https://<hostname> <commit>`).

## Rotating credentials

- **The Cloudflare token.** Make the new token with the same scopes, set it as `CLOUDFLARE_API_TOKEN` in all four
  environments, deploy the same commit to staging (an ordinary deploy that changes nothing), then revoke the old token.
- **The Access service token** (the infrastructure repository makes it, outside Terraform): set the new
  `TUNNEL_SERVICE_TOKEN_ID` and `TUNNEL_SERVICE_TOKEN_SECRET` in all four environments before the old one expires. A refused
  token shows as `migrate` failing to connect, with the tail of `cloudflared`'s log printed; the expand step only runs when a
  migration changed, so an expired token is found by the next deploy that has one.
- **The database role's password**: change it in the database, in the Hyperdrive configuration, and in `DATABASE_URL` of all
  four environments.
- **The Origin CA root** (`.github/deploy/cloudflare-origin-ca-rsa-root.pem`) expires on 2029-08-15. Replace it here and in the
  infrastructure repository before then; a test pins the fingerprint, so the change has to be made on purpose.

## What the first staging deploy settles

Not verified until then: the shape of `wrangler deployments list --json` and `versions list --json` (the parser sends any
other shape to the manual gate, never to an automatic rollback); the token's narrowest scopes; that `wrangler deploy`
attaches the queue consumer with its dead-letter queue; the attestation flow end to end (`gh attestation verify` with the
flags in `verify-bundle.sh`); that the workflow's own token can read an environment's protection rules (if it cannot, `plan`
fails closed with a message that says so); the route to the database and Access accepting the service token; `verify-ca`
meeting a real Origin CA certificate (the root is known to be accepted as a trust anchor); and the retention of web files
over two real releases.
