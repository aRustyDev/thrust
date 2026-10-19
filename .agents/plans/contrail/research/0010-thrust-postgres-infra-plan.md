# Research 0010 — Thrust Postgres in the Woven infra repo: authored and validated; only the `aws` root is applied

Date: 2026-10-18. Bead `plane-byd.4`. Design of record: `adrs/infra/0094-postgres-hosting-cnpg-tunnel-vpc.md` (accepted, round 27, two
clusters). Code: Woven infra repo worktree `/Users/asmith/repos/woven/infrastructure/infrastructure/.claude/worktrees/thrust-cnpg-tunnel-vpc`,
branch `feat/thrust-cnpg-tunnel-vpc` (from `origin/main` `d05c7586`; local only, **nothing pushed, no PR**). The infra repo has its own
ADR for the decisions that belong to it: `docs/src/adrs/0022-thrust-postgres-cnpg-tunnel-vpc.md`.

Commits on the branch, all signed (`git log --format=%G?` = `G`), each ending `Refs: plane-byd.4` then `Co-Authored-By: Claude Sonnet 5.5 <noreply@anthropic.com>`: `245e4a09` feat(thrust) cloudflare root · `f6a551a5` feat(thrust) aws root · `1901b482` feat(thrust) kube root · `a1cfc6b6` docs(thrust) ADR 0022, runbook, ledger rows. **Round-27 follow-up (section 11):** `c780a9c8` feat(cnpg-cluster) opt-in best-effort `zoneSpread` · `a14bb895` feat(thrust) `thrust-pg` 250m + zone spread · `a6aaea94` feat(thrust) Hyperdrive caching disabled by default · `9cba15ee` docs(thrust) TLS limit, capacity, applied aws root · `88ff5e64` docs(thrust) measured memory headroom. All signed (`%G?` = `G`), no signing hang.

**The author applied nothing.** No `tofu plan` against live state, no `kubectl`, no AWS or Cloudflare API call, no mutating `wrangler`, no secret
read, no 1Password. Network use was limited to the OpenTofu and Helm registries and public Cloudflare docs. **`products/thrust/aws` was applied by the
owner/coordinator** (4 added, re-plan clean, then the two owner passwords were put) — see section 10; the Cloudflare and kube roots are not applied.

Tags: **[tested]** I ran it locally (the command is given) · **[doc]** a vendor/repo page I read (path or URL) · **[inferred]** reasoned from
doc or code, not observed · **[unverified]** a prediction or an unknown. `[tested]` never means "on Cloudflare" or "on the cluster".

## 1. What was built, and where

All paths relative to the infra worktree. 45 files: 41 new, 4 modified.

| Area | Files |
|---|---|
| `products/thrust/aws/` | `versions.tf` `providers.tf` `variables.tf` `secrets.tf` `outputs.tf` `.terraform.lock.hcl` `README.md` |
| `products/thrust/cloudflare/` | `versions.tf` `providers.tf` `variables.tf` `tunnel.tf` `vpc-services.tf` `outputs.tf` `contract.tftest.hcl` `.env.example` `.terraform.lock.hcl` `README.md`; `scripts/hyperdrive.sh` `scripts/store-tunnel-token.sh` `scripts/test-scripts.sh` |
| `products/thrust/kube/` | `versions.tf` `providers.tf` `data.tf` `variables.tf` `namespace.tf` `externalsecret.tf` `datastore.tf` `cloudflared.tf` `contract.tftest.hcl` `.terraform.lock.hcl` `README.md`; `values/thrust-pg.yaml` `values/thrust-pg-dev.yaml`; `scripts/pr-schema.sh` `scripts/test-pr-schema.sh` |
| `products/thrust/README.md` | product index and local-validation commands |
| `backends/` | `products-thrust-{aws,kube,cloudflare}-646.s3.tfbackend` (+ three rows in `backends/README.md`) |
| `docs/src/` | `adrs/0022-…md` (+ `adrs/README.md` row, `SUMMARY.md`), `operations/thrust-postgres.md` (runbook, restore drill, rotation, per-PR schemas), two rows in `dev/strategies/delivery/ownership-layers.md` |

Changed in the round-27 follow-up (section 11): the generic chart `modules/kube/cnpg-cluster/chart` (`templates/cluster.yaml`, `values.yaml`, `Chart.yaml` 0.2.0 → **0.3.0**) and its `README.md`,
the conformance harness `conformance/check.sh` (new section D), `values/thrust-pg.yaml`, `contract.tftest.hcl`, `kube/README.md`, `cloudflare/scripts/hyperdrive.sh` and `test-scripts.sh`,
`cloudflare/README.md`, the runbook, ADR 0022 (D9, D10) and the product README. Not changed: `CODEOWNERS` (governance files only), `INVENTORY.md` (no new chart, template count unchanged),
any workflow, any shared root, the conformance goldens.

## 2. Design decisions, and why

**Placement [doc: module-boundaries.md, AGENTS.md].** A new product root `products/thrust/{aws,kube,cloudflare}` on R2 state, naming and key schema copied
from `products/plane` (account `<aws-account-id>`, suffix `646`, key `products/thrust/<provider>/<aws-account-id>/terraform.tfstate`). **The tunnel is in the
product root, not `organization/cloudflare`**: that root has a LOCAL backend (canonical-checkout-only applies, and the apply-guard hook blocks it from a
worktree), is the single-tenant control plane, and already owns the woven-o11y WARP tunnel. `shared/cloudflare/proton` is the worked example of a
dedicated tunnel on R2 state. It is not `shared/`: one consumer.

**Ownership layers [doc: ownership-layers.md, delivery-ownership.md].** Tunnel, VPC services, Secrets Manager containers: L0. Namespace and `cloudflared`: L1,
raw `kubernetes_deployment_v1` like `kube/cloudflared.tf` (no chart, so nothing for `inventory-guard` or `local-chart-version-guard`; no helm release for
`single-owner-guard`). CNPG Clusters: LD, OpenTofu, like the existing ones (`infra-dswl.13` decides whether LD ever moves). Hyperdrive: out-of-band (section 3).

**Two clusters, one namespace `thrust`.** `thrust-pg` (2 instances) and `thrust-pg-dev` (1), both from `modules/kube/cnpg-cluster/chart`. Because they share a
namespace, `backup.objectStoreName` and `backup.creds.secretName` must differ per release (the chart renders an ExternalSecret and an ObjectStore from them)
— a collision would fail the second `helm install` `[inferred]`; a test asserts the names, prefixes and schedules differ `[tested]`.

**HA and anti-affinity.** `instances: 2` with `podAntiAffinity: required` on `kubernetes.io/hostname`, as `gitea` does `[doc: shared/kube/gitea/values/gitea-pg.yaml]`
(three `pool: stateful` nodes in three AZs). Async streaming replica: a failover can lose the last seconds of WAL, far inside RPO 24 h. Resources
**250m**/1Gi request, 2Gi memory limit (chart default is 250m/512Mi/1Gi; **lowered from 500m by the owner's capacity decision, section 11**): Hyperdrive pools up to its origin connection limit and each backend costs memory.
A **best-effort zone spread** (opt-in chart value `zoneSpread`, `ScheduleAnyway`) prefers the two instances in different AZs; see section 11 for how, and for its limits.
Storage 20 Gi prod / 10 Gi dev gp3 per instance (CNPG cannot shrink a volume; WAL headroom matters more than data, target ≤ 100k work items).
**Required anti-affinity needs two schedulable stateful nodes, and the stateful pool was recorded as ~94 % full during the EKS upgrade [doc: eks-upgrade.md]**
— measured since: 1,830m CPU free over four nodes (section 10), which is why section 11 lowers the request.

**Backups vs RPO 24 h / RTO 8 h.** Continuous WAL archiving + one daily base backup to the shared R2 `backups` bucket via the Barman plugin, shared estate
credential `/agentic-platform/r2-cnpg-backup-creds` (granted explicitly: ESO has no wildcard grants), prefix `thrust/thrust-pg` and `thrust/thrust-pg-dev`.
Retention 30 d prod / 7 d dev (explicit, unlike five older clusters). **Schedules 03:10 UTC prod, 03:30 UTC dev**: the slots already taken, found by grep, are
03:00 (aegra, affine, gitea, langfuse, zitadel), 03:17 (beads), 03:20 (plane), 03:23 (qdrant), 03:35 (terrapod plan), 03:37 (mlflow), 03:40 (litterer,
agentregistry), 03:43 (zenml), 03:53 (growthbook) `[tested: grep of the tree]`. RPO: WAL is archived continuously, so recovery to latest loses minutes
`[inferred]`. RTO: the only measured CNPG restore in the estate (affine, 4.5 MiB) took ~60 s `[doc: affine-pg-restore-drill.md]` and "does not
extrapolate"; the Thrust drill is documented but **not run**.

**TLS to Hyperdrive (Q-0094.2) — the answer, and the surprise.**
* **Owner decision (round 27): keep `verify_ca` on the VPC services, `disabled` as the documented fallback, default unchanged. The README, runbook and ADR 0022 now say plainly that the hop is
  encrypted and the server is NOT authenticated against our CA, and that serving a Cloudflare Origin CA / public-CA certificate so `verify_full` works is a recorded follow-up, not done.**
* `pg_hba` is untouched (`hostnossl all all all reject`, the chart default); a test asserts the values files never override it `[tested]`. Hyperdrive always uses TLS `[doc: hyperdrive/reference/supported-databases-and-features]`.
* **Hyperdrive cannot be handed CNPG's CA when the origin is a VPC service.** The Create-Hyperdrive API reference says of the `mtls` block (CA upload,
  `sslmode verify-*`): *"Cannot be used with VPC Service origins; TLS must be managed on the VPC Service."* `[doc: developers.cloudflare.com/api/resources/hyperdrive/subresources/configs/methods/create/]`
* TLS is therefore managed on the VPC service: `cert_verification_mode` defaults to `verify_full`, which trusts only public CAs and Cloudflare Origin CA, and
  **"uploading a custom CA certificate to Workers VPC is not supported yet"** `[doc: developers.cloudflare.com/hyperdrive/configuration/connect-to-private-database-vpc/]`.
  CNPG signs with a cluster-internal CA, so `verify_full` fails the handshake `[inferred]`.
* So: `var.cert_verification_mode` defaults to **`verify_ca`**, the setting Cloudflare documents for a private-CA origin; `disabled` is the fallback. **Whether
  `verify_ca` actually accepts a private-CA chain is [unverified]** (the docs say to use it, and do not say how a chain is judged without our CA). Either way the
  hop is encrypted and the server is **not authenticated against our CA** — the same gap the estate records for every in-cluster client (D-TLS-4,
  `datastore-tls-requirements.md`).
* Cloudflare's docs contradict themselves about Hyperdrive's own default (`require` "validated based on WebPKI" in one table, "does not verify the origin
  server certificate by default" in the VPC page) `[doc, both]`; irrelevant for a VPC origin, but a reason to expect the first real connection to teach us something.
* **Open risk and the way out**: real server-identity verification needs (a) Workers VPC to accept an uploaded CA, or (b) CNPG to serve a certificate chaining to a
  Cloudflare Origin CA / public CA — which needs the generic chart to expose `spec.certificates` (a six-cluster chart change, a `Chart.yaml` bump and every release
  re-planned) and a hostname Origin CA can issue for (not `*.svc.cluster.local`), or (c) ADR-0094 Option 2 (Tunnel + Access), whose Hyperdrive config does accept
  `--ca-certificate-id` + `--sslmode verify-full`. If CNPG's own CA is ever uploaded anywhere it must be tracked: CNPG renews its CA and leaf certificates itself,
  which would silently break a pinned copy `[inferred]`.

**Secrets.** Owner passwords come from Secrets Manager (`/thrust/pg-owner-password`, `/thrust/pg-dev-owner-password`), through ESO as a `kubernetes.io/basic-auth`
Secret that `bootstrap.initdb.secret` names (the plane pattern, minus plane's chart limitation): the same value is given once to Hyperdrive, and reading CNPG's
generated Secret with `kubectl` to paste it into Cloudflare would route a plaintext credential through a terminal. A password stored with a trailing newline
breaks every login — the runbook says `tr -d '\n'`. No value is ever in state, a values file or git; remoteRefs are name-keyed.

**Mesh.** `thrust` is **not meshed**, deliberately (no `linkerd.io/inject`, so no allowlist line; `mesh-injection-guard` passes). CNPG pods are unmeshed by estate
policy, the only client is `cloudflared` (QUIC out, Postgres TLS in), and a meshed Job pod never completes without native sidecars. `mesh.optOut: true` stays on both
clusters and the connector pods carry `linkerd.io/inject: disabled`, so an eventual opt-in cannot widen the mesh onto them. This differs from the brief's "mesh opt-out
as in plane-pg" only in that the namespace itself is not injected; the opt-out is still set.

**Connector.** Own tunnel/token/Secret/Deployment (never the WARP connector). 2 replicas, `maxUnavailable 0`, `topologySpread` by host (ScheduleAnyway), PDB
`minAvailable 1`, `--protocol quic` explicit (Workers VPC needs QUIC; with `auto` a blocked UDP/7844 would silently fall back to http2 and look healthy), non-root,
read-only rootfs, drop ALL, token via ESO, no service-account token mounted. Image `cloudflare/cloudflared:2026.6.0` — the tag `kube/cloudflared.tf` already runs;
Workers VPC needs ≥ 2025.7.0 `[doc: workers-vpc/configuration/tunnel/]`. I did **not** verify that 2026.6.0 is still a supported/latest release (no registry call) —
`[unverified]`. Not autoscaled and not pinned to the stateful pool (replicas are availability-only; cloudflared does not load-balance `[doc]`).

**No NetworkPolicy.** The estate has almost none (six objects in two namespaces `[doc: cluster-egress-requirements.md]`); I could not validate enforcement against the
CNPG operator's status port or the connector's QUIC egress without a cluster, and a wrong policy on a database is worse than none. Recorded as an open item.

**Per-PR schemas (Q-0094.1).** `products/thrust/kube/scripts/pr-schema.sh create|drop|list <n>`: login role `pr_<n>` (no superuser/createdb/createrole, connection limit 8)
+ schema `pr_<n>` owned by it + a **role-level** `search_path` (`ALTER ROLE … IN DATABASE thrust SET search_path`), because Hyperdrive does not support per-session state
`[doc: supported-databases-and-features]`. Runs `psql` via `kubectl exec` on the dev primary over the local socket (the one path `pg_hba` never gates; no superuser
Secret), password on stdin from a 0600 file, hard-wired to `thrust-pg-dev` (a test proves `CLUSTER=thrust-pg` is ignored). **Not automated**: any PR event wiring,
garbage-collection of schemas from PRs that closed without the hook, per-PR Hyperdrive creation (`hyperdrive.sh ensure` can do it; nobody calls it), the PR's migrations,
per-PR storage bounds, staging data copy.

## 3. What the provider supports, and what is out-of-band

`tofu providers schema -json` against `cloudflare/cloudflare` **5.28.0** (the lockfile pin; `~> 5.13`) `[tested]`:

| Need | Resource | Result |
|---|---|---|
| Tunnel | `cloudflare_zero_trust_tunnel_cloudflared` | **declared**, `config_src = "cloudflare"`; no config object, no route, no token data source (state is on R2) |
| Workers VPC service | `cloudflare_connectivity_directory_service` (needs ≥ 5.13.0) | **declared**: `type tcp`, `tcp_port 5432`, `app_protocol postgresql`, `host.hostname` = the `-rw` Service FQDN, `host.resolver_network.tunnel_id`, `tls_settings.cert_verification_mode`. A mocked-provider test pins all of it `[tested]` |
| Hyperdrive config | `cloudflare_hyperdrive_config` (`origin.service_id` exists) | **exists but NOT used**: `origin.password` is `required, sensitive`, **not write-only** — it would be written into the R2 state `[tested: schema dump]`. Created out-of-band |
| Connector token | data source exists | **not used** (would persist the credential in state); fetched once by script |

**Out-of-band, exact commands** (full runbook with order: `docs/src/operations/thrust-postgres.md`):
* Hyperdrive, reference form `[tested: wrangler 4.123.0 --help]`: `wrangler hyperdrive create thrust-pg --service-id <vpc-service-id> --database thrust --user thrust --password <…> --scheme postgresql --origin-connection-limit 40`.
  **`--password` is argv-only** (as is `--connection-string`), where `ps` shows it, which breaks the repo's "secrets via env, never argv" rule — so the shipped path is
  `products/thrust/cloudflare/scripts/hyperdrive.sh ensure …`, which builds the REST body from the environment, sends it on `curl`'s stdin, keeps the API token in a
  0600 header file, and reads the password from Secrets Manager inside the process. It also has `check` (read-only, idempotent, exit 0/1/3), `rotate-password`
  (PUT with the live origin rebuilt around the new password, **carrying the live `caching` block over because a PUT resets omitted caching to enabled**) and `--dry-run`. **`ensure` creates every config
  with query caching DISABLED unless given `--caching-enabled` (owner decision, section 11), and `check` fails a standard config with caching on or absent.** 43 hermetic cases for the Cloudflare scripts with a fake `curl`/`aws` (and 17 for `pr-schema.sh` with a fake `kubectl`), each behaviour paired with a control that must fail `[tested]`.
* VPC service by hand (fallback if the token's identity cannot hold the role): `wrangler vpc service create thrust-pg --type tcp --tcp-port 5432 --app-protocol postgresql --tunnel-id <id> --hostname thrust-pg-rw.thrust.svc.cluster.local --cert-verification-mode verify_ca` `[tested: --help]`, then `tofu import … '<account_id>/<service_id>'` `[doc: provider docs]`.
* **Roles and token scopes.** VPC service create/update/delete: account role **Connectivity Directory Admin**; for an API token "ensure the token belongs to a user with
  the required roles" `[doc]` (the exact token permission-group name is `[unverified]`). Hyperdrive: **`Hyperdrive Write`** (`Read` for list) `[doc: API reference, Accepted
  Permissions]`; Workers roles "do not grant access to … Hyperdrive" and wrangler's OAuth login "does not currently support granular authorization" `[doc]`, which matches
  the owner's report that the OAuth login has `connectivity (admin)` but not Hyperdrive. **A Cloudflare API token with Hyperdrive Write is needed** — and the scripts
  prefer a separate `HYPERDRIVE_API_TOKEN` so the token that can edit tunnels cannot also rewrite database credentials.

## 4. Apply runbook (condensed; every command is a prediction until run)

| # | Step | Credential / who | Expect | Rollback |
|---|---|---|---|---|
| 0 | Preflight: stateful-pool "Allocated resources" per node, CRDs and `aws-secretsmanager` store present, `status.woven` baseline. **Measured 2026-10-18: 1,830m CPU free, memory not binding (section 10)** | operator, kubeconfig | room for 3 more instance pods **and two distinct nodes**: 750m of 1,830m | stop |
| 1 **DONE** | *(applied by the owner/coordinator 18:46Z, 4 added, re-plan clean; step 2 values put afterwards)* `! aws sso login --profile tf-admin-aiml-dev`; `tofu -chdir=products/thrust/aws init -backend-config=…products-thrust-aws-646…`; `plan`; `apply` | human (the `!` is required); R2 keys via `woven-r2-tofu`; only a real `plan` proves the SSO token | **4 to add** (3 containers + `thrust-eso-read`) | `tofu destroy` (SM names are reserved for a recovery window) |
| 2 | put the two owner passwords: `openssl rand -hex 24 \| tr -d '\n' > $f; aws secretsmanager put-secret-value … --secret-string file://$f` | operator's SSO admin (OpenTofu is denied PutSecretValue, ADR 0004) | values exist; never echoed | overwrite |
| 3 | `tofu -chdir=products/thrust/cloudflare init/plan/apply` under `op run --env-file .env` | `CLOUDFLARE_API_TOKEN` (Tunnel Edit + VPC write; identity holds **Connectivity Directory Admin**) | **3 to add** | delete Hyperdrive first, scale connector to 0, `tofu destroy` |
| 4 | `scripts/store-tunnel-token.sh` (`--dry-run` first) | the same token (Tunnel Read) + SM PutSecretValue | prints a length, never the token | overwrite / rotate the tunnel token |
| 5 | `tofu -chdir=products/thrust/kube init/plan/apply` (no saved plan: EKS token ~15 min); no `-target` | SSO admin; cluster admin | **8 to add** | before data: `tofu destroy`; after data: final `Backup`, prove it, then destroy — volumes reclaim `Delete`, namespace deletion is unrecoverable |
| 6 | Verify: 5 ExternalSecrets `SecretSynced`; `thrust-pg` 2/2 and dev 1/1 healthy, two pods on different nodes; 2 connector pods Ready and "Registered tunnel connection"; tunnel `healthy` via API; on-demand `Backup` `completed`; `pg_stat_archiver.failed_count = 0`; **`sslmode=disable` probe MUST be rejected** | operator | as stated | — |
| 7 | `hyperdrive.sh ensure thrust-pg …` / `thrust-pg-dev …` (dry-run first), then `check` | `HYPERDRIVE_API_TOKEN` (Hyperdrive Write) + AWS read of the SM value | exit 0 | `wrangler hyperdrive delete <id>` |
| 8 | Smoke Worker `thrust-pg-smoke` (`nodejs_compat`, `pg ≥ 8.16.3`): `select … from pg_stat_ssl where pid = pg_backend_pid()` (expects `ssl true`) and `BEGIN … COMMIT` over one connection; then `wrangler delete` | Workers Scripts Edit | `{"who":{"u":"thrust","ssl":true},"txn_rows":2}` | `wrangler delete` |
| 9 | Restore drill into `thrust-pg-drill` incl. the wrong-`serverName` negative control; record RTO | operator | measured RTO ≪ 8 h | delete the drill cluster |

If step 7 fails with a TLS error: apply the cloudflare root with `-var cert_verification_mode=disabled`, retry, and record the result (that is the answer to the unverified `verify_ca` question).

## 5. Cost estimate for the in-cluster pieces — **an estimate, list prices from memory, not verified against today's price sheets**

| Item | Basis | ≈ per month |
|---|---|---|
| EBS gp3 | 2×20 GiB + 10 GiB = 50 GiB × $0.08 | **$4.00** |
| Node share, Postgres | instance-pod requests **0.75 vCPU** / 2.5 GiB (after the 250m decision) on `m7i.xlarge` (4 vCPU / 16 GiB, ~$0.20/h ≈ $147/mo): 18.75 % by CPU, 15.6 % by memory | **≈ $28** allocated (OpenCost charges max(request, usage)) — was ≈ $46 at 500m |
| Node share, connectors | 2 × (100m / 128 Mi) ≈ 5 % of a node | **≈ $7** |
| R2 backups | tiny DBs; ~8.6k WAL PUTs/cluster/month at Class A $4.50/M; ≤ 10 GB stored × $0.015 | **< $1** |
| Secrets Manager | 3 containers × $0.40 | **$1.20** |
| NAT data processing | DB responses leave the VPC through the NAT: $0.045/GB processed + internet egress beyond the free tier; ≤ 50 users | **$1–7** |
| Tunnel, Workers VPC, Hyperdrive | Workers VPC "free while in beta" `[doc]`; Hyperdrive has no separate charge I found `[unverified]` | **$0** |

Allocated total **≈ $42–48/month** (was ≈ $60–70 at 500m). **The marginal cash cost is ≈ $7–13 (EBS + Secrets Manager + R2 + NAT) if the stateful pool already has headroom — and the owner's measurement says it does for 750m — plus up to ≈ $7 if the connectors force a new Karpenter node; a whole extra node (~$147) only if headroom turns out not to hold** — and the pool's
`max_size` default is 4, the size the EKS runbook says it was already raised to, so "another node" is itself a change to `products/agentic-platform/aws`. This sits
outside the under-$100/month Cloudflare-side envelope (Q-G5.1).

## 6. Risks

1. **Availability coupling to woven-o11y.** The product now depends on a dev-tagged observability cluster that was upgraded 1.32 → 1.37 over 2026-10-13/14, whose hazards
   fired (PDB at 0, stateful pool 94 % full). `thrust-pg` at 2 instances **still** has CNPG's primary PDB at 0 (as `gitea`): drains need the replica-delete procedure.
   `thrust-pg-dev` needs `nodeMaintenanceWindow`. `status.woven` does **not** probe this path (its prober is WARP/`.woven`; this is not a `.woven` name) — Q-0094.4 stays open.
2. **Node-pool capacity.** Measured (section 10): 1,830m CPU free over four nodes (775m / 625m / 160m / 270m). With 250m requests the three instance pods (750m) fit on three nodes in two AZs, but a feasible
   placement leaves the us-east-1a node at ~99.5 % requested (20m spare), so any later pod pinned there goes Pending. The zone spread is a scheduler *preference*, not a guarantee: both `thrust-pg` instances can still
   land in one AZ. If that is unacceptable the levers are more headroom (`products/agentic-platform/aws`, `stateful_ng_max_size`, which the owner keeps at 4) or a hard rule, which the chart deliberately does not offer.
3. **Workers VPC is beta** (`cloudflare_connectivity_directory_service` needs ≥ 5.13.0; fields and the resource may change). Fallback: ADR-0094 Option 2 over the same tunnel.
4. **TLS**: encrypted, server not authenticated against our CA (section 2); `verify_ca` on a private CA unverified.
5. **Hyperdrive pooling over the tunnel and its limits**: unmeasured; origin connection limit set explicitly (40/20) against PostgreSQL's default `max_connections` 100 `[inferred]`;
   Hyperdrive does not support advisory locks, `LISTEN/NOTIFY`, SQL `PREPARE`, per-session state `[doc]`; **query caching is now DISABLED** on the configs this repo creates (owner decision, section 11): strict read-after-write, at the cost of whatever caching would have saved; spike 0002 may measure caching on a
   separate config (`--caching-enabled`).
6. **CNPG operator 1.30.0 (chart 0.29.0) and Barman plugin 0.7.0** are pinned by another root; an operator upgrade rolls these clusters too. CRDs from exactly those chart
   versions validated the rendered CRs `[tested]`, but only a live operator proves behaviour.
7. **One tunnel, two clusters**: a dev load test shares production's two connector pods (and 100m/1 CPU limits). Splitting it is three-root work; flagged, not built.
8. **Shared backup credential**: the estate token for the `backups` bucket can read every product's backups; per-product tokens are the recorded isolation follow-up.
9. **Owner-password rotation has no zero-downtime variant** (seconds of auth failures between `ALTER ROLE` and the Hyperdrive replace).
10. **The chart bump is visible in nine other roots' plans.** `Chart.yaml` 0.2.0 → 0.3.0 makes every consuming root's next `tofu plan` show its `helm_release` as updated in place (the provider diffs on chart
    version); the rendered manifests are byte-identical, so no pod should roll, but the plan noise will appear in nine roots and the Terraform side of each is "not applied" until someone does. The green copy in
    `ApiraTech/helm-charts charts/cnpg-cluster` needs the same value to stay in step (`blue-green-parity` is local-only and does not block merge).
11. **`env: prod` label on a dev-tagged cluster** deviates from the estate's single-environment `dev` convention (a deliberate choice to tell the two databases apart).

## 7. What is NOT verified, because only the `aws` root was applied

Everything live (all of it except the `aws` root, which the owner applied and re-planned clean): that the plan counts are 4 / 3 / 8; that the EKS token, SSO and R2 backend credentials work; that ESO syncs (and the `thrust-eso-read` grant is sufficient);
that CNPG bootstraps with the supplied Secret and schedules `required` anti-affinity; that the connector registers over QUIC and the Workers VPC service resolves
`thrust-pg-rw.thrust.svc.cluster.local` from the connector pod; that `verify_ca` (or `disabled`) lets Hyperdrive create succeed; the token permission-group names; that
`cloudflare/cloudflared:2026.6.0` is current; Hyperdrive/VPC latency and pooling; backup completion, WAL archiving and any restore time; the stateful pool's real headroom;
that a Worker holds `BEGIN … COMMIT` through the VPC service. Also not verified: that `file://` secret values are what ESO delivers byte-for-byte (the script tests use fakes);
the `Connectivity Directory Admin` path for an API token.

What **was** verified locally: `tofu fmt -check`, `init -backend=false` and `validate` in each of the three roots; five-platform lockfiles (aws and kube are **byte-identical** to
`products/plane/{aws,kube}`'s: aws 5.100.0 / helm 2.17.0 / kubernetes 2.38.0; cloudflare 5.28.0, 17 hash lines like the other cloudflare roots); `tofu test` contracts for the
cloudflare (3 runs) and kube (5 runs) roots, each with expect-failure controls and a **mutation check** (a wrong port and `required→preferred` made them fail); both CNPG
releases rendered with `helm template` exactly as `datastore.tf` injects values, and **all four CNPG/Barman kinds validated against the CRDs of the pinned operator and plugin
chart versions with unknown keys forbidden** (a typo'd key and a name collision both failed the control run); `kubeconform` ran but **skipped all 8 objects** (every kind is a CRD),
so it asserted nothing beyond YAML shape; `shellcheck` clean on all five scripts; `mdbook build` and a relative-link check over the new pages. Re-verified after the round-27 changes: section 11.

## 8. Gate results

Baseline = clean `origin/main` `d05c7586`, before any change. `just pr-gates` lists 27 recipes; `just` has no `--keep-going`, so each recipe was run individually
(`scratchpad/infra/run-gates.sh`) so one red could not hide the rest.

| Gate (`just pr-gates` order) | Baseline `origin/main` | With the change |
|---|---|---|
| pki-test | pass | pass |
| codex-wrappers-test | pass | pass |
| fmt-check | pass | pass |
| chart-render | pass | pass |
| cnpg-cluster-conformance | pass | pass (still "all 6 releases": I added no golden, see below) |
| coredns-guard-test | pass | pass |
| single-owner-guard | pass (69 releases) | pass (**71 releases**: it sees `thrust-pg` and `thrust-pg-dev`, no collision) |
| appprotocol-guard | pass | pass |
| grafana-dashboards-guard | pass | pass |
| dashboard-tools-test | pass | pass |
| dashboard-fmt | pass | pass |
| edns-suffix-guard | pass | pass |
| mesh-injection-guard | pass | pass (no allowlist line: `thrust` carries no injection marker) |
| local-chart-version-guard | pass | pass (chart untouched) |
| edns-hostname-ratchet | pass | pass |
| edns-verify-test | pass | pass |
| forward-compat-guard | pass | pass |
| module-source-guard | pass | pass |
| seam-register-guard | pass | pass |
| module-convention-guard | pass | pass |
| inventory-guard | pass (34 charts, cnpg chart consumed by 9) | pass (34 charts, consumed by **11**) |
| chart-request-guard | pass | pass |
| blue-green-parity-test | pass | pass |
| beads-config-commit-test | pass | pass |
| beads-backup-health-test | pass | pass |
| required-checks-guard | pass | pass |
| agentregistry-prerequisites-test | pass | pass |

**All 27 pass on the final tree, and all 27 pass on the baseline.** Environment findings, none a repo defect and none "fixed" in the repo:
* On this Mac **eight gates fail on a clean `origin/main`** (pki-test, chart-render, single-owner-guard, seam-register-guard, inventory-guard, chart-request-guard, required-checks-guard, agentregistry-prerequisites-test): the ambient `python3` has no PyYAML, so the guards exit 3 "PyYAML is not installed … could not evaluate" or die with `ModuleNotFoundError`. I ran them with PyYAML from a uv-managed scratch venv put first on `PATH`.
* `agentregistry-prerequisites-test` **still failed** with that, with `ModuleNotFoundError: No module named 'yaml'` from inside `uv run --with PyYAML==6.0.2`: the recipe defaults `UV_CACHE_DIR` to `/tmp/infrastructure-uv-cache`, which on this machine holds a stale archive whose `--with` environment is empty. A fresh cache directory fixes it `[tested]`. I did not touch that cache.
* CI's `actions/checkout` ships PyYAML; a laptop without it reports "exit 3", which `ci-gates.md` says is never a pass. These runs are the evidence that exit 3 was environmental.

Gates not in `pr-gates` and **not run**: `just coredns-guard` (needs a cluster; this root writes no CoreDNS state), `required-checks-audit` (needs network and a token), `just check` (touches live state), `hook-tests` (guard a laptop, not a PR). **No gate in the repo lints Markdown or checks links**; I ran `mdbook build` (passes) and my own relative-link check over the new pages (0 broken). `tofu test` and the script tests are local only and are not `pr-gates` jobs (adding a gate means registering its required context after its first run on `main`).

## 9. Open questions for the owner

1. ~~**TLS**~~ **Decided (round 27): keep `verify_ca`, `disabled` as fallback; hardening (Origin CA / public-CA certificate so `verify_full` works) is a recorded follow-up, not done.**
2. **One tunnel or two?** Today a dev load test shares production's connector. Worth a second tunnel, token container and connector now?
3. Is an **un-meshed `thrust` namespace** acceptable (ADR 0022 D6), given you asked for "mesh opt-out as in plane-pg"? The opt-out is set either way.
4. **Who holds Connectivity Directory Admin** and the Hyperdrive Write token, and where do the per-operator `op://` items live (`.env.example` ships placeholders only)?
5. ~~**Stateful pool**~~ **Decided (round 27): 250m requests + best-effort zone spread; the pool maximum stays 4.**
6. ~~**Caching**~~ **Decided (round 27): disabled for `thrust-pg`; this author also made it the default for `thrust-pg-dev` (parity) and per-PR configs.**
7. **Per-PR**: who owns the CI wiring and the sweep of stale `pr_*` schemas, and are per-PR Hyperdrive configs acceptable?
8. A least-privilege **`thrust-deploy` role** for `products/thrust/aws` now (the `plane-deploy` precedent) or later (`mlflow` runs ambient)?
9. **`env` label** `prod` vs the estate's `dev`, and the `team: UNOWNED` sentinel — who owns Thrust?
10. **NetworkPolicy**: wanted? It needs a live cluster to validate.
11. **ESO-generated owner passwords** (ADR 0019) instead of hand-injected ones — the estate's own preferred pattern for self-invented secrets, at the cost of a PutSecretValue grant.
12. Should `status.woven` grow a probe for this path (Q-0094.4)?

## 10. Coordinator preflight, owner decisions and the first apply (2026-10-18)

* **Read-only preflight of the stateful pool** `[tested]` (`kubectl --context woven-o11y`, 17:3xZ): four `m7i.xlarge` nodes with label `pool=stateful`, allocatable 3920m CPU each; nodes `10-0-16-124` and `10-0-19-248` (us-east-1b), `10-0-47-188` (us-east-1c), `10-0-5-50` (us-east-1a). CPU requests 3145m, 3295m, 3760m and 3650m (80, 84, 95 and 93 %), memory requests 42, 39, 77 and 48 %. **Free CPU requests: 775m, 625m, 160m, 270m (1,830m).** As authored the change adds about 1,450m (thrust-pg 2x500m anti-affine on hostname, thrust-pg-dev 250m, two connectors 100m each), which fits only if both thrust-pg pods land on the two us-east-1b nodes, i.e. **one availability zone**, leaving about 25m free on one node and no slack for a switchover.
* **Owner decisions** (round 27, AskUserQuestion answers): (1) **TLS** `verify_ca` now, `disabled` as the fallback, hardening later; (2) **capacity**: lower `thrust-pg` requests to 250m and add a best-effort (preferred) zone spread, the pool max stays 4; (3) **caching**: the production Hyperdrive config is created with caching disabled; (4) apply `products/thrust/aws` now.
* **First apply** `[tested]`: `products/thrust/aws`, `AWS_PROFILE=tf-admin-aiml-dev`, a saved plan applied as reviewed: **4 added, 0 changed, 0 destroyed** at 18:46Z (`/thrust/pg-owner-password`, `/thrust/pg-dev-owner-password`, `/thrust/cloudflared-tunnel-token`, and the inline policy `thrust-eso-read` on `woven-o11y-dev-eso-irsa`); a re-plan afterwards reported no changes. The containers are **empty**: the owner passwords and the tunnel token are put out-of-band (steps 2 and 4). The policy grants `secretsmanager:DescribeSecret` and `GetSecretValue` on exactly those three secrets plus the shared CNPG R2 backup credential, additively on the existing ESO role. Everything after step 1 (the Cloudflare root, the kube root, the Hyperdrive config) is **not applied**.
* **Runbook step 2 done** (2026-10-18, owner go-ahead for the next stage) `[tested]`: both owner passwords were generated with `openssl rand -hex 24` and put into `/thrust/pg-owner-password` and `/thrust/pg-dev-owner-password` (48 characters each, one version each; values never printed, temp files removed; the script skips a container that already has a version, so a re-run cannot overwrite them). `/thrust/cloudflared-tunnel-token` is still empty by design (step 4).
* **Runbook step 3 blocked on credentials** `[tested]`: the account-owned token `op://<vault>/<item>/token` first could read tunnels only as an empty list (**a false negative**: the Woven account has six tunnels, which a token with tunnel-read permission lists) and got **403 on the Workers VPC connectivity directory and on Hyperdrive**. The Cloudflare root needs a token with Account, Cloudflare Tunnel, Edit and the Workers VPC (connectivity directory) write permission, held by an identity with the **Connectivity Directory Admin** role; the Hyperdrive step needs Hyperdrive Write (Q-0094.6). The kube root also waits for the author's pending edits (250m requests, zone spread, caching default).

## 11. Round-27 decisions implemented (author, after the owner reviewed this note)

Authoring and local validation only: no apply, no live plan, no `kubectl`, no cloud call, no push, no PR.

**1. TLS wording.** `products/thrust/cloudflare/README.md`, `docs/src/operations/thrust-postgres.md` and ADR 0022 D4 now say plainly: the hop is encrypted, the server is **not** authenticated against our CA; `verify_ca` stays the
default with `disabled` as the documented fallback; the hardening (CNPG serving a Cloudflare Origin CA / public-CA certificate so `verify_full` works — needs a chart `spec.certificates` change, a hostname Origin CA can issue
for, and the services moved back to `verify_full`) is a recorded follow-up, **not done**. No code default changed.

**2. Capacity.**
* `thrust-pg` requests **250m** CPU (memory request 1Gi, limit 2Gi kept); `thrust-pg-dev` unchanged (chart default 250m/512Mi).
* **CPU requests added: thrust-pg 2 × 250m = 500m, thrust-pg-dev 250m, two connectors 2 × 100m = 200m — 950m in all, of which 750m lands on the stateful pool, against the 1,830m currently free there** (was 1,450m / 1,250m at 500m).
  Per node: 250m fits the 775m, 625m and 270m nodes (two AZs: 1b ×2, 1a), not the 160m node (1c); 500m fitted only the two 1b nodes (one AZ). One feasible placement (prod on the 625m-1b and the 1a node, dev on the 775m-1b node)
  leaves 1,080m free but the 1a node at ~99.5 % requested. Memory (coordinator's measurement, 42/39/77/48 % requested) is not binding for the 2.5 GiB asked.
* **Zone spread — possible, but only with a chart change.** The generic chart rendered only one anti-affinity (`podAntiAffinityType` + `topologyKey`), so it could not express "required hostname AND preferred zone". CNPG 1.30's Cluster CRD
  has `spec.topologySpreadConstraints` (full Kubernetes shape; and `affinity.additionalPodAntiAffinity`) `[tested: pinned CRD]`, so the chart gained a **strictly opt-in** value `zoneSpread: {enabled: false, topologyKey: topology.kubernetes.io/zone, maxSkew: 1}`
  rendering exactly one constraint with `whenUnsatisfiable: ScheduleAnyway` (deliberately not a value: a full zone must degrade to co-location, never to a Pending database pod) and selector `cnpg.io/cluster=<name>, cnpg.io/podRole=instance`.
  `thrust-pg` sets `zoneSpread.enabled: true`. **Proof the other releases are untouched:** a raw `helm template` of all **nine** pre-existing releases' values files (affine, aegra, zitadel, langfuse, agentregistry, litterer, mlflow,
  plane, gitea) is **byte-identical** before and after (`cmp`; control: opting in changes the render by 8 lines) `[tested]`; `just cnpg-cluster-conformance` passes with all six goldens **unchanged** and a new section D (absent by default; soft
  constraint when enabled; no value can make it hard) `[tested]`; a **mutation** (template that always renders it) fails both section A and D `[tested]`. `Chart.yaml` is bumped 0.2.0 → 0.3.0 as `local-chart-version-guard` requires.
  **Consequence to know:** each of the nine consuming roots shows its `helm_release` as updated in place on its next plan (the provider diffs on chart version) with identical manifests; and the green chart copy in `ApiraTech/helm-charts` needs the
  same value to stay in step. It is a *preference* weighed against other scheduler scores, not a guarantee. `thrust-pg-dev` is unchanged.

**3. Hyperdrive caching.** `hyperdrive.sh ensure` now creates every config with `caching: {disabled: true}` — `thrust-pg` (owner decision) and, by this author's decision, `thrust-pg-dev` (parity) and per-PR configs — unless
`--caching-enabled` (the old `--caching-disabled` flag is refused, exit 2). `check` fails a standard config whose caching is enabled or absent (absent = Cloudflare's default = enabled). **`rotate-password` carries the live `caching` block
and `origin_connection_limit` into its PUT**, because the API's PUT resets omitted caching to defaults (`[doc: Replace Hyperdrive]`): without that, a rotation would silently re-enable caching. 43 hermetic cases (was 36), with controls and two
mutation runs (default flipped back; carry-over removed) that fail as required `[tested]`. Spike 0002 may measure caching on a **separate** config, e.g. `ensure thrust-pg-cache --vpc-service thrust-pg --user thrust --caching-enabled --origin-connection-limit 10`.

**4. Applied aws root.** Runbook step 1 is marked DONE (owner-applied, 4 added, re-plan clean); the runbook, ADR 0022, the ADR index and `products/thrust/README.md` say which roots are applied. The counts predicted for the remaining roots are
unchanged: **3 to add** (cloudflare) and **8 to add** (kube; the chart bump adds no resource). `products/thrust/aws` and `products/thrust/cloudflare/*.tf` were not touched in this round.

**Re-run results (this round).** `tofu fmt -check -recursive products/thrust` clean (the cloudflare and aws `.tf` were not edited); `tofu validate` kube **Success**; `tofu test` kube **5 passed, 0 failed** (with new assertions on 250m/1Gi/2Gi and `zoneSpread`; a 500m mutation made it fail); `bash test-scripts.sh`
**43 passed**; `bash test-pr-schema.sh` **17 passed** (re-run; the script did not change); `shellcheck` clean on all five scripts; `helm template` of both releases + **CRD validation against the pinned CNPG 0.29.0 / Barman 0.7.0 schemas**
(unknown keys forbidden): all objects valid, `thrust-pg` carries exactly one `ScheduleAnyway` zone constraint selecting only its own instances, requests 250m/1Gi, memory limit 2Gi, **stateful-pool CPU added = 750m** (control: a hard rule + 500m fails) `[tested]`;
`kubeconform` again skipped all objects (CRDs); `mdbook build` passes and a relative-link check over the new/edited pages found 0 broken; **`just pr-gates` — all 27 recipes pass** (run individually with the PyYAML venv and a fresh `UV_CACHE_DIR`; `single-owner-guard`
71 releases, `inventory-guard` chart consumed by 11, `local-chart-version-guard` and `chart-request-guard` green with the bumped chart, **`cnpg-cluster-conformance` PASS: six goldens identical + section D**).
* **Runbook step 3 prepared** (2026-10-18, after the owner added the tunnel and VPC permissions to `thrust-spikes`) `[tested]`: the re-probe lists 6 tunnels in the Woven account and reads the VPC directory (0 services); **Hyperdrive read is still 403**. `products/thrust/cloudflare` was initialised against the R2 backend and a saved plan reads **3 to add, 0 to change, 0 to destroy**: the tunnel `thrust-pg` (`config_src = cloudflare`) and the two Workers VPC TCP services `thrust-pg` and `thrust-pg-dev` (port 5432, `app_protocol postgresql`, `cert_verification_mode = verify_ca`, hosts `thrust-pg-rw.thrust.svc.cluster.local` and `thrust-pg-dev-rw.thrust.svc.cluster.local`, resolver network = the new tunnel). Not applied yet.
* **Runbook steps 3 to 6 done** (2026-10-18, owner go-ahead, each apply shown as a plan first) `[tested]`:
  * **Step 3, `products/thrust/cloudflare`:** saved plan applied, **3 added, 0 changed, 0 destroyed**, re-plan shows no changes: tunnel `thrust-pg` (id `<thrust-tunnel-id>`) and the Workers VPC TCP services `thrust-pg` (`01a1508b-22c6-7b41-8a34-3d64dd6d2d82`) and `thrust-pg-dev` (`01a1508b-22b4-7443-bdd4-88075eeec578`), port 5432, `postgresql`, `verify_ca`. The API shows the tunnel **healthy with 8 connections** and both services as configured.
  * **Step 4, tunnel token:** `store-tunnel-token.sh` (dry run first) stored a **240-character** connector token in `/thrust/cloudflared-tunnel-token` (one version); the value was never printed.
  * **Step 5, `products/thrust/kube`:** applied with no saved plan, **8 added, 0 changed, 0 destroyed**: namespace `thrust`, five ExternalSecrets, `thrust-pg`, `thrust-pg-dev`, the `cloudflared` Deployment and its PodDisruptionBudget.
  * **Step 6, verification:** all **5 ExternalSecrets `SecretSynced`** (so the `thrust-eso-read` grant is sufficient, including the shared R2 backup credential); `thrust-pg` **2 of 2 ready**, primary `thrust-pg-1` on node `10-0-19-248` (us-east-1b) and replica on `10-0-5-50` (us-east-1a): **the best-effort zone spread worked**; `thrust-pg-dev` 1 of 1 on `10-0-16-124`; both connector pods Running with **4 registered QUIC connections each**; **plain-text probe rejected** (`pg_hba.conf rejects connection ... no encryption`) with the **control** (`sslmode=require`) getting past `pg_hba` and failing only at authentication (`no password supplied`); an on-demand `Backup` (`thrust-pg-verify-1`) **completed in 4 s** (19:52:01 to 19:52:05Z), and `pg_stat_archiver` shows **4 archived, 0 failed**, so the R2 backup path and WAL archiving work.
  * **Not yet done:** step 7 (the Hyperdrive configs, which need an API token with **Hyperdrive Write**; Hyperdrive read is still 403 for `thrust-spikes`), step 8 (the smoke Worker doing `BEGIN ... COMMIT` and reading `pg_stat_ssl`, which also answers whether `verify_ca` accepts CNPG's CA and whether the VPC service resolves `thrust-pg-rw.thrust.svc.cluster.local` from the connector), step 9 (the restore drill and the measured RTO). The two connector pods both sit on us-east-1b nodes (a replica spread was not requested for them).
  * **State versus code:** these roots were applied from the **local, unpushed** branch `feat/thrust-cnpg-tunnel-vpc` (nine signed commits). The state is in R2; the code exists only in that worktree until the branch is pushed. Do not prune the worktree.

## 12. Live run folded back into the infra branch (author, 2026-10-18; sections 1-11 untouched)

Authoring and local validation only; the live facts below were run and reported by the coordinator. Commits `5686de71` fix(thrust) and `8c63159a` docs(thrust), signed, not pushed.

* **TLS, tested: `verify_ca` FAILED, `disabled` WORKED.** On `verify_ca`, `hyperdrive.sh ensure thrust-pg` failed ("Failed to connect to the provided database: Internal error."); after `-var cert_verification_mode=disabled` it succeeded. This overturns the section-2 hope that `verify_ca` would pass a private CA (it was marked unverified). The hop is encrypted (`pg_stat_ssl`: TLSv1.3) and the server is **not** authenticated against our CA. `var.cert_verification_mode` now defaults to **`disabled`** (`products/thrust/cloudflare/variables.tf`), so a plain `tofu plan` matches live; `verify_ca`/`verify_full` stay allowed. The contract test pins the new default and that `verify_ca` stays selectable; mutation runs fail for the right reason (default reverted -> the default assertion fails; `verify_ca` dropped from the allowed values -> the selectable run fails) `[tested]`. Hardening (Origin CA / public-CA certificate via a chart `spec.certificates` value, Workers VPC custom-CA upload, or ADR-0094 Option 2) remains a follow-up, not done.
* **Provider bug on update.** An in-place update of `cloudflare_connectivity_directory_service` returns a new `created_at`, and the provider fails with "Provider produced inconsistent result after apply … .created_at" although the change took effect (API-verified; a re-plan showed no changes). **`ignore_changes = [created_at]` does not help** `[tested: scratch copy]`: OpenTofu itself emits "Redundant ignore_changes element … `created_at` is decided by the provider alone and therefore there can be no configured value to compare with", and the failure is an apply-time consistency check, not drift. I could not reproduce the provider's behaviour hermetically (a mock never changes `created_at`), so "only an upstream fix removes it" is reasoning from that diagnostic, not a demonstration. Documented in the cloudflare README and runbook step 3: verify via the API and a re-plan, do not retry blindly; `-replace` would change the service id and break the Hyperdrive configs.
* **Live ids (non-secret) recorded in the runbook:** tunnel `<thrust-tunnel-id>`; VPC services `thrust-pg` `01a1508b-22c6-7b41-8a34-3d64dd6d2d82`, `thrust-pg-dev` `01a1508b-22b4-7443-bdd4-88075eeec578`; Hyperdrive `thrust-pg` `8294c22a2a304146821a8bf11258e74d` (limit 40) and `thrust-pg-dev` `b231d16b672b469d8834a071fa842b9c` (limit 20), caching disabled, `hyperdrive.sh check` OK for both.
* **Smoke test passed** (Worker colo IAD, `pg` 8.16.3, `nodejs_compat`): `ssl true`, TLSv1.3, `txn_rows 2`, commit and rollback controls "table gone (expected)", connect 4-5 ms, txn 42-81 ms, `select 1` p50 9-10 ms / p95 12-16 ms (two warm calls; not the spike's p95). The runbook's step 8 now carries the Worker as run (random-path guard `SMOKE_PATH`, rollback and after-commit controls, 30 timings) and the lessons (first call 404 from `workers.dev` propagation; keep the path guard; delete with OAuth). Steps 1-8 are marked done and **step 9 (restore drill) is the only one remaining**. The apply counts and per-step output of steps 4-6 were not reported to the author, and the runbook says so rather than asserting the predicted 8 to add.
* **Re-run (this round):** `tofu fmt -check -recursive products/thrust` clean; `tofu validate` cloudflare **Success**; `tofu test` cloudflare **4 passed, 0 failed**; `test-scripts.sh` **43 passed**; `shellcheck` clean on all five scripts; `mdbook build` rc 0; relative-link check 0 broken; **`just pr-gates`: all 27 recipes pass** (run individually, PyYAML venv, fresh `UV_CACHE_DIR`).
* **PR #810 merged and the restore drill ran** (2026-10-18) `[tested]`: the owner merged `ApiraTech/infrastructure#810` at 21:10:27Z as one squash commit `46c22f3a` on `main`; its `products/thrust`, shared-chart and runbook files equal the branch head, so the code on `main` is what was applied. Two review threads were answered and resolved (the Bugbot per-PR connection-limit finding, fixed in `eebccec7`; the security review of the disabled TLS verification, accepted by the owner as temporary with a deadline, `infra-qnyi` and `plane-byd.9`). **Restore drill (runbook step 9) on `thrust-pg`:** a new cluster `thrust-pg-drill` was **Ready 60 s** after apply; sentinels written before and after an on-demand base backup (backup 4 s) were all present, so WAL replay worked; `thrust` owns the restored table and is not a superuser; the recovered cluster was promoted; the **negative control** (`serverName: thrust-pg-typo`) failed hard with `no target backup found`, retried, never reached Ready and did not fall back to an empty `initdb`; all drill objects were removed and the live cluster's UID, primary and conditions equal the baseline. **Caveats:** the database was near-empty (60 s does not extrapolate); the figure excludes repointing Hyperdrive or the VPC service and an application cutover; the `s3://backups/thrust/` prefix listing was not checked (needs the shared backup credential); `thrust-pg-dev` and a PITR restore were not drilled. The runbook outcome is in docs PR `ApiraTech/infrastructure#811` (draft, docs only).
