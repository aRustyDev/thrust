# TRACEABILITY — features and requirements to ADRs, spikes and tests

Generated round 25 from FEATURES.md plus a reviewed mapping. **GAP** = no accepted ADR yet (an ADR or an explicit "no ADR
needed" must be recorded before the row starts). "Test home" is where the acceptance evidence lives (ADR-0121/0122/0123).

## Features

| Feature | Name | Tier | ADRs | Spikes | Test home |
|---|---|---|---|---|---|
| F-A01 | OIDC SSO (Zitadel) login; JIT user provisioning | P1 | 0050, 0053 | 0003 | e2e vs Zitadel (mock) |
| F-A02 | Cloudflare Access JWT validation (outer gate) | P1 | 0050, 0090 | 0004 | integration (Access JWT fixtures) |
| F-A03 | Email + password sign-in/up, password reset | P0 | 0050, 0052, 0073 | 0003 | contract + e2e |
| F-A04 | Magic-code (email OTP) sign-in | P1 | 0050, 0073 | 0003 | e2e |
| F-A05 | OAuth: Google / GitHub / GitLab / Gitea | P2 | — GAP: ADR when scheduled (P2) |  |  |
| F-A06 | API keys (`X-Api-Key`, `APIToken`), per-key rate limit, activity log | P0 | 0033, 0052 | 0003 | contract (X-Api-Key) + oracle |
| F-A07 | Workspace membership, invites (`WorkspaceMemberInvite`), roles | P0 (invites P1) | 0051 — GAP: P0 onboarding without invites (Q-F7) |  | e2e |
| F-A08 | Project membership, roles (5/15/20), project invites | P0 | 0051 |  | permission-matrix parity (ADR-0122) |
| F-A09 | Profile, user preferences, themes, onboarding | P0 | no ADR needed (CRUD over core use-cases; ADR-0045 pattern) |  | e2e |
| F-A10 | Instance admin (`god-mode`): instance config, auth-provider toggles, e | P0 (settings subset; apps/admin retired) | — UNMAPPED |  |  |
| F-B01 | Workspaces (CRUD, slug, settings) | P0 | no ADR needed (CRUD over core use-cases; ADR-0045 pattern) |  | e2e + oracle |
| F-B02 | Projects (CRUD, identifier, archive, network, favorites) | P0 | no ADR needed (CRUD over core use-cases; ADR-0045 pattern) |  | e2e + oracle |
| F-B03 | Work items (CRUD, `sequence_id` per project, priority, dates, parent/s | P0 | 0043, 0040, 0046 | 0002 | R-F2 concurrency test + oracle |
| F-B04 | States (groups backlog/unstarted/started/completed/cancelled) | P0 | no ADR needed (CRUD over core use-cases; ADR-0045 pattern) |  | oracle |
| F-B05 | Labels, assignees, subscribers | P0 | no ADR needed (CRUD over core use-cases; ADR-0045 pattern) |  | oracle |
| F-B06 | Relations (blocks/duplicate/relates…), links, attachments | P0 | 0047 |  | oracle + contract (file-asset) |
| F-B07 | Comments, reactions, mentions, votes | P0 | no ADR needed (CRUD over core use-cases; ADR-0045 pattern) |  | oracle |
| F-B08 | Activity history / audit trail per work item | P0 | 0046 |  | e2e + outbox contract |
| F-B09 | Versions of work item & description | P0 | 0061 — GAP: amend for IssueVersion/IssueDescriptionVersion |  | oracle |
| F-B10 | Cycles (sprints), cycle progress, transfer issues, archive | P0 | 0072, 0043 |  | oracle + cron test |
| F-B11 | Modules (epics), links, members | P0 | no ADR needed (CRUD over core use-cases; ADR-0045 pattern) |  | oracle |
| F-B12 | Views (saved filters, display properties, public/private) | P0 | 0051 |  | oracle |
| F-B13 | Estimates (points/time/categories) | P0 | no ADR needed (CRUD over core use-cases; ADR-0045 pattern) |  | oracle |
| F-B14 | Intake (triage inbox, accept/decline/snooze/duplicate) | P0 | 0073 |  | oracle |
| F-B15 | Draft work items; stickies | P0 | no ADR needed (CRUD over core use-cases; ADR-0045 pattern) |  | oracle |
| F-B16 | Bulk operations, grouped/sub-grouped listing, pagination, filters (`gr | P0 | 0034, 0044, 0040 | 0002, 0009 | oracle recordings + p95 test |
| F-B17 | Auto-archive/auto-close old work items (daily) | P1 | 0072 |  | cron test |
| F-B18 | Favorites, recent visits, home | P0 | no ADR needed (CRUD over core use-cases; ADR-0045 pattern) |  | e2e |
| F-B19 | Teams | P2 | — GAP: ADR when scheduled (P2) |  |  |
| F-C01 | Pages (project pages): CRUD, lock, archive, access, favorites, labels | P0 (plain API save; DO arrives with coll | 0012, 0061 | 0006 | e2e + sanitiser XSS corpus |
| F-C02 | Real-time collaborative editing (Yjs), presence | P1 | 0060, 0061 | 0005 | load test |
| F-C03 | Page versions / history | P1 | 0061 | 0005 | contract |
| F-C04 | Page ↔ work-item embeds / mentions | P1 | 0012, 0061 |  | e2e |
| F-C05 | Export page to PDF | P2 | — GAP: PDF export lives in apps/live (@react-pdf/renderer); non-Rust exception candidate (ADR-0006) when scheduled (P2) |  |  |
| F-C06 | Workspace pages (wiki) — stub in CE | drop | dropped |  |  |
| F-D01 | Notifications (in-app), preferences, mark read/snooze/archive | P0 | 0046, 0073 |  | outbox contract + e2e |
| F-D02 | Email notifications (5-min digest) | P0 | 0073, 0072 |  | integration (SMTP/CF Email) |
| F-D03 | Webhooks (workspace/project), delivery log, retry | P0 | 0073, 0141, 0046 |  | delivery contract + SSRF tests |
| F-D04 | File assets: upload/presign, avatars, covers, inline images | P0 | 0047, 0080, 0081 |  | contract (file-asset) |
| F-D05 | Export (CSV/JSON/XLSX) | P1 | 0081, 0071 |  | integration |
| F-D06 | Importers (Jira/Linear/CSV/Asana…) | P1 | 0085 |  | integration |
| F-D07 | Analytics (workspace/project dashboards, saved `AnalyticView`) | P1 | — GAP: ADR when P1 is scheduled |  |  |
| F-D08 | Global search (P0 adapter: database FTS, ADR-0083) | P0 | 0083 | 0002 | search parity corpus (substring + FTS) |
| F-D09 | Published/public boards (`spaces`) | P0 | 0022, 0141 |  | e2e + unauthenticated-boundary tests |
| F-D10 | Data retention cleanups (api logs, email logs, versions, webhook logs; | P0 (hard-delete at 30 d; deviation D-001 | 0043, 0072 |  | cron test + retention test |
| F-D11 | i18n (locale files in `@plane/i18n`) | P0 (inherited) | no ADR needed (CRUD over core use-cases; ADR-0045 pattern); reuse @plane/i18n (ADR-0012) |  | e2e |
| F-D12 | Telemetry (OTel only; PostHog dropped) — likely opt-in | P2 (opt-in) | 0130 |  | integration |
| F-D13 | Link crawler/unfurl (SSRF-guarded) | P1 | 0141, 0073 |  | SSRF tests |
| F-E00a | Feature flags | P0 | 0008 (proposed) |  | contract |
| F-E00b | Permission seam | P0 | 0051 |  | permission-matrix parity + scope property tests |
| F-E00c | Event/outbox backbone | P0 | 0046 |  | outbox contract |
| F-E01 | RBAC (custom roles) | P1 | 0051 |  | contract |
| F-E02 | ReBAC fine-grained | P2 | 0051 |  |  |
| F-E03 | Guests | P1 | 0051 |  |  |
| F-E04 | SCIM 2.0 | P2 | 0053 |  |  |
| F-E05 | OIDC/SAML SSO | P1 | 0050 | 0003 |  |
| F-E06 | Audit logging | P0 | 0048 (proposed) |  | audit registry test |
| F-E07 | Custom fields | P1 | — GAP: ADR at P1 start; conflicts with schema frozen at the pinned version (ADR-0042) |  |  |
| F-E08 | Work item types | P1 | — GAP: ADR at P1 start; same schema conflict (ADR-0042) |  |  |
| F-E09 | Multiple workflows | P2 | — GAP (P2) |  |  |
| F-E10 | Workflow state control | P2 | — GAP (P2) |  |  |
| F-E11 | Workflow approvals | P2 | — GAP (P2) |  |  |
| F-E12 | Dashboards | P2 | — GAP (P2) |  |  |
| F-E13 | Advanced dashboard widgets | P2 | — GAP (P2) |  |  |
| F-E14 | Initiatives | P2 | — GAP (P2) |  |  |
| F-E15 | Team spaces | P2 | — GAP (P2) |  |  |
| F-E16 | Project templates | P2 | — GAP (P2) |  |  |
| F-E17 | Work-item / page templates | P2 | — GAP (P2) |  |  |
| F-E18 | Recurring work items | P1 | 0072 |  | cron test |
| F-E19 | Nested pages & embeds | P1 | 0061, 0012 |  |  |
| F-E20 | Workspace wiki (AFFiNE-style) | drop | dropped |  |  |
| F-E21 | Time tracking (WakaTime) | P2 | — GAP (P2) |  |  |
| F-E22 | Intake email & forms | P1 | 0073, 0085 |  |  |
| F-E23 | Customers / CRM | drop | dropped |  |  |
| F-E24 | Slack integration | P2 | 0085 |  |  |
| F-E25 | External sync (GitHub/Beads) | P1 | 0085 |  | integration |
| F-X01 | Port/adapter runtime: Cloudflare adapter set | P0 | 0002, 0007 | 0001 | contract suite on CF set |
| F-X02 | Port/adapter runtime: self-hosted adapter set | P0 (DB, queue, blob, session, scheduler, | 0007, 0091 | 0001 | contract suite on self-hosted set |
| F-X03 | Multi-tenant isolation (tenant key; quotas out of scope) | P0 | 0003, 0041 | 0002 | tenant-predicate property tests (R-S1) |
| F-X04 | One-time import of existing Plane Postgres data | **drop** (round 5: start fresh) | dropped |  |  |
| F-X05 | Contract-test harness (every port × every adapter) | P0 | 0121 | 0001 | self |
| F-X06 | v1 oracle / parity test harness | P0 | 0122, 0120 | 0008 | self |
| F-X07 | Observability (logs, traces, metrics) across Queue/Workflow/DO hops | P0 | 0130 | 0001 | integration |
| F-X08 | AI: P0 API-based access (public API) · P1 MCP server · P2 AI assistant | P0 / P1 / P2 | 0084 |  | e2e |
| F-X09 | Real-time push of work-item updates (beyond SWR polling) | P1 | 0034, 0060 |  |  |
| F-X10 | Rust (Dioxus) clone of the Plane UI — web target | P2 (starts when P0+P1 shipped, SLO 4 wks | 0013 | 0007 |  |
| F-X11 | Dioxus desktop target | P2 (after v1 web; Q-0013.3) | 0013 | 0007 |  |
| F-X12 | Dioxus mobile target | P2 (after v1 web; Q-0013.3) | 0013 | 0007 |  |
| F-X13 | `compat-plane` internal API adapter for the React app (deleted when Di | P0 | 0034 | 0009 | recorded-journey replay |
| F-X14 | SSRF-safe outbound fetch guard (webhooks, link unfurl; R-S3) | P0 | 0141, 0073 |  | SSRF test corpus |

## Requirements

| Requirement | ADRs | Verification |
|---|---|---|
| R-F1 every FEATURES row at its tier | all | this table |
| R-F2 sequence_id/sort_order allocation | 0043, 0040 | spike 0002 concurrency test (10k creates, ≥50 writers) |
| R-F3 soft-delete + four hidden classes | 0043 | oracle recordings of list/count behaviour |
| R-F4 page forms persisted consistently | 0012, 0061 | contract + XSS corpus (spike 0006) |
| R-F5 /api/v1 matches pinned schema | 0031, 0032, 0122 | schema diff + response diff (spike 0008) |
| R-F6 bgtasks mapped | 0073, 0072, 0071 | ADR-0073 table review; per-task contract tests |
| R-F7 compat-plane + core API | 0034 | recorded-journey replay (spike 0009) |
| R-P1 no platform SDK outside adapters | 0002 | lint/`cargo deny` + CI |
| R-P2/R-P5 adapter selection compile-time | 0007 | build matrix (ADR-0100) |
| R-P3 P0 self-hosted set | 0007, 0091 | contract matrix with `pending-adapter` markers |
| R-P4 CF-only code isolated | 0002, 0090 | crate dependency check |
| R-S1 tenant predicate | 0041, 0051 | property tests (user never sees rows outside scope) |
| R-S2 secrets never in code/config | 0092 | secret scanning in CI |
| R-S3 SSRF guard | 0141 | SSRF corpus (F-X14) |
| R-S4 API key hashing and rate limits | 0033, 0052 | contract tests |
| R-S5 public spaces expose only published data | 0022, 0141 | unauthenticated-boundary tests |

## Parity matrix (UI, ADR-0020)

"Functionally equivalent UI" is tested as: the adopted React app, changed only in the four allowed areas (base URL/service
layer, auth client, collab provider, branding — Q-R2), passes a Playwright smoke set covering the P0 journeys. The set is
defined during spike 0009 (journey recordings) and listed here when written: _TBD_.

## Gaps to close

F-A05, F-A07, F-A10, F-B09, F-B19, F-C05, F-D07, F-E07, F-E08, F-E09, F-E10, F-E11, F-E12, F-E13, F-E14, F-E15, F-E16, F-E17, F-E21

## Spike evidence (round 26)

| Spike | Result file | Rows and ADRs it informs |
|---|---|---|
| 0001 Rust on Workers | `research/0004-rust-on-workers-binding-matrix.md` | X01, X02, X05, X07; ADR-0006 (conditional pass), 0002, 0007, 0043, 0044, 0045, 0046, 0070, 0071, 0072, 0080, 0081, 0121 |
| 0004 topology, public half | `research/0006-topology-public-half.md` | A02, D09; ADR-0011, 0022, 0048, 0050, 0090 (both real runs done; `.woven` half is a desk recipe) |
| 0008 oracle harness | `research/0005-oracle-harness.md` | A06, B16, X06; ADR-0031, 0032, 0033, 0120, 0122 |
| 0006 editor conversions | `research/0008-editor-conversions.md` | C01, C02, C03, C04; ADR-0012, 0061, 0013, 0141 |
| 0009 compat-plane inventory (desk half) | `research/0003-compat-plane-inventory.md` | X13 (acceptance list = 290 P0 rows), E00b; ADR-0034, 0051 |
| (not a spike) Plane quirks catalogue | `research/0007-plane-quirks-catalog.md` | X06, X13, A06, E00b; ADR-0031, 0033, 0034, 0122 |
| (not a spike) `space` SSR feasibility | `research/0009-space-ssr-worker.md` | D09; ADR-0006 (exception register), 0011, 0022 |

The parity-matrix smoke set (section above) stays _TBD_: it is the dynamic half of spike 0009.
