---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0110: Deployment pipeline, progressive delivery and rollback

## Context and Problem Statement

We must define how merged code reaches Cloudflare (and the self-hosted twin) safely. Verified CF facts: Wrangler can
`versions upload` without deploying, then `versions deploy` with a traffic split (gradual deployment); you can only
create a deployment from the **last 100** uploaded versions; rollback creates a new deployment from a prior version
but **does not change connected resources** (so data-structure changes can break old code); gradual deployments cause
**version skew** between versions and between Workers linked by service bindings (mitigated by version affinity via the
`Cloudflare-Workers-Version-Key` header and version overrides); **Durable Object lifecycle changes** are atomic, deployed
only via `wrangler deploy`, can't be gradual, and rollbacks cannot cross them. Plane today deploys via images/compose and
a `feature-deployment.yml` preview workflow **[S]**.

## Decision Drivers

* Zero-downtime deploys; fast rollback.
* DB schema changes must be compatible with N and N−1 code (rollbacks and skew).
* Keep DO class lifecycle changes isolated (ADR-0090).
* Static assets use content-hashed names: affinity avoids 404s mid-rollout.
* Deploy identity via OIDC, no long-lived tokens (ADR-0101).

## Considered Options

* Option 1: `wrangler deploy` (100% immediately) on merge to main, per Worker
* Option 2: `versions upload` on merge → gradual deployment (e.g. 10% → 50% → 100%) with version affinity and automated health gates; DO lifecycle changes via a separate manual-gated `wrangler deploy`; expand/contract DB migrations run first
* Option 3: OpenTofu-managed `cloudflare_workers_deployment` as the release mechanism

## Decision Outcome

**Decision (accepted, round 24): Option 2 with measured gates** — expand migration (CI) → upload at 0% traffic → smoke suite and oracle subset via the version-override header → 10% for 10 minutes → 100%; absolute-count gates (≤3 5xx, probe p95 < 500 ms, no CPU-limit exceptions) with automatic `wrangler rollback`; DO lifecycle changes via a manual-gated `wrangler deploy` (see the round-24 update).

**Superseded in part (round 29), implemented (round 30), environments changed by the release flow (2026-11-15, below):** the stages, the version-override smoke and the absolute-count gates of this decision were dropped in round 29 for a 100 % deploy proved by the smoke; the rest (expand migration first, the DO-lifecycle manual gate, automatic rollback) stands, and the pipeline is built. Read the round-29 and round-30 updates below for what is in force.

(Earlier hypothesis, retained as rationale — the accepted outcome is above: Leading hypothesis: **Option 2** (confidence: medium).
Order of operations: (1) expand-phase DB migration; (2) deploy callee Workers (`collab`, `consumer`) compatibly;
(3) `api` gradual with version affinity keyed on user/session; (4) contract-phase migration in a later release.
Self-hosted: images tagged by release-please version, rolled via Compose/K8s rolling update.)

### Consequences

* Good, because errors are caught on a slice of traffic; rollback is one command (within the 100-version window).
* Bad, because every migration must be expand/contract; pipeline needs health gates (error rate from Workers metrics/OTel, ADR-0130).
* Neutral: preview environments per PR are a separate question (Q-0092.2).

### Confirmation

Pipeline integration test: simulate deploying N+1 `api` next to N `consumer`; schema N−1 compatibility test (ADR-0121
fixtures); runbook drill for rollback and for a DO-lifecycle deploy.

## Pros and Cons of the Options

### Option 1: Direct deploy

* Good, because simple.
* Bad, because no blast-radius control; skew-free but all-or-nothing.

### Option 2: Gradual + affinity + gates

* Good, because supported platform feature; observable in Workers Metrics release annotations.
* Bad, because skew handling and migration discipline required; DO changes can't use it.

### Option 3: TF-driven deployment

* Good, because auditable in IaC.
* Bad, because slows deploys, awkward gating, conflicts with ADR-0093 leading option.

## Update — interview round 16 (2026-10-17)

Direction: expand-then-contract migrations run by CI before deploy (Q-0110.2); gradual deployments with a health gate. Gate signal and thresholds (Q-0110.1) remain open, so this stays `proposed`.

## Update — interview round 24 (2026-10-18)

Accepted: **Option 2 with measured gates** (Q-0110.1). Order: expand migration (CI) → upload the new version at **0% traffic** → run the **smoke suite and an oracle subset against it via the version-override header** → promote to **10% for 10 minutes** → promote to 100%. Gates use **absolute counts, not rates**, because ≤50 users make percentage slices statistically meaningless: smoke 100% pass, **≤3 5xx**, synthetic-probe **p95 < 500 ms**, **no CPU-limit exceptions**. Any gate failure triggers an automatic `wrangler rollback`. Durable Object lifecycle changes use a separate manual-gated `wrangler deploy`. Self-hosted: rolling update gated on the health endpoint. Version-override header behaviour to be confirmed against current Cloudflare docs during implementation.

## Update — review fixes, round 25 (2026-10-18)

Outcome rewritten to the accepted decision; the earlier hypothesis and order of operations are kept as rationale.

## Update — review round 29 (2026-10-19)

Per FR-12 (`reviews/2026-10-19-fable-architecture-review.md`; the owner's decision is its §5 table: apply now, "ADR-0110's rollout stages superseded"), the release pipeline is: **expand migration (CI) → `wrangler deploy` at 100 % → the smoke suite against the live hostname → automatic `wrangler rollback` on failure.** This supersedes the rollout stages of the round-24 Decision and update (upload at 0 %, the smoke suite and an oracle subset through the version-override header, 10 % for 10 minutes, then 100 %) and the absolute-count gates on that slice (≤3 5xx, probe p95 < 500 ms, no CPU-limit exceptions); the deploy gate is now the smoke suite, and Q-0110.1's round-24 answer goes with the slice. The p95 and availability targets are watched by the SLO probes (Q-G5.2; ADR-0094, round 29), not by the deploy.

Why: `research/0004` §9 assumes about 5 M requests a month (about 2 a second), so 10 % for 10 minutes is roughly 100 requests, most of them the owner's; the round-24 note already says percentage slices are statistically meaningless at ≤50 users; and a gate on that slice cannot observe anything the smoke suite does not. Dropping the stages also drops the version-override plumbing that would have to be built and maintained. The cost: a bad version serves every user from the deploy until the smoke suite fails and the rollback lands. Gradual rollout is revisited when traffic makes a 10-minute slice meaningful.

What stays:

* **Expand/contract discipline.** Every migration is compatible with N and N−1 code, because `wrangler rollback` does not change connected resources (Context).
* **The DO-lifecycle rule.** A Durable Object lifecycle change can be neither gradual nor rolled back across, so it ships through its own manual-gated `wrangler deploy`. With one Worker (ADR-0090, round 29) no DO class is in production before the P1 room, and a DO lifecycle change that must ship with an API change reopens ADR-0090.
* **Self-hosted:** the rolling update gated on the health endpoint (round 24) is unchanged.

With one Worker there is no callee-first ordering at P0 (step 2 of the earlier order of operations); it returns with the `collab` Worker at P1. The Confirmation's simulation of N+1 `api` next to N `consumer` has no subject while one Worker serves both; the schema N−1 compatibility test and the rollback and DO-lifecycle drills stay. The "affinity avoids 404s mid-rollout" driver no longer holds, because a 100 % deploy has no version affinity (second open item below).

Two facts the review leaves out are open items for the implementing issue (issue #49):

1. **Where the expand migration runs.** Q-0110.2 (round 16) says CI runs the expand migrations before the deploy, but CI has no network route to `thrust-pg` today. The database has no public endpoint and its tunnel serves the Workers VPC services, not WARP (ADR-0094); the tunnel is declared with no route (`research/0010` §3); and the one administrative path the infra scripts use is `kubectl exec` on the dev primary over its local socket, with a PR's migrations listed as not automated (`research/0010` §2, "Per-PR schemas"). No file under `.github/` references `thrust-pg` or Hyperdrive; the Postgres adapter tests start their own container on the runner (`.github/workflows/ci.yml:155-172`). Where the expand migration runs is part of issue #49. **Answered in round 30, below.**
2. **Hashed assets at 100 %.** ADR-0011 (round 26, `adrs/ui/0011-static-asset-hosting.md:76`) records that a missing hashed asset is answered `200 text/html`, not 404, and says to deploy with version affinity and keep old hashes until clients age out. A 100 % deploy has no version affinity, so it must keep the previous build's hashed assets; how is part of issue #49. **Answered in round 30, below.**

## Update — implementation, round 30 (2026-11-14)

The round-29 pipeline is built (issue #49, in four pull requests: #153, #154, #165 and the one that records this note). What it
decided where the ADR was open, and where it differs:

* **The two open items of round 29, answered.**
  1. *Where the expand migration runs:* in the deploy job, on a GitHub-hosted runner, through `cloudflared access tcp` to
     a tunnel with an Access application and a service token (the ADR-0094 fallback), over TLS that `migrate` verifies against
     the CA (`DATABASE_SSLMODE=verify-ca`; `require` and `verify-full` are refused, so a setting that cannot be honoured is an
     error and not a downgrade). It runs only when a file under the migrations directory differs between the live version's
     commit and the new one, because `migrate` re-runs every file each time, `DROP` and `ADD CONSTRAINT` included. The route is
     the infrastructure repository's: a **separate** tunnel from the `thrust-pg` one, which is declared with no route on
     purpose. It has not yet run against the real database.
  2. *Hashed assets at 100 %:* the deploy keeps **one release back**. The previous release's own web build, taken from its
     attested bundle, goes under the new one's (`scripts/merge-assets.sh`), and afterwards a script only the previous release had
     must be served as itself, not as `text/html` (`scripts/check-asset-retention.sh`). Chosen over answering a missing asset
     with 404, which collides with Q-0050.7 (#50). This covers `web` only; whether `space` shares the asset directory
     (ADR-0011, round-26 correction) is not settled here.
* **Version identity.** `wrangler deploy --tag <commit>`; the Worker sends the tag, else the version id, as `x-thrust-version`;
  the smoke requires the commit just deployed and a real read to work. The smoke is then asked for a version nobody deployed
  and must refuse it: a smoke that passes that cannot tell one version from another, and the run fails on it.
* **The manual gate, and a correction to round 29.** Round 29 says that with one Worker no Durable Object class is in
  production before the P1 room. The Worker already declares `RoomDo` with the migration `v1`, so the **first deploy to any
  environment is itself a lifecycle change**: it has no previous version to go back to and Cloudflare refuses a rollback
  across it. It takes the manual path: `deploy` finds that out first and deploys nothing, and `deploy-do`, in an environment
  with required reviewers, deploys without an automatic rollback. A change to `[[migrations]]`, a live state the pipeline cannot
  read, and a previous release's web files that cannot be found take the same path.
* **Deploy identity: a scoped Cloudflare API token, not OIDC.** This deviates from the Decision Driver above and from ADR-0101
  (see its round-30 note). Cloudflare documents no OIDC federation for wrangler, only an API token and the account id; the owner
  decided on 2026-11-13 to hold the token in each environment's secrets and keep OIDC as a follow-up.
* **Environments.** `staging` (a push to `main` after `ci`, switched on by the repository variable `THRUST_DEPLOY_ENABLED`) and
  `production` (by hand, behind reviewers), each with a `-do-lifecycle` twin. Preview environments per PR stay with #57 and
  Q-0092.2.
* **Still open, not built:** a migration ledger and an advisory lock (`migrate` has neither; the pipeline serialises deploys
  per environment and never cancels a queued one); the schema N−1 compatibility test of the Confirmation; a decision on
  `THRUST_CSRF_ENFORCE` for production (it is a per-environment variable with no default); gradual rollout, which is revisited
  when traffic makes a slice meaningful; `space`.
* **Confirmation, as it stands.** The rollback drill is a manual run on staging with `inject_smoke_failure`, and the
  DO-lifecycle drill is the first deploy to staging; neither has been run, because no environment exists yet. The pipeline's
  control flow was run against a fake Cloudflare that keeps state, and `wrangler deploy --dry-run` accepts its rendered
  configuration. The steps are in `docs/src/architecture/deploy.md`.

## Update — the release flow (2026-11-15)

The owner's decisions of 2026-11-14 and 2026-11-15 (`.agents/plans/cicd-review/`) change **what** reaches each environment and
**from where**; the pipeline above (expand migration, 100 %, the smoke, rollback, the manual gate) is unchanged. This supersedes the
round-30 "Environments" bullet.

* **Releases.** release-plz (`release-plz.toml`, `release-plz.yml`) keeps a release-candidate pull request from an `rc/<timestamp>`
  branch after every push to `main`, with the version and `CHANGELOG.md`. Merging it tags `vX.Y.Z` on the merge commit and creates
  the GitHub release as a draft. Only the release GitHub App and admins can write `rc/*`; only the App can make a `v*` tag; releases
  are immutable once published.
* **Staging is the release candidate,** not a push to `main`: `ci` passing on the rc pull request (which always runs in full)
  dispatches the deploy on the `rc/*` branch. GitHub checks an environment's branch rule against the run's own ref and a
  `workflow_run` runs on `main`, hence the dispatch. `staging` and its twin take only `rc/*` branches. `main` never deploys.
* **Production is the release,** from its tag: `ci` passing on the tagged merge commit dispatches the deploy on the tag, and it
  waits for the `production` reviewers. `production` and its twin take only `v*` tags.
* **Built once, at the tag** (the owner's choice on 2026-11-15, over promoting staging's bundle). `release.yml` builds the bundle at
  the tag, attests its build provenance and an SPDX SBOM of the source, attaches the bundle, the SBOM and `SHA256SUMS` to the draft,
  and publishes it; GitHub then attests the immutable release and its assets. Production deploys that asset and builds nothing. The
  bundle staging tested was built from the rc commit, whose tree is the tag's (a squash merge of the rc branch), with the same
  locked inputs, but it is a different build: production does not run byte-for-byte what staging ran. Promoting staging's bundle
  would have made the deployed version the rc commit, which nothing on `main` points at once the branch is deleted, and would have
  stopped production until staging works.
* **Still open:** the previous production release's web files come from its 90-day artifact, not yet from the release itself.
* **Settled by the first release, `v0.1.0` (2026-11-15).** The App made the `v*` tag through the tag ruleset without the
  Administration permission. `release.yml` did not start on the draft: GitHub runs no workflow for a draft release's `created`
  event. release-plz.yml now dispatches `release.yml` on the tag it made, so the run's ref is still the tag.

## Update — one hostname, N targets (2026-11-16)

ADR-0111 changes where each kind of deploy lands. The pipeline above is unchanged in kind: an attested bundle, the
expand migration, the smoke with its control, the manual gates and rollback.

* **Production** deploys as before (`wrangler deploy` at 100 %, the smoke, `wrangler rollback`). It is reached only
  through `thrust-edge` on `thrust.sow.foo`, so its smoke goes through the edge. A contract migration
  (`NNNN_contract_*.sql`, ADR-0042's 2026-11-16 update) runs only in this deploy, and only when no older shared target
  is live.
* **Staging is no longer a Worker of its own.** The release candidate deploys as a Preview of the production Worker on
  production's data (the `shared` profile), named after its release, and `alias-staging` points at it.
  * It migrates production's database only when it is the newest shared target and its migrations changed, and only
    after a reviewer approves (the owner, 2026-11-15).
  * Its smoke compares the Preview deployment id, because a Preview leaves the version tag empty (`research/0016`).
* **`dev-*` targets** (pull requests) deploy as isolated Previews. That answers Q-0092.2's preview environments.
* **`thrust-edge`** deploys only with a release, at 100 %, with its own smoke and rollback.
* **Retirement:** `thrust-staging`, `thrust-staging.sow.foo` and staging's own data go once the first release target
  has passed its drill.

## Open Questions

* Q-0110 — pipeline shape. Answered: round 24, superseded in part by round 29, implemented in round 30.
* Q-0110.1 — Health-gate signal and thresholds for auto-promotion/rollback. Answered: round 24's gates went with the rollout stages in round 29; the gate is the smoke and its control.
* Q-0110.2 — Migration runner: who runs expand migrations on Hyperdrive/D1/DO-SQLite (ties to Q-0042)? Answered: CI (round 16), through the Access tunnel, only when migration files changed (round 30).
* Q-0092.2 — preview environments. Answered by ADR-0111 (2026-11-16): `dev-*` Previews with isolated data.

## More Information

CF docs: gradual deployments, version affinity/overrides, rollbacks, DO migrations. Related: [[ADR-0090]], [[ADR-0093]], [[ADR-0100]], [[ADR-0101]], [[ADR-0130]].
