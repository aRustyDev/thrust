# PROMPT — Contrail → Thrust: refine the SPEC and produce an implementation plan

Paste or reference this file to start the follow-up session. Work from `.agents/plans/contrail/` (it moves to
`docs/plan/` in `git@github.com:aRustyDev/thrust.git` when that repo is first populated, ADR-0005).

## Your role
You are a staff-level architect/engineer. A prior session interviewed the owner over 23 rounds and drafted this package to
rebuild **Plane** (this repo is an AGPL-3.0 fork, frozen as the v1 oracle) as a **Cloudflare-native,
portable-by-construction** application: MVP codename **Contrail**, v1 product **Thrust**, written in **Rust**.
Your job: **review and refine the SPEC, run the spikes, close the remaining open questions with the owner, promote ADRs,
then write the implementation plan.** You do not write product code in this session.

## Read in this order
0. `REVIEW-NOTES.md` (independent review, round 25), `TRACEABILITY.md` (feature/requirement → ADR → spike → test; lists gaps) and `DEVIATIONS.md` (intentional departures from Plane).
1. `SPEC.md` — goals, principles, requirements, mapping, phasing, risks.
2. `ARCHITECTURE.md` — deployed (Cloudflare set + self-hosted twin) and application (hexagonal) architecture.
   (§2a is the Rust layout, redrawn in round 25 and annotated with the spike 0001 findings in round 26.)
3. `FEATURES.md` — fully tiered feature inventory (P0/P1/P2/drop).
4. `adrs/README.md`, then the ADRs for the area you work on (MADR). Status: 38 accepted · 20 proposed · 2 superseded (60 ADRs; ADR-0094 added and ADR-0012 accepted in round 27).
5. `spikes/README.md` — nine spikes, their exit criteria and (round 26) their status. `research/0002-rust-dioxus-fit.md` (verification tags matter), then the wave-1 results: `research/0003-compat-plane-inventory.md` (spike 0009), `0004-rust-on-workers-binding-matrix.md` (0001), `0005-oracle-harness.md` (0008), `0006-topology-public-half.md` (0004).
6. `QUESTIONS.md` — the work queue. `research/0001-admin-capability-inventory.md` (accepted, auth rows to redo).
7. Plane source (`apps/api`, `apps/live`, `apps/web`, `packages/*`) and the superseded Open-EE program on branch
   `woven/plan`: `git show woven/plan:.claude/plans/plane-fork/PROGRAM.md`, `…/features/*.md`, `…/research/*.md`.

## Direction (owner, rounds 21–22)
- **Backend: Rust from the start** — ports are traits; adapter sets are cargo-feature bundles; one codebase builds a
  `wasm32` Cloudflare Worker (workers-rs) and a native axum/tokio server for self-hosting. (ADR-0006, 0002, 0007)
- **Frontend phase 1: Plane's React apps adopted with minimal changes** — only base URL/service layer, auth client,
  collab provider and branding. A transitional `compat-plane` adapter serves Plane-shaped internal endpoints; one new
  REST + OpenAPI core API serves `/api/v1`, MCP and the Dioxus UI. (ADR-0034)
- **Frontend phase 2: Dioxus clone of the Plane UI/UX**, then diverge after parity, for multi-target builds. Starts when
  P0+P1 are shipped, the SLO has held 4 weeks and the v1 oracle diff is clean; **v1 targets web only**, desktop/mobile
  after. (ADR-0013)
- **Non-Rust exceptions** only as bounded, documented shims behind a port, each with an exit plan. (ADR-0006)
- **Fallback:** if spike 0001 fails, a TypeScript core on Workers reopens ADR-0006 and ADR-0013's premise.

## Decided — do not reopen without new evidence
**Architecture & scope**
- Hybrid keep-frontend-first (ADR-0001, now transitional); ports & adapters with CF *and* self-hosted adapter per port,
  compile-time bundles (0002, 0007); single-org first, multi-tenant-capable (0003); Open-EE-on-Django superseded, features
  built natively (0004); repo = `aRustyDev/thrust`, `crates/thrust-*` Cargo workspace + `web/` pnpm workspace (0005);
  AGPL-3.0 for everything, no clean-room claim, never copy proprietary EE code (0140); start fresh, no data import.
- **P0 foundations:** permission seam + RBAC (fixed Admin/Member/Guest, no Viewer), event/outbox backbone, feature
  flags, audit logging. **P0 scope:** work-management core F-B01–B16 (+B18), platform basics, minimal auth (user+password),
  API keys, membership (invites P1), public spaces, admin settings subset (F-A10, `apps/admin` retired), oracle harness,
  observability, `compat-plane` adapter. **Plain API save for pages/descriptions at P0**; Durable-Object collab is P1.
  Open-EE rows are tiered in FEATURES.md (round 10). Fixed tiers for all CE rows (round 12).
- Sizing ≤50 users, ≤100k work items; SLO 99.5% / p95 < 500 ms; RPO 24 h, RTO 8 h; ≤50 concurrent editors per page;
  cost < $100/month (Workers Paid). Business-grade targets are backlog (bead `plane-9y3`).

**API, auth, data**
- `/api/v1` stays wire-compatible (0031); oracle = this fork updated to latest upstream, pinned (bead `plane-b6a`), run via
  compose nightly, synthetic seed data, plus its OpenAPI schema; Django CI kept only as oracle runner (0120, 0122).
- OpenAPI: v1 code-first constrained by the frozen Plane schema diff, core API code-first (0032). Authz: static capability map at P0, no cache, role read per request, collab revocation ≤5-min token, permission proof by types (0051). Deploy: 0% upload + override-header smoke, 10% for 10 min, absolute-count gates, auto-rollback (0110).
- **Round 25 (review decisions):** sessions are DB-authoritative with no cache at P0 (ADR-0082's DO mandate withdrawn); P0 self-hosted set = DB, queue, blob, session, scheduler (in-process), SMTP email; one same-origin hostname per surface with path-scoped Access at the Access phase; P0 rich text = Option B (store client forms verbatim, sanitise in Rust), spike 0006 runs right after 0001. Also: search = FTS **plus trigram** for substring parity (Plane uses `icontains`); retention default 30 d is a recorded deviation (D-001) and hard-delete cleanup F-D10 is **P0**; compat-plane gets an inventory spike 0009 before P0 planning; SSRF guard is a P0 row (F-X14). Review findings: `REVIEW-NOTES.md`.
- Auth: BetterAuth-shaped identity; phased user+password (only P0 login) → CF Access (UI hosts only) → OIDC/Zitadel with
  JIT via claims at P1; `X-Api-Key` (hashed, no scopes v1, approximate per-location rate limits); 30-day sessions,
  sign-out-everywhere (0050, 0052, 0033). How BetterAuth maps to Rust is spike 0003. SCIM P2, only if Zitadel supports it (0053).
- Rich text stored as Yjs binary + html + json with an optimistic version check; versions every 10 min; ≤5 s staleness (0061).
- Search = database FTS, fresh within 1 min, no semantic search (0083). Outbox = immediate publish + Cron sweeper, 90-day
  retention (0046 direction). Sequence key changes on move with alias (0043 direction). 5 MB asset default; public assets via
  Worker proxy (0047 direction). Daily hard-delete, 30-day default retention. Schema frozen at the pinned Plane version.

**Platform & ops**
- Static assets on Workers static assets for `web`, Caddy/nginx self-hosted (0011); `space` is a plain SPA with Rust-injected link-preview metadata (Q-0011.3); collab on Durable Objects (Yjs), Blob = R2,
  Queue/Workflow/Cron = Queues/Workflows/Cron Triggers, cache = KV / Valkey (0060, 0070–0072, 0080, 0082).
- Self-hosted reference = Docker Compose with a **native Rust binary**; P0 self-hosted adapters: DB, queue, blob, session (DB-backed), scheduler (in-process), SMTP email (round 25).
- **Round 27 (owner):** Postgres = CloudNativePG in woven-o11y, reached through a new Cloudflare Tunnel and a Workers VPC service (ADR-0094), its cost outside the $100 envelope (Q-G5.1); reproduce all of Plane's 500s and document every quirk (Q-0033.2, `research/0007`); no tags are pushed to the oracle (Q-0122.4).
- **Round 27 (owner, cont.):** compat-plane cap = tripwires (reopen ADR-0034 if the minimum slice costs more than 66 pd or the P0 total passes 364 pd; Q-0009.1); `space` is a plain SPA with per-board metadata injected by Rust, no JS exception (Q-0011.3, which superseded the SSR Worker after the feasibility build); the infra for the Postgres is authored on the infra branch `feat/thrust-cnpg-tunnel-vpc` and `products/thrust/aws` is applied (ADR-0094, `research/0010`); CNPG as two clusters, `thrust-pg` (prod, 2 instances) and `thrust-pg-dev` (Q-0094.1); spike 0002 is Postgres only.
- **Editor (ADR-0012, accepted round 27):** reuse `@plane/editor`, Yjs binary format unchanged; P0 stores the client forms verbatim and sanitises in Rust; P1 conversion is the Rust `thrust-editor-doc` behind an `EditorDocs` port (about 16 pd) with an oracle CI; a JS conversion Worker is a documented fallback only.
- Email = CF Email Service / SMTP; AI = default-off, per-workspace opt-in, AI Gateway for both sets, MCP P1 separate from
  assistant P2 (0084); webhooks at-least-once, HMAC, 5 retries, 30-day logs; exports CSV/JSON via Workflow, 24 h links (P1).
- Public spaces: unauthenticated, explicit publish per project, own hostname via a separate cached Worker, 60 req/min/IP (0022).
- Observability: Workers Observability + OTLP to the woven stack, PostHog dropped, Gatus SLIs (0130). IaC: OpenTofu in the
  infra repo + wrangler for code; envs dev (PR previews, shared DB)/staging/prod; 1Password local, GitHub env secrets in CI;
  GitHub OIDC deploy; supply-chain gates carried over (0092, 0093, 0100, 0101). `.woven` via tunnel origin to a Worker custom
  domain — to be proven (spike 0004).

## Spike results (round 26) — evidence, not owner decisions
Detail is in each ADR's "Update — spike results, round 26" and in `research/0003`–`0006`. Nothing was promoted: proposed ADRs stay proposed until the owner accepts them. Figures are the spike agents' reports; the coordinator verified the branches, commits, tag and files.
- **0001 Rust on Workers: conditional pass.** One HTTP suite green on 4 targets (16 cells, 77 executions); the P0 bindings are native; Workflows needs a JS shim; **ports return `Send` futures** (ADR-0002; the `?Send` sketches in ADR-0045/0070/0071/0080 were corrected); `sqlx` does not build for wasm32 but `tokio-postgres` works over a Hyperdrive socket; a Durable Object needs its own Worker root. Stage 2 (real platform plus a Postgres) has not run: Q-0006.2.
- **0008 oracle harness (+ `plane-b6a`): done.** The fork is merged with upstream and pinned (local tag `oracle/v1-pin-20261018`, woven OIDC kept); the schema covers 130 of 135 v1 operations; Plane fails its own schema in 12 classes, so the oracle is **schema + exception register + golden corpus**; v1 estimates are not routed. Not pushed: Q-0122.4.
- **0009 compat-plane inventory (desk half): done** on the pre-pin and the pinned trees. 545 compat rows, 290 P0 rows used by React; minimum slice 48 routes (about 39 person-days), all P0 about 208 pd; the auth client is the biggest lever inside the allowed change areas; **`apps/space` is server-rendered in Plane** (resolved by Q-0011.3: SPA plus Rust head injection). The cap is Q-0009.1. The dynamic half (HAR, replay) is next.
- **0004 topology, public half: done** (two real runs; the second with the Access gate on: 35 assertions in 3 passes). One security finding: **Access's bypass match is case-insensitive** (Q-0050.7); `/api/*` does cover deep paths; the edge normalises `..` and `%2e` before matching; **a service-binding hop costs +2.3 to +2.8 ms on the real edge** (the docs say zero). Q-0030.2, Q-0011.1 and Q-0090.2 are answered; the `.woven` half is a desk recipe that needs four owner decisions (ADR-0090). Two certificate packs remain to delete by hand.
- **`space` SSR feasibility: done** (round 27, `research/0009-space-ssr-worker.md`): an SSR Worker is feasible with config-only changes (about 8.5 pd), but SSR only produces the per-board `<head>`, so the owner chose an SPA plus Rust head injection with no JS exception (Q-0011.3); the SSR build is the fallback.
- **0006 editor conversions: done** (round 27, `research/0008-editor-conversions.md`): Option A (Rust, `thrust-editor-doc`) is byte-identical to the JS oracle on 1,337 of 1,337 measured pairs and 6,000 fuzz inputs; recommended P0 = B, P1 = A behind an `EditorDocs` port (about 16 pd), C only as a fallback; the real JS editor opens Rust-produced binaries; sanitiser at parity with nh3 (hardened variant recommended). ADR-0012 was accepted (Q-0012.3); still open for the owner: Q-0012.2 (sanitiser) and Q-0012.4 (browser-flavour renderer).
- **Plane quirks catalogue: done** (round 27): `research/0007-plane-quirks-catalog.md`, 157 entries (53 break clients, 93 affect parity tests), the reference for "reproduce all 500s". It found 88 quirks the spikes had not listed, corrected EX-010/EX-011 and the 500 set, and flagged two security-relevant defects (PQ-412, PQ-413), which the owner decided to **fix** as deviations D-014 and D-015 (Q-0033.3 fix).

## Superseded — do not follow older text that says otherwise
- ADR-0010 (regenerated TS client) and ADR-0030 (Hono/typed RPC) → ADR-0034. Drizzle as leading data layer → open (spike 0002).
- "Editing on Durable Objects at P0" (round 4) → plain API save at P0. "Node/Bun" self-hosted runtime → native Rust binary.
- "Web apps only, no mobile/desktop" (Q-G8) → true until Dioxus Phase 2 only.
- Anything in ADR text that assumes TypeScript, vitest, Zod or Miniflare as the primary toolchain: see the round-21 notes.

## Still open
- **Spikes (`spikes/README.md` has the status table):** done: 0008, 0009 (desk half); 0001 conditional (Stage 2 pending); 0004 public half and 0006 done; not started: 0002 (DB engine + the 0041–0045 data layer, FTS), 0003 (identity), 0005 (collab yrs), 0006 (editor conversions), 0007 (Dioxus). Next: 0002, 0003 and 0001's Stage 2 together once the CNPG Postgres, tunnel and VPC service exist (ADR-0094, bead `plane-byd.4`).
- **Proposed ADRs (20):** 0008 feature flags (new), 0048 audit logging (new), 0040–0045 data, 0046/0047 contracts (details), 0061 collab persistence,
  0073 pipeline mapping, 0081 uploads, 0090/0091 topology and twin, 0120–0123 test model/standards, 0141 threat model. Most wait on spikes 0001/0002/0004/0006.
- **Owner-level questions still open:** Q-0094.2 to .4 (TLS to Hyperdrive, the tunnel and VPC service, availability coupling), Q-0006.2 (Stage 2 inputs), Q-G5.2 (SLO measurement), Q-F7 (second user at P0), Q-R7 (upstream security triage), Q-0050.7 (case-insensitive Access bypass), Q-0090.3 (four `.woven` decisions), Q-0085.2, Q-0053.1, plus the data-layer rows that follow the DB spike. See QUESTIONS.md for the full list.
- **Nothing is committed.** Commit/push/repo population are the owner's call.

## Tasks
1. **Sanity-review** SPEC/ARCHITECTURE/ADRs against the code. Verify [S]-tagged claims (advisory-lock sequencing,
   soft-delete/partial-unique constraints, bgtasks inventory, live persistence flow). Record corrections in place and in
   `REVIEW-NOTES.md`. Update ARCHITECTURE §2a to the Rust layout and reconcile TypeScript-era text in ADRs.
2. **Finish the spikes** in isolated worktrees (status table in `spikes/README.md`; wave 1 is done or in progress); write results into the ADRs.
3. **Drive QUESTIONS.md to zero open rows.** Interview style: ≤4 questions per AskUserQuestion round, lead with a
   recommendation, fold each answer into ADR/SPEC/FEATURES/ARCHITECTURE. Never mark an ADR `accepted` without a recorded
   owner decision. Add new questions at the end; never renumber.
4. **Produce `PLAN.md`:** phases, milestones, dependency graph, per-phase acceptance tests, risk burndown, and a beads
   epic/child breakdown (`bd create`, parent/child, dependencies). Phase 0 includes the ports crate, contract-test
   harness, adapter-selection (cargo features) config, CI matrix (wasm32 + native + React jobs) and the oracle harness.

## Constraints & repo rules
- **Always work in a git worktree on a feature branch** (`docs/…`, `feat/…`); never in the canonical checkout or on `main`.
- Conventional Commits; work-item ID in a `Refs:` **footer**, never the subject. Do not commit/push unless asked.
- Track work with `bd` (beads), not TodoWrite or markdown TODOs.
- ADRs use MADR in `adrs/<group>/NNNN-slug.md`, are indexed in `adrs/README.md`; new decisions take the next free number;
  supersede rather than edit accepted decisions (mark `superseded by ADR-NNNN`).
- Verify Cloudflare limits/APIs with the Cloudflare docs tools before relying on them; cite in ADRs.
- A probe that cannot observe the failing subsystem is not evidence — prefer the real operation (see global rules).
- Python tooling, if any, uses `uv`.

## Definition of done for this session
SPEC and ADR set reviewed; every spike has a recorded result; zero `open` rows in QUESTIONS.md (or each remaining one has an
owner, a date and a non-blocking justification); `PLAN.md` written with a beads breakdown.

## Round log (newest last; details live in the ADR "Update" notes and QUESTIONS.md)
1–8: approach, tenancy, DB/API/auth/Open-EE direction, portability ports, P0 groups, search/email/AI/admin, oracle, roles, SLO ·
9: cost, internal API leaning (superseded), plain-save confirmation · 10–12: all features tiered · 11: rich text, DB spike scope ·
13: BetterAuth bar, `.woven` tunnel, RPO/RTO, 50 editors · 14: AGPL, Compose, parity, EE branches · 15: repo/IaC/keys/sessions ·
16: environments, outbox, CI/CD gates · 17: webhooks/exports, workflow runner, security · 18: names, observability, search/versions ·
19: AI policy, public spaces hosting, rename to `contrail` · 20: Access scope, oracle data, small policies ·
**21: Rust backend + Dioxus direction; spikes and research written** · **22: ADR-0034 compat layer + core API, Dioxus gate, shim policy** ·
23: repo layout (`thrust-*`, `crates/`, `web/`), SCIM/integrations, admin inventory accepted, ADR-0020/0021/0053/0085 accepted, PROMPT consolidated.
24: OpenAPI hybrid (ADR-0032), authz details (ADR-0051), deploy gates (ADR-0110) accepted; independent package review started.
25: independent package review (`REVIEW-NOTES.md`); toolchain wording fixed; sessions, self-hosted P0 set, host/Access layout, P0 rich text decided.
25 (cont.): should-fix batch applied across 3 ADR groups by forks; retention 30 d deviation + F-D10 P0; FTS+trigram; spike 0009; ADR-0008/0048 added (proposed); TRACEABILITY.md and DEVIATIONS.md created.
26: wave-1 spikes run in parallel (0001, 0008 with `plane-b6a`, 0004 public half, 0009 desk half); results consolidated into the ADRs, QUESTIONS (7 new rows), ARCHITECTURE, DEVIATIONS (candidates), TRACEABILITY and REVIEW-NOTES; nothing promoted, committed or pushed (spike branches are local).
27: owner decisions (Postgres = CNPG in woven-o11y via a new Tunnel and Workers VPC service, ADR-0094; provider outside the cost envelope; reproduce all 500s, document every quirk; no tags pushed to the oracle); spike 0004 run 2 consolidated (case-insensitive Access bypass; +2.3 to +2.8 ms per service-binding hop); spike 0006 and the quirks catalogue started; Q-0009.1 and Q-0011.2 under discussion.
27 (cont.): owner chose tripwires for the compat cap, an SSR Worker for `space` (feasibility build first), two CNPG clusters, and a Postgres-only spike 0002.
27 (cont.): the quirks catalogue is in (157 entries); it corrected the oracle register and flagged two security-relevant defects for a decision (Q-0033.3).
27 (cont.): the `space` SSR feasibility build shows SSR only produces link-preview metadata; an SPA plus Rust head injection is offered as an alternative (Q-0011.3).
27 (cont. 2): owner chose an SPA plus Rust head injection for `space` (superseding the SSR Worker) and to fix the two security defects (D-014, D-015).
27 (cont. 3): spike 0006 consolidated (Option A passes, recommended for P1; sanitiser stance and acceptance are Q-0012.2 to .4).
27 (cont. 4): owner accepted ADR-0012 as recommended (P0 = B, P1 = A behind an `EditorDocs` port, C a fallback); 38 accepted / 20 proposed / 2 superseded.
27 (cont. 11): the plan PR (aRustyDev/plane#61) is docs-only: the 26 `.py` scripts fail the repository's required Copy Right Check and `pin-verify.json` trips gitleaks (two tree SHAs, false positives), so the scripts are on branch `chore/contrail-plan-scripts`; the thrust spike branches are pushed with draft PRs #17 (spike 0001, `main`) and #18 (spike 0006, stacked on #17, conflicting in `Cargo.lock`). 27 (cont. 10): spike 0001 STAGE 2 RAN on the real platform (Woven account, `thrust-pg-dev` via Hyperdrive) and was torn down and verified: the shared suite 15/15 (8/8 on repeat) after one adapter change (Hyperdrive fails tokio-postgres' default prepare-then-execute flow; every statement now goes through `query_typed` / `execute_typed`); bundle 1.67 MiB, startup 2 to 3 ms; DO hibernation, real cron (53 s), DLQ after 3 retries, Rate Limiting binding approximate (3 allowed 4); Email not exercised; the local 16-cell matrix green; the `thrust-spikes` token lacked KV, R2 and Secrets Store reads so the OAuth login was used; new questions Q-0006.4 to .6; `research/0004` section 10 has the record. 27 (cont. 9): infra PR #810 MERGED by the owner (squash `46c22f3a`); the restore drill ran on `thrust-pg` (recovered cluster Ready after 60 s on a near-empty database, WAL replay verified, wrong-serverName control failed hard); runbook outcome in draft docs PR #811; follow-ups: authenticated TLS before real data (`plane-byd.9`), repeat the drill with real data (`plane-byd.10`). 27 (cont. 8): PR ApiraTech/infrastructure#810 review: the Bugbot per-PR connection-limit finding was fixed (shared constant, eebccec7) and the security review's TLS finding was accepted by the owner as TEMPORARY with a deadline (authenticated TLS before real data; `plane-byd.9`, `infra-qnyi`); both threads resolved. 27 (cont. 7): Hyperdrive configs created (`thrust-pg`, `thrust-pg-dev`, caching disabled) and a Worker smoke test passed end to end (role `thrust`, TLSv1.3, commit and rollback work, `select 1` p50 ~10 ms from IAD); `verify_ca` FAILED and `disabled` works, so the hop is encrypted but the server is not authenticated (Q-0094.9); restore drill remains; the infra draft PR is ApiraTech/infrastructure#810. 27 (cont. 6): the Postgres is LIVE: the aws, cloudflare and kube roots were applied after plans were shown (4 + 3 + 8 added), verified (secrets synced, `thrust-pg` 2 of 2 across two AZs, tunnel healthy, TLS-only enforced, backup and WAL archiving work); Hyperdrive, the smoke Worker and the restore drill remain. 27 (cont. 5): infra authored (plane-byd.4: 45 files, 27 gates pass, local branch; the owner decisions are implemented in five more signed commits, with one side effect: the shared CNPG chart bump touches the next plan of nine other roots, Q-0094.8) and `products/thrust/aws` applied (4 secrets/policy objects); owner chose verify_ca TLS, 250m requests with a preferred zone spread, and caching disabled; the Cloudflare and kube roots wait for credentials and a go-ahead.
