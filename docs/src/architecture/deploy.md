# Deploying the Worker

How to set an environment up, deploy to it, and get back out when a deploy goes wrong. How the pipeline is built, and what
each control guarantees, is on [CI/CD](cicd.md#the-deploy-pipeline); this page is what a person does. The decisions behind
it are ADR-0110 (the pipeline and its round-29 and round-30 updates), ADR-0101 (the token that stands in for OIDC) and
ADR-0011 (why old web files are kept). Staging is no longer "a push to `main`", as ADR-0110 has it: it is the release
candidate on an `rc/*` branch, and production a `v*` release tag (the owner's decision of 2026-11-14,
`.agents/plans/cicd-review/`), recorded in ADR-0110's update of 2026-11-15.

**Staging has had deploys through this pipeline (2026-11-15) and its rollback drill has passed (2026-11-16); production has not
been deployed.** What a run does is also what the scripts
do against a fake Cloudflare. The first attempt (run 43676097831, 04:34Z, a manual staging deploy of `d16dbd6`) deferred
correctly to `deploy-do`, which then failed at `wrangler deploy`: Cloudflare refused the request to
`.../workers/scripts/thrust-staging/assets-upload-session`, so nothing was deployed. The token held only Workers read
permissions `[verified 2026-11-15: the same call with the same token gave 403 before the owner added the write permission,
and 200 after]`. Its second attempt, once the token could write, created the Worker (version `9b0550ae`, no smoke: it was
the first deploy). Then, under the `rc/*` flow and from a throwaway `rc/manual-*` branch at `a767aadf`: run 43758233044
(deferred to the manual gate, because the previous release's bundle came from a run on `main`; approved; deployed; the
smoke and its control passed), run 43763466550 (a deliberately wrong `THRUST_HYPERDRIVE_ID`: **failed**, Cloudflare code
10157, the previous version untouched) and run 43763954773 (the right id: deployed, the smoke passed). The list at the end
says what that settled and what has not been shown.

## A run ends in one of five ways

| The run | It means | What to do |
|---|---|---|
| `deploy` green, `deploy-do` skipped | The commit is live, the smoke passed, its control refused a version nobody deployed, and (when there was a previous release) a script only that release had is still served | Nothing |
| `deploy` green, `deploy-do` waiting | `deploy` deferred: the first deploy to this environment, a change to `[[migrations]]`, a deploy of older code than what is live, a live commit it cannot fetch, a live state it cannot read, or a previous release whose web files cannot be found. **Nothing was deployed.** The job summary says which | Read the summary. Approve `deploy-do` if you want it deployed, knowing a Durable Object lifecycle change cannot be rolled back |
| `deploy` red before "deployed" | A check refused it and nothing changed: a missing variable (named in the error), a bundle that failed its attestation or is for another commit, an expand migration that failed, `wrangler` that could not list what is live, or Cloudflare refusing the upload (a `THRUST_HYPERDRIVE_ID` that is not a Hyperdrive config gives `Hyperdrive binding 'HYPERDRIVE' references config '<id>' which was not found [code: 10157]` `[run 43763466550]`, and the previous version keeps serving) | Read the failing step. Every migration is safe to run again, so re-run |
| `deploy` red after "deployed", `rolled_back: yes` | The smoke, its control or the asset control failed, and the previous version is serving again (a probe showed it answer) | The release is not live. Find out why from the run, and fix forward |
| `deploy` red after "deployed", `rolled_back` anything else | No rollback was possible (a lifecycle change, or the first deploy), it failed, or it could not be verified. **A bad version may be serving** | [Roll back by hand](#rolling-back-by-hand), now |

## Setting an environment up

An environment is a GitHub environment with the same set of variables and secrets in each of its two copies (the same
values, except `CLOUDFLARE_API_TOKEN`, which is a different token in each: see "The Cloudflare token" below): `staging` and
`staging-do-lifecycle`, `production` and `production-do-lifecycle`. (`deploy` runs in the first, `deploy-do` in the second;
a copy that is missing a value fails its job with the name of the value.)

- **Reviewers.** Required reviewers on `staging-do-lifecycle`, `production` and `production-do-lifecycle`. `plan` refuses to
  start a deploy if they are missing, because GitHub creates an unprotected environment when a job names one that does not
  exist, which would silently open the manual gate.
- **Branch rules.** `staging` and `staging-do-lifecycle` take deployments only from `rc/*` branches, `production` and
  `production-do-lifecycle` only from `v*` tags (deployment branches and tags, "Selected branches and tags"). GitHub checks
  them against the run's own ref, which is why a staging deploy is a run dispatched on the release candidate's branch.
- **Variables** (none is secret): `CLOUDFLARE_ACCOUNT_ID`, `THRUST_WORKER_NAME`, `THRUST_BASE_URL` (`https://<hostname>`),
  `THRUST_HYPERDRIVE_ID`, `THRUST_R2_BUCKET`, `THRUST_QUEUE`, `THRUST_QUEUE_DLQ`,
  `THRUST_RL_SIGNIN_ID`, `THRUST_RL_V1_ID`, `THRUST_CSRF_ENFORCE` (`on` or `off`, no default, so turning it on for
  production is a decision), `THRUST_EMAIL_FROM` (the address the password reset mails are sent from, one plain
  lowercase address on a domain the account has Email Sending enabled for; no default, so an environment that mails has
  chosen who it mails as. The Worker's links in those mails point at `THRUST_BASE_URL`, which is why there is no second
  variable for them), `DB_ACCESS_HOSTNAME`, `SMOKE_EMAIL`, `SMOKE_WORKSPACE`, and optionally `SMOKE_PROJECT`
  (default `smoke`). `scripts/render-wrangler.sh` checks each one's shape and refuses an all-zero id, a `thrust-spike`
  name, a sender with a name or a list in it, and a sender on a placeholder domain (`example.com`, `.test`, ...).
- **Secrets**: `CLOUDFLARE_API_TOKEN`, `TUNNEL_SERVICE_TOKEN_ID`, `TUNNEL_SERVICE_TOKEN_SECRET`, `DATABASE_URL`
  (`postgres://<role>:<password>@127.0.0.1:15432/thrust`: the host and port are fixed, they are the local end of the tunnel)
  and `SMOKE_PASSWORD`.
- **The Cloudflare token** is a different token in the two copies of an environment, with the names the token picker
  shows `[measured 2026-11-15]`. `staging` and `production`, where `deploy` runs, hold the *ordinary* token: Account:
  **Workers Editor** (the picker's *Workers* role at its Editor level) and **Queues Write**. It updates the existing Worker
  and attaches its queue consumer, and it **cannot create a Worker**. `staging-do-lifecycle` and
  `production-do-lifecycle`, where the reviewer-gated `deploy-do` runs and the first deploy creates the Worker, hold the
  *lifecycle* token: **Workers Admin** and **Queues Write**. Neither has a zone permission, **yet both pass Cloudflare's
  permission check for a Custom Domain write on the zone**: the Custom Domain API is filed under Workers Scripts, not
  Workers Routes. So the domain is protected by review (it is declared in the infrastructure repository, and a plan shows
  any change), not by the token. The picker cannot scope a Workers permission to one Worker, so either token can update
  every Worker in the account. The infrastructure repository's runbook ("Credentials, by step") has the whole table and
  how each token was measured. The token deviates from ADR-0101's OIDC because Cloudflare documents no OIDC federation
  for wrangler.
- **The repository variable `THRUST_DEPLOY_ENABLED`** (a repository variable, not an environment's) switches the automatic
  deploys on when it is `true`: staging for a release candidate, production for a release (which still waits for the
  reviewers). Until then a green `ci` on an `rc/*` pull request or on a tagged push to `main` plans `skip` and nothing else
  runs. Manual runs do not need it.
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

1. Run **deploy** by hand from the open release candidate's `rc/*` branch for staging, or from a published release's `v*`
   tag for production ("Use workflow from"), with the environment and `first_deploy_skip_smoke` ticked.
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

Staging: `ci` passing on the release-candidate pull request, once `THRUST_DEPLOY_ENABLED` is `true` (the workflow then
dispatches itself on the `rc/*` branch for that commit, and the dispatched run deploys it; if release-plz has moved the branch
since, that run skips and the newer commit's own `ci` run starts its own). Or **deploy** by hand from an `rc/*` branch.
Production: `ci` passing on the tagged merge commit of a release, with the same switch (the workflow dispatches itself on the
tag), or **deploy** by hand from a `v*` tag; either waits for the reviewers. `main` never deploys. ([Releasing](release.md) is
the whole path from a merge.) `plan` first checks that the commit's own `gate` passed (asked for by name on that commit) and
that the environments are gated. For staging, `build` then makes the bundle with no secret anywhere in the job, and attests
it; production builds nothing and takes the bundle its release carries, only from a published, immutable release and only the
file GitHub's attestation of that release names. `deploy` verifies the build attestation,
reads what is live, and runs the expand migration only if a file under `crates/adapters/postgres/migrations/` differs
from the live version's commit. It puts the previous release's web files under this release's (below), deploys at 100 %, and
proves it. The job summary has a line for each step.

## Web files and open tabs

A web build has hashed file names. Workers static assets answer a missing hashed file with the single-page fallback, `200
text/html`, so a browser tab still running the last release would be given HTML where it asked for a script, and the app
breaks until the tab is reloaded. The deploy therefore keeps one release back: it fetches the previous release's bundle
(the artifact `deploy-bundle-<its commit>`, kept 90 days), verifies its attestation as it does the new one, and puts its
files under the new release's. Afterwards it asks the live hostname for a script only the previous release had, and requires
it to be served as that script, not as HTML.

If the previous release's bundle cannot be found (expired, never made by this pipeline, or made by a run on `main`: only a
bundle from a run on an `rc/*` branch or a `v*` tag counts), `deploy` defers to the manual gate: approving it deploys
without keeping the old files, and open tabs may break until they reload. So the **first deploy after a version that was
deployed from `main`** always takes the gate (run 43758233044 did, on 2026-11-15). If it is found and fails
a check (a forged or damaged one), the run fails, and there is no way past that at the gate.

## What the deploy compares with what is live

The live version's tag is the commit that is running. `deploy` compares that commit with the one it is deploying by what they
hold: the files under `crates/adapters/postgres/migrations/` (a difference runs the expand migration) and the
`[[migrations]]` tables of `wrangler.deploy.toml` (a difference is a Durable Object lifecycle change, below). Neither commit has to
descend from the other. release-plz force-updates the release candidate's branch, so the commit that is live is usually in no
branch of the checkout any more; `deploy` asks GitHub for that one commit by its sha, with no secret, before it compares. GitHub
serves a commit no branch holds to an anonymous fetch `[verified 2026-11-16: the earlier tip of
rc/2026-11-16T02-58-22Z, which release-plz had just force-updated, fetched by its sha into an empty repository; a sha that exists
nowhere is refused]`.

It defers to the manual gate, deploying nothing, when it cannot say: there is no live version, the live tag is not a commit, the
live commit could not be fetched, the checkout is too shallow to say which of the two is older, or **the deploy is older than
what is live** (a redeploy of earlier code is a person's decision). The job summary gives the reason, and says when the two were
compared by content alone. A diverged or out-of-order migration is refused by the database side (`migrate`'s ledger, issue #195),
which the comparison never was.

## Deploying a Durable Object lifecycle change

A change to `[[migrations]]` in `crates/worker/wrangler.deploy.toml` cannot be rolled back across and is not gradual
(ADR-0110, ADR-0090). `deploy` defers it, you approve `deploy-do`, and it deploys with no automatic rollback. Plan to fix
forward: if the new version is bad, deploying the next commit is the way out, because Cloudflare refuses a rollback past the
migration. A lifecycle change that has to ship together with an API change reopens ADR-0090.

## The rollback drill

Run **deploy** by hand on `staging`, from the `rc/*` branch just deployed, with `inject_smoke_failure` ticked, after a normal deploy. Every real check passes and
then the smoke fails on purpose. Expect a red run, a `wrangler rollback` to the previous version, and a line saying the
previous version answers. It is refused for production, and together with `first_deploy_skip_smoke`.

It has been run once: [run 43811076183](https://github.com/aRustyDev/thrust/actions/runs/43811076183), on 2026-11-16, from a
throwaway branch `rc/manual-drill` at the live commit `0c2c0f3`. Staging was on version `5cd7ec87`. The deploy made `4b508caf`,
every real check passed (three answers carried the tag, the smoke account signed in and read the project), and the injected
failure then put staging back on `5cd7ec87` (`rollback of 0c2c0f3…: the smoke failed` in `deployments list`) and ended the job
red. The run redeployed the same commit, so the script's own check after the rollback, which compares tags, could not tell the
two versions apart; what shows the rollback is the version id in `wrangler`'s output and in `deployments list`. A drill from a
different commit would exercise that check too. When the drill was run, a newer release candidate could not be used: it was
diverged from what was live, and the classifier stopped it at the manual gate before the smoke. It no longer does (the
classifier now compares by content, [above](#what-the-deploy-compares-with-what-is-live)), so a newer release candidate with no
Durable Object change in it should get as far as the smoke, which makes it a drill from a different commit (not yet tried on
staging). An older commit is still stopped.

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

- **The Cloudflare tokens.** Make each new token with the same scopes (the ordinary one for `staging` and `production`, the
  lifecycle one for the two `-do-lifecycle` copies), set it as `CLOUDFLARE_API_TOKEN` there, deploy the same commit to
  staging (an ordinary deploy that changes nothing), then revoke the old token. Editing a token's permissions in
  Cloudflare keeps its value, so that needs nothing changed in GitHub.
- **The Access service token** (the infrastructure repository makes it, outside Terraform): set the new
  `TUNNEL_SERVICE_TOKEN_ID` and `TUNNEL_SERVICE_TOKEN_SECRET` in all four environments before the old one expires. A refused
  token shows as `migrate` failing to connect, with the tail of `cloudflared`'s log printed; the expand step only runs when a
  migration changed, so an expired token is found by the next deploy that has one.
- **The database role's password**: change it in the database, in the Hyperdrive configuration, and in `DATABASE_URL` of all
  four environments.
- **The Origin CA root** (`.github/deploy/cloudflare-origin-ca-rsa-root.pem`) expires on 2029-08-15. Replace it here and in the
  infrastructure repository before then; a test pins the fingerprint, so the change has to be made on purpose.

## What the first staging deploys settled

**Settled on 2026-11-15**, by runs 43676097831 (second attempt), 43758233044, 43763466550 and 43763954773:

- the shape of `wrangler deployments list --json` and `versions list --json`: the parser read both (`state=none`, then
  `state=single` with the version id and its tag);
- that `wrangler deploy` attaches the queue consumer with its dead-letter queue (Cloudflare shows the consumer with
  `thrust-staging-events-dlq`);
- the attestation flow end to end: `deploy` verified the bundle's attestation in every run;
- that the workflow's own token can read an environment's protection rules: `plan` read them and passed;
- the route to the database and Access accepting the service token, **from a GitHub-hosted runner**: `migrated (tls:
  verify-ca)`, which also shows `verify-ca` meeting a real Origin CA certificate;
- a wrong `THRUST_HYPERDRIVE_ID` fails the deploy with Cloudflare's code 10157 before anything is served, and the
  control, the same dispatch with the right id, passes;
- the token's scopes, in part: Workers Editor and Queues Write do an ordinary deploy, and creating the Worker takes Workers
  Admin. Whether anything narrower would do was not tried.

**Settled on 2026-11-16:**

- the keeping of web files over two real releases: run 43796469831 deployed `0612ae3` over `3679c70` and kept 138 files only
  the earlier release had (`merge-assets: new=1366 retained=138 total=1504`); run 43803593395 deployed `0c2c0f3` over `e4da5b3`
  and kept 29; the control that asks for a script of the earlier release passed in both. The log does not say whether that
  control found a retired script to request or had nothing to look for;
- the manual gate: runs 43796746005 and 43803593395 stopped at `staging-do-lifecycle` with nothing deployed until the owner
  approved them, then deployed with `automatic rollback: false`;
- the rollback drill (above).

**Found by those runs, and changed since:** the gate stopped runs 43796746005 and 43803593395 for a reason that was not a Durable
Object change. The classifier failed closed when the live commit was not an ancestor of the new one ("not an ancestor", and "not
a commit in this checkout"), and release-plz force-updates the release candidate's branch, so the next candidate's history
usually lacks the live commit. The classifier now fetches the live commit by its sha and compares by content ([above](#what-the-deploy-compares-with-what-is-live)).
That is tested against a made-up history with diverged commits and against a fetch of a commit no branch holds, and **has not run
on staging yet**: the first release candidate after it is the check.

**Still not shown:** production has not been deployed through the pipeline.
