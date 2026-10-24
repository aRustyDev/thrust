---
status: proposed
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0040: System-of-record database

## Context and Problem Statement

The Workers backend needs one authoritative relational store for ~150 Plane models (workspaces, projects, work items, cycles, modules, pages metadata, notifications, webhooks, file assets …). Plane CE is Postgres 15 via Django ORM with `psycopg` 3 **[S]**. Behaviours that do not port trivially: `Issue.save()` takes an advisory lock to assign a per-project `sequence_id` and runs `ChangeTrackerMixin`; soft delete (`deleted_at`) is universal with **partial unique indexes**; grouped/sub-grouped issue listing with filters is query-heavy **[S]**. The interview left this **open: ADR + spike** **[I]**. Whatever is chosen must have a self-hosted twin (ADR-0002) and support tenant isolation (ADR-0003/0041).

## Decision Drivers

* Behavioural parity with Plane's relational semantics (transactions, partial unique, concurrent sequence allocation).
* Must run on both the Cloudflare adapter set and the self-hosted adapter set with minimal dialect divergence (ADR-0002).
* Operational cost and data-size ceilings (Q-G4, Q-G5).
* Latency from Workers to the store (many queries per list view).
* ~~One-time import of existing `projects.woven` data~~ — **dropped** (Q-G3: start fresh; schema is frozen at the pinned Plane version, ADR-0042).
* Multi-tenant capability without a rewrite (ADR-0003).

## Considered Options

* Option 1: Postgres reached through Hyperdrive (external Postgres: Neon/Supabase/self-run)
* Option 2: Cloudflare D1 (SQLite), single database (sharded by workspace if needed)
* Option 3: Durable Object SQLite, one DO per workspace
* Option 4: Hybrid — small control-plane DB (instance/users/workspace directory) + per-workspace data plane (D1-per-workspace or DO-SQLite)
* Option 5: Durable Object SQLite, one object for the instance (added round 29, FR-04; see the round-29 update)

## Decision Outcome

Decision pending — see Q-0040. Leading hypothesis: **Option 1 (Postgres + Hyperdrive)**, confidence **medium**, because it keeps one SQL dialect across the CF and self-hosted adapter sets, preserves partial unique indexes / advisory-lock-free-but-transactional sequencing / rich grouped queries, and keeps the schema baseline a direct copy of Plane's frozen end-state. Option 4 is the strongest alternative if the spike shows Hyperdrive latency or cost is unacceptable or Q-G4 demands very large multi-tenant scale.

### Consequences

* Good (Opt 1): single dialect; mature tooling; the self-hosted adapter is "just Postgres"; the baseline schema is Plane's own.
* Bad (Opt 1): not fully Cloudflare-native — requires an external Postgres; round-trip latency Workers→DB; connection-pool/Hyperdrive limits; extra vendor.
* Good (Opt 2/3/4): fully in-platform, low latency (3: co-located with collab/compute), natural tenant isolation (3/4).
* Bad (Opt 2/3/4): SQLite dialect ≠ Postgres (two dialects to support if self-hosted stays Postgres); D1 has no interactive transactions (batch only) and a 10 GB/DB cap; DO-per-workspace makes cross-workspace queries/analytics hard.
* Neutral: whichever is chosen, `Database` stays a port (ADR-0045); a second adapter may be added later.

### Confirmation

Spike (below) results recorded in this ADR before it moves to `accepted`; port contract tests (ADR-0121) run against the chosen adapter and the self-hosted adapter.

## Pros and Cons of the Options

### Option 1: Postgres + Hyperdrive

Workers connect via Hyperdrive (pooling + query caching) to an external Postgres.

* Good, because dialect parity with Plane and the self-hosted twin; full `pg` feature set (partial indexes, `RETURNING`, JSONB, FTS, RLS, advisory locks).
* Good, because the frozen Plane baseline (ADR-0042) applies as-is.
* Bad, because latency/egress cost and an external dependency; Hyperdrive caching must be disabled for read-your-writes paths.
* Bad, because not "Cloudflare native" for data residency/ops stories.

### Option 2: D1

* Good, because zero-ops, in-platform, cheap, read replication available.
* Bad, because no interactive transactions (batched statements only), 10 GB per DB, SQLite type/JSON semantics differ, no RLS, no advisory locks; a second SQL dialect for self-hosting unless self-hosted also uses SQLite.
* Bad, because grouped issue listing on large projects may stress a single-writer SQLite.

### Option 3: DO SQLite per workspace

* Good, because serialized single-writer per workspace gives sequence allocation and transactional integrity for free; strong isolation; co-located with collab.
* Bad, because cross-workspace queries (my work, instance admin analytics, global search) need fan-out or a replicated read store; per-DO storage cap; migrations must be rolled across all DOs; self-hosted twin needs an emulation layer.

### Option 4: Hybrid control-plane + data-plane

* Good, because isolates tenant data while keeping a global directory (users, memberships, instance config).
* Bad, because the most moving parts; two stores to migrate, back up, and keep consistent; cross-store transactions impossible.

## Spike (exit criteria for `accepted`)

**Scope** (time-box 1–2 weeks; build thin vertical slice behind the `Database` port, not production code):

| # | Workload | What to measure | Exit criterion (to be ratified in Q-0040.1 / Q-G4) |
|---|---|---|---|
| S1 | **Sequence allocation**: 10k creates from ≥ 50 parallel writers in one project (replacing `Issue.save` advisory lock **[S]**; Plane takes `MAX(IssueSequence.sequence)+1` under the lock and allocates `sort_order` in the same block) | duplicates, reuse after delete/move, `sort_order` collisions | zero duplicates; numbers never reused; `sort_order` allocated in the same critical section; mechanism expressible in every candidate (see ADR-0043) — matches spike 0002 |
| S2 | **Partial unique indexes** with `deleted_at IS NULL` (e.g. project identifier, state name per project, label name) | constraint behaviour on insert/soft-delete/re-insert | semantics identical to Plane's migrations' constraints |
| S3 | **Grouped issue listing**: project with 100k work items, `group_by=state`, `sub_group_by=assignee`, filters (labels, priority, cycle), pagination **[S]** | p50/p95 latency from a Worker | p95 ≤ 300 ms at 100k items; no N+1 across the port |
| S4 | **Size ceiling**: load a synthetic large workspace (1M work items, 5M activity rows) | DB size, query plan stability; for D1 compare against the 10 GB cap | fits with ≥ 2× headroom or a defined sharding plan |
| S5 | **Hyperdrive latency**: Worker→pooled Postgres in same vs different region, cached vs uncached, 1 vs 20 sequential queries per request | added ms per query, connection errors | per-request DB overhead ≤ 50 ms for a 10-query list view in the home region |
| S6 | **Transactional outbox** (ADR-0046): insert entity + outbox row atomically; crash between | atomicity, relay lag | no lost/duplicated events beyond at-least-once |
| S7 | ~~Import of a Plane `pg_dump`~~ — **dropped (Q-G3)**; replaced by: apply the frozen baseline schema (ADR-0042) cleanly to the candidate on an empty database | migration applies; constraints present | all tables/constraints/indexes created; partial unique indexes enforced |
| S8 | **Cost model** at three sizes (10 / 100 / 1000 active users) | $/month | within envelope from Q-G5 |

**Deliverable:** results table appended here, plus a recommended option; if S1/S2/S3 fail for an option, that option is rejected.

## Update — interview round 5 (2026-10-17)

Sizing target answered (Q-G4, partly): **≤50 users, ≤100k work items**, and **no import of existing data** (Q-G3). Effects on this ADR: (a) D1's 10 GB per-database limit and single-writer model are no longer disqualifying at v1 scale, so **D1 is back in contention** for cost/simplicity; (b) the sharding argument for DO-SQLite-per-workspace weakens (single org, small data); (c) the spike's scale workloads (S-series) should be sized to ~100k work items plus proportional activity/comments (≈10×) rather than SaaS scale; (d) the advisory-lock/sequencing, partial-unique-index and grouped-listing semantics remain the discriminating tests. The leading hypothesis (Postgres + Hyperdrive) should be re-examined against D1 on cost and operational simplicity once the spike runs — tracked in Q-0040/Q-0040.1.

## Update — interview round 11 (2026-10-17)

Spike scope (Q-0040.1): prototype **Postgres+Hyperdrive and D1**; **DO-SQLite is desk-reviewed only** and rejected on paper unless cross-workspace needs are nil. Exit criteria: `sequence_id` race-safety, partial unique indexes, FTS (ADR-0083), p95 API < 500 ms at 100k work items, and projected monthly cost against the < $100/month envelope (Q-G5). The ADR stays `proposed` until the spike reports.

## Update — review fixes, round 25 (2026-10-18)

Aligned with spike 0002 and the owner's start-fresh decision (Q-G3): import-era drivers, bullets and spike row S7 removed or struck. S1 now reads 10k creates from ≥ 50 parallel writers with no number reuse and `sort_order` in the same critical section; the p95 target stays ≤ 300 ms at 100k items (S3). S4 (1M-item size ceiling) is dropped as out of scope for the ≤100k sizing target; S5 (Hyperdrive latency) and S6 (outbox atomicity) are covered by spike 0002 steps 7 and 5. The spike also adds FTS **plus trigram** (ADR-0083) and Rust data-layer viability on wasm32 (ADR-0044).

## Update — spike results, round 26 (2026-10-18)

Spike 0001 exercised Postgres through wrangler's **local** Hyperdrive emulation only; real Hyperdrive (pooling, upstream TLS, caching, per-request connect cost) is unverified, and each request opens its own socket because a socket belongs to one request in workerd. Stage 2 needs a TLS-reachable Postgres 14 or later and its region (Q-0040.5, Q-G5.1). The engine decision still waits for spike 0002. Evidence on partial unique indexes (Q-0040.3) and counter-row sequencing (Q-0040.2) is in ADR-0043.

## Update — owner decisions and spike results, round 27 (2026-10-18)

Owner decision, round 27 (Q-G5.1, Q-0040.5): the Postgres provider is **CloudNativePG in woven-o11y** reached through a new Cloudflare Tunnel and a Workers VPC service (ADR-0094), and its cost is outside the under-$100/month envelope. Spikes 0001 (Stage 2), 0002 and 0003 run against that Postgres. The engine decision here still waits for spike 0002; the ADR stays `proposed`.

## Update — owner decision, round 27 (2026-10-18)

Owner decision on the scope of spike 0002 (round 27): **Postgres only** — prototype against the CNPG Postgres through Hyperdrive and the Workers VPC service (ADR-0094); D1 and DO-SQLite stay desk reviews (D1 matters only if Postgres fails). The timebox drops from 5 to about 3.5 days.

## Update — Hyperdrive and the Worker smoke test, round 27 (2026-10-18)

The Postgres path is live: a Worker reached `thrust-pg` through Hyperdrive, the Workers VPC service and the tunnel (smoke test, `select 1` p50 9 to 10 ms from IAD, transactions commit and roll back correctly; ADR-0094). Spike 0002 can now run against `thrust-pg-dev` (Hyperdrive config `b231d16b672b469d8834a071fa842b9c`, caching disabled; use a separate config to measure caching). The engine decision still waits for the spike; the ADR stays `proposed`.

## Update — review round 29 (2026-10-19)

Per FR-04 and FR-08 (`reviews/2026-10-19-fable-architecture-review.md`), the round-27 narrowing of spike 0002 to Postgres only (Q-0040.1) is reopened, the spike gains an instance-level Durable Object cell, and rows S3 and S5 gain a placement setting, a round-trip budget and a measurement definition. The review's §5 table records the owner's decision for both findings ("apply now"; for FR-04, "Postgres stays the default if the cell fails any exit criterion"). The engine is still undecided, so the ADR stays `proposed`.

**Why the narrowing is reopened (FR-04).** Four facts are new since round 27:

1. The Hyperdrive statement-flow constraint is documented, so it is permanent. Hyperdrive supports named prepared statements only as `postgres.js` and `node-postgres` implement them, and the docs tell rust-postgres users to use `query_typed` (hyperdrive/concepts/connection-pooling; hyperdrive/examples/connect-to-postgres). Every statement stays typed and uncached (ADR-0044, accepted in round 29), and advisory locks are unsupported through Hyperdrive (ADR-0002, round 29), which rules out Plane's own sequencing mechanism.
2. TLS from the Workers VPC service to CNPG is not authenticated (`cert_verification_mode = disabled`), and the fix touches a shared chart used by nine releases (ADR-0094 round 27, Q-0094.9, bead `plane-byd.9`, deadline "before Thrust holds real data").
3. Availability now includes the `woven-o11y` cluster (ADR-0094 Consequences; `research/0010` §6 risk 1). FR-10 accepts the coupling for the internal-tool SLO under three conditions (ADR-0094, round 29).
4. The first database-backed request on a fresh isolate takes about 0.6 s (`research/0004` §10.3, F7), against the p95 < 500 ms target, at a traffic level (≤ 50 users) where fresh isolates are common.

The round-11 desk review rejected DO-SQLite only on cross-workspace fan-out, migrations across all objects and self-hosted emulation. All three assume Option 3's one object per workspace.

**The added candidate: Option 5, Durable Object SQLite, one object for the instance.** One object (single org, ADR-0003) holds the SQLite database. The `fetch` Worker forwards to it, and the axum router and the use-cases run inside it. The self-hosted twin runs the same SQL on `rusqlite` in the native binary, so both adapter sets share one dialect (the second decision driver above). Sequence allocation and transactions are serialised by construction. It removes the tunnel, the VPC service, Hyperdrive, the TLS question and the cluster coupling; backups are point-in-time recovery plus a nightly dump to R2. Its costs: every request serialises through one object in one region, there are no Postgres extensions, the live CNPG work becomes unused, and SQLite storage is billed (FR-04). Durable Object SQLite is GA with 10 GB per object, point-in-time recovery over the last 30 days, FTS5 and the JSON extension (review §4). Spike 0002 runs it as the `worker-do` cell beside the Postgres `worker-pg` cell, on the same HTTP contract suite and `bench`, with a timebox of 3.5 + 3 days. How ADR-0045's explicit `Tx` maps onto the object is an open question that cell must answer (spike 0002, risk R1).

**Decision rule.** Option 1 stays the leading hypothesis and the default. If the `worker-do` cell fails any of spike 0002's exit criteria (the 50-writer create test, FTS plus trigram, list p95 < 300 ms from the origin region, a migration story, projected cost), Postgres is chosen. If both cells pass, Q-0040.6 decides. Option 5 replaces Option 4 as the alternative the spike measures; the Decision Outcome's sentence about Option 4 no longer holds. ADR-0094 is not reopened: it decides where Postgres lives, not whether Postgres is the engine.

**Rows S3 and S5 (FR-08).**

* **Placement.** `placement.region = "aws:us-east-1"` is the P0 Worker setting (issue #39), next to the database. It replaces the Smart Placement question of Q-0040.5. Cloudflare: each sequential query adds 20 to 30 ms from a distant region and 1 to 3 ms when placed nearby (hyperdrive/concepts/how-hyperdrive-works).
* **S3.** "No N+1 across the port" becomes a budget: a list endpoint makes at most 5 database round trips per request, enforced by a counting `Connector` in the contract suite (issue #39). The p95 ≤ 300 ms criterion stays, measured as SPEC §6.4 now defines it: p95 of a synthetic probe from the origin's region on the warm path, cold starts reported separately. The slice has no states or assignees, so the grouped listing is measured at M1 under the same budget (spike 0002, step 1).
* **S5.** Re-run with the placement set and unset (spike 0002, step 7). Under the budget a list view makes at most five round trips, so S5's 10-query case measures headroom, not a shape the code may take.

S1's 50-writer test and the FR-22 allocation order are spike 0002's step 2 (ADR-0043, round 29).

## Update — spike 0002 results, issue #37 (2026-10-24)

**This ADR stays `proposed`. Both cells are measured on the platform, and the spike's decision rule does not decide the engine without two owner readings.** The numbers, their conditions and everything not measured are in `research/0011-database-engine-cells.md`; nothing below accepts an option or answers Q-0040.6.

**The rule applied to the five criteria** (conditions: one laptop in Virginia, in the origin's region, as the main vantage; one run at a time; n=200 per bench cell; `worker-do` on a Durable Object with hint `enam`, `worker-pg` through Hyperdrive and the Workers VPC service to CNPG `thrust-pg-dev`, placed in `us-east-1`):

| # | `worker-do` | `worker-pg` |
|---|---|---|
| 1. 50-writer test | **passes**: 10,500 creates, 1,428 deletes, no duplicate, no reuse, `sort_order` increasing; the controls (a duplicate-number build, a build without the transaction gate) fail the right way | **passes**: the same test on the deployed Worker, 4 of 4 in the engine pass |
| 2. FTS plus trigram | **passes**: FTS5 `trigram` is in workerd's SQLite (R3); 400 generated queries identical to `icontains`; a created item is found by the next search | **passes**: the same 400 queries identical to `icontains`; freshness and scope pass; `pg_trgm` is installed once by an administrator in `public`, and the operator class must be schema-qualified for the per-PR roles |
| 3. list p95 < 300 ms at 100k items | **passes for a 1,000-item project** (77 ms c=1, 216 ms c=8; database of about 124k items); **fails for an unpaged 10,000-item project** (350 ms, 1,687 ms). A cold object adds nothing measurable (71-132 ms) | **passes for a 1,000-item project** (161 ms, 220 ms; about 132k items); **fails for an unpaged 10,000-item project** (642 ms, 1,231 ms). The first request after a fresh deployment adds nothing measurable (127-209 ms; stage 2 measured 595-615 ms) |
| 4. migration story | **passes**: expand 0.90 s, FTS5 build 3.8 s, contract 0.67 s over 124k rows; a migration that fails at its third statement leaves nothing; **a build whose migration fails takes the instance down** (every request, `/healthz` included, answers 500) until a good build is deployed | **passes**: the migrations apply as the per-PR role; expand 0.54 s, two GIN builds 1.6 s, contract 1.5 ms over 132k rows (database-side); a failing migration leaves nothing and does not touch the running Worker. Applied with `psql`, not through the Rust runner |
| 5. projected cost | about $6.8 a month at the sizing target, about $63 at ten times it; **not reconciled** | Cloudflare side as `research/0004` section 9 ($6.38), the Postgres host outside the envelope (round 27); **not reconciled** |

**The rule's outcome.** Criterion 3 is the same on both: it passes for a realistic list and fails for the slice's unpaged list of a big project, because the response (509 bytes an item, no paging) is the cost, not the engine; and criterion 5 is a projection on both (the owner skipped the 24-hour reconciliation: no real traffic). So:
- **If criterion 3 is read on a 1,000-item project and a projection satisfies criterion 5, both cells pass all five, and Q-0040.6 is the owner's.**
- **If criterion 3 is read on the unpaged 10,000-item list, or criterion 5 must be reconciled, `worker-do` fails and Postgres stays the default** by the rule's first line; `worker-pg` fails criterion 3 on that reading too, a finding about the unpaged list that goes to the owner under Q-0040 (paging is needed whichever engine is chosen).

**What the owner decides:** (a) the reading of criterion 3; (b) whether a projection satisfies criterion 5; (c) if both pass, Q-0040.6. The measured differences between the cells, as facts for (c) and not a recommendation: near the origin the object's small reads are about 30 ms faster at p50 (a few-item list 36-40 ms against 66-69 ms); from us-west-2 (mid-distance, not antipodal) it is 40-70 ms faster than placed Postgres at p50, and Postgres **without** placement is 1.5 to 3.8 times slower than placed, by operation; c=8 creates are close at p95 (302-385 ms against 300-381 ms) and the object's p50 is lower (102-126 against 174-190 ms); 8 concurrent 5 MB lists cost 1,687 ms on the object against 1,231 ms on Postgres (R2: one object serialises the instance), while seeding is 3 times faster on Postgres (5,490 against 1,830 rows a second) and search is steadier on the object (43-77 ms against 70-105 ms); a failed migration takes the object's instance down until redeployed, and does not touch a running Postgres Worker; Postgres needs `pg_trgm` installed by an administrator, a role connection limit sized with Hyperdrive's pool (a second config on a role limited to 8 failed every request), caching off, and placement for distant users.

**The other things measured** (reported, not pass or fail): **Hyperdrive caching** takes 10-20 ms off a repeated read and nothing off a write, and **breaks read-after-write**: 4 of 21 contract tests fail with it enabled (the activity row never appears; a list is stale after writes), so it stays disabled for the system of record. **Placement** changes nothing from the origin's own region and is what makes the cell usable from a distance (create 3.3 to 3.8 times faster, list 1.5 to 1.7 times, from us-west-2); this is the evidence for the open half of Q-0040.5 (the Postgres cell, `placement.region = "aws:us-east-1"`); the question is not answered here. **Round trips by reading the code:** a list is 3 statements on one Hyperdrive connection, a create 7 statements on two connections plus the rate limiter's object (issue #39's budget is not enforced by a counting connector yet). **FR-22 on the platform** is in ADR-0043's note: correct and harmless, worth about 15 percent of the c=8 create p50. **One real email** was accepted by the Email binding (delivery to the mailbox not verified).

What the `worker-do` cell cost, as observed: R2 is real (above); the outbox fast path loses relays when the object is evicted (the cron sweeper delivered the 100 of about 10.5k that were lost, within a minute); health checks go through the object; SQLite's planner reads a project's rows for an indexed search (O(project)); argon2id sign-in would run inside the one object (`research/0012`). R1 is answered (ADR-0045 note): a buffered `Tx` committed in one `transactionSync`, with a control that fails against the naive mapping on the real runtime.

**Not measured:** the 24-hour reconciliation (skipped); any vantage beyond us-west-2; the Rust `migrate` runner and a migration under load; why a concurrent create still costs 1.4 times a sequential one on `worker-pg`; delivery of the email. The deployed numbers were taken before the branch was reconciled onto the auth slice and were not re-taken (`research/0011` sections 12 and 14).

## Open Questions

* Q-0040 — which option?
* Q-0040.1 — spike scope and exit criteria ratification. (reopened round 29, FR-04)
* Q-0040.2 — portable sequence allocation (see ADR-0043). (answered round 29, FR-17)
* Q-0040.3 — partial unique indexes across adapters. (answered round 29, FR-07)
* Q-0040.4 — cross-workspace query needs.
* Q-G3, Q-G4, Q-G5 — data import, scale targets, cost envelope.
* Q-0040.5 — Which external Postgres provider/region for Hyperdrive, and do we enable Smart Placement so Workers run near it?
* Q-0040.6 — If both spike 0002 cells pass their exit criteria, which engine? (added round 29, FR-04)

## More Information

* Plane models: `apps/api/plane/db/models/` ; research `woven/plan:.claude/plans/plane-fork/research/backend-data-model.md`.
* Related: [[ADR-0002]], [[ADR-0003]], [[ADR-0041]], [[ADR-0042]], [[ADR-0043]], [[ADR-0045]].
