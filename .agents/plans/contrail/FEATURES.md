# FEATURES — Contrail desired feature list

Tier: **P0** must ship in first usable release · **P1** next · **P2** later · **—** unassigned (open: Q-F1) ·
**drop** explicitly out. Tiers below are from interview round 4 (2026-10-17); rows still `—` are not yet ruled on
(open: Q-F1). Round-4 rulings: P0 = work-management core (B01–B16), platform basics (D01–D04, D08, D09), minimal auth
(user+password via BetterAuth, API keys, membership/roles; invites P1), and the four foundations (permission seam +
RBAC, event/outbox, feature flags, audit). Round 6 refined this: page and description editing at P0 is a **plain API save** (no DO); Durable Objects arrive
with multi-user *collaboration* (F-C02, P1). Auth order: user+password → Cloudflare Access → OIDC (OIDC high priority, P1). Exports/importers/
analytics (D05–D07) are P1 (round 6). Self-hosted adapters at P0 cover DB, queue and blob only (F-X02).
`Ports` = abstractions from ARCHITECTURE.md §Ports the feature depends on. `Src` = where the current behavior lives
(Plane CE paths under `apps/api/plane/…`; `[O]` = Open-EE spec on branch `woven/plan` at
`.claude/plans/plane-fork/features/<name>.md`).

Hybrid rule: the React apps are kept, so *UI parity is mostly inherited*; the cost of each feature is the backend
behavior + its API contract. Mark features whose frontend also needs new work.

## A. Plane CE parity (behavior to reproduce)

### A1 Identity & access
| ID | Feature | Src | Ports | Tier |
|---|---|---|---|---|
| F-A01 | OIDC SSO (Zitadel) login; JIT user provisioning | `authentication/`, `[O] oidc-saml-sso` | Session, Identity | P1 |
| F-A02 | Cloudflare Access JWT validation (outer gate) | new | Identity | P1 |
| F-A03 | Email + password sign-in/up, password reset | `authentication/` | Session, Email | P0 |
| F-A04 | Magic-code (email OTP) sign-in | `authentication/` | Session, Email, Cache | P1 |
| F-A05 | OAuth: Google / GitHub / GitLab / Gitea | `authentication/` (app+space viewsets) | Session | P2 |
| F-A06 | API keys (`X-Api-Key`, `APIToken`), per-key rate limit, activity log | `api/`, `APIToken`, `APIActivityLog` | Cache/RateLimit, DB | P0 |
| F-A07 | Workspace membership, invites (`WorkspaceMemberInvite`), roles | `db/models/workspace` | DB, Email | P0 (invites P1) |
| F-A08 | Project membership, roles (5/15/20), project invites | `db/models/project` | DB | P0 |
| F-A09 | Profile, user preferences, themes, onboarding | `Profile`, `WorkspaceUserProperties` | DB | P0 |
| F-A10 | Instance admin (`god-mode`): instance config, auth-provider toggles, email config. **Round 7: review capabilities; fold most into workspace settings, the rest IaC/env-only; `apps/admin` likely retired (Q-0021.2)** | `license/`, `apps/admin` | DB, Secrets | P0 (settings subset; apps/admin retired) |

### A2 Work management
| ID | Feature | Src | Ports | Tier |
|---|---|---|---|---|
| F-B01 | Workspaces (CRUD, slug, settings) | `Workspace` | DB | P0 |
| F-B02 | Projects (CRUD, identifier, archive, network, favorites) | `Project`, `ProjectIdentifier` | DB | P0 |
| F-B03 | Work items (CRUD, `sequence_id` per project, priority, dates, parent/sub-items) | `Issue`, `IssueSequence` | DB (sequencing) | P0 |
| F-B04 | States (groups backlog/unstarted/started/completed/cancelled) | `State` | DB | P0 |
| F-B05 | Labels, assignees, subscribers | `Label`, `IssueLabel`, `IssueAssignee`, `IssueSubscriber` | DB | P0 |
| F-B06 | Relations (blocks/duplicate/relates…), links, attachments | `IssueRelation`, `IssueLink`, `IssueAttachment` | DB, Blob | P0 |
| F-B07 | Comments, reactions, mentions, votes | `IssueComment`, `*Reaction`, `IssueMention`, `IssueVote` | DB, Queue | P0 |
| F-B08 | Activity history / audit trail per work item | `IssueActivity`, `bgtasks/issue_activities_task` | DB, Queue | P0 |
| F-B09 | Versions of work item & description | `IssueVersion`, `IssueDescriptionVersion` | DB, Queue | P0 |
| F-B10 | Cycles (sprints), cycle progress, transfer issues, archive | `Cycle`, `CycleIssue` | DB, Cron | P0 |
| F-B11 | Modules (epics), links, members | `Module*` | DB | P0 |
| F-B12 | Views (saved filters, display properties, public/private) | `IssueView` | DB | P0 |
| F-B13 | Estimates (points/time/categories) | `Estimate`, `EstimatePoint` | DB | P0 |
| F-B14 | Intake (triage inbox, accept/decline/snooze/duplicate) | `Intake`, `IntakeIssue` | DB | P0 |
| F-B15 | Draft work items; stickies | `DraftIssue*`, `Sticky` | DB | P0 |
| F-B16 | Bulk operations, grouped/sub-grouped listing, pagination, filters (`group_by`) | `app/views/issue` | DB | P0 |
| F-B17 | Auto-archive/auto-close old work items (daily) | `bgtasks` (`archive_and_close_old_issues`) | Cron, Queue | P1 |
| F-B18 | Favorites, recent visits, home | `UserFavorite`, `UserRecentVisit`, `WorkspaceHomePreference` | DB | P0 |
| F-B19 | Teams | `Team` | DB | P2 |

### A3 Pages & collaboration
| ID | Feature | Src | Ports | Tier |
|---|---|---|---|---|
| F-C01 | Pages (project pages): CRUD, lock, archive, access, favorites, labels | `Page`, `ProjectPage`, `PageLabel` | DB | P0 (plain API save; DO arrives with collab, P1) |
| F-C02 | Real-time collaborative editing (Yjs), presence | `apps/live`, `@plane/editor` | Collab | P1 |
| F-C03 | Page versions / history | `PageVersion`, `PageLog` | DB, Queue | P1 |
| F-C04 | Page ↔ work-item embeds / mentions | editor extensions | DB | P1 |
| F-C05 | Export page to PDF | `apps/live` (`@react-pdf/renderer`) | Workflow/Queue | P2 |
| F-C06 | Workspace pages (wiki) — stub in CE | `[O] workspace-wiki-affine` | Collab | drop |

### A4 Platform
| ID | Feature | Src | Ports | Tier |
|---|---|---|---|---|
| F-D01 | Notifications (in-app), preferences, mark read/snooze/archive | `Notification`, `UserNotificationPreference` | DB, Queue | P0 |
| F-D02 | Email notifications (5-min digest) | `EmailNotificationLog`, `stack_email_notification` | Email, Cron, Queue | P0 |
| F-D03 | Webhooks (workspace/project), delivery log, retry | `Webhook*` | Queue, DB | P0 |
| F-D04 | File assets: upload/presign, avatars, covers, inline images | `FileAsset`, `bgtasks/file_asset_task` | Blob | P0 |
| F-D05 | Export (CSV/JSON/XLSX) | `ExporterHistory`, `bgtasks/exporter_*` | Workflow, Blob | P1 |
| F-D06 | Importers (Jira/Linear/CSV/Asana…) | `Importer` | Workflow | P1 |
| F-D07 | Analytics (workspace/project dashboards, saved `AnalyticView`) | `analytics/`, `AnalyticView` | DB | P1 |
| F-D08 | Global search (P0 adapter: database FTS, ADR-0083) | `app/views/search` | Search | P0 |
| F-D09 | Published/public boards (`spaces`) | `space/`, `ProjectDeployBoard`, `apps/space` | DB | P0 |
| F-D10 | Data retention cleanups (api logs, email logs, versions, webhook logs; hard-delete) | `bgtasks` daily | Cron | P0 (hard-delete at 30 d; deviation D-001) |
| F-D11 | i18n (locale files in `@plane/i18n`) | `packages/i18n` | static | P0 (inherited) |
| F-D12 | Telemetry (OTel only; PostHog dropped) — likely opt-in | settings | Observability | P2 (opt-in) |
| F-D13 | Link crawler/unfurl (SSRF-guarded) | `bgtasks` (link crawl) | Queue | P1 |

### A5 Public REST API v1 (wire-compatible, ADR-0031)
Resources: assets, cycles, intake, invites, labels, members, modules, projects, states, stickies, users,
work items (+ comments, links, attachments, relations, activities). (Round 26: v1 *estimates* are not routed on the pinned Plane, so they are not in this list; internal estimates remain F-B13.) Tier **—** per resource; recommended default:
all resources P0/P1 since this is the stable external contract.

## B. Open-EE features (superseded program — implement natively, ADR-0004)
From `woven/plan:.claude/plans/plane-fork/features/`. These are *requirements input*, not Django designs.

| ID | Feature | Spec file | Cross-cutting seam? | Tier |
|---|---|---|---|---|
| F-E00a | Feature flags | `f0-feature-flags` | yes | P0 |
| F-E00b | Permission seam | `f0-permission-seam` | yes | P0 |
| F-E00c | Event/outbox backbone | `f0-event-backbone` | yes (ADR-0046) | P0 |
| F-E01 | RBAC (custom roles) | `rbac` | seam consumer | P1 |
| F-E02 | ReBAC fine-grained | `rebac-fine-grained` | seam consumer | P2 |
| F-E03 | Guests | `guests` | | P1 |
| F-E04 | SCIM 2.0 | `scim` | | P2 |
| F-E05 | OIDC/SAML SSO | `oidc-saml-sso` | (OIDC overlaps F-A01) | P1 |
| F-E06 | Audit logging | `audit-logging` | consumes backbone | P0 |
| F-E07 | Custom fields | `custom-fields` | schema-affecting | P1 |
| F-E08 | Work item types | `work-item-types` | schema-affecting (CE has dormant stub) | P1 |
| F-E09 | Multiple workflows | `multiple-workflows` | | P2 |
| F-E10 | Workflow state control | `workflow-state-control` | | P2 |
| F-E11 | Workflow approvals | `workflow-approvals` | | P2 |
| F-E12 | Dashboards | `dashboards` | | P2 |
| F-E13 | Advanced dashboard widgets | `advanced-dashboard-widgets` | | P2 |
| F-E14 | Initiatives | `initiatives` | | P2 |
| F-E15 | Team spaces | `team-spaces` | | P2 |
| F-E16 | Project templates | `project-templates` | | P2 |
| F-E17 | Work-item / page templates | `work-item-page-templates` | | P2 |
| F-E18 | Recurring work items | `recurring-work-items` | Cron | P1 |
| F-E19 | Nested pages & embeds | `nested-pages-embeds` | | P1 |
| F-E20 | Workspace wiki (AFFiNE-style) | `workspace-wiki-affine` | Collab | drop |
| F-E21 | Time tracking (WakaTime) | `time-tracking-wakatime` | | P2 |
| F-E22 | Intake email & forms | `intake-email-forms` | Email routing | P1 |
| F-E23 | Customers / CRM | `customers-crm` | | drop |
| F-E24 | Slack integration | `slack-integration` | Integration | P2 |
| F-E25 | External sync (GitHub/Beads) | `external-sync` | Integration | P1 |

## C. Platform / non-functional features (new, required by the target)
| ID | Feature | ADR | Tier |
|---|---|---|---|
| F-X01 | Port/adapter runtime: Cloudflare adapter set | 0002, 0007 | P0 |
| F-X02 | Port/adapter runtime: self-hosted adapter set | 0002, 0091 | P0 (DB, queue, blob, session, scheduler, SMTP email); rest P1 |
| F-X03 | Multi-tenant isolation (tenant key; quotas out of scope) | 0003, 0041 | P0 |
| F-X04 | One-time import of existing Plane Postgres data | 0042 (Q-G3) | **drop** (round 5: start fresh) |
| F-X05 | Contract-test harness (every port × every adapter) | 0121 | P0 |
| F-X06 | v1 oracle / parity test harness | 0122 | P0 |
| F-X07 | Observability (logs, traces, metrics) across Queue/Workflow/DO hops | 0130 | P0 |
| F-X08 | AI: P0 API-based access (public API) · P1 MCP server · P2 AI assistant via AI Gateway (AI port) | 0084 | P0 / P1 / P2 |
| F-X09 | Real-time push of work-item updates (beyond SWR polling) | 0034 | P1 |
| F-X10 | Rust (Dioxus) clone of the Plane UI — web target | 0013 | P2 (starts when P0+P1 shipped, SLO 4 wks, oracle clean; Q-R3) |
| F-X11 | Dioxus desktop target | 0013 | P2 (after v1 web; Q-0013.3) |
| F-X12 | Dioxus mobile target | 0013 | P2 (after v1 web; Q-0013.3) |
| F-X13 | `compat-plane` internal API adapter for the React app (deleted when Dioxus lands). Acceptance list = the 290 P0 rows of `scripts/compat-inventory/out-pinned/endpoint-inventory.csv`, minimum slice 48 routes first (spike 0009) | 0034 | P0 |
| F-X14 | SSRF-safe outbound fetch guard (webhooks, link unfurl; R-S3) | 0141, 0073 | P0 |

## D. Explicitly dormant in CE (decide: implement / drop — Q-F3)
Integration models (`GithubRepository*`, `SlackProjectSync`), workspace pages, work-item-type management layer.
