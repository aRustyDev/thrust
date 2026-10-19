# REVIEW-NOTES — independent review of the Contrail/Thrust planning package

Reviewer: independent pass, 2026-10-18. No existing file was modified.

**Snapshot caveat.** The package was being edited while I reviewed (round 24 landed mid-review: ADR-0032/0051/0110 flipped to
`accepted`, README now says 36/19/2, QUESTIONS rows answered). Line numbers refer to the files as I read them; they may
drift by a few lines. Each finding is tagged with the evidence level. Severity: **B** blocker, **S** should-fix, **N** nit.

Mechanical checks that PASSED (no action): every `Q-` id referenced anywhere is defined in QUESTIONS.md (182/182); every
`ADR-NNNN` reference resolves to a file (57 ADRs); every spike's Q-ids exist; all repo paths cited under `apps/`, `packages/`,
`.github/`, `deployments/` exist; all 57 front-matter `status:` values match the README index rows (after round 24, 36
accepted · 19 proposed · 2 superseded); the 31 Open-EE feature specs and 5 research files exist on `woven/plan`.

---

## 1. Contradictions

| # | Sev | Finding | Where |
|---|---|---|---|
| 1.1 | **B** | **Sessions: three accepted ADRs disagree.** ADR-0082 says `SessionStore` MUST be a Durable Object on CF and that a KV-backed SessionStore "fails the suite". ADR-0052 S1 (accepted) says DB row authoritative + KV/Redis cache with TTL <= 60 s. ADR-0050 / Q-0050.4 spike bar says "KV/Redis sessions" for BetterAuth. Spike 0003 step 2 says "sessions in KV". P0 login depends on this. | adrs/storage/0082:35-57; adrs/auth/0052:37-39; adrs/auth/0050:78; spikes/0003:22 |
| 1.2 | **B** | **SPEC acceptance #1 is unattainable as phased.** Acceptance 1 requires all P0 features e2e on the self-hosted set. R-P3 / Q-F6 limit P0 self-hosted adapters to DB, queue, blob. But P0 needs a self-hosted session store (login), scheduler (outbox Cron sweeper ADR-0046, email digest F-D02, cleanups), and email (F-D02, password reset). Q-F6.2 ("what runs scheduled work there?") is still open. | SPEC.md:66-68, :124; QUESTIONS Q-F6, Q-F6.2; ADR-0046 outcome |
| 1.3 | **B** | **Access scope vs same-origin hosting.** ADR-0011 requires same-origin UI+API+collab (cookies/CSRF/WS). Round 20 says Access gates UI hosts only and the API authenticates directly. If UI and `/api` share a hostname (as ADR-0011/ARCH imply) Access gates the API too and machine clients need a path-based bypass; if they don't, cookies/CSRF are cross-origin. ARCH 1a draws Access in front of the whole edge. ADR-0090 round 5 says "Access is part of the P1 step for public product hosts". Spike 0004 tests only a hello Worker. | adrs/ui/0011:17-18; adrs/auth/0050:82; ARCHITECTURE.md:41-46; adrs/infra/0090:76 |
| 1.4 | S | **DO class ownership.** ADR-0090 Confirmation: only `collab` declares DO exports/migrations, to keep DO lifecycle off API deploys. ADR-0082 puts `SessionStore`, one-time codes and `RateLimiter` on DOs (and ADR-0033 allows a DO limiter). Those classes must live in some Worker; either `api` (violates 0090) or `collab` (couples auth to collab deploys). | adrs/infra/0090:51-52; adrs/storage/0082:41-43 |
| 1.5 | S | **Ordering.** ADR-0070 pins the common subset as "no ordering guarantee"; ADR-0046 guarantees ordering per (workspace_id, entity_id); Q-0070.2 (answered r20) requires per-work-item ordering. No mechanism is named (CF Queues has none; RabbitMQ needs consistent-hash/single-active-consumer). | adrs/jobs/0070:60; adrs/data/contracts/0046:53; QUESTIONS Q-0070.2 |
| 1.6 | S | **Guest role P0 vs P1.** SPEC/ADR-0051 fix P0 roles at Admin/Member/Guest; FEATURES F-E03 "Guests" is P1. | SPEC.md:89; FEATURES.md:98; adrs/auth/0051 r8/r10 |
| 1.7 | S | **"Decision pending" bodies in accepted ADRs.** The Decision Outcome text still reads "Decision pending... leading hypothesis X" in at least 19 accepted ADRs: 0005, 0006, 0020, 0021, 0022, 0032, 0033, 0051, 0052, 0053, 0084, 0085, 0092, 0093, 0100, 0101, 0110, 0130, 0140. In several the hypothesis is the *opposite* of the accepted outcome: 0005 (extend this repo vs new repo), 0006 (TypeScript/Hono vs Rust), 0021 (keep god-mode vs retire `apps/admin`), 0022 (defer/Access-gated vs unauthenticated P0), 0140 (clean-room process "required regardless" vs "no clean-room process"), 0085 (Option 3 separate services for sync vs accepted GitHub/Beads P1 with no option named). Recommend a one-line "Decision (accepted, round N)" paragraph at the top of each Outcome section, per MADR. | adrs/architecture/0005:30; 0006:31,:67; adrs/ux/0021:30; 0022:30; adrs/security/0140:35-45,:71; adrs/integrations/0085:42 |
| 1.8 | S | **ADR-0006 contradicts itself on status.** Front matter `accepted`; round-9 update says "No decision is recorded; the ADR stays `proposed`". ADR-0006 and ADR-0013 also differ from ADR-0001 on frontend size (~300k vs 175k+100k vs ~195k LOC). | adrs/architecture/0006:67; adrs/architecture/0001:17 vs adrs/ui/0013:42 |
| 1.9 | S | **ADR-0007 label drift.** Chosen "Option 1 compile-time"; Consequences say "Good (Option 3)... valid mixed deployments remain possible" and Drivers cite mixed deployments, which compile-time bundles do not give. | adrs/architecture/0007:30,:36 |
| 1.10 | S | **Two apps vs three.** ADR-0021 (accepted) retires `apps/admin` and says ADR-0011 now builds two apps; ADR-0011, ARCH 1a (`web · admin · space`), ARCH 2a, ADR-0090, Q-0011.1 and spike 0004 still assume `/god-mode`. | adrs/ui/0011:35; ARCHITECTURE.md:45,:142; adrs/ux/0021:61 |
| 1.11 | S | **ADR-0034 depends on an undecided choice** (code-first vs spec-first) and, until round 24, on proposed ADR-0032; also unclear surface count: ADR-0034 describes two driving HTTP adapters, but `/api/v1` must be Plane-shaped (wire-compatible) while the "core API" is deliberately not Django-shaped and "v1 builds on it". That is three surfaces (compat-plane, v1, core) or v1 is a thin shim over core; say which. | adrs/api/0034:32-36; SPEC.md:60 |
| 1.12 | S | **Retention.** Owner decision "30 days default" (r20) vs Plane's actual `HARD_DELETE_AFTER_DAYS` default **60** (`apps/api/plane/settings/common.py:404`). ADR-0043 and the PROMPT present it as "keep the cadence". It is a deliberate deviation; record it as one. Also F-D10 (retention cleanups, hard-delete) is P1 while P0 soft-deletes everything: nothing purges at P0. | adrs/data/schema/0043:87; FEATURES.md:78 |
| 1.13 | S | **ADR-0042 r20 update** says "soft-deleted rows included" while Q-0042.2 says nothing is imported (start fresh). Meaningless; delete. ADR-0040 spike S7 (import a `pg_dump`) and driver "one-time import" contradict Q-G3; spike 0002 silently dropped S4/S5/S6/S7 and changed S1 from 50 to 10k concurrent writers, so the two documents define different exit criteria. | adrs/data/base/0040:21,:86; adrs/data/schema/0042:90; spikes/0002:31-34 |
| 1.14 | S | **SPEC vs FEATURES vs ADR-0003.** F-X03 "tenant isolation (tenant key, quotas)" is P0, SPEC non-goal 3 excludes quotas, ADR-0003 excludes quotas. F-D12 (PostHog/OTel) P2 while ADR-0130 dropped PostHog. F-X09 realtime push P1 cites superseded ADR-0030. | FEATURES.md:80,:133; ARCHITECTURE; SPEC.md:33 |
| 1.15 | N | **Counts.** PROMPT/README counts changed during review (22 proposed -> 19). PROMPT line 87 still says "57 open rows"; my parse finds 60 rows with an `open` status (Q-F5.1 is mis-marked "open **Answered round 11**", so 59 real). SPEC §10 says "24 Open-EE features"; FEATURES section B has 28 rows (3 F-E00x + 25). ADR-0042 and Q-0042 say "122 migrations"; `apps/api/plane/db/migrations` holds 0001-0121 = 121. ADR-0073 says "34 modules"; `bgtasks/` has 34 files of which `__init__.py` and `apps.py` are not tasks = 32 task modules (all 32 are in the ADR-0073 table, good); ARCH says "~36", ADR-0070 "~40 tasks", actual `@shared_task` count is 49. | PROMPT.md:87; SPEC.md:141 |
| 1.16 | N | **Ordering/numbering.** SPEC G9 precedes G8; R-F7 precedes R-F6; R-P5 precedes R-P4; FEATURES F-X13 sits before F-X11. README index is fine. Stale "(new)"/"NEW (suggested)" tags remain on answered questions in ~40 ADR Open Questions sections (e.g. 0042 Q-0042.1/.2, 0043 Q-0043.1/.2, 0072, 0110, 0130). ADR-0021 updates are out of chronological order (r12 after r23). | passim |

## 2. Stale TypeScript-era text that still reads as current

The ADR groups below carry a round-21 "tooling follows Rust" note at the bottom but the body (Options, Outcome, code
sketches, Confirmation) is unmodified TypeScript. A reader of the Outcome will implement the wrong thing.

| Sev | File:line | Stale text |
|---|---|---|
| **B** | adrs/test/standards/0123:27-44 | Option 1 "Vitest everywhere", confidence **high**, `@cloudflare/vitest-plugin`; contradicted by its own r21 note (vitest only for React). |
| S | adrs/ci/0100 (accepted):36 | CF leg uses `@cloudflare/vitest-plugin`; a wasm32 crate cannot use it. Needs `wasm-bindgen-test`/workerd harness wording. |
| S | adrs/test/patterns/0121:16-19,29,35 | Vitest pool-workers helpers, `describePortContract`, `fast-check`, `packages/ports`; Rust equivalents (proptest, cargo-nextest) not named. |
| S | adrs/data/layer/0044:25-32 | Options are Drizzle/Kysely/Prisma; Rust candidates exist only in the r21 note. Outcome "Kysely" is misleading. |
| S | adrs/data/schema/0042:25,:32,:60-63; 0043:48-52 | Drizzle option and "Drizzle if ADR-0044 picks it"; TS-style `named scopes`. |
| S | adrs/data/interfaces/0045:25,:32 | `UnitOfWork.run(async (tx) => ...)`; in Rust, async closures borrowing `tx` across `.await` is a real design problem (lifetimes), not a syntax change. Also lists DO-SQLite adapter as a contract target though it is desk-review only. |
| S | adrs/api/0031:13; 0032:23-29,:50; 0030; 0010 | 0031 cites ADR-0030 (superseded) for "internal redesign"; 0032 options are Zod/zod-openapi; 0030/0010 are correctly marked superseded but are still linked as "Related" from 0001, 0005, 0006, SPEC §7, ARCH 1a, FEATURES F-X09. |
| S | adrs/architecture/0002:24,:33,:44; 0007:24; 0001:43; 0005:24,:40 | `packages/ports`, `worker.ts`/`server.ts`, `env.*`, `pg`, `amqplib`; ADR-0001 Confirmation says "unchanged apps/web ... through the adapted service layer" (vs minimal changes, no service layer, ADR-0034). ADR-0005 Option list/Confirmation are `packages/*` + turbo. |
| S | adrs/auth/0050:36,:52,:96 | Still says imported data maps to `Account` shape, "no local break-glass login" (invalid after A1), Open Questions list Q-0050.2/.5 as open. Accepted ADR with a wrong Consequences section. |
| S | adrs/auth/0052:40,:51,:61 | "On import, existing tokens are re-hashed", test "imported legacy key authenticates" (Q-G3: nothing imported); "scopes" metadata field while Q-0052.2 says no scopes. |
| S | adrs/auth/0051:36-42 | `interface Authorizer` in TS with `Promise<Decision>`. |
| S | adrs/collab/0060:43-54 | TS interfaces (`WebSocketLike`, `Uint8Array`). Also claims the editor "wire protocol stays unchanged" while Q-R2 lists the collab provider as an allowed change, and see 6.x re Hocuspocus protocol. |
| S | adrs/jobs/0070:41-49, 0071:43-49, 0072:38-45, 0080:38-47 | TS port sketches; 0072's `packages/core/schedules.ts`; 0071 "Postgres-backed runner inside the Node server" (Q-0071.1 answer, Q-0091 "Node server"). |
| S | adrs/jobs/0073:85 | Task table says `instance_traces` -> "C, opt-in" but r20 (ADR-0072) dropped it; `track_event` (PostHog) still mapped though PostHog dropped. |
| S | adrs/infra/0091:31,:37 | Option 3 "Node/Bun server". |
| S | adrs/security/0141:40 | "validate message schemas (zod/JSON-schema)". |
| S | adrs/infra/0092:70-74 | r15/16: "Cloudflare-side secrets provisioned by OpenTofu" is not Option 1 (wrangler secrets) and puts secret values into tofu state; reconcile. |
| S | adrs/ux/0020:40; 0120:45; 0122:44; 0140:49 | Confirmation sections require artefacts "in SPEC.md" (parity matrix, traceability matrix, exception register, per-feature provenance); SPEC.md contains none. |
| S | SPEC.md:25-27 | G8 still says "page editing runs on Durable Objects" (round-4 text; superseded by plain API save, F-C01). |
| S | SPEC.md:85 | §6.4 "Internal API: leaning Hono + typed RPC ... Runtime language stays open" - plainly contradicts G9/ADR-0006/0034. |
| S | SPEC.md:99,:105,:108; ARCHITECTURE.md:62 | Mapping table cites ADR-0030 for the API Worker; "router per ADR-0030". |
| S | SPEC.md:133,:139 | Risk rows cite "ADR-0010 compat option" and "BetterAuth ... fallback hand-rolled" as if BetterAuth were still the plan; G6 and FEATURES:6,F-A03 state "via BetterAuth" as a decision. It is a TypeScript library; spike 0003 may reject it. |
| S | ARCHITECTURE.md:104-105,:138-154,:149,:152 | Package layout still TS (`worker.ts`, `server.ts`, `zod`, `api-client replaces @plane/services`). The note says "read as Rust"; the PROMPT task list already asks to redraw it. `api-client` and `packages/contracts (zod)` conflict with ADR-0034 (no generated TS client). |
| S | spikes/0008:20-21 | Step 6: "how the TS client for the React app is generated from it" - ADR-0034 says no regenerated TS client. |
| S | QUESTIONS.md Q-G2 answer (line ~33), Q-0005 | Still say "shift to regenerated client" (superseded) / "new repo consuming @plane/* packages" (superseded by r15). |
| N | adrs/ci/0101:18; adrs/architecture/0005:13 | "npm packages (`packages/ports`)" - now crates; add cargo-deny/cargo-vet/`cargo audit` to the carried-over gates. |

## 3. Testability, ambiguity, tier dependencies, traceability

**Untestable / ambiguous requirements**
- R-F2 "gap-tolerant, monotonic, race-safe" has no number; ADR-0040 S1 says >= 50 creates/s, spike 0002 says 10k concurrent creates with no duplicate. Pick one. (SPEC.md:53)
- R-F3 "hidden row classes preserved": enumerate. Plane's `IssueManager` excludes **four** classes: triage state group, `archived_at`, **project archived**, `is_draft` (`apps/api/plane/db/models/issue.py:93-102`); the docs name three.
- R-F5 "MUST match Plane's published OpenAPI shapes": Plane's published docs differ from the CE instance (ADR-0122 itself says so) and the schema endpoint is **off by default** (`ENABLE_DRF_SPECTACULAR`, `settings/common.py:502`). Define "match" as "the pinned instance's schema", not "published".
- R-S4 "per-key rate limits": the Workers Rate Limiting binding only accepts `period` of **10 or 60** seconds and is per-location (verified); `API_KEY_RATE_LIMIT` in Plane is a free-form string (e.g. `60/minute`). State the supported subset.
- 6.4 "p95 API < 500 ms" has no measurement point (Worker, edge, client), no load profile, no per-endpoint scope; 99.5% availability has no window. ADR-0040 S3 uses p95 <= 300 ms. "Cost < $100/month" excludes the external Postgres/Hyperdrive provider unless said so.
- "Functionally equivalent UI" (ADR-0020) has no test: define as "React app unchanged except the four allowed areas passes a Playwright smoke set X".
- Dioxus gate: "oracle diff clean" - the oracle covers `/api/v1` only (see 4/6.x); there is no oracle for the UI or the compat endpoints. "SLO held 4 weeks" needs an owner and a source of truth (Gatus).
- F-D08 "database FTS" parity: see 4(f). F-B16 "bulk operations, grouped listing, pagination, filters" is one P0 row covering the heaviest endpoints; no acceptance list.
- Acceptance criteria exist only at SPEC §9 (5 bullets). No per-feature acceptance criteria in FEATURES.md.

**P0 scope that depends on P1/P2 or on an undecided ADR**
- P0 pages/descriptions "plain API save persisting binary+html+json" needs a server that can produce/validate all three forms. The React editor in non-collab mode does not send the binary (only pages with `use-page-fallback.ts` do), and API v1 writes `description_html` only. That makes **spike 0006 a P0 blocker** (it says tier P0) while the spike README orders it after 0002/0003 and calls it "P0/P1". Also ADR-0012 (editor) is `proposed`.
- P0 F-B08/F-D01/F-D03 (activity, notifications, webhooks) depend on ADR-0046 outbox, ADR-0070 queue, ADR-0073 (proposed), and on the SSRF guard (R-S3), which is first specified for the P1 link crawler (F-D13). Make the guard a P0 deliverable explicitly.
- P0 F-D02 email digest + F-A03 password reset need the `Email` port; the CF adapter (Email Service) is unverified from Rust (spike 0001). P0 self-hosted twin has no Email/Scheduler adapter (1.2).
- P0 F-A07: membership without invites (invites P1) - how does the 2nd user get in? Needs an admin-creates-user path or the invite email at P0.
- P0 F-B10 (cycles, "DB, Cron") and F-B14 intake need Scheduler on self-hosted.
- P0 permission seam + RBAC, outbox, tenancy (F-X03), search, uploads, topology all rest on ADRs that are `proposed` until spikes 0001/0002/0004 report. That is by design, but FEATURES shows no gate "ADR X accepted before row Y starts".
- F-X13 compat-plane (P0) has no inventory (Q-0010.1 open) and no size estimate (see risks).

**Feature rows with no ADR coverage (or only a proposed one)**
F-A09 profile/preferences/onboarding, F-B15 drafts/stickies, F-B18 favorites/home/recent-visits, F-B12 views (public/private), F-B13 estimates, F-B19 teams, F-B09 versions (ADR-0061 covers pages only, not `IssueVersion`/`IssueDescriptionVersion`), F-D07 analytics (no ADR; spike none), F-D06 importers (ADR-0085 leaves Q-0085.2 open and no importer source exists in CE), F-D10 retention (only 0043 update), F-D13 link crawler, F-E00a feature flags (no ADR for the flags port at all, yet it is a P0 foundation), F-E06 audit logging (no ADR: only consumer of 0046), F-E07/E08 custom fields and work-item types (schema-affecting P1, ADR-0042 frozen-schema decision conflicts), F-E18 recurring items, F-E19, F-E22, F-C05 PDF export (lives in `apps/live` using `@react-pdf/renderer`: a TS/React dependency with no Rust home; unaddressed).
**ADRs with no feature/requirement tracing:** 0007 and 0005 trace to F-X01/X02 only loosely; 0020, 0123, 0141 have no feature row; 0034 has F-X13 only. No document maps requirements to ADRs bidirectionally; ADR-0120's promised traceability matrix does not exist.

## 4. Verification of [S] claims against the source

| Claim | Verdict | Evidence |
|---|---|---|
| (a) `Issue.save` advisory-lock `sequence_id` | **Verified, with two omissions.** `pg_advisory_xact_lock(convert_uuid_to_integer(project.id))` in a `transaction.atomic()`, then `Max(IssueSequence.sequence)+1`, then inserts an `IssueSequence` row (FK `SET_NULL`, so numbers are never reused after delete). ADR-0043 B2 says `MAX(sequence_id)+1`; it is `MAX` over **`IssueSequence`**, not `Issue` - that matters because B1's counter row is only equivalent if deleted/moved issues never give numbers back. **Omitted:** the same locked block also allocates `sort_order` (`largest+10000` per state) and `description_stripped`; ADR-0043 and R-F2 do not cover `sort_order`. Bulk paths bypass `save()` as stated. | `apps/api/plane/db/models/issue.py:179-235,:549-560` |
| (b) soft delete + partial unique indexes | **Verified.** `deleted_at` on `SoftDeleteModel`; per-model `UniqueConstraint(..., condition=Q(deleted_at__isnull=True))` plus a redundant `unique_together` including `deleted_at`. Two-tier managers confirmed (`IssueManager`: triage, archived, **project archived**, draft). `hard_delete` covers an explicit list of ~18 models, not "all tables". Default `HARD_DELETE_AFTER_DAYS` is 60 not 30. | `db/models/{issue,state,project}.py`, `db/mixins.py:51-75`, `bgtasks/deletion_task.py:114-175`, `settings/common.py:404` |
| (c) beat schedule (ADR-0072) | **Verified exactly**: 12 entries (5-min email, 6-h `instance_traces`, 00:00 hard_delete, 01:00 archive/close, 01:30 and 03:45 `delete_old_s3_link`, 02:00 file assets, 02:30 api logs, 02:45 email logs, 03:00 page versions, 03:15 description versions, 03:30 webhook logs). `DatabaseScheduler` confirmed. | `apps/api/plane/celery.py:28-83,:107` |
| (c) bgtasks inventory (ADR-0073) | **Mostly verified, one gap, one miscount.** All 32 task modules appear in the table. **Missing: `webhook_send_task`** (the `@shared_task(bind=True, autoretry_for=..., retry_backoff=600, max_retries=5)` sender in `webhook_task.py:254`); this is the task that owns the "5 retries" the owner chose, so the P0 webhook row has no mapped task (Q-0073.5 open). Module count "34" includes `__init__.py`/`apps.py`. 49 `@shared_task`s overall. | `bgtasks/*.py`, `license/bgtasks/tracer.py` |
| (d) `apps/live` persistence flow | **Verified, with scope corrections.** Hocuspocus `Database` extension: `fetch` gets the binary from the API; if empty it converts `description_html` -> Yjs binary with `@plane/editor` and writes all three forms back; `store` converts binary -> {binary(base64), html, json} with `@plane/editor` and PUTs via the page service. **But:** (1) `getPageService` only supports `project_page`; CE `apps/live` does **not** serve work-item descriptions, so ADR-0060 / Q-F5 "descriptions use the same DO path" is new scope, not parity; (2) there are extra duties not in ADR-0060/0061: `title-sync` extension (page titles over stateless messages), `force-close` admin command over Redis, PDF export controller, 413 "content too large" force-close path; (3) the server-side conversion depends on the **JS** editor schema, confirming spike 0006's importance; (4) the client uses `HocuspocusProvider` (`packages/editor/src/core/hooks/use-yjs-setup.ts:7,62`), i.e. the **Hocuspocus** framing (document name, auth message, stateless), not bare y-protocol as spike 0005 step 1 assumes. | `apps/live/src/extensions/database.ts`, `services/page/handler.ts` |
| (e) roles 20/15/5 + decorator | **Verified.** `ROLE` enum ADMIN=20, MEMBER=15, GUEST=5; `allow_permission(allowed_roles, level, creator, model)`; workspace-admin-who-is-a-project-member fallback; 38 view files use `@allow_permission` (40 mention it). ADR-0051 says roles are "duplicated in >= 4 files": there is a second `allow_permission` in `utils/permissions/base.py`. Citation `issue/base.py:1039-1050` is the *guest queryset filter*, not the `creator=True, model=Issue` branch it describes. | `app/permissions/base.py:13-100`, `utils/permissions/base.py` |
| Other [S] | **Search is `icontains`, not FTS.** `app/views/search` builds `Q(field__icontains=query)` per entity. ADR-0083/F-D08 choose tsvector/FTS5 "for parity": FTS is word-token based and will change results (no substring/prefix matching) unless trigram (`pg_trgm`, FTS5 `trigram` tokenizer) is used. | `app/views/search/base.py:46-139` |
| Other [S] | **drf-spectacular covers `/api/v1/` only** (`SCHEMA_PATH_PREFIX`, `preprocess_filter_api_v1_paths`) and is disabled unless `ENABLE_DRF_SPECTACULAR=1`. The internal `/api/` (**233** `path()` entries in `plane.app`), `/api/public/` and `/auth/` (66 entries incl. space) have **no schema**. | `settings/openapi.py:35-41`; `urls.py:18-23` |
| Other [S] | **This fork is not CE-pure for auth.** `authentication/provider/oauth/oidc.py` and `views/app/oidc.py` exist (woven addition), contra ADR-0050 "no OIDC in CE"; after "update to latest upstream, pin" (bead plane-b6a) the oracle may or may not keep it. Spike 0008 should state this explicitly. | `apps/api/plane/authentication/` |
| Other [S] | Migrations: 121, not 122 (1.15). v1 surface: ~70 `path()` entries. | `db/migrations` |

## 5. Cloudflare / Rust platform claims

Verified against Cloudflare docs (2026-10):
- Worker size limit **64 MiB uncompressed** (all plans; compressed limit removed 2026-09-04) - VERIFIED. 1 s startup - VERIFIED.
- Cron Triggers **250 per account** (Paid) - VERIFIED (ADR-0072 correct). Cron CPU 30 s (<1 h interval)/15 min - VERIFIED.
- Workers Rate Limiting binding periods must be 10 or 60 s, per-location - VERIFIED (not stated in ADR-0033; add).
- **Queues message size: 128 KB**; batch <= 100 msgs / 256 KB; delay 0-86400 s; retention 60 s-14 d (default 4 d) - VERIFIED. This **closes Q-0070.1**. RabbitMQ has no native per-message delay (needs the delayed-message plugin or TTL+DLX) - the "delay <= smaller of two adapters" rule needs a number (24 h on CF).
- Workflows: step result 1 MiB, event payload 1 MiB, sleep <= 365 d, 10k steps default / 25k max, 300 creations/s per account (100/s per workflow), 50k concurrent running, retention 30 d Paid, <=100 cron schedules - VERIFIED (ADR-0071 correct).
- DO WebSocket message limit 32 MiB; `serializeAttachment` max **16,384 bytes** - VERIFIED; 32,768 hibernating sockets per DO (not stated in ADR-0060, useful for the 50-editor target).
- Vectorize 20 M vectors / 1,536 dims - VERIFIED (2026-08-04).
- `McpAgent` deprecated, `createMcpHandler` stateless - VERIFIED.

UNVERIFIED (need a spike or a docs read; do not rely on them yet):
1. workers-rs bindings status: Queues "beta", D1 "alpha", `panic=abort`, no Tokio, DO WebSocket hibernation, **Workflows binding and Email Service from Rust** - research/0002 marks them [doc]/[unverified]; spike 0001 is the gate. Docs search did not return a Rust-specific page.
2. Hyperdrive + `tokio-postgres` on wasm32 (Hyperdrive docs list rust-postgres 0.19.8); `sqlx`/`diesel`/SeaORM on wasm32 (likely not buildable; spike 0002 step 6). Also Hyperdrive connection-count and per-request subrequest limits at 20 queries per list view.
3. D1 FTS5 (and the `trigram` tokenizer), D1 "no interactive transactions", 10 GB cap, read replication GA/beta status.
4. R2: presigned PUT only on the S3 domain; per-key concurrent-write 429 limit (ADR-0080/0061); R2 binding in Rust and SigV4 presign in wasm.
5. CF Email Service availability/pricing/GA status (ADR-0073 r7 makes it a P0 CF adapter).
6. Service-binding rules: "target must be deployed first", version-skew/affinity header `Cloudflare-Workers-Version-Key`, "last 100 versions" (ADR-0090/0110).
7. `yrs` hibernating-DO viability: in-memory Yjs state is lost on hibernation; rehydration cost for 1-5 MB docs (spike 0005).
8. `tracing` + OTLP on wasm32 (ADR-0130 r21); Workers Observability OTLP export format and sampling semantics.
9. Secrets Store beta status, `secrets.required` wrangler field; `cloudflare_worker*` Terraform resources (ADR-0092/0093).
10. `better-auth-rs` on wasm32 and its security posture (alpha); argon2 CPU time on Workers (30 s default CPU is fine, but measure p95 login).
11. Zitadel outbound SCIM (Q-0053.1) and Access JWT behaviour with WebSocket upgrades through a Worker custom domain.

## 6. Dangling / unresolved references (beyond 1.x)

All Q-ids, ADR numbers and in-repo file paths resolve. Remaining issues:
- **Documents promised but absent:** parity matrix, traceability matrix, deviation/exception register, per-feature provenance note "in SPEC/FEATURES" (ADR-0020:40, 0120:45, 0122:44, 0140:49). `docs/plan/` and the `thrust` repo are external and unverifiable from here.
- `research/backend-data-model.md`, `features/*.md` and `PROGRAM.md` live only on branch `woven/plan` (exists; paths verified) - fine, but `git show woven/plan:...` links break when that branch is pruned. Copy the few load-bearing sections into `research/`.
- Spike dependency graph is inconsistent: README order "0001 -> (0002 || 0003 || 0004 || 0008) -> 0006" but spike 0003 says depends on 0002; spike 0004 depends on 0001; 0006 is tagged **P0** yet scheduled after the parallel group and gates P0 descriptions. Spike 0008 depends on bead `plane-b6a` (not defined in the package; confirm it exists).
- Beads `plane-b6a`, `plane-9y3` are cited as decided inputs; neither is traceable from the package.
- Q-0040.1 etc. "partly answered" rows and `Q-F5.2` ("open") should be closed or given an owner/date per the PROMPT's own definition of done.
- PROMPT.md:7 says "ARCHITECTURE §2a still shows TypeScript names... Redraw it" (a known TODO), PROMPT:87 stale counts.

## 7. Five biggest risks not already in SPEC §10

1. **The compat-plane surface is large, unscoped and has no oracle schema.** `plane.app` exposes ~233 route entries (+66 public/auth, +70 v1); drf-spectacular covers only v1, so the compat inventory (Q-0010.1, still open) must be recovered from React store code and recorded traffic, including grouped/sub-grouped listings, Django CSRF/session cookie flows and the server-redirect `/auth/*` form posts. It is a P0 deliverable (F-X13) on the critical path with no size estimate. SPEC risk 1 is framed as "redesign breaks assumptions", which ADR-0034 already removed; the real risk is *fidelity of a 300-endpoint Django-shaped facade in Rust*. Mitigation: time-boxed inventory spike before P0 planning; rank endpoints by React call frequency; ship the minimum slice first.
2. **Rich-text on the server is a P0 problem, not P1.** Plane never produced the Yjs/html/json triple outside the JS editor. P0 plain-save + API v1 `description_html` writes require Rust (or a JS shim) conversions, sanitisation (XSS surface on stored HTML rendered by React) and Hocuspocus-compatible docs for later collab. If spike 0006 chooses the JS shim, ADR-0006's "Rust only" claim and the cost envelope change; if it chooses Rust, fidelity >= 99% (spike exit) still means data-corrupting 1% on user content. Mitigation: store client-produced forms verbatim at P0 (option B), version the schema, and never rewrite stored binary server-side until collab ships.
3. **Single-region, single-writer data and cost reality.** Hyperdrive->external Postgres (or D1's single writer) behind many-query list views, plus DO-backed sessions/rate limits, plus per-PR dev schemas on a shared dev DB, against "<$100/month including Workers Paid" and p95 < 500 ms. The cost cap likely excludes the Postgres provider; PR-preview schema-per-PR on a shared DB has no cleanup/ownership story (and no migration-lock story with CI-run expand migrations). Mitigation: put the DB provider and previews inside the cost model now; decide DB-per-PR vs prefix before spike 0002.
4. **Oracle fidelity and drift.** The oracle is "this fork, updated to latest upstream, pinned", but the fork carries non-upstream code (woven OIDC), the schema endpoint is off by default and v1-only, the differential run is nightly only, and seed data is synthetic (so real `projects.woven` shapes, long descriptions, legacy states are never exercised). Upstream Plane keeps shipping security fixes the frozen frontend and oracle will not get (accepted in SPEC, but not costed: someone must triage upstream CVEs for the adopted React/`@plane/*` packages indefinitely). Mitigation: owner for upstream-security triage; record the pin and the fork delta in spike 0008; add a small anonymised real-shape sample for the golden corpus.
5. **Security regression surface from reimplementation in Rust/wasm.** Django gave CSRF, session handling, auth rate limiting, password hashing, HTML escaping, SSRF-safe fetch (`pinned_fetch`) and permission checks "for free"; all are rebuilt, with an alpha auth crate as one candidate, DO-backed sessions, a new SSRF guard, and AGPL source-offer obligations (§13) on a public deployment. ADR-0141 (threat model) was `proposed` and is a prerequisite for P0 public spaces, yet P0 ships an unauthenticated surface. Mitigation: promote ADR-0141 before any public host exists; add an external review gate for the identity adapter and SSRF guard; fuzz/XSS corpora in CI are already in spike 0006 - extend to auth and webhooks.

(Honourable mentions: no capacity/timeline anywhere - one owner, 28 Open-EE rows, 49 background tasks, ~300 endpoints, two UIs; the Phase-2 Dioxus gate depends on an oracle that does not cover the UI.)

---

## Prioritized fix list

**Blockers (resolve before the next interview round / before PLAN.md)**
1. Resolve the session-store contradiction (1.1): pick DB+cache vs DO and conform ADR-0082, ADR-0052, ADR-0050 spike bar, spike 0003.
2. Re-state SPEC acceptance #1 / R-P3 for what the P0 self-hosted set actually contains, or add SessionStore/Scheduler/Email self-hosted adapters to P0 (1.2); close Q-F6.2.
3. Decide host layout vs Access (same-origin UI+API vs Access-gated UI only) and write it into ADR-0011/0050/0090 and spike 0004 (1.3).
4. Rewrite ADR-0123 Outcome (vitest, "high" confidence) to the Rust toolchain; same for ADR-0100's `@cloudflare/vitest-plugin` leg (2).
5. Promote spike 0006 to a hard P0 dependency in the spike order and PLAN; decide Option B (client-produced forms) as the P0 default (3, risk 2).

**Should-fix (this session)**
6. Add a "Decision (accepted, round N)" paragraph at the top of the Outcome of the 19 accepted ADRs whose body still says "Decision pending" (1.7); fix ADR-0006 status text, ADR-0007 option labels (1.8, 1.9).
7. Add `webhook_send_task` to ADR-0073's table; remove `instance_traces`/`track_event` rows to match r20/r18; fix module counts (4, 1.15).
8. Correct ADR-0043: counter row must replicate `MAX(IssueSequence.sequence)` semantics (no reuse after delete/move) and add `sort_order` allocation; record 30-day vs Plane's 60-day retention as a deviation (4(a),1.12).
9. ADR-0083/F-D08: decide substring (`trigram`/ILIKE) vs FTS parity given Plane's `icontains` search (4).
10. Close Q-0070.1 with 128 KB; add per-message delay and ordering mechanism to ADR-0070/0046; add the 10/60 s period constraint to ADR-0033 (5, 1.5).
11. Reconcile DO-class placement (1.4) and the 2-vs-3 apps drift across ADR-0011/ARCH/spike 0004 (1.10).
12. Redraw ARCHITECTURE §2a/§2 and replace the TS interface sketches with Rust trait signatures in ADRs 0045/0051/0060/0070/0071/0072/0080; update SPEC G8, §6.4, §7, §10 (2).
13. Remove import-era text (ADR-0040 S7/driver, 0042, 0052 tests, 0050 Account mapping) and align spike 0002 exit criteria with ADR-0040 (1.13).
14. Add the missing artefacts the ADRs promise (parity matrix, traceability matrix R-/F- -> ADR -> test, exception register) or delete the promises (3, 6).
15. Fix spike graph (0003 depends on 0002; 0006 order) and spike 0008 step 6 (no generated TS client) (6, 2).
16. Clarify the number of HTTP surfaces in ADR-0034 and add an explicit compat-inventory spike with a size estimate (1.11, risk 1).
17. Give every FEATURES row an ADR or an explicit "no ADR needed"; add ADRs for feature flags and audit logging (P0 foundations with none) (3).

**Nits**
18. Renumber/reorder SPEC G8/G9, R-F6/R-F7, R-P4/R-P5, FEATURES F-X11-13; strip stale "(new)"/"NEW (suggested)" tags; fix Q-F5.1 status cell; update PROMPT counts (57 open, 33/22/2) (1.15, 1.16).
19. Fix the ADR-0051 citation (`issue/base.py:1039-1050` is guest filtering) and the "roles duplicated in >= 4 files" claim (4(e)).
20. Note the woven OIDC addition in the oracle description (4).

## 8. Round 26: status of the section 5 unverified platform claims

Numbers refer to the list under section 5. `[tested]` means local workerd or local Postgres, not Cloudflare's network.

| # | Claim | Status after the wave-1 spikes |
|---|---|---|
| 1 | workers-rs bindings | **Mostly resolved.** KV, R2, Queues (producer and consumer), DO with SQLite storage and WebSocket hibernation, Hyperdrive and Cron are native in workers-rs 0.8.7 and were exercised `[tested]`; Workflows has no Rust API and needs a JS shim (`[tested]`); Email is native with a local simulation only; AI Gateway is REST; `panic=abort` has a defined policy; core is Tokio-free (spike 0001). Queue retry and DLQ, real Hyperdrive, and D1 are untested |
| 2 | Hyperdrive + `tokio-postgres`; sqlx/diesel/SeaORM on wasm32 | **Partly resolved.** `tokio-postgres` runs over a Hyperdrive socket (local emulation `[tested]`); `sqlx` does not build for wasm32 `[tested]`; SeaORM and diesel-async are untried; real Hyperdrive limits untested |
| 3 | D1 FTS5 and trigram | Open (spike 0002) |
| 4 | R2 presigned PUT, SigV4 in wasm | R2 put, get and delete are native `[tested]`; presign and SigV4 not exercised |
| 5 | Email Service availability and pricing | Pricing was read from the vendor page by spike 0001 (3,000 per month included, then $0.35 per 1,000); availability and real delivery unverified |
| 6 | Service-binding rules | The binding works, including WebSocket across it on the real edge (spike 0004 run 1); version skew and affinity untested |
| 7 | `yrs` on hibernating DOs | Open (spike 0005). Locally a DO was re-instantiated after 14 s idle while its socket, attachment and SQLite rows survived, so state must be rehydrated from SQLite |
| 8 | `tracing` + OTLP on wasm32 | Not exercised |
| 9 | Secrets Store | Native binding, tested against the local store only |
| 10 | `better-auth-rs` on wasm32 | Open (spike 0003) |
| 11 | Zitadel SCIM; Access with WebSocket upgrades | SCIM open (Q-0053.1). Access: Worker-level Access answers upgrades with 403 `[doc]`; WebSocket through a hostname-based gated application works (spike 0004 run 2, service token, text, binary and 1 MiB) |

New findings the review did not list: `apps/space` is server-rendered (Q-0011.2); the pinned oracle's own schema fails 67 conformance checks in 12 classes (ADR-0122); `ADR-0045`'s premise that wasm32 futures force `?Send` was wrong (ADR-0002).
