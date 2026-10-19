---
status: accepted
date: 2026-10-18
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0094: Postgres hosting — CloudNativePG in woven-o11y, reached through a Cloudflare Tunnel and a Workers VPC service

## Context and Problem Statement

The system of record is Postgres reached through Hyperdrive (ADR-0040, leading engine). Spikes 0001 (Stage 2), 0002 and 0003 need a TLS-reachable Postgres, and the product needs one for good. The owner answered Q-G5.1 in round 27: **the Postgres provider is not inside the under-$100/month envelope**. The Woven infra repo already runs six CloudNativePG (CNPG) clusters in the `woven-o11y` EKS cluster from one generic chart (`modules/kube/cnpg-cluster/chart`), including Plane's own `plane-pg`; the chart gives TLS-only `pg_hba` (`hostnossl all all all reject`), daily backups to R2 through the Barman Cloud plugin, ESO-managed credentials, a PodMonitor and cost-attribution labels. Cloudflare's recommended way for Hyperdrive to reach a database that is not on the public Internet is a **Workers VPC service** backed by a **Cloudflare Tunnel** (changelog 2026-04-29); the older path is Tunnel plus an Access application and service token.

## Decision Drivers

* The provider cost sits outside the Cloudflare-side envelope (Q-G5.1); an existing, operated CNPG pattern.
* No public database endpoint.
* The self-hosted twin keeps its own plain Postgres (ADR-0091).
* SLO 99.5 % availability, RPO 24 h, RTO 8 h (SPEC).
* Spike 0001: `tokio-postgres` runs over a Hyperdrive socket (locally); real Hyperdrive is unproven.

## Considered Options

* Option 1: CNPG in woven-o11y, a **new** Cloudflare Tunnel, a Workers VPC service, Hyperdrive on top **(chosen)**
* Option 2: the same CNPG, reached through the tunnel plus an Access application and a service token (the older Hyperdrive private-database path)
* Option 3: a managed external Postgres with a public TLS endpoint (the original Q-0040.5 framing)
* Option 4: D1 instead of Postgres (ADR-0040 alternative)

## Decision Outcome

**Decision (accepted, round 27): Option 1.** The Thrust Postgres is a CNPG cluster created in `woven-o11y` from the infra repo's generic chart. A **new, dedicated Cloudflare Tunnel** (a `cloudflared` deployment in the cluster, separate from the WARP/o11y tunnel) connects it to Cloudflare. A **Workers VPC service** (`type tcp`, port 5432, `app_protocol postgresql`, host = the cluster's read-write Service name, the tunnel as DNS resolver) is attached to a Hyperdrive configuration (`wrangler hyperdrive create … --service-id <id>`). The Postgres provider is outside the cost envelope. The self-hosted twin runs a plain Postgres container (ADR-0091).

**Reading of "create a new VPC and tunnel" (owner to confirm):** a *Workers VPC service* backed by a new tunnel — not an AWS VPC. A Workers VPC *network* (which binds a whole tunnel) is not used: a service pins the destination host and port, which is what prevents server-side request forgery `[doc]`.

### Consequences

* Good, because there is no public database endpoint; the estate's CNPG pattern is reused (TLS-only `pg_hba`, R2 backups, ESO secrets, monitoring, attribution); the provider cost is outside the envelope; Workers VPC is free while in beta `[doc]`; the same Postgres can host the dev and per-PR schemas (Q-0094.1).
* Bad, because **product availability now includes `woven-o11y`** (an observability cluster with its own upgrade windows), the tunnel's `cloudflared` pods and a Cloudflare **beta** feature ("features and APIs may change before general availability" `[doc]`); the request path gains a hop (Hyperdrive → VPC service → tunnel → pod) whose latency, pooling and caching behaviour are unmeasured; the region is the cluster's (the infra repo's Stage 1 root defaults `aws_region` to `us-east-1`; verify live); the generic chart defaults to one instance; `pg_hba` rejects non-TLS, and Hyperdrive cannot be given our CNPG CA when it reaches the database through a Workers VPC service, so the server is not authenticated against our CA (Q-0094.2, decided below); `woven-o11y` is documented as a single-environment (dev) cluster, so staging and production need a layout decision (Q-0094.1); creating a VPC service needs the Connectivity Directory Admin role (Q-0094.3).
* Neutral: no `.woven` hostname is involved (the tunnel serves Workers VPC, not WARP), so the CoreDNS and external-dns rules do not apply `[inferred]`.

### Confirmation

Spike 0001 Stage 2 (steps S2-0 to S2-3 and S2-8) and spike 0002 run against this Postgres: `BEGIN … COMMIT` atomic over one Hyperdrive socket through the VPC service; create and list p95 from the Worker; behaviour of Hyperdrive pooling over the tunnel. **Reopen this ADR if** Hyperdrive through a VPC service cannot hold a transaction, create p95 from the Worker exceeds 300 ms, Workers VPC changes incompatibly or is withdrawn before general availability (fallback: Option 2 over the same tunnel), or `woven-o11y` cannot meet the availability SLO (fallback: Option 3).

## Pros and Cons of the Options

### Option 1: CNPG + new Tunnel + Workers VPC service

* Good, because it is the Cloudflare-recommended private-database path, needs no Access application or service token, and reuses a pattern operated in this estate.
* Bad, because it depends on a beta feature and on the health of the cluster and tunnel.

### Option 2: CNPG + Tunnel + Access application + service token

* Good, because it is GA and documented in detail; the same tunnel works.
* Bad, because it adds an Access application and a service token to manage, and Cloudflare now recommends Workers VPC instead.

### Option 3: managed external Postgres, public TLS endpoint

* Good, because availability is the vendor's and there is no tunnel.
* Bad, because the provider cost is real (and the owner has put it outside the envelope but is not choosing this), and the endpoint is public.

### Option 4: D1

* Good, because it is Cloudflare-native with no tunnel.
* Bad, because the closest-to-Plane engine is Postgres (ADR-0040) and D1 reportedly lacks interactive transactions (REVIEW-NOTES section 5, item 3, unverified) and Postgres features the design uses; it stays a desk review.

## Update — owner decision, round 27 (2026-10-18)

Owner decision on Q-0094.1: **two clusters.** `thrust-pg` for production with 2 instances (HA) and `thrust-pg-dev` for dev, staging and per-PR schemas with 1 instance, so a bad dev migration or load test cannot share production's failure domain. Storage sizes, backup retention and the exact names are set in the infra work (bead `plane-byd.4`); the generic chart's defaults are 1 instance, `gp3` and 10 Gi, and `plane-pg` sets a 30-day backup retention (the other clusters set none). Open: Q-0094.2 to Q-0094.4.

## Update — infra work started, round 27 (2026-10-18)

Owner confirmations: **go-ahead granted** to start the infra work, and "new VPC" means a **Workers VPC service** backed by a new dedicated tunnel (Q-0094.3, partly answered). Authoring started on branch `feat/thrust-cnpg-tunnel-vpc` of the infra repo (bead `plane-byd.4`); it is author-and-validate only, with no apply, no live plan, no push and no PR until the owner confirms. The infra repo's own rules shape the work: CNPG Clusters stay Terraform-owned (datastores), `cloudflared` is a Terraform-owned L1 component, secrets flow Secrets Manager to ESO to a Kubernetes Secret, and the tunnel belongs in an S3-backed root (the existing tunnels sit in `organization/cloudflare`, which has a local backend that applies from the canonical checkout only). The wrangler OAuth login has `connectivity (admin)` (needed to create a VPC service) but no Hyperdrive scope, so creating the Hyperdrive configuration probably needs an API token with Hyperdrive Edit or the provider.

## Update — infra authored, first apply, owner decisions, round 27 (2026-10-18)

**Infra authored, first apply done, three owner decisions** (`research/0010-thrust-postgres-infra-plan.md`; infra repo branch `feat/thrust-cnpg-tunnel-vpc`, four signed local commits, 45 files, all 27 `just pr-gates` recipes pass on the clean baseline and the changed tree; not pushed; bead `plane-byd.4`).

* **What is authored (nothing but the aws root is applied):** `products/thrust/aws` (three empty Secrets Manager containers plus an additive read grant on the ESO role), `products/thrust/cloudflare` (the tunnel `thrust-pg` and two Workers VPC TCP services; the provider has `cloudflare_connectivity_directory_service` from 5.13.0, the lockfile pins 5.28.0), `products/thrust/kube` (namespace `thrust`, deliberately not meshed; `thrust-pg` with 2 instances, required anti-affinity, 20 Gi, 30-day retention; `thrust-pg-dev` with 1 instance, 10 Gi, 7-day retention; the `cloudflared` connector with 2 replicas and a PodDisruptionBudget; per-PR schema script), runbook, ADR 0022 in the infra repo.
* **Hyperdrive is created by script, not OpenTofu:** `cloudflare_hyperdrive_config` exists, but its `origin.password` is a plain sensitive string that would land in the R2 state, and `wrangler hyperdrive create --password` is argv-only, so `scripts/hyperdrive.sh` builds the request from the environment. It needs an API token with **Hyperdrive Write** (the wrangler OAuth login does not have it); creating a VPC service needs the **Connectivity Directory Admin** role (the OAuth login has `connectivity (admin)`).
* **Q-0094.2, TLS (owner: `verify_ca` now, fallback `disabled`):** the Hyperdrive API reference says its CA upload cannot be used with VPC service origins, and Workers VPC trusts only publicly trusted CAs and Cloudflare Origin CA `[doc]`, with modes `verify_full`, `verify_ca` and `disabled`. The VPC services default to `verify_ca` (the mode the docs show for a Postgres TCP service); `pg_hba` stays TLS-only. The hop is encrypted, but **the database is not authenticated against our CA**. Whether `verify_ca` accepts CNPG's CA is unverified; the first Hyperdrive create is the test. Hardening (a server certificate that Workers VPC trusts, then `verify_full`) is a recorded follow-up.
* **Capacity (owner: cut requests, prefer zone spread):** a read-only check of the `stateful` pool found four nodes at 80 to 95 % CPU requests with 1,830m free, only the two us-east-1b nodes able to take a 500m pod, so the authored change would have put both `thrust-pg` replicas in one AZ. Decision: `thrust-pg` requests drop to 250m and the replicas get a best-effort zone spread; the pool maximum stays 4.
* **Caching (owner):** the production Hyperdrive config is created with **caching disabled** (strict read-after-write); caching on a separate config can be measured in spike 0002.
* **First apply:** `products/thrust/aws` was applied (4 added, no drift on re-plan). The remaining roots wait for the owner's go-ahead and the credentials in the plan note (Hyperdrive Write token, Connectivity Directory Admin).
* **Risks recorded by the author:** availability coupling to woven-o11y (upgraded 1.32 to 1.37 four days earlier; `thrust-pg` still has a primary PodDisruptionBudget of 0, so drains need the replica-delete procedure); Workers VPC is beta; both clusters share one tunnel and connector; the shared R2 backup credential can read every product's backups; owner-password rotation has seconds of auth failures. Estimated in-cluster cost about $60 to 70 a month allocated, marginally $0 to 13 if the pool absorbs it (outside the cost envelope, Q-G5.1).

## Update — owner decisions implemented, round 27 (2026-10-18)

**Owner decisions implemented on the infra branch** (five more signed local commits, `c780a9c8` to `88ff5e64`; all 27 `just pr-gates` recipes pass; nothing applied or pushed):

* **Capacity:** `thrust-pg` requests 250m CPU (memory request 1 Gi, limit 2 Gi). CPU added in all: `thrust-pg` 2 x 250m, `thrust-pg-dev` 250m, connectors 2 x 100m = 950m, of which 750m lands on the `stateful` pool against the 1,830m free (it was 1,250m on the pool at 500m). 250m fits three nodes in two AZs.
* **Zone spread:** the generic chart `modules/kube/cnpg-cluster/chart` could express one anti-affinity only, so a strictly **opt-in `zoneSpread`** value was added (one `topologySpreadConstraints` entry, always `ScheduleAnyway`, a scheduler preference and not a guarantee); `thrust-pg` enables it, the other nine releases leave it off. `helm template` output of the nine existing releases is byte-identical before and after, the six conformance goldens are unchanged, a new conformance section D guards it, and a mutation that always renders the constraint fails the gate.
* **Side effect on other products:** the chart version is bumped 0.2.0 to 0.3.0 (the version guard requires it, and a local chart changes only when its version does). Each of the **nine other roots that consume the chart will show its `helm_release` as updated in place on its next plan**, with identical manifests. The green chart copy in `ApiraTech/helm-charts` needs the same value.
* **Caching:** `hyperdrive.sh ensure` creates every config (`thrust-pg`, `thrust-pg-dev`, per-PR configs) with caching **disabled** unless given `--caching-enabled`; `check` fails a config with caching on; `rotate-password` carries the live caching block into its PUT (the API resets omitted caching to enabled, so a rotation would otherwise have undone it).
* **TLS wording:** the README, runbook and infra ADR 0022 now say plainly that the hop is encrypted and the server is not authenticated against our CA, with the Origin CA or public-CA hardening recorded as a follow-up.

## Update — the Postgres is live, round 27 (2026-10-18)

**The Postgres is live** (2026-10-18; infra branch `feat/thrust-cnpg-tunnel-vpc`, local and unpushed, nine signed commits; details and ids in `research/0010` section 10). The owner approved each apply after seeing its plan, and each matched the prediction:

* `products/thrust/aws`: 4 added. `products/thrust/cloudflare`: 3 added (tunnel `thrust-pg`, two Workers VPC TCP services). `products/thrust/kube`: 8 added. Re-plans show no drift where they were run.
* **Verified `[tested]`:** all five ExternalSecrets synced (the ESO grant is sufficient); `thrust-pg` 2 of 2 healthy with the primary in us-east-1b and the replica in us-east-1a (the best-effort zone spread worked); `thrust-pg-dev` healthy; the connector registered 4 QUIC connections per pod and the tunnel is healthy with 8 connections; a plain-text connection is rejected by `pg_hba` while a TLS connection passes that check (a paired control); an on-demand backup completed in 4 s and WAL archiving shows 4 archived and 0 failed.
* **Not yet verified:** the Hyperdrive configs (they need an API token with Hyperdrive Write), that Hyperdrive reaches the database through the VPC service, whether `verify_ca` accepts CNPG's CA, that a Worker holds `BEGIN ... COMMIT` and sees `ssl = true` in `pg_stat_ssl`, the restore drill and the measured RTO, and Hyperdrive latency and pooling over the tunnel.
* **Q-0094.3 answered:** the tunnel and VPC services were created with an API token by OpenTofu, `cloudflared` runs in the `thrust` namespace, and the documented fallback (tunnel plus an Access application and service token) stays available if Workers VPC changes.

## Update — Hyperdrive and the Worker smoke test, round 27 (2026-10-18)

**Hyperdrive created and a Worker reached the database** (`research/0010`; 2026-10-18, after the owner added Hyperdrive Write to the `thrust-spikes` token):

* **`verify_ca` failed, `disabled` worked `[tested]`.** With `verify_ca` on the VPC services, creating the production Hyperdrive config failed with `Failed to connect to the provided database: Internal error.` After the documented fallback (`cert_verification_mode = disabled`, an in-place update of both services) the same command succeeded. This is consistent with Workers VPC trusting only public CAs and Cloudflare Origin CA, while CNPG's certificate is signed by a private CA; the only change between the failure and the success was the verification mode, but the connector logs showed nothing and the error text does not name TLS, so the attribution is an inference. **Q-0094.2 is answered in effect:** the hop runs with certificate verification **disabled**: encrypted (the origin backend reports TLSv1.3) but the database is **not authenticated**. Hardening (a server certificate that chains to Cloudflare Origin CA or a public CA, Workers VPC custom-CA upload, or Option 2) is Q-0094.9.
* **Configs `[tested]`:** `thrust-pg` (`8294c22a2a304146821a8bf11258e74d`, connection limit 40) and `thrust-pg-dev` (`b231d16b672b469d8834a071fa842b9c`, limit 20), both over their VPC service, database `thrust`, role `thrust`, **caching disabled**; `hyperdrive.sh check` reports OK for both.
* **Smoke Worker `[tested]`** (a throwaway Worker in the Woven account behind a random path, `pg` 8.16.3 with `nodejs_compat`, Worker colo IAD; deleted afterwards and the account inventory matches its baseline): the role is `thrust`, `pg_stat_ssl` shows `ssl = true`, `TLSv1.3`; `BEGIN ... COMMIT` of a temp table with two rows returned 2 and the table was gone after the commit; `BEGIN ... ROLLBACK` left no table; warm timings from IAD: connect 4 to 5 ms, the five-statement transaction 42 to 81 ms, `select 1` p50 9 to 10 ms and p95 12 to 16 ms (indicative, two warm calls, a JavaScript client). The first call after deploy returned 404 (workers.dev propagation), and the wrong-path control returned 404.
* **Caveat for the Rust decision:** this proves the **path** and the transaction semantics through Hyperdrive from a JavaScript client. It does **not** prove the Rust path that ADR-0006's reopen trigger 1 names (`tokio-postgres` over `Hyperdrive::connect()` and a `worker::Socket`); that is spike 0001 Stage 2. The reopen triggers of this ADR are not triggered (a transaction held, create-class latency is far below 300 ms on `select 1`).
* **Provider bug:** an in-place update of `cloudflare_connectivity_directory_service` succeeds remotely but the provider fails with "inconsistent result after apply" on `created_at` (the API returns a new creation time on update); a re-plan afterwards shows no changes. The committed default was changed to `disabled` on the branch to match the live state.
* **Remaining:** step 9, the restore drill with the measured RTO; real latency and pooling validation (spike 0002); hardening (Q-0094.9).

## Update — PR #810 security review, round 27 (2026-10-18)

**PR #810 security review (Cursor Agentic Security Review, MEDIUM) and the owner's decision.** The review objected to `cert_verification_mode = disabled` on the production and dev Postgres paths: whoever answers the connector's connection to the `-rw` Service becomes the TLS peer for Hyperdrive's pooled owner session. The finding is accurate. The owner **accepted it as a temporary state** (round 27): `verify_ca` was tested and failed against CNPG's private CA, so `disabled` is the only mode that works today; the hop is encrypted (origin backend TLSv1.3), `pg_hba` rejects non-TLS, authentication is SCRAM-SHA-256, and the scenario needs an in-cluster foothold (the same class as the estate's D-TLS-4 gap). **Deadline: authenticated TLS must be closed before Thrust holds real data.** Tracked as `plane-byd.9` and `infra-qnyi`. The thread was answered on the PR (with the facts and the tracking issue) and resolved.

## Update — merged and the restore drill, round 27 (2026-10-18)

**Merged and drilled** (2026-10-18). The owner merged the infra PR (`ApiraTech/infrastructure#810`, squash `46c22f3a` on `main`); the code on `main` equals what was applied. **The restore drill ran on `thrust-pg` `[tested]`:** a recovery cluster was Ready after **60 s**; sentinel rows written before and after an on-demand base backup were all present (WAL replay verified); `thrust` is not a superuser; the negative control (a wrong `serverName`) failed hard and never reached Ready, with no fall-back to an empty database; the live cluster was unchanged (UID, primary, conditions). The 60 s is for a near-empty database, covers database recovery only (not repointing Hyperdrive or the VPC service, nor an application cutover) and does not extrapolate; the SPEC target is RTO 8 h; the drill is to be repeated before real data lands and after the first week. Not verified: the R2 prefix listing (needs the shared credential), `thrust-pg-dev`, a PITR restore. The runbook outcome is in `ApiraTech/infrastructure#811` (draft). **Confirmation status for this ADR:** the transaction semantics, create-class latency and recovery checks passed on the paths tested (JavaScript client; the Rust path is spike 0001 Stage 2); the TLS hardening (Q-0094.9, `plane-byd.9`) and the repeat drill remain.

## Update — spike 0001 Stage 2 on the real platform, round 28 (2026-10-18)

**Stage 2 of spike 0001 used this Postgres** (2026-10-18; `research/0004` section 10) and closes the caveat above for the Rust driver, with a constraint.

* **The Rust path through Hyperdrive works only through the single-pipeline statement flow.** A Worker built on `tokio-postgres` reached `thrust-pg-dev` through a new Hyperdrive config, the dev VPC service and the tunnel, and passed the shared suite 15/15 (8/8 on repeat); with the library's default prepare-then-execute flow the same Worker failed (`connection closed` on every statement with no result columns, `unexpected message from server` intermittently). Postgres logged no error, so the connection was ended upstream of the database. Details and the 18-case probe: `research/0004` section 10.4 F1. A JavaScript client doing parameterless statements (the earlier smoke test) never exercised the failing flow.
* **The per-PR design worked live.** `pr-schema.sh create 1001` made the role and schema, `hyperdrive.sh ensure thrust-spike-pg --vpc-service thrust-pg-dev --user pr_1001 --origin-connection-limit 8` made the config, the spike's own migration ran as that role, and the Worker found its tables with **no `search_path` set by the application**: the role-level setting is honoured through Hyperdrive. Under load (concurrency 1 and 8) the origin saw a constant 5 connections for the role and at most 3 active, inside the role limit of 8. `pr-schema.sh drop 1001` then removed everything; a catalog check found no leftover roles, schemas or role settings, and the Hyperdrive config was deleted.
* **Costs seen:** the first database-backed request on a fresh isolate took about 0.6 s; a simple read was 56 to 68 ms at p50 from one US-east laptop. Caching stayed disabled (D10) and was not compared.
* **Unchanged:** the VPC service still runs with `cert_verification_mode = disabled` (Q-0094.9, `plane-byd.9`); `thrust-pg` (production) was not touched; `thrust-pg-dev` carries no spike state now.

## Open Questions

* Q-0094.1 — CNPG layout: HA, environments, per-PR schemas, backup retention.
* Q-0094.2 — TLS verification between Hyperdrive and CNPG.
* Q-0094.3 — The tunnel and the VPC service: ownership, tooling, fallback.
* Q-0094.4 — Availability coupling to `woven-o11y`.

## More Information

Infra repo (read-only references): `modules/kube/cnpg-cluster/` (chart, README, conformance), `products/plane/kube/datastore.tf` and `values/plane-pg.yaml` (the closest worked example), `kube/cloudflared.tf` (an existing connector pattern). Cloudflare: https://developers.cloudflare.com/hyperdrive/configuration/connect-to-private-database-vpc/ · https://developers.cloudflare.com/workers-vpc/configuration/vpc-services/ · https://developers.cloudflare.com/hyperdrive/configuration/connect-to-private-database/ . Related: [[ADR-0040]], [[ADR-0041]], [[ADR-0090]], [[ADR-0091]], [[ADR-0092]], [[ADR-0093]]. Bead `plane-byd.4`.
