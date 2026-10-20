# Handoff: apply the Fable architecture review (2026-10-19)

Paste this file into a fresh session. It stands on its own; the session that wrote it is gone.

## Context

Thrust is a rebuild of Plane (the open-source work tracker) as a Cloudflare-native, self-hostable Rust application:
a wasm32 Worker and a native axum server from the same crates, ports and adapters, Plane's React frontend kept for
Phase 1 behind a transitional `compat-plane` layer, a Dioxus clone of the UI in Phase 2. The plan package lives in
`.agents/plans/contrail/` (SPEC, ARCHITECTURE, FEATURES, TRACEABILITY, DEVIATIONS, 60 ADRs, 9 spikes, research notes,
QUESTIONS). One vertical slice is built across 13 crates with a 16-cell contract suite and one required CI check.
On 2026-10-19 an independent architecture and plan review produced 23 findings; the owner gave every one a
disposition in four triage rounds (18 apply now, 4 backlog, 0 rejected). The review, its evidence and the
dispositions are in **`.agents/plans/contrail/reviews/2026-10-19-fable-architecture-review.md`**; read it first and
cite finding ids (`FR-NN`) in every commit body and issue. Your job: land the plan edits (Track A), create the GitHub
issues (Track B), append the owner questions (Track C). You do not write product code in this session.

**Setup.** The canonical checkout `/Users/asmith/repos/arustydev/thrust` has a stale local `main` (84 commits behind at
the time of writing). Never read or edit its working tree. From it, run `git fetch origin`, then for each PR below
`git worktree add -b <branch> .agents/worktrees/<name> origin/main` and enter the worktree with `EnterWorktree`
(`path` argument). Rebase each later PR on `origin/main` after the previous one merges. The Plane oracle fork at
`/Users/asmith/repos/woven/forks/plane` (tag `oracle/v1-pin-20261018` = `ba85d4d3d3`) is read-only.

## Repo conventions

- One worktree and one branch per coherent change: `docs/<slug>`, `fix/<slug>`, `chore/<slug>`, `feat/<slug>`.
  Worktrees live under `.agents/worktrees/` (already excluded in `.git/info/exclude`). Prune them after merge.
- Conventional Commits for every subject and PR title. Work-item and issue ids go in a `Refs:` footer, never in the
  subject. End commit messages with `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>` and PR bodies with
  `🤖 Generated with [Claude Code](https://claude.com/claude-code)`.
- ADRs use MADR in `adrs/<group>/NNNN-slug.md`, indexed in `adrs/README.md`; a new ADR takes the next free number in
  its group. Supersede an accepted ADR rather than edit its decision; record changes as a dated
  `## Update — review round 29 (2026-10-19)` note that cites the finding. Never mark an ADR `accepted` without a
  recorded owner decision; the review's section 5 table is that record for every finding marked "apply now", so
  cite it: "Decision (accepted, round 29): per FR-NN, `reviews/2026-10-19-fable-architecture-review.md` §5".
- New `QUESTIONS.md` rows go at the end of the file under a new heading `## Added in round 29 (Fable review)`;
  existing rows are never renumbered. Closing a row means setting its status to `answered` with the round and the
  finding id.
- The `planning` skill (`.claude/skills/planning/SKILL.md`) governs where documents live; load it before creating
  `PLAN.md`. Load `mdbook` only if you touch `docs/src/`.
- Bash plumbing: one command per call for anything compound; never `--body "$(…)"`; write bodies to files.
- Do not commit or push without the owner's go-ahead per PR; the issues are public, so Track B waits for a dry-run
  confirmation (procedure below).

## Track A: plan edits

Five PRs, in dependency order. Each item names the finding, the target files, the exact change, and the acceptance
check that proves it landed. Every edited ADR gets a round-29 update note; every closed question cites the finding.

### PR A1 `docs/round-29-data-layer`: port shapes and the data-layer decisions (FR-02, FR-03, FR-07, FR-22, FR-17 part 1)

| # | Finding | Files | Change | Acceptance check |
|---|---|---|---|---|
| A1.1 | FR-02 | `adrs/data/base/0041-tenant-isolation-strategy.md`, `adrs/data/interfaces/0045-repository-and-unit-of-work-ports.md`, `adrs/auth/0051-authorization-model-rbac-rebac.md` | Round-29 note: repository and `Tx` methods take `&Scope` (minted only by `authorize`) or a `SystemScope` (minted only by composition roots and the consumer) instead of bare `WorkspaceId`/`ProjectId`; `Scope` getters are not public outside `thrust-core`; `project_sequences` and the membership join carry `workspace_id`; the two-tenant property test stays and the "ignore the tenant predicate" mutant should fail to compile where possible. Cite `crates/thrust-ports/src/lib.rs:92-138`, `crates/thrust-core/src/lib.rs:139-155`, `crates/thrust-adapters-postgres/src/lib.rs:399`. | `grep -n "round 29" <each file>` finds the note; each note names FR-02 |
| A1.2 | FR-03 | `adrs/auth/0052-sessions-and-api-keys.md`, `adrs/storage/0082-cache-and-session-port-kv.md`, `adrs/auth/0051-…`, `DEVIATIONS.md` (D-003 row) | Round-29 note: `Principal { user_id }` only; the workspace is resolved per request from the URL slug inside `authorize`; `SessionStore` becomes a repository over the `Database` port (session row, `revoked_at`, lazy `last_seen_at`); `KvSessionStore` is deleted from the Cloudflare set; the memory adapter may remain for tests. D-003 note: "the spike's KV store is removed (FR-03)". | notes present; D-003 row mentions FR-03 |
| A1.3 | FR-07 | `adrs/architecture/0002-…`, `adrs/architecture/0006-…`, `adrs/data/layer/0044-query-layer-orm.md` | ADR-0002 and ADR-0006 round-29 note quoting the two Cloudflare docs facts (named prepared statements supported only as `pg`/`postgres.js` implement them; rust-postgres told to use `query_typed`; advisory locks, `LISTEN/NOTIFY`, SQL `PREPARE`, per-session state unsupported through Hyperdrive). ADR-0044: status `accepted`, "Decision (accepted, round 29): hand-written SQL on `tokio-postgres` with typed parameters through one `Pg` wrapper; no ORM; no server-side statement cache; `sqlx`, `sea-orm`, `diesel` forbidden by `check-deps.sh` (issue B6)". | ADR-0044 front matter `status: accepted`; `adrs/README.md` row updated |
| A1.4 | FR-22, FR-08(4) | `adrs/data/schema/0043-ids-sequencing-and-soft-delete.md` | Round-29 note: allocate the sequence number in its own short transaction (or last) so the counter-row lock spans one round trip; gaps on failure accepted; confirm with spike 0002's 50-writer test. Accept A2, B1, C1: status `accepted`, "Decision (accepted, round 29): as built in PR #17 (B1 counter row, `sort_order` in the same critical section, partial unique indexes), with the FR-22 allocation order". | status `accepted`; README row |
| A1.5 | FR-17 (built) | `adrs/data/schema/0042-…`, `adrs/data/interfaces/0045-…`, `adrs/data/contracts/0046-…`, `adrs/test/patterns/0121-…` | Each gets `status: accepted` and "Decision (accepted, round 29): as built in PR #17" plus the specific shape: 0042 = SQL migration files applied by the `migrate` binary, expand/contract, no ORM codegen; 0045 = explicit `Tx` object, commit consumes, Drop discards the connection, FR-02 scope argument; 0046 = envelope v1 as in `thrust-contracts`, after-commit relay plus sweeper, 90-day retention, consumer idempotent on `event_id`; 0121 = one HTTP suite, four targets, mutation controls, count guard. | four front matters `accepted`; README counts updated |
| A1.6 | FR-02/03/07/22 | `QUESTIONS.md` | Close: Q-0006.4 (answered: documented constraint, no support case), Q-0044, Q-0045, Q-0045.1 (no; serializable isolation only for allocation, which is a single statement), Q-0043, Q-0040.2 (B1), Q-0040.3 (yes on Postgres; D1 moot), Q-0046, Q-0121, Q-0121.1 (the vocabulary is the FR-05 port list: ordering per key, delay ≤ 24 h, message ≤ 128 KB), Q-0121.2 (only Stage 2 items: real cron, DLQ, hibernation timing). Each answer cites the finding. | `grep -c "round 29" QUESTIONS.md` ≥ 10 |
| A1.7 | FR-02 | `docs/src/contributing/patterns.md` | One paragraph: "repository calls take a `Scope`, never ids" with the FR-02 rationale (link the review). | paragraph present; `mdbook build` not required (book is not built in CI) |

### PR A2 `docs/round-29-ports-and-twin`: cut the port catalogue, redefine the twin, complete the non-Rust register (FR-05, FR-19, FR-15)

| # | Finding | Files | Change | Acceptance check |
|---|---|---|---|---|
| A2.1 | FR-05 | `adrs/jobs/0071-workflow-port.md` | New ADR? No: supersede by round-29 note "WorkflowEngine port withdrawn (FR-05): long jobs are queue messages with a `jobs` table and idempotent steps; `entry.mjs` and the Workflow binding are removed (issue B9); the Postgres step runner is not built". Status stays `accepted` with the note; add `superseded-in-part` wording in the title line. | note present; ADR-0006 register no longer lists the Workflows shim as live |
| A2.2 | FR-05 | `adrs/jobs/0070-queue-port.md` | Round-29 note: self-hosted adapter = the outbox (in-process fast path + sweeper) in a single process; RabbitMQ and `lapin` dropped; a broker is reconsidered only when a second native process exists; ordering per key is trivial in-process. | note present |
| A2.3 | FR-05 | `adrs/storage/0082-cache-and-session-port-kv.md`, `adrs/api/0033-rate-limiting-and-api-keys.md` | Round-29 notes: Valkey dropped from P0 and P1 (no session cache, no auth cache); `Cache` port added only when a measured hot path needs it; rate limiting = Workers Rate Limiting binding on Cloudflare (D-007) and an in-process counter natively; the DO limiter is removed from the API Worker. | notes present |
| A2.4 | FR-05, FR-15 | `adrs/collab/0060-yjs-collab-provider-port.md` | Round-29 note: one `yrs` room implementation hosted in a Durable Object on Cloudflare and inside the axum server natively; Hocuspocus+Redis is withdrawn as the self-hosted adapter (it was a shipped TypeScript service with no exit plan); the room speaks Hocuspocus framing or the provider is swapped under Q-R2. Spike 0005 exit criterion added: "the same room runs on axum". | note present; `spikes/0005-…md` exit criteria updated |
| A2.5 | FR-15 | `adrs/architecture/0006-runtime-language-and-framework.md` | Replace the register with the FR-15 table: Workflows shim (removed), spike-0006 TS/Python oracle tooling (permanent tooling, never imported by product crates), Hocuspocus (withdrawn), React apps and `@plane/editor` (Phase 1), **editor island in Dioxus: planned permanent exception** (owner, round 29), Python inventory scripts (tooling; home per issue B21). | table present with six rows |
| A2.6 | FR-19 | `SPEC.md` (R-P3, §7 table, §8 Phase 5), `FEATURES.md` (F-X02), `adrs/infra/0091-self-hosted-twin.md`, `adrs/storage/0080-blob-port-r2.md` | R-P3: "P0 self-hosted set = database, the outbox as queue, local-disk `BlobStore`, SMTP via `lettre`, in-process scheduler; S3-compatible blob adapter at P1". §7 rows for RabbitMQ, Valkey, Hocuspocus, Celery/Workflows updated to the cut. §8 Phase 5 removed (it is the `native-pg` CI cell; roadmap PR A5 replaces §8 anyway). ADR-0091 `accepted`: "one binary, one database, one object store; a Compose file with at most three services". ADR-0080 note: local-disk adapter at P0. | SPEC and FEATURES diffs show the text; ADR-0091 `accepted` |
| A2.7 | FR-05 | `ARCHITECTURE.md` §1b, §1c, §2a | §1b twin diagram: binary, Postgres (or SQLite per FR-04), local disk, SMTP; no RabbitMQ, Valkey, Hocuspocus, worker process. §1c table reduced to the real ports (Database connector, Queue, BlobStore, Scheduler, Email, Collab host, Clock/IdGen/Background) with a line "search, flags, AI, identity, secrets, telemetry are core code on one dialect". §2a: drop `thrust-collab-do` as a separate root until M5; drop `thrust-api-core` at P0 (FR-06, PR A5). | diagrams and table updated |
| A2.8 | FR-05, FR-19, FR-15 | `QUESTIONS.md` | Close Q-0061.2 (answered: yrs room on axum), Q-0073.4 and Q-0071.1 (superseded: no workflow engine), Q-0091, Q-F6.2 (answered per R-P3 text), Q-0013.2 (editor island registered; renderer per target stays for spike 0007), Q-0082.2 (superseded: no Valkey). | rows closed with FR ids |
| A2.9 | FR-05 | `adrs/test/patterns/0121-…`, `TRACEABILITY.md` | Contract matrix rows for Workflow, RabbitMQ, Valkey, Hocuspocus removed; F-X02 row text updated. | no `pending-adapter` row for a cut adapter |

### PR A3 `docs/round-29-engine-and-infra`: the engine hearing, latency, TLS, availability, topology, deploy (FR-04, FR-08, FR-09, FR-10, FR-11, FR-12, FR-16 part)

| # | Finding | Files | Change | Acceptance check |
|---|---|---|---|---|
| A3.1 | FR-04, FR-16 | `spikes/0002-database-engine.md` | Rewrite: two cells, `worker-pg` (CNPG through Hyperdrive) and `worker-do` (one instance-level Durable Object with SQLite; the axum router and use-cases run inside it; native twin on `rusqlite`), both driven by the existing HTTP contract suite and `bench`; drop step 6 (decided by FR-07); keep steps 2, 3, 4, 7, 9; add Stage 2b items from Q-0006.6 (caching on/off for Postgres, distant-region bench, 24-hour cost reconciliation, real email send); exit criteria per cell: 50-writer create test, FTS plus trigram (FTS5 trigram tokenizer on the DO cell), list p95 < 300 ms from the origin region, migration story, projected cost; decision rule: Postgres stays the default if the DO cell fails any criterion; if both pass, Q-0040.6 (Track C) decides. Timebox 3.5 + 3 days. | file rewritten; `spikes/README.md` status row and order updated |
| A3.2 | FR-04 | `adrs/data/base/0040-system-of-record-database.md` | Round-29 note: Q-0040.1's round-27 narrowing is reopened per FR-04 with the four new facts (docs-explained Hyperdrive constraint, unauthenticated TLS, woven-o11y coupling, 0.6 s first DB request); the instance-level DO cell is added; ADR-0094 is not reopened (it decides where Postgres lives). Status stays `proposed`. | note present; Q-0040.1 row updated |
| A3.3 | FR-08 | `adrs/data/base/0040-…` (S3, S5), `adrs/infra/0094-…`, `SPEC.md` §6.4, `QUESTIONS.md` Q-G5.2 | `placement.region = "aws:us-east-1"` recorded as the P0 Worker setting (issue B5); the per-request budget "at most 5 database round trips for a list endpoint", enforced by a counting `Connector` in the contract suite; Q-G5.2 answered: "p95 of a synthetic probe from the origin's region on the warm path, 5-minute cadence, cold starts reported separately; availability over a rolling 30 days on the same probe". | SPEC §6.4 text; Q-G5.2 `answered` |
| A3.4 | FR-09 | `adrs/infra/0094-postgres-hosting-cnpg-tunnel-vpc.md`, `QUESTIONS.md` Q-0094.9, Q-0094.8 | Round-29 note: closing plan = Cloudflare Origin CA server certificate mounted through CNPG `spec.certificates` (chart exposes it as an opt-in value, same pattern as `zoneSpread`), VPC services back to `verify_ca` (Workers VPC trusts Origin CA; `verify_ca` skips hostname verification); fallback = Option 2 with the CNPG CA uploaded to Hyperdrive and `verify-full`; both untested until the first `wrangler` update; work starts after the FR-04 decision (issue B7). Q-0094.8: accept the chart churn (opt-in values, manifests unchanged). | note present; Q-0094.9 carries the plan; Q-0094.8 answered |
| A3.5 | FR-10 | `adrs/infra/0094-…`, `QUESTIONS.md` Q-0094.4, Q-0094.7 | Q-0094.4 answered: coupling accepted for the 99.5 % internal-tool SLO under three conditions (deep probe `GET /healthz?deep=1` on the product hostname in `status.woven`; SLO measured end to end on that probe; exit trigger: two monthly budget breaches in a quarter, or an EKS upgrade exceeding the budget, moves to managed Postgres or the FR-04 outcome). Q-0094.7: the probe is issue B8; the per-PR schema sweep is issue B23. | rows answered |
| A3.6 | FR-11 | `adrs/infra/0090-cloudflare-topology.md`, `ARCHITECTURE.md` §1a | Status `accepted`: "Decision (accepted, round 29): Option 1, one Worker (fetch, queue, scheduled, static assets, placement set) for P0 and the daily-use release; a `collab` Worker is added when the P1 room lands; reopen if the bundle exceeds 20 MiB or a DO lifecycle change must ship with an API change". §1a diagram: one API Worker box. Close Q-0090. | front matter `accepted`; Q-0090 answered |
| A3.7 | FR-12 | `adrs/cd/0110-deploy-pipeline.md`, `QUESTIONS.md` Q-0110.1 | Round-29 note superseding the rollout stages: expand migration (CI) → `wrangler deploy` at 100 % → smoke suite against the live hostname → automatic `wrangler rollback` on failure; keep expand/contract and the DO-lifecycle rule; gradual rollout revisited when a 10-minute slice carries meaningful traffic. Q-0110.1 updated. | note present |
| A3.8 | FR-16 | `QUESTIONS.md` Q-0006.6 | Answered: folded into spike 0002 (A3.1). | row answered |

### PR A4 `docs/round-29-auth-and-text`: the auth slice spike and the rich-text P0 stance (FR-16, FR-14, FR-23)

| # | Finding | Files | Change | Acceptance check |
|---|---|---|---|---|
| A4.1 | FR-16 | `spikes/0003-identity-betterauth-rust.md` (rename to `0003-auth-slice-rust.md` and fix links in `spikes/README.md`, ADR-0050, ADR-0052, PROMPT.md) | Rewrite as a two-day auth slice: argon2id with OWASP parameters on the real platform (CPU ms measured, must fit the default 30 s budget with margin), session row + HttpOnly cookie + CSRF double-submit, hashed API keys accepted as `X-Api-Key`, one OIDC authorization-code login through the `openidconnect` crate against Zitadel (or a mock), on both targets, through the FR-03 `Principal`. Options A (`better-auth-rs`) and B (TypeScript sidecar) dropped; C is the plan. Exit: the slice passes the contract suite on four targets. | file rewritten; README row updated |
| A4.2 | FR-16 | `adrs/auth/0050-identity-oidc-zitadel-access.md`, `QUESTIONS.md` Q-0050.4, Q-0050.6 | Round-29 note: "BetterAuth-shaped" means the HTTP shapes only; the implementation is the Rust auth slice. Q-0050.4 answered (hand-rolled Rust adapter, spike 0003 as rewritten); Q-0050.6 answered (no import; `User`/`Profile`/`Account` tables are defined fresh in the schema). | note present; rows answered |
| A4.3 | FR-14 | `adrs/ui/0012-editor-and-i18n-reuse.md`, `adrs/collab/0061-collab-persistence-and-versioning.md`, `QUESTIONS.md` Q-F5.1, Q-0061.4 | Round-29 note: P0 work items store sanitised `description_html` and `description_stripped` only; JSON and binary are derived lazily at P1 by `thrust-editor-doc` on first collab open; pages store the triple the fallback path sends, with HTML sanitised and the binary validated to decode. Q-F5.1 reopened and answered accordingly; Q-0061.4 answered (no staleness window at P0 because only one form is stored). | notes present; rows answered |
| A4.4 | FR-23 | `adrs/ui/0012-…`, `DEVIATIONS.md`, `QUESTIONS.md` Q-0012.2, Q-0012.4 | New deviation rows D-016 (hardened sanitiser: all 125 payloads neutralised), D-017 (`issue-embed-component` and its attributes allowed), D-018 (`rel` preserved), D-019 (ammonia ≥ 4.1 output not byte-identical to nh3 on ammonia 4.0.0), D-020 (work items store one form at P0). Q-0012.2 answered; Q-0012.4 answered: no browser-flavour renderer. | five rows with the next free ids |

### PR A5 `docs/round-29-scope-and-plan`: scope, surfaces, PLAN.md, the remaining acceptances (FR-01, FR-06, FR-18 tiers, FR-17 part 2)

| # | Finding | Files | Change | Acceptance check |
|---|---|---|---|---|
| A5.1 | FR-01 | `PLAN.md` (new, in `.agents/plans/contrail/`) | Status table at the top (dated), then the review's section 3 verbatim in substance: milestones M0 to M6 with contents and exit criteria, the critical path, the Mermaid graph, the "smallest usable release" paragraph, and the M2 screen list the owner confirmed: work-item list (group-by, filters, pagination); work-item detail (description, comments, activity, attachments, labels, assignees, state, priority, dates); project create and settings; workspace members (no invites); in-app notifications; global search. Issue numbers from Track B are added once created. | file exists; `README.md` of the plan links it (create the plan README if absent per the `planning` skill) |
| A5.2 | FR-01 | `SPEC.md` §8 | Replace the indicative phasing with "see `PLAN.md`" plus the diff table from review §3. | §8 rewritten |
| A5.3 | FR-01 | `FEATURES.md` F-X13, `adrs/api/0034-…`, `TRACEABILITY.md` | F-X13 acceptance list = the 48-route minimum slice plus the compat rows behind the M2 screens; every other route the React shell calls is a recorded stub (empty list, 404, or fixture) listed in `DEVIATIONS.md` as D-021 "compat stubs outside the acceptance list". ADR-0034 round-29 note with the same and "compat rows are deleted as each screen moves to Dioxus". TRACEABILITY parity matrix defined as "the Playwright journey suite over the M2 screens (issue B11)". | F-X13 text; ADR note; TRACEABILITY no longer `_TBD_` |
| A5.4 | FR-01 | `adrs/ui/0013-rust-frontend-dioxus-multitarget.md`, `QUESTIONS.md` Q-R3, Q-0013.1, Q-0009.1 | Round-29 note: Phase 2 starts screen by screen when the M2 release has held the SLO for 4 weeks and its journey suite is green (the UI oracle); the `/api/v1` oracle diff gate applies to M3's operations. Q-R3 and Q-0013.1 updated; Q-0009.1 updated (the tripwires now apply to the acceptance list, not 290 rows). | note present; rows updated |
| A5.5 | FR-06 | `adrs/api/0034-…`, `adrs/api/0032-…`, `ARCHITECTURE.md` §2a, `SPEC.md` R-F7, `docs/src/flows/app.md` | Two surfaces at P0 (compat-plane, `/api/v1`); the core OpenAPI surface is generated from the use-cases at M4; the slice's non-Plane routes move out of the `/api/v1` prefix (issue B13). `docs/src/flows/app.md` route table notes the pending rename. | notes present |
| A5.6 | FR-18 | `FEATURES.md` F-E06, F-E00a, `adrs/data/contracts/0048-audit-logging.md`, `adrs/architecture/0008-feature-flags-port.md` | F-E06 tier P1 with "the retained outbox is the P0 audit trail"; F-E00a "a `flags` table read per request; no port". ADR-0048 `accepted` (Option 2 + Option 1 at P1); ADR-0008 `accepted` (Option 1, table, no port). | tiers changed; two ADRs `accepted` |
| A5.7 | FR-17 | `adrs/data/contracts/0047-…`, `adrs/jobs/0073-…`, `adrs/storage/0081-…`, `adrs/test/models/0120-…`, `adrs/test/standards/0123-…` | Each `accepted` on its leading hypothesis with "Decision (accepted, round 29): per FR-17"; 0073 drops the Workflow column and the `instance_traces`/`track_event` rows (REVIEW-NOTES 7 still open) and adds `webhook_send_task`; 0123 points at `docs/src/testing/` as the graduated standard. | five front matters `accepted`; README counts: 38 + 15 = 53 accepted, 5 proposed (0040, 0041, 0061, 0141 and 0090 if A3.6 is not merged yet), 2 superseded |
| A5.8 | FR-17 | `QUESTIONS.md` | Close with defaults: Q-0008, Q-0047, Q-0048, Q-0048.1 (90-day retention, JSON export at P1), Q-0073, Q-0073.5 (table completed), Q-0081, Q-0120, Q-0122, Q-0123, Q-0123.1 (mutation testing on the permission seam, no line-coverage threshold), Q-0123.2 (e2e on the Cloudflare set only; the native cell is the contract suite), Q-0010.1 (scoped to the acceptance list; the dynamic half runs per screen), Q-0041.1 (directory tables: `users`, `profiles`, `accounts`, `sessions`, `api_tokens`, `instance_*`), Q-0040.4 (yes: a user's workspaces and notifications; one database makes it trivial), Q-F7 (admin creates the user with a temporary password at P0; invites in M3), Q-F5.2 (none; the collab provider is stubbed at P0), Q-0001.1 (the fork is frozen as the oracle; nothing to retire), Q-0005.1 (moot: the React apps are consumed as a pinned workspace), Q-0011.4 (same hostname, route group of the main Worker: no CORS), Q-0080.2 (`<workspace>/<project>/<item>/<name>`, as built), Q-0093.2 (Terraform-created ids), Q-0060.2 (closed: not reproduced; reconnect-on-close required), Q-0085.2 (importers at P1 use the same allocator in a bulk range), Q-0053.1 (stays open, P2), Q-0094.5 (one tunnel; default), Q-0094.6 (owner, keep open with a date), Q-R7 (owner, keep open with a date), Q-0141 (accept Option 1 when ADR-0141 is promoted at M2). Every closed row names FR-17 and the default. | ≤ 8 rows remain `open`, each with an owner and a date |
| A5.9 | all | `adrs/README.md`, `PROMPT.md` | README status counts and rows; PROMPT round log entry "29: Fable architecture review applied (reviews/2026-10-19-…); PLAN.md written". | counts match `grep -c 'status: accepted' adrs/*/*.md adrs/*/*/*.md` |
| A5.10 | Track C | `QUESTIONS.md` | Append the three Track C rows (below). | rows present under the round-29 heading |

## Track B: GitHub issues in `aRustyDev/thrust`

The repository has no issues, GitHub's default labels only, and no milestones. Create these first.

**Labels** (name, colour, description): `area:app` `#1d76db` "core, ports, adapters, HTTP" · `area:infra` `#0e8a16` "Cloudflare,
Postgres, deploy" · `area:plan` `#5319e7` "the Contrail plan package" · `area:process` `#bfd4f2` "repo conventions, CI, tooling" ·
`type:spike` `#fbca04` "time-boxed experiment" · `type:security` `#b60205` "tenant isolation, auth, TLS" · `prio:p0` `#d93f0b` ·
`prio:p1` `#fef2c0` · `review:fable-2026-10-19` `#c5def5` "filed from the Fable architecture review".

**Milestones:** `M0 Foundations fixed` · `M1 Walking skeleton` · `M2 Daily-use release (Contrail)` · `M3 Breadth` ·
`M4 Dioxus screen by screen` · `M5 Collab` · `M6 Identity and P1`.

**Issues.** Each body has the sections Context / Problem / Proposal / Acceptance criteria / References / `Finding: FR-NN`.
References list paths with line numbers, ADR and `Q-` ids, and the review file. `Blocked by` is written as a
"Blocked by #N" line once numbers exist, and as a GitHub dependency where the UI allows it.

| Id | Title | Labels | Milestone | Blocked by | Beads | Finding |
|---|---|---|---|---|---|---|
| B1 | feat(core): make `Scope` the argument of every repository and `Tx` method | area:app, type:security, prio:p0, review | M0 | | | FR-02 |
| B2 | feat(core): user-only `Principal`, database-authoritative sessions, delete `KvSessionStore` | area:app, type:security, prio:p0, review | M0 | B1 | | FR-03 |
| B3 | spike(db): run spike 0002 with a `worker-pg` cell and an instance-level `worker-do` SQLite cell | area:infra, type:spike, prio:p0, review | M0 | B1 | plane-byd.4 | FR-04, FR-16 |
| B4 | spike(auth): the two-day Rust auth slice (argon2id, sessions, CSRF, API keys, one OIDC login) | area:app, type:spike, prio:p0, review | M0 | B2 | | FR-16 |
| B5 | chore(worker): set `placement.region = aws:us-east-1`; add the 5-round-trip budget counter to the contract suite | area:infra, prio:p0, review | M0 | | | FR-08 |
| B6 | chore(ci): enforce the clippy deny list, `cargo fmt --check` and `cargo-deny`; forbid `sqlx`, `sea-orm`, `diesel` in `check-deps.sh` | area:process, prio:p0, review | M0 | | | FR-07, ADR-0006 panic policy |
| B7 | chore(infra): authenticated TLS to CNPG through a Cloudflare Origin CA server certificate and `verify_ca` (infra repo work, tracked here) | area:infra, type:security, prio:p0, review | M0 | B3 | plane-byd.9, infra-qnyi | FR-09 |
| B8 | chore(infra): `GET /healthz?deep=1` through the database and a `status.woven` probe; SLO measured on it | area:infra, prio:p0, review | M2 | B3 | | FR-10 |
| B9 | chore(worker): single-Worker topology; remove the Workflow binding, `entry.mjs`, the DO rate limiter and the `SELF` binding | area:app, prio:p0, review | M0 | | | FR-05, FR-11 |
| B10 | feat(compat): the 48-route minimum slice with the auth-client JSON change | area:app, prio:p0, review | M1 | B1, B2, B3, B4, B13 | plane-byd.3 | FR-01 |
| B11 | test(e2e): Playwright journey suite on the stock React app as the UI oracle | area:process, prio:p0, review | M1 | B10 | | FR-01 |
| B12 | feat(compat): the M2 daily-use screens (acceptance list) with every other route stubbed and recorded | area:app, prio:p0, review | M2 | B10, B11 | | FR-01 |
| B13 | chore(api): move the slice's routes out of the `/api/v1` prefix; reserve it for wire-compatible routes | area:app, prio:p0, review | M0 | | | FR-06 |
| B14 | feat(editor): work-item descriptions store sanitised HTML only; pages store the validated triple; hardened sanitiser | area:app, prio:p0, review | M2 | B10 | plane-byd.6 | FR-14, FR-23 |
| B15 | chore(cd): deploy pipeline per ADR-0110 round 29 (expand, deploy 100 %, smoke, auto-rollback) | area:infra, prio:p0, review | M1 | B9 | | FR-12 |
| B16 | feat(worker): reject non-canonical-case `/api`, `/auth`, `/live` paths and run the Worker first at the Access phase | area:infra, type:security, prio:p1, review | M6 | | | FR-13 |
| B17 | chore(api): one `plane_500()` helper for reproduced 500s; compat parity scoped to the acceptance list | area:app, prio:p0, review | M1 | B10 | plane-byd.5 | FR-18 |
| B18 | chore(contracts): keep compat DTOs out of `thrust-contracts`; add the `check-deps.sh` rule | area:app, prio:p0, review | M1 | B10 | | FR-20 |
| B19 | feat(queue): permanent versus transient consumer errors; undecodable messages to the DLQ; native `dead_letters` table | area:app, prio:p1, review | M2 | B9 | | FR-21 |
| B20 | feat(collab): one `yrs` room hosted on a Durable Object and on axum (spike 0005 first) | area:app, prio:p1, review | M5 | B12 | | FR-05, FR-15 |
| B21 | chore(plans): bring the compat-inventory and spike-0004 Python scripts into this repository under `tools/` (`uv`) | area:process, prio:p1, review | M0 | | | FR-15 |
| B22 | feat(jobs): exports and seeding as queue jobs with a `jobs` table (no workflow engine) | area:app, prio:p1, review | M3 | B9, B12 | | FR-05 |
| B23 | chore(infra): per-PR schema sweep owner and CI wiring for preview environments | area:infra, prio:p1, review | M1 | | plane-byd.4 | Q-0094.7 |

Body template (write each to `/tmp/issue-BN.md` or the scratchpad, never inline):

```
## Context
<one paragraph: what exists today, with paths>

## Problem
<the finding, with the evidence lines>

## Proposal
<the recommendation from the review, concrete>

## Acceptance criteria
- [ ] <observable check>
- [ ] <test or gate that proves it>

## References
- Review: .agents/plans/contrail/reviews/2026-10-19-fable-architecture-review.md (FR-NN)
- <paths, ADR ids, Q- ids>

Finding: FR-NN
```

**Procedure.**
1. `gh issue list --state all --search "<keyword>"` for each title; the repository had zero issues on 2026-10-19,
   so any hit is new since and must be reconciled, not duplicated.
2. Create the labels (`gh label create`) and milestones (`gh api repos/aRustyDev/thrust/milestones -f title=…`).
3. Print the whole list (title, labels, milestone, blocked-by, first line of the body) as a dry run and **wait for the
   owner to confirm**; the issues are public.
4. Create each issue with `gh issue create --title … --body-file <file> --label … --milestone …`. Never pass the
   body through `--body "$(…)"`.
5. Check each with `gh issue view <n> --json number,title,labels,milestone` and compare with the table.
6. Add the `Blocked by #N` lines by editing bodies (`gh issue edit --body-file`) once numbers exist; then print a
   table mapping every finding to its issue numbers, and add those numbers to `PLAN.md` (PR A5) if it has not merged.

## Track C: owner questions to append to `QUESTIONS.md`

Under `## Added in round 29 (Fable review)`, next free ids in their families:

| ID | Question | Status and recommended answer |
|---|---|---|
| Q-0040.6 | If both spike 0002 cells pass their exit criteria, which engine? The Postgres cell keeps the CNPG investment and the Plane-like dialect; the Durable Object cell removes the tunnel, VPC service, Hyperdrive, the TLS deadline and the cluster coupling, and gives the twin one dialect (SQLite) in one binary. **[I]** | open, owner, decide when the cells report. Recommended: the Durable Object if the owner accepts single-region, single-writer serialisation and SQLite storage billing; otherwise Postgres. |
| Q-R8 | Which screens make up the M2 daily-use release (the F-X13 acceptance list)? **[I]** | **answered** (round 29, FR-01): work-item list (group-by, filters, pagination); work-item detail (description, comments, activity, attachments, labels, assignees, state, priority, dates); project create and settings; workspace members without invites; in-app notifications; global search. Cycles, modules, views, pages, intake, webhooks, public spaces and exports are M3. |
| Q-0013.5 | Is a JavaScript editor island inside the Dioxus UI an accepted permanent non-Rust exception, or should spike 0007 cost a Rust ProseMirror view? **[I]** | **answered** (round 29, FR-15): registered as a planned permanent exception in ADR-0006; spike 0007 may still size a Rust view for information. |

## Rejected findings

None. Every finding received "apply now" or "backlog". Two owner-call recommendations were accepted as written
(FR-04 the engine hearing, FR-10 the availability conditions); FR-09's TLS plan is accepted as the recorded plan
with the work deferred until the engine decision.

## Definition of done

- Five Track A PRs opened in dependency order from worktrees under `.agents/worktrees/`, each with Conventional
  Commit subjects, a `Refs:` footer listing the finding ids, and the attribution lines; each merged only on the
  owner's go-ahead.
- `PLAN.md` exists with the status table, milestones, critical path, graph and the M2 screen list; SPEC §8 points at it.
- `adrs/README.md` counts match the front matters; no accepted ADR lacks a round-29 decision line citing the review.
- `QUESTIONS.md` has at most eight `open` rows, each with an owner and a date; the three Track C rows are appended.
- Labels and milestones exist; 23 issues created from body files after the owner's dry-run confirmation, each
  verified with `gh issue view --json`; a finding-to-issue table printed and added to `PLAN.md`.
- Nothing pushed, merged or created on GitHub before the owner confirmed that step.
