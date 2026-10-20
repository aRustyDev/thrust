# SPEC — Contrail (MVP) / Thrust (v1): Plane rebuilt Cloudflare-native, portable by construction

> Status: **rough draft v0** from the 2026-10-17 interview. To be reviewed and refined by the next session.
> Normative words (MUST/SHOULD/MAY) are used only where the user decided; everything else is a *proposal*.
> Source tags: **[I]** interview · **[S]** repo survey · **[O]** Open-EE plan. Companion files: FEATURES.md,
> ARCHITECTURE.md, QUESTIONS.md, adrs/.

## 1. Purpose
Build an application equivalent to **Plane** (open-source work tracker; this repo is a fork, AGPL-3.0) that runs
**Cloudflare-native**, by keeping Plane's React frontend and replacing its Django/Celery/Hocuspocus backend with
Workers-native services. **[I]**

## 2. Goals
- G1. Functional equivalence with Plane CE for the features tiered P0/P1 in FEATURES.md. **[I]**
- G2. The Open-EE feature set (RBAC, ReBAC, SCIM, audit, custom fields, workflows, …) is delivered *natively* in the
  new backend; the Django-based Open-EE program is superseded. **[I][O]**
- G3. **Portability by construction**: every Cloudflare primitive is used only through a port with both a
  Cloudflare adapter and a self-hosted adapter; switching is a config/compile-time change plus at most a redeploy. **[I]**
- G4. Single-org first; data layer MUST be multi-tenant-capable. **[I]**
- G5. Public `/api/v1` stays wire-compatible with Plane; the internal UI API is redesigned. **[I]**
- G6. Auth: end state is OIDC (Zitadel) behind Cloudflare Access plus `X-Api-Key` for machine clients, built on a **BetterAuth-shaped identity** (round 29, FR-16: BetterAuth's HTTP shapes only, implemented in Rust as core code; spike 0003 is the two-day auth slice, Q-0050.4) and phased **user+password (P0) → Cloudflare Access → OIDC**. **[I]**
- G7. The frontend is owned code: **fork and diverge** from upstream Plane; no upstream-merge requirement. **[I, round 4]** **Round 21:** the React frontend is the phase-1 UI, adopted with minimal changes; it is later replaced by a Rust (Dioxus) clone, then diverges (G9).
- G8. P0 foundations: permission seam + RBAC, event/outbox backbone, feature flags, audit logging (round 29, FR-18: audit logging F-E06 moves to P1 and the retained outbox is the P0 audit trail; feature flags are a `flags` table read per request, with no port). P0 product scope:
  work-management core, platform basics, minimal auth; page editing is a plain API save at P0 (client-produced forms stored verbatim); Durable-Object collaboration is P1 (see FEATURES.md for the tier of every row). **[I, round 4]**
- G9. **Backend in Rust from the start** (wasm32 Worker + native server from the same crates) and a **Dioxus UI for multi-target builds** (web, desktop, mobile) once backend and React frontend are stable (ADR-0006, ADR-0013). **[I, round 21]**

## 3. Non-goals (proposed — confirm)
- Rewriting the React apps, editor or design system **in phase 1** beyond minimal changes (the Dioxus rewrite is a later phase, ADR-0013).
- Binary-compatible Django internals, Celery task names, or Django admin.
- Feature work beyond Plane CE + the Open-EE list without a tiering decision.
- Day-one multi-tenant SaaS operations (billing, quotas) — only the data-layer capability.

## 4. Principles
1. **Hexagonal**: domain/use-cases import ports only; adapters are the sole holders of platform SDKs (ARCHITECTURE §2).
2. **Contracts first**: events, API schemas, file-asset lifecycle are versioned artifacts with contract tests.
3. **One suite, N adapters**: each port ships a shared contract-test suite run against every adapter (ADR-0121).
4. **Transactional outbox** for all side effects (activity, notifications, webhooks, search indexing).
5. **Permission seam in the use-case layer**, never only in routers.
6. **Reversible decisions stay reversible**: open choices (DB engine, API style) are ADRs with spikes, not assumptions.
7. **Behavior oracle**: Plane itself (OpenAPI schema + recorded responses) defines "equivalent" (ADR-0122).

## 5. System context
See ARCHITECTURE.md §0–§1. Today: Caddy → {static React apps, Django/DRF, Hocuspocus} with Postgres, Redis, RabbitMQ,
MinIO and Celery. Target (CF set): Access → one Worker (static assets, fetch, queue consumer, cron; round 29, FR-11) plus a collab
Worker with its Durable Object at P1, with DB (⚠), R2, Queues, Cron (Workflows withdrawn and no KV session store,
round 29, FR-05, FR-03). **[S]**

## 6. Requirements

### 6.1 Functional
- R-F1. Implement every feature in FEATURES.md at its assigned tier. IDs F-A…F-X are the traceability keys.
- R-F2. Work-item `sequence_id` allocation MUST be gap-tolerant, monotonic per project, **never reused after delete/move** (Plane uses `MAX(IssueSequence.sequence)+1`), allocate `sort_order` in the same critical section, and be race-safe: **no duplicate in 10k creates issued by ≥50 parallel writers** on every supported DB adapter (Q-0040.2, spike 0002). **[S]**
- R-F3. Soft-delete semantics (`deleted_at`) and the "hidden" row classes (triage, archived, **project-archived**, draft) MUST be preserved
  in listing/count behavior (ADR-0043). **[S]**
- R-F4. Collaborative pages MUST persist `binary`, `html` and `json` description forms consistently and support
  versioning (ADR-0061). **[S]**
- R-F5. `/api/v1` responses MUST match **the pinned instance's drf-spectacular schema** (v1 only; the published docs differ from CE) for P0/P1 resources, and its recorded behaviour (ADR-0031, ADR-0122).
- R-F6. Background duties of `plane/bgtasks` MUST be mapped to Queue/Cron/sync (ADR-0073; Q-0073); round 29 (FR-05): no Workflow, long jobs are queue messages with a `jobs` table (ADR-0071). **[S]**
- R-F7. **Two HTTP surfaces at P0 (round 29, FR-06):** a transitional `compat-plane` adapter serves the internal endpoints the adopted React app expects, bounded by the F-X13 acceptance list (FR-01, `PLAN.md`), and `/api/v1` is its own Plane-shaped surface (ADR-0031), not a shim over a core API. The core REST+OpenAPI API (code-first, ADR-0032) is generated from the `thrust-core` use-cases at M4, when the Dioxus UI is its first client. The slice's routes that are not Plane-shaped move out of the `/api/v1` prefix, which is reserved for wire-compatible routes (issue #47). Compat rows are deleted as each screen moves to Dioxus, and the compat adapter is deleted when ADR-0013 Phase 2 completes (ADR-0034). This replaces the round-22 text, in which one core API served v1, scripts, MCP and the Dioxus UI. **[I, round 22; round 29]**

### 6.2 Portability
- R-P1. No import of platform SDKs outside `adapters-*` packages (lint-enforced).
- R-P2. A single typed `AppConfig` selects one adapter per port; invalid combinations fail at startup (ADR-0007).
- R-P3. Every port has ≥1 CF adapter and ≥1 self-hosted adapter before the port is considered done. **Phasing
  (round 29, FR-19; replaces the round-6 and round-25 lists, Q-F6, Q-F6.2):** at P0 the self-hosted set is **the
  database, the outbox as queue, a local-disk `BlobStore`, SMTP via `lettre` and an in-process scheduler**; sessions
  are rows in that database (FR-03). The S3-compatible blob adapter (for MinIO) arrives at P1. After the FR-05 cut no
  P0 port is CF-only, so the contract-test matrix has no `pending-adapter` row; a port that later lands with only its
  CF adapter is marked `pending-adapter` until its self-hosted adapter lands (wording confirmed round 9, Q-F6.1).
  Self-hosted parity is checked by the `native-pg` contract cell that already runs in CI (FR-19).
- R-P4. Cloudflare-only features (DO classes; the Workflows binding is withdrawn, round 29, FR-05) MUST be isolated in `adapters-cloudflare`.
- R-P5. Adapter sets are **compile-time bundles** (ADR-0007): one composition root per set; invalid combinations fail the build.

### 6.3 Security & tenancy
- R-S1. No repository query without the tenant predicate (ADR-0041).
- R-S2. Secrets never in code/config files; per-environment (ADR-0092).
- R-S3. SSRF guard on all server-initiated fetches (links, webhooks). **[S]**
- R-S4. API keys stored hashed; per-key rate limits (ADR-0033) expressed in the supported subset of `API_KEY_RATE_LIMIT`: **10 s or 60 s periods, per-location and approximate** (Workers Rate Limiting binding).
- R-S5. Public spaces expose only explicitly published data (ADR-0022/0141).

### 6.4 Non-functional (partly set — Q-G4/Q-G4.1)
- **Sizing target (round 5): ≤50 users, ≤100k work items** in a single org; no import of existing data (Q-G3: start fresh).
- **v1 targets (round 8): internal-tool grade — 99.5% availability, p95 API < 500 ms.** Business-grade targets and
  RPO/RTO are backlogged follow-on work (bead `plane-9y3`, Q-G4.2).
- **Recovery (round 13):** RPO 24 h, RTO 8 h (daily DB export to R2 plus provider PITR where available). **Collab sizing:** up to 50 concurrent editors per page at P1; the DO spike must stress-test this (ADR-0060).
- **Cost (round 9):** Workers Paid baseline, under $100/month as a design constraint; spikes report projected monthly cost at the sizing target (Q-G5). **The Postgres provider is not inside the envelope (round 27, Q-G5.1):** the CNPG cluster in woven-o11y (ADR-0094) is paid for outside it; the envelope covers the Cloudflare side.
- **Internal API:** transitional `compat-plane` for React plus the new core REST/OpenAPI API (ADR-0034); round 29 (FR-06): the core API is generated at M4, so P0 has two surfaces (R-F7). No live push at P0 (refetch/poll), live push with collab in P1. Runtime is Rust (ADR-0006).
- **Latency budget (round 29, FR-08):** the P0 Worker runs with **`placement.region = "aws:us-east-1"`**, next to the database (ADR-0094; issue B5). A list endpoint makes **at most 5 database round trips** per request, enforced by a counting `Connector` in the contract suite (issue B5). Cloudflare: each sequential query adds 20 to 30 ms from a distant region and 1 to 3 ms when placed nearby (hyperdrive/concepts/how-hyperdrive-works). Spike 0002 re-runs the bench with the placement set and reports the equivalent counts for its `worker-do` cell (ADR-0040, round 29).
- **Measurement (round 29, FR-08, FR-10; answers Q-G5.2): one SLI per probe (owner, round 29).** **p95 latency** comes from a synthetic probe from the origin's region on the warm path at a 5-minute cadence, with cold starts reported separately (FR-08). **Availability (99.5 % over a rolling 30 days)** comes from the `status.woven` deep probe `GET /healthz?deep=1` every 60 s; the endpoint is on the product hostname and runs `select 1` through Hyperdrive, so the SLO is measured end to end and cluster upgrades count (FR-10, ADR-0094; issue #42). This corrects the earlier round-29 sentence, which measured availability "on the same probe" as p95. The first database-backed request on a fresh isolate (about 0.6 s, `research/0004` §10.3) is such a cold start. The cost envelope **excludes** the Postgres provider (answered, round 27, Q-G5.1). This replaces the earlier proposal (p95 at the Worker edge by synthetic probes plus Workers metrics).
- **Oracle coverage:** `/api/v1` by schema plus recordings; compat-plane endpoints by recordings only (no schema exists for the internal API).
- **Oracle (round 8):** the v1 reference is this fork updated to latest upstream, then pinned (bead `plane-b6a`), used as a
  running instance plus its OpenAPI schema (ADR-0122).
- **Public spaces (round 8):** unauthenticated, explicit publish per project, P0 (ADR-0022).
- **Roles (round 8):** fixed Admin/Member/Guest at P0, no Viewer; custom roles decided in tiering (ADR-0051).
- Hostnames (round 5): product on public hostnames behind Access; admin/internal on private `.woven` names (ADR-0090, Q-0090.3). Round 29 (owner default, Q-0090.3): no `.woven` host at P0 or M2; the product is on its public hostname; reopen when an internal-only surface is planned.

### 6.5 Operability
Trace correlation across HTTP → Queue → DO (ADR-0130; no Workflow hop after round 29, FR-05); IaC for all CF resources (ADR-0093);
deploys with migration ordering: expand migration, deploy at 100 %, smoke against the live hostname, automatic
rollback (ADR-0110; gradual rollout dropped in round 29, FR-12).

## 7. Plane → Cloudflare mapping (starting hypotheses)
| Plane component | Target (CF adapter) | Self-hosted adapter | ADR |
|---|---|---|---|
| Django/DRF API | API Worker (Rust, workers-rs, wasm32) | same crates as a native axum server | 0006, 0034 |
| Postgres | ⚠ Hyperdrive→PG / D1 / DO-SQLite | Postgres | 0040 |
| Celery + RabbitMQ | Queues; long jobs are queue messages with a `jobs` table (no Workflows) | the outbox: in-process fast path + sweeper, one process (no RabbitMQ) | 0070, 0071 |
| Celery beat | Cron Triggers | scheduler | 0072 |
| Redis/Valkey (cache, sessions) | none: sessions are database rows (FR-03); rate limits on the Workers Rate Limiting binding (D-007) | none: sessions are database rows; in-process rate limiter (no Valkey) | 0082, 0033 |
| MinIO/S3 | R2 | local disk at P0; S3-compatible (MinIO) at P1 | 0080 |
| apps/live (Hocuspocus+Redis) | one `yrs` room in a Durable Object (P1) | the same `yrs` room inside the axum server (P1; no Hocuspocus) | 0060 |
| Caddy + static builds | Workers static assets | the native binary serves the built React apps itself; Caddy or nginx in front is the operator's option (owner, round 29) | 0011, 0091 |
| SMTP | CF Email Service / SMTP | SMTP via `lettre` | 0073 |
| session/OAuth auth | OIDC + Access + API keys | OIDC | 0050, 0052 |
| Django admin/license | admin app on instance-config API | same | 0021 |

**Round 29 (2026-10-19, FR-05, FR-19):** the Celery, Redis/Valkey, MinIO/S3, `apps/live` and SMTP rows follow the
cut: no Workflows, RabbitMQ, Valkey or Hocuspocus, and a local-disk blob store at P0. A `Cache` port is added only when
a measured hot path needs it (ADR-0082). **Static hosting (owner, round 29):** the self-hosted native binary serves the
built React apps itself, as Workers static assets do on Cloudflare, and a front proxy (Caddy or nginx) is the
operator's optional choice. This keeps ADR-0091's "Compose file with at most three services" (binary, Postgres, MinIO
at P1) and supersedes ADR-0011's self-hosted Caddy/nginx line.

## 8. Phasing (round 29: see `PLAN.md`)
The phasing is the milestone roadmap in [`PLAN.md`](PLAN.md): M0 to M6 with contents, exit criteria, the critical path
and the M2 screen list (round 29, FR-01, `reviews/2026-10-19-fable-architecture-review.md` §3). It replaces the
indicative phases this section listed until round 29, which map to the milestones as follows (review §3, "Diff
against SPEC §8"):

| SPEC §8 until round 29 | Round 29 | Why |
|---|---|---|
| Phase 0 Foundations: ports package, harness, config, CI, DB spike | M0: the same, plus the two port-shape fixes and the engine decision with a DO cell | the harness exists; the shapes are wrong (FR-02, FR-03) |
| Phase 1 Core vertical slice | M1 plus M2, with a named screen list instead of "web app wired to the new API" | the compat obligation must be bounded (FR-01) |
| Phase 2 Breadth | M3, after a daily-use release rather than before it | ship something usable first |
| Phase 3 (P1) Collab, exports, importers, analytics | M5 for collab; exports move into M3 as queue jobs (FR-05); importers and analytics stay P1 | no workflow engine |
| Phase 4 Open-EE superset | folded into M6 and later; audit at P1 (FR-18) | |
| Phase 5 Self-hosted parity in parallel | already struck earlier in round 29 (FR-19, R-P3): it is the `native-pg` contract cell that runs in CI (`scripts/contract-all.sh`), not a phase | |
| Phase 6 Dioxus after P0+P1 and 4 weeks of SLO | M4, after M2 and 4 weeks of SLO, screen by screen (FR-01) | shrink the throwaway; the React app becomes the UI oracle |

## 9. Acceptance criteria (proposed)
1. All P0 features pass e2e on the CF adapter set **and** the self-hosted adapter set (the P0 self-hosted set defined in R-P3).
2. Contract-test suite green for every port × adapter.
3. `/api/v1` oracle diff shows no breaking differences for P0 resources.
4. Switching adapter set requires only config/build flags (+ redeploy) — demonstrated by a CI job.
6. Every FEATURES row has an ADR or an explicit "no ADR needed" in TRACEABILITY.md, and every R- requirement names a test home there; intentional departures from Plane are recorded in DEVIATIONS.md.
5. ~~Import of a Plane Postgres dump~~ — **dropped** (Q-G3: start fresh). Replace with: a seeded ≈100k-work-item workspace meets the agreed latency targets (Q-G4.1).

## 10. Risks
| Risk | Impact | Mitigation |
|---|---|---|
| Compat-plane fidelity: ~233 internal endpoints (+66 auth/public, ~70 v1) with no schema; the inventory (Q-0010.1) is not done and F-X13 is P0 | schedule | spike 0009 inventory; rank by React call frequency; ship the minimum slice; recordings as oracle |
| DB choice constrains sequencing/transactions/analytics | rework | spike before commit (ADR-0040) |
| Abstraction tax: ports lowest-common-denominator, lose CF strengths | quality | per-port "capability" flags; contract tests only on shared semantics |
| ~~Self-hosted workflow engine has no drop-in equivalent~~ | — | **retired (round 29, FR-05):** no workflow engine; long jobs are queue messages with a `jobs` table (ADR-0071) |
| DO limits for large docs / many editors | collab | spike; snapshot strategy (ADR-0061) |
| Upstream Plane drift vs forked frontend | maintenance | **Accepted** — Q-G2 answered "fork and diverge" (round 4) |
| Identity on wasm32/Workers (round 29, FR-16: hand-rolled Rust; the framework options are dropped); security regression surface from rebuilding CSRF, sessions, hashing, SSRF guard | P0 auth blocked / security | spike 0003; external review gate for the identity adapter and SSRF guard; promote ADR-0141 before any public host |
| AGPL obligations (everything AGPL-3.0, no clean-room claim; keep Plane attribution) | legal | ADR-0140 (accepted) |
| Scope: Plane CE + 25 Open-EE features | schedule | tiering (Q-F1) |
| Rust on Workers maturity (workers-rs: Queues beta, D1 alpha, panic=abort; Workflows/Email unconfirmed) | P0 blocked | spike 0001; bounded JS shim fallback (Q-R4) |
| Rich-text editor has no mature Rust equivalent (server conversions, Dioxus editor) | quality/schedule | spikes 0006, 0007 |
| Oracle drift: the fork carries woven OIDC code, the schema is v1-only and off by default, synthetic seed data never exercises real shapes; upstream security fixes bypass the pinned frontend | quality/security | record the pin delta (spike 0008); owner for upstream CVE triage; small anonymised real-shape sample for the corpus |
| Cost/latency reality: external Postgres behind Hyperdrive, per-PR dev schemas, <$100/mo may exclude the DB provider | budget | cost model inside spike 0002; decide schema-per-PR cleanup/ownership before it |
| Capacity: one owner, 25 Open-EE rows, 49 background tasks, ~300 endpoints, two UIs over time | schedule | `PLAN.md` carries the capacity assumptions and the cut line (round 29, FR-01): the smallest usable release is M1 plus M2 |

## 11. Open items
All open items live in QUESTIONS.md; ADR index with status in adrs/README.md.
