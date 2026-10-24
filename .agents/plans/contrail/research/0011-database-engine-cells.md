# Research 0011: database engine cells (spike 0002, issue #37)

**Date:** 2026-10-24. **Spike:** `spikes/0002-database-engine.md`. **Gates:** ADR-0040 (engine), ADR-0041, ADR-0083; confirms ADR-0043's FR-22 order. **Code:** thrust PR #77 (branch `feat/engine-cells`, draft), beads `plane-byd.11.4.1` to `.9`. **Decided 2026-10-24 (owner): Postgres is the system of record (ADR-0040 `accepted`).** The `worker-do` cell and the SQLite adapter are **archived on the tag `spike/0002-worker-do`** (commit 021dc65 of the PR's branch), not merged: every path in this note under `crates/thrust-adapters-sqlite/`, `crates/thrust-worker/src/app_do.rs`, `crates/thrust-worker/wrangler.do*.toml` and `scripts/spike-0002-do.sh`, and the `native-sqlite` and `worker-do` targets and `do` pass of `scripts/contract.sh`, resolve on that tag, not in the merged tree (section 15).

## 1. Verdict: both cells are measured on the platform and pass; the owner decided Postgres (2026-10-24).

This note records what was built, what was measured and under what conditions, and what was **not** measured. It was written before the owner's decision, which section 15 records; the sections below are the evidence. The decision rule (spike 0002, "Decision rule") is: Postgres stays the default if the `worker-do` cell fails any exit criterion; if both cells pass, Q-0040.6 is the owner's; a `worker-pg` failure is a finding against the default and goes to the owner under Q-0040.

**The rule's outcome.** Both cells ran on the platform (`worker-do` on 2026-10-23 and 24, `worker-pg` on 2026-10-24; section 11 is the ledger). Criteria 1, 2 and 4 pass on both. **Criterion 3 passes on both for a 1,000-item project and fails on both for an unpaged 10,000-item list**, so it separates the engines on neither reading. **Criterion 5 is a projection for both** (the owner skipped the 24-hour reconciliation). The rule therefore gives two answers, and which one applies is the owner's reading, not a measurement:
- Read criterion 3 on a project of 1,000 items (the realistic list, section 6.1) and accept a projection for criterion 5: **both cells pass all five, and Q-0040.6 is the owner's.**
- Read criterion 3 on the unpaged 10,000-item list, or require criterion 5 to be reconciled: `worker-do` fails, so **Postgres stays the default** by the rule's first line, and `worker-pg` fails the same criterion 3, a finding against the default that goes to the owner under Q-0040 (it is a finding about the unpaged list, not about either engine: paging is needed whichever is chosen).

What the owner had to decide: (a) the reading of criterion 3; (b) whether a projection satisfies criterion 5; (c) if both pass, Q-0040.6. **Decided 2026-10-24: (a) the realistic paged list, so both pass; (b) yes, both pass as projected; (c) Postgres.** Section 1.1 puts the measured differences between the cells next to each other, as facts for (c); section 15 records the decision.

Tags, as in `research/0004`: **[tested]** = ran locally (Apple-silicon macOS laptop, loopback, local workerd from wrangler 4.123.0 or a local Docker Postgres 16); **[tested-cf]** = ran against the Worker deployed on Cloudflare's network; **[derived]** = arithmetic on measured numbers, vendor prices, or reading the code; **[unverified]** = not run.

**Conditions that apply to every deployed latency below.**
- **Laptop vantage.** One US laptop is the client for every number unless a section says otherwise. For the `worker-pg` runs `wrangler tail` showed it reaching Cloudflare's **IAD** colo with a client TCP round trip of 8 ms, so it is **in the origin's region** (Virginia, `us-east-1`; the Durable Object's hint is `enam`); the colo was not recorded for the `worker-do` runs, which used the same laptop and network. Each figure carries that last mile (the 43 ms p95 of a list of a nearly empty project on `worker-do` is the floor of what this client sees).
- **Distant vantage (section 6.9).** A `t4g.small` in **us-west-2**, about 65 ms from `us-east-1`: **mid-distance across the continent, not antipodal**; nothing here says what Europe or Asia would see. Its connect time to the Cloudflare edge was about 6.5 ms.
- The Workers were `workers.dev` on the Woven account (`<cloudflare-account-id>`), built with the cells' release profile, one deploy at a time. n is small (200 requests per bench cell, 20 to 50 for the list probes); where runs disagree the spread of three runs (two on the distant vantage) is printed rather than averaged.
- **The deployed numbers were taken on the code of commits up to `c9d52da` (branch `feat/engine-cells` before the reconcile onto `main`).** The branch was later reconciled onto the auth slice (#38 and its follow-ups); the numbers were not re-taken (section 14).

### 1.1 The cells next to each other (measured, same laptop unless noted)

| Measure | `worker-do` | `worker-pg` (Hyperdrive, placement on, caching off) |
|---|---|---|
| `GET list`, a few items, p50 · p95, c=1 | 36-40 · 44-57 ms | 66-69 · 83-92 ms |
| `POST create` p50 · p95, c=1 | 86-87 · 134-211 ms | 125-135 · 177-198 ms |
| `POST create` p50 · p95, c=8 | 102-126 · 302-385 ms | 174-190 · 300-381 ms |
| `GET list` of 1,000 items, p95, c=1 · c=8 (database of 124k and 132k items) | 77 · 216 ms | 161 · 220 ms |
| `GET list` of 10,000 items (5 MB), p95, c=1 · c=8 | 350 · 1,687 ms | 642 · 1,231 ms |
| search p95 (four query shapes) | 43-77 ms | 70-105 ms |
| seed rows per second (5,000 per request) | 1,830 | 5,490 |
| distant vantage (us-west-2), `POST create` · `GET list` · `GET activity`, p50, c=1 | 147-153 · 111-117 · 85-89 ms | 197-200 · 182-187 · 131-135 ms placed; 665-738 · 281-322 · 268-307 ms **not** placed |
| first DB-backed request after a fresh deployment | 71-132 ms (the object, section 6.5) | 127-209 ms (the same as the second and third) |
| migration: expand · index build · contract on a copy of the live table | 0.90 s · 3.81 s (FTS5) · 0.67 s over 124k rows, client-timed | 0.54 s · 1.56 s (two GIN) · 1.5 ms over 132k rows, database-side |
| a migration that fails | nothing changes; **the instance is down until a good build is deployed** | nothing changes; the running Worker is unaffected |
| projected cost, Cloudflare side | about $6.8 a month at the sizing target, about $63 at 10 times | `research/0004` section 9 ($6.38) plus the Postgres host, outside the envelope |
| what it needs | nothing outside the account | `pg_trgm` installed by an administrator, the operator class qualified, a role connection limit sized with Hyperdrive's pool, caching off, placement for distant users |

These are facts about two builds, one laptop, one distant host and n of 200; they are not a ranking.

### Five exit criteria, as measured

| # | Criterion (spike 0002) | `worker-do` | `worker-pg` |
|---|---|---|---|
| 1 | 50-writer test: no duplicate `sequence_id` in 10k creates, no reuse after delete, `sort_order` with it | **passes** [tested-cf]: 10,500 creates + 1,428 deletes, every number once and contiguous from 1, no deleted number reused (section 4). Control shown failing | **passes** [tested-cf]: the engine pass 4 of 4 on the deployed Worker (238 s), with the Hyperdrive pool at 8 origin connections and the role at 8 (section 4.1). Also local on `native-pg` and `worker-cf` [tested] |
| 2 | FTS plus trigram: substring and prefix queries return Plane's `icontains` results, 1-minute freshness | **passes** [tested-cf]: 400 generated queries identical to `icontains`; a created item is found by the very next search (section 5) | **passes** [tested-cf]: the same 400 queries (14,196 hits, 98 empty, 57 capped) identical to `icontains`; freshness and scope tests pass; `pg_trgm` needs an administrator and a qualified operator class (section 5.4) |
| 3 | List p95 < 300 ms from the origin's region at 100k items, warm; cold first requests reported beside it | **passes for a 1,000-item project** (p95 77 ms at c=1, 216 ms at c=8, in a database of about 124k items); **fails for an unpaged 10,000-item project** (350 ms at c=1, 1,687 ms at c=8). Cold: no measurable cost (section 6) | **passes for a 1,000-item project** (p95 161 ms at c=1, 220 ms at c=8, about 132k items); **fails for an unpaged 10,000-item project** (642 ms, 1,231 ms). Cold: no measurable cost (6.2b, 6.5b). Vantage: Virginia, the origin's region |
| 4 | Migration story, failure case included | **passes** [tested-cf]: expand 0.90 s, FTS5 build 3.8 s, contract 0.67 s on 124,081 rows; a failing migration changes nothing; a build whose migration fails takes the instance down until redeployed (section 7) | **passes** [tested-cf]: the migrations apply as the per-PR role; expand 0.54 s, GIN builds 1.6 s, contract 1.5 ms on 132,265 rows; a failing migration changes nothing (section 7.4). Applied with `psql`, not through the Rust `migrate` function |
| 5 | Projected cost within the envelope, reconciled against the 24-hour run | **projected, not reconciled**: about $6.8 a month at the sizing target and about $63 at ten times it, Cloudflare side (section 8). The 24-hour run was skipped by the owner on 2026-10-24: with no real traffic it would only have measured the idle floor | **projected, not reconciled**: Cloudflare side as `research/0004` section 9 ($6.38) plus the Postgres host, which is outside the envelope (ADR-0040, round 27); Hyperdrive and the Workers VPC service were not billed separately in this run's accounting |

Reported, not pass or fail: the FR-22 allocation-order check (section 9: on the platform it is a modest effect), Hyperdrive caching on versus off (6.7: caching breaks read-after-write), the placement re-run (6.8, 6.9), the distant-region bench (6.9), the round-trip counts (6.10), and the one real email send (section 10, item 9). Section 12 lists what was not run.

## 2. What was built

This is what the spike built. The SQLite adapter, the object (`AppDo`), the `durable` adapter set, `DoSql`, the `worker-do` and `native-sqlite` targets and `scripts/spike-0002-do.sh` below are **archived on the tag `spike/0002-worker-do`** and are not in the merged tree (section 15); the rest merges.

| Part | Where | Notes |
|---|---|---|
| SQLite adapter | `crates/thrust-adapters-sqlite` | the `Database` port written once over a synchronous `Sql` trait (`query`, `execute`, `atomically`); `NativeSql` on rusqlite 0.40 (bundled SQLite 3.53.2, never compiled for wasm32); `DoSql` in `thrust-adapters-cloudflare` over `SqlStorage` |
| SQL | `crates/thrust-adapters-sqlite/migrations/` | `0001_init.sql` = the end state of Postgres 0001-0004 (a SQLite database starts empty, so there is nothing to patch); `0006_search.sql` (FTS5). Ids are text, times are microseconds since the epoch (below 2^53 until 2255, so they survive a JS number), booleans 0/1 |
| Migration runner | `migrate.rs` | each file once, as one transaction with its ledger row (`schema_migrations`), one statement per `exec`; refuses a database that records a version the build does not know |
| The cell | `crates/thrust-worker/src/app_do.rs`, `wrangler.do.toml`, `wrangler.do.stage.toml` | one instance-level `AppDo` (`get_by_name_with_location_hint("instance", "enam")`): the `App` is built once in `new`, migrations run in `new` inside `blockConcurrencyWhile`, `Background` is `State::wait_until`; the front Worker forwards every API request, and the queue consumer and cron handler forward through it (`/_do/consume`, `/_do/sweep`). `/_do/*` is the Worker-to-object protocol: a client path that starts with it is a 404 |
| Targets | `scripts/contract.sh` | `native-sqlite` and `worker-do`, opt-in: **not** in `contract-all.sh`, so not required checks until ADR-0040 decides. New passes `engine`, `do` (worker-do), `scale` |
| Engine pass | `crates/thrust-contract-tests/tests/engine.rs` | the 50-writer test, search parity, freshness and scope, the bulk seed; own test binary, so `EXPECT_MAIN` is untouched (21 when this note was written, 37 after the auth slice, section 14) |
| Routes | `thrust-api` | `GET /api/_slice/workspaces/{slug}/projects/{id}/work-items/search?q=` (icontains, at most 100, sequence order, q not trimmed, blank or over 100 characters is a 400), `POST /internal/seed-work-items` (admin) |
| FR-22 | `thrust-adapters-postgres` | the sequence is taken in its own short transaction before the create's (section 9) |
| Tooling | `scripts/spike-0002-do.sh`, `bench_scale`, the `AppDo` probes | deploy and teardown with a UTC ledger; `/_do/usage`, `/_do/cost-probe`, `/_do/migration-story`, `/_do/fts-probe`, `/_do/tx-control`, all behind test routes and the admin token |

Bundle: 2,000 KiB raw / 657 KiB gzip with both cells compiled in, startup 3 ms [tested-cf]. Stage 2's `worker-cf`-only bundle was 1,709 KiB / 568 KiB.

## 3. R1: how ADR-0045's explicit `Tx` maps onto the object [tested, tested-cf]

`sql.exec()` cannot run `BEGIN` or `SAVEPOINT`; `transactionSync` takes a callback that must finish without yielding; workers-rs 0.8.7 binds neither it nor anything that keeps a transaction open across awaits. A use-case drives a `Tx` across awaited calls. The mapping built:

- the `Tx` **buffers** `insert_work_item`, `soft_delete_work_item` and `insert_outbox`, and answers the two questions a use-case asks mid-transaction (is this external key taken, is this row live) from the committed rows plus its own buffer; a duplicate key is a `Conflict` at the call, as on Postgres;
- `allocate` reads the project's counter row and hands out numbers above it; nothing is written yet;
- `commit` applies the counter advance and every buffered write in **one `Sql::atomically`**: `BEGIN IMMEDIATE` on rusqlite, `ctx.storage.transactionSync` in the object. workers-rs has no `transactionSync`, so `DoSql` calls it through `Storage::as_raw()` and `js_sys::Reflect`, with a borrowed wasm-bindgen closure whose `Err` is thrown, which is how `transactionSync` is told to roll back;
- the counter upsert returns the counter and `commit` checks it is where `allocate` read it;
- `rollback` and `Drop` discard the buffer: nothing was written, so there is nothing to undo;
- a **gate** lets one `Tx` be open at a time. Inside the object, nothing between `begin` and `commit` yields in today's create and delete use-cases, so two transactions cannot interleave; that is a property of the use-case, not of the type, and natively (tokio threads) it is false at once. The gate (std only, cancel-safe) makes it the adapter's own property, and the counter check turns a hole in it into an error instead of a duplicate.

What it gives up against Postgres: a `Tx` cannot read its own uncommitted writes through the database, and transactions are serialised (which is risk R2, section 6.4).

**Control** (must fail first): the scenario `thrust_adapters_sqlite::control` fails each statement a commit issues in turn (the counter upsert, each item insert, the soft delete, each outbox insert), over one to three creates plus one soft delete in the transaction, 36 cases of which 18 name a statement the commit issues that many times and so fire; after each failed commit the database must be exactly as it was (items, live items, outbox rows, counter) and the same transaction must then commit; the 18 that do not fire must commit and apply everything.
- On rusqlite [tested]: passes; against `mutant_non_atomic_commit` (statements applied one by one) 15 of the 36 fail ("a failed commit left a work item behind", "advanced the counter").
- On the real `SqlStorage` under local workerd [tested]: with `DoSql::atomically` temporarily a plain call (no transaction) `worker_do_tx_commit_is_atomic_on_the_objects_sqlite` **failed** ("a failed commit changed the database": an item without its outbox row, a counter advanced for nothing); with `transactionSync` 0 violations and all 18 faults fired.
- On the deployed object [tested-cf]: the same test, 36 cases, 18 fired, 0 violations; and `worker_do_tx_naive_mapping_is_caught` (the same scenario over a `Sql` whose `atomically` is no transaction) reports violations, so the scenario can tell a transaction from a script.

The three candidates the spike listed: bind `transactionSync` by hand (**chosen**); move owned state into `Storage::transaction` (an async `'static` closure: it cannot be the `Tx`, because the use-case holds the `Tx` across awaits, and as a commit-time wrapper it would add an async JS closure to buy nothing `transactionSync` does not give); rely on "writes with no intervening `await` are submitted atomically" (no explicit rollback: the control above is exactly the case that breaks it). R1 has a workable mapping.

## 4. Criterion 1: the 50-writer create test [tested, tested-cf]

`engine_fifty_writers_ten_thousand_creates_never_duplicate_a_sequence_id`: 50 bounded tasks, 200 creates each (10k in total, as the issue and spike say, not 50 x 10k), one project, then 1,428 soft deletes (every 7th number) from 50 tasks, then 500 more creates, then an admin's `with_deleted` listing. It asserts every number once and contiguous from 1, `sort_order` unique and increasing with it, the next numbers continue at 10,001 and none of the deleted numbers comes back, and the listing's tombstones are exactly the deleted ones. Writers stop at the first non-201, so a broken engine fails in milliseconds and says why.

| Target | Result | Wall time of the test |
|---|---|---|
| `native-memory` | pass | 1.4 s |
| `native-sqlite` | pass | 3.2 s |
| `native-pg` (Docker Postgres 16) | pass | 18.9 s (17.9 s after FR-22) |
| `worker-do` (wrangler dev) | pass | 20.4 s |
| `worker-cf` (local Hyperdrive emulation, Docker Postgres) | pass, 2 of 2 runs after the fix below | 112 s and 117 s (125 s after FR-22) |
| **`worker-do` deployed** | **pass** (`stage2-contract.sh engine`, 4 of 4) | 85 s for the whole pass (the 50-writer test plus 400 search queries), from the laptop |

**Controls, each against a deliberately broken build** (`scripts/mutants.sh` runs all of them):
- `native-memory` + `mutant_dup_sequence`: "9999 duplicate sequence ids in 10000 creates, the first: 1".
- `native-sqlite` + `mutant_dup_sequence`: 50 of 50 writers get HTTP 409, "UNIQUE constraint failed: work_items.project_id, work_items.sequence_id" (the engine's own index refuses the duplicate).
- `native-sqlite` + `mutant_no_gate` (transactions not serialised): 39 of 50 writers get HTTP 500 and the server logs "the sequence counter of project ... moved between allocate and commit" 39 times. The gate is what makes allocation safe on the twin, and the counter check turns a hole in it into an error, never a duplicate.
- The first version of the SQLite `mutant_dup_sequence` only duplicated within one transaction and left the main pass at 21 of 21: the slice allocates once per transaction, so it was a no-op. It was rewritten (always sequence 1) and the main pass then fails 8 tests.

**A finding about the Postgres cell's harness.** On `worker-cf` (the Worker path opens one connection for its reads and one for its transaction, and wrangler's local Hyperdrive emulation does not pool) one of 50 writers got HTTP 500 ("postgres handshake: db error") in one run at the image's default `max_connections=100`. The engine pass now starts the container with `max_connections=400`; 0 failures in the next 2 runs. The connection limit is the inferred cause: the server's own message is not in the Worker's log.

### 4.1 `worker-pg` on the platform [tested-cf]

`stage2-contract.sh engine` against the deployed `thrust-spike-pg` (Cloudflare adapter set, Hyperdrive caching off, origin connection limit 8, role connection limit 8, 2026-10-24T02:29Z): **4 of 4 passed in 238 s**, the 50-writer test included (10,500 creates and 1,428 deletes by the same sequence as above), against 85 s for the same pass on the deployed object. The harness finding above did not repeat on the platform: Hyperdrive queues statements when the pool is busy, so 50 writers on a pool of 8 origin connections all got their 201 (a create uses two connections, one for its reads and one for its transaction, so with 50 writers the pool was oversubscribed by construction). The main pass on the same deployment was 21 of 21 (86 s), as it was on the deployed object (79 s). The controls are the local ones above and `scripts/mutants.sh`'s Postgres rows (the duplicate-number build fails the same test); they were not re-run against the deployed Worker.

## 5. Criterion 2: FTS plus trigram [tested, tested-cf]

### 5.1 R3: does workerd's SQLite have the FTS5 `trigram` tokenizer? Yes.

Asked of the real runtime through `/_test/do/fts-probe`, which builds a scratch FTS5 table and drops it (test `worker_do_fts5_trigram_tokenizer_is_available`), on local workerd (wrangler 4.123.0) and on the deployed object, with identical answers:

| Probe | Answer |
|---|---|
| `CREATE VIRTUAL TABLE ... USING fts5(x, tokenize = 'trigram')` | created |
| MATCH `"WORLD"` on `Hello World` | found (case folded) |
| MATCH `"eedl"` on `needle in a haystack` | found (inside a word) |
| MATCH `"in a h"` | found (across spaces) |
| MATCH `"école"` on `ÉCOLE normale` | found (non-ASCII case folded) |
| MATCH of a 2-character query `"he"` | **nothing** (a trigram needs three characters) |
| LIKE `%he%` | found; `LIKE ... ESCAPE` with `%` and `_` escaped works |
| `json_each` | works |
| `SELECT sqlite_version()`, `PRAGMA compile_options` | **refused in a Durable Object** (`SQLITE_AUTH`), so the SQLite version is not known |

### 5.2 Parity with `icontains`

The search is substring parity, not ranking: Plane's search is `icontains`, so results are in sequence order, capped at 100, and what is compared is the set. Postgres: `pg_trgm` GIN indexes on `name` and `description`, `ILIKE '%q%' ESCAPE '\'` with the query's own `\`, `%`, `_` escaped. SQLite (native and object): an external-content FTS5 table with the `trigram` tokenizer kept in step by insert, delete and update triggers; a query of 3 characters or more is an FTS5 phrase (quotes doubled); a shorter one scans the project, with LIKE when it is ASCII and in Rust, a page of 500 rows at a time, when it is not. Memory: a scan with `to_lowercase().contains` (the reference). There is no `tsvector` ranking path on Postgres: nothing in the slice or in Plane's search ranks, and the criterion is parity and freshness.

`engine_search_matches_icontains_on_a_generated_seed`: a deterministic proptest corpus of 300 items (words that collide on purpose: shared stems and case variants, and `%`, `_`, `\`, quotes), seeded through the bulk route, 20 soft-deleted through the API, 400 generated queries (1 to 8 characters, taken from the items with the case flipped half the time, or arbitrary). Every engine answers exactly what the oracle in the test answers: **14,196 hits, 98 empty, 57 at the 100-result cap, on `native-memory`, `native-sqlite`, `native-pg`, `worker-do`, `worker-cf` locally and on the deployed `worker-do`.** The corpus is ASCII: engines fold case beyond it by different rules; the non-ASCII case is covered by the hermetic differential test below and by the 1-2-character finding.

Controls: the test requires the corpus to tell a case-sensitive matcher and a prefix-only matcher from `icontains` on more than 20 queries each; `mutant_search_case_sensitive` (memory) fails 2 of the 4 engine tests; the hermetic differential `prop_search_matches_the_reference_adapter` (SQLite adapter against the memory adapter, with `é`/`É` in the alphabet) fails with LIKE unescaped and with an unquoted FTS phrase; with the FTS insert trigger dropped a 6-character query misses a row that a 2-character query still finds, so the index is what answers a long query.

### 5.3 Freshness and scope

`engine_search_is_fresh_literal_and_scoped`: a created item is found by the very next search (the bound is 60 s; nothing waited), a deleted one is not; a description is searched; `%`, `_` and `\` are literal; another tenant's item with the same text is not returned; a cross-tenant search is 404, a workspace member who is not in the project 403, no token 401, blank, over-long or missing `q` 400. Passes on all engines including the deployed object. The trigram index is maintained by triggers (SQLite) and by the GIN index (Postgres) in the writing transaction, so there is no lag to measure.

### 5.4 Findings

- **SQLite's LIKE folds ASCII case only.** The differential test found `é` missing a row holding `ÉCOLE` (a 1-character non-ASCII query goes to the LIKE path). Pinned in `crates/thrust-adapters-sqlite/tests/search.proptest-regressions`; short non-ASCII queries now scan in Rust. Folding beyond simple case mapping (`ß`, `İ`) is not claimed.
- **A 3-character indexed search reads about the project's rows.** SQLite's planner scans the project through `work_items_list` and uses the FTS5 matches as a membership filter, instead of driving from the index. On the deployed object the billed rows read per search were 1,086 for the common word `Fix`, 1,046 for a miss and 1,046 for a 2-character scan, in a project of 1,030 live items (`search` in section 8.1). It is O(project), not O(matches). A prototype with the FTS table as the outer loop (a join) was faster for a miss and slower for a common word, because the index is global and the matches of every project must be sorted; the planner's choice was kept. At 100k items in one project each search would read 100k rows; reported, not asserted.
- **`pg_trgm` and the per-PR roles.** `CREATE EXTENSION pg_trgm` is allowed to a role with CREATE on the database (the extension is trusted since PostgreSQL 13) and not to the schema-only `pr_<n>` roles of the shared dev database, so an administrator installs it once, in `public`. The per-PR roles also have a **role-level `search_path = pr_<n>` only** (`products/thrust/kube/scripts/pr-schema.sh`), so the unqualified `gin_trgm_ops` of the first version of migration 0006 was not found: reproduced on a throwaway postgres:16-alpine as such a role, `ERROR: operator class "gin_trgm_ops" does not exist for access method "gin"`. Migration 0006 now qualifies it (`public.gin_trgm_ops`, and `CREATE EXTENSION ... WITH SCHEMA public`); as `pr_1001` it applies, applies again, and `ILIKE '%q%'` is a BitmapOr of the two trigram indexes; as a superuser with a non-public search_path it installs the extension in `public`; the `native-pg` target (main 21 of 21, engine 4 of 4) and the 10 adapter tests pass. Not shown: the same run on the shared dev database itself.

## 6. Criterion 3: latency [tested, tested-cf]

### 6.1 What was measured, and the shape that decides the criterion

The slice's list has no paging: `GET .../work-items` returns every visible item of the project, about 509 bytes each as JSON (506 KB for 1,000 items, 5.0 MB for 10,000, about 45 MB for 100,000). "List p95 < 300 ms at 100k items" therefore has two readings, and the answer differs between them:
- **A realistic list**: a project of 1,000 items in a database of about 124k items (`bench_scale`: 100 projects of 1,000 items, one project of 10,000, and what the earlier passes left behind: 124,081 `work_items` rows, a 131 MB database). **Passes.**
- **The unpaged list of a big project**: 10,000 items (5 MB). **Fails at c=1 (350 ms) and collapses at c=8.** A single project of 100k items would be a 45 MB response through a 128 MB object; it was not tried, and it is not an API shape anyone should have.

This depends on list shape, not on the engine: the response is the cost. It will need paging (a limit and a cursor) whichever engine is chosen; the Postgres cell was not measured for the 10,000-item list.

### 6.2 Deployed `worker-do` at about 124k items [tested-cf]

One laptop; times are client-side, so they include its round trip to the edge (about 40 ms, from the list of a nearly empty project).

| Measure | c=1 | c=8 |
|---|---|---|
| seed: 100,000 items in 100 projects (5,000 rows per request) | 54.7 s, 1,830 rows/s | |
| seed: the 10,000-item project | 5.5 s, 1,818 rows/s | |
| `GET list`, project of 1,000 items (506 KB), n=50 | p50 53.7 · **p95 76.7** · p99 120.1 ms | p50 173.1 · **p95 215.9** · p99 217.3 ms |
| `GET list`, project of 10,000 items (5,091 KB), n=20 | p50 285.4 · **p95 350.4** · p99 393.4 ms | p50 1,471 · **p95 1,687** · p99 1,777 ms |
| `GET search` common word `Fix` (hits the cap), n=40 | p50 43.6 · p95 76.8 · p99 142.9 ms | |
| `GET search` phrase `billing page #4`, n=40 | p50 44.3 · p95 60.7 · p99 65.1 ms | |
| `GET search` no match `zzzzqq`, n=40 | p50 33.5 · p95 43.0 · p99 46.2 ms | |
| `GET search` 2 characters `ou` (a scan), n=40 | p50 34.4 · p95 52.6 · p99 63.7 ms | |
| `POST create` into a project of 1,000+, n=200 | p50 92.9 · p95 237.4 · p99 568.7 · max 700.9 ms | p50 118.7 · p95 333.6 · p99 496.0 · max 499.7 ms |

### 6.2b Deployed `worker-pg` at about 132k items [tested-cf]

The same `bench_scale 100000 1000` run, against the deployed `thrust-spike-pg`, on the Postgres cell: the Cloudflare adapter set, Postgres through a Hyperdrive config (**caching disabled**, origin connection limit 8) and the Workers VPC service, to the CNPG cluster `thrust-pg-dev` as the per-PR role `pr_1101`, `placement.region = "aws:us-east-1"`. The database held about 132k `work_items` rows (the 100,000-item seed in 100 projects of 1,000, one project of 10,000, and what the main and engine passes left behind) with the `pg_trgm` GIN indexes of migration 0006. The same laptop; `wrangler tail` showed this laptop reaching Cloudflare's **IAD** colo with a client TCP round trip of 8 ms, so this vantage is in the origin's region (Virginia; `us-east-1`), not a distant one. (The `worker-do` runs of section 6.2 and 6.3 were taken from the same laptop and network; the colo was not recorded for them.)

| Measure | c=1 | c=8 |
|---|---|---|
| seed: 100,000 items in 100 projects (`unnest` bulk insert, 5,000 rows per request) | 18.2 s, 5,490 rows/s | |
| seed: the 10,000-item project | 1.2 s, 8,599 rows/s | |
| `GET list`, project of 1,000 items (506 KB), n=50 | p50 107.0 · **p95 160.7** · p99 189.1 ms | p50 142.4 · **p95 220.3** · p99 233.0 ms |
| `GET list`, project of 10,000 items (5,091 KB), n=20 | p50 448.0 · **p95 641.9** · p99 651.4 ms | p50 933.8 · **p95 1,231** · p99 1,242 ms |
| `GET search` common word `Fix` (hits the cap), n=40 | p50 63.4 · p95 77.8 · p99 101.2 ms | |
| `GET search` phrase `billing page #4`, n=40 | p50 62.5 · p95 103.1 · p99 162.4 ms | |
| `GET search` no match `zzzzqq`, n=40 | p50 57.0 · p95 105.3 · p99 296.1 ms | |
| `GET search` 2 characters `ou` (a scan), n=40 | p50 55.0 · p95 70.2 · p99 163.2 ms | |
| `POST create` into a project of 1,000+, n=200 | p50 139.8 · p95 187.7 · p99 334.7 · max 429.2 ms | p50 192.7 · p95 337.8 · p99 408.1 · max 457.6 ms |

Against `worker-do` (section 6.2, same laptop, a database of about 124k items):
- **The criterion's shape is the same on both cells.** A 1,000-item project passes on both (p95 161 and 220 ms on `worker-pg`, 77 and 216 ms on `worker-do`); an unpaged 10,000-item project fails on both (642 and 1,231 ms against 350 and 1,687 ms). The list is the cost, not the engine; paging is needed whichever engine is chosen.
- At c=1 the Postgres cell is slower for the small list (161 against 77 ms p95) and for the 10,000-item list (642 against 350 ms): three round trips through Hyperdrive and the tunnel against zero network hops inside the object, and a 5 MB result that crosses the tunnel. At c=8 it is the other way for the 10,000-item list (1,231 against 1,687 ms): each request serialises its JSON in its own isolate in parallel, where the object serialises the instance (section 6.4). That is R2 on the platform, measured against the cell it was predicted against.
- Search through `pg_trgm` is steady at 70-105 ms p95 whatever the query (a hit-capped common word, a phrase, no match, a 2-character scan), against 43-77 ms for the object.
- Seeding is 3 times faster (5,490 against 1,830 rows/s, 5,000 rows a request on both): one `unnest` statement against the object's per-row SQLite writes, each with its index and FTS5 shadow rows. The cause was not isolated.

### 6.3 The shared `bench` on the deployed object [tested-cf]

n=200 per cell, a fresh project, three runs (ordered by run; stage 2 is `research/0004` section 10.3, `worker-pg`, **before** FR-22, another day, the same kind of vantage; the vantages are not guaranteed equal).

| Measure | `worker-do` c=1 | `worker-do` c=8 | stage 2 `worker-pg` c=1 | stage 2 c=8 |
|---|---|---|---|---|
| `POST create` p50 | 83.4 / 84.4 / 77.2 ms | 102.3 / 102.8 / 90.7 ms | 143 ms | 282 ms |
| `POST create` p95 | **191.0 / 119.0 / 121.6** ms | **305.7 / 184.7 / 361.9** ms | **296** ms | **559** ms |
| `POST create` p99 | 341.5 / 541.4 / 368.3 ms | 468.4 / 277.5 / 567.6 ms | 424 ms | 1,244 ms |
| `GET list` p95 (a few items) | 42.9 / 50.2 / 44.3 ms | 52.1 / 48.5 / 42.1 ms | 86 ms | 75 ms |
| `GET activity` p95 | 31.7 / 34.3 / 35.4 ms | 39.2 / 31.2 / 34.3 ms | 89 ms | 65 ms |
| create -> activity visible (n=40, fast path on) | p50 4,997 (run 1, a 10.5k-event queue backlog from the engine pass was still draining), 1,284, 1,293 ms; p95 9,687, 2,037, 1,912 ms | p50 5,010, 1,218, 1,260 ms; p95 10,028, 1,439, 1,560 ms | p50 1,392 · p95 1,842 ms | p50 1,359 · p95 2,270 ms |
| first create of the run (includes seeding the fixture; not a cold-object figure) | 368 / 624 / 337 ms | 404 / 293 / 359 ms | n/a (stage 2's 615 and 595 ms are the first database-backed request on a fresh isolate, a different thing) | n/a |

The create at c=8 is 1.5 to 3 times the create at c=1 on `worker-do`, against 1.9 times on stage 2's Postgres before FR-22. Those columns are two different days and different code; the same-day Postgres cell is the next table.

### 6.3b The shared `bench` on the deployed Postgres cell [tested-cf]

Same bench, same laptop (Virginia), same session as the second `worker-do` column; n=200 per cell. `worker-pg`: Hyperdrive caching off, placed in `us-east-1`, FR-22 order, three runs. `worker-do`: the object redeployed for this comparison (2026-10-24T03:30Z), three runs. Stage 2 is `research/0004` section 10.3 (`worker-pg`, **before** FR-22, another day).

| Measure | `worker-pg` c=1 | `worker-pg` c=8 | `worker-do` same day c=1 | `worker-do` c=8 | stage 2 c=1 | stage 2 c=8 |
|---|---|---|---|---|---|---|
| `POST create` p50 | 124.8 / 128.1 / 134.5 ms | 189.7 / 174.3 / 176.4 ms | 85.5 / 85.8 / 87.3 ms | 106.9 / 102.4 / 126.3 ms | 143 ms | 282 ms |
| `POST create` p95 | **198.4 / 186.4 / 176.5** ms | **341.5 / 300.0 / 380.7** ms | 134.5 / 133.9 / 210.5 ms | 302.3 / 385.1 / 334.0 ms | **296** ms | **559** ms |
| `POST create` p99 | 453.2 / 474.8 / 304.9 ms | 422.9 / 425.2 / 442.1 ms | 266.4 / 286.7 / 622.3 ms | 395.0 / 411.8 / 6,208 ms | 424 ms | 1,244 ms |
| `GET list` p95 (a few items) | 86.6 / 82.8 / 91.9 ms | 91.1 / 85.6 / 84.6 ms | 57.2 / 46.7 / 44.4 ms | 55.8 / 88.6 / 79.1 ms | 86 ms | 75 ms |
| `GET activity` p95 | 72.9 / 70.8 / 78.7 ms | 78.4 / 69.0 / 71.0 ms | 33.7 / 38.4 / 32.4 ms | 39.6 / 32.0 / 37.0 ms | 89 ms | 65 ms |
| create -> activity visible (n=40, fast path on) p50 · p95 | 1,352 · 1,615 / 1,317 · 1,609 / 1,325 · 1,717 ms | 1,336 · 1,636 / 1,344 · 1,776 / 1,356 · 1,919 ms | 1,289 · 1,726 / 1,275 · 1,552 / 1,272 · 1,650 ms | 1,269 · 1,643 / 1,334 · 1,552 / 1,262 · 1,537 ms | 1,392 · 1,842 ms | 1,359 · 2,270 ms |
| first create of the run (includes seeding the fixture) | 694 / 420 / 497 ms | 401 / 547 / 1,155 ms | 566 / 436 / 369 ms | 482 / 387 / 439 ms | n/a | n/a |

- **Next to stage 2's 296 and 559 ms, the Postgres cell now reads 176-198 ms at c=1 and 300-381 ms at c=8 (p95)**, and 125-135 and 174-190 ms at p50 (stage 2: 143 and 282). The comparison is across days, code and vantage (stage 2's vantage was the same kind of US laptop, not necessarily the same colo); it is next to the numbers, not an attribution. Section 9 isolates FR-22 by building the pre-FR-22 shape and running it the same hour.
- At c=8 the create p50 is 1.3 to 1.5 times its c=1 value on `worker-pg` (stage 2: 2.0) and 1.2 to 1.5 on `worker-do`; at p95 the ratio is 1.6 to 2.2 on `worker-pg` (stage 2: 1.9) and 1.6 to 2.9 on `worker-do`.
- The delivery latency (create to activity visible, p50 about 1.3 s) is the same on both cells: it is the queue and the consumer, not the engine.
- `worker-do` showed a 6.2 s p99 at c=8 in one of its three runs (the other two: 0.4 s); it was not explained.

### 6.4 R2: one object serialises the instance [tested, tested-cf]

Every request, every consumer message and every cron sweep goes through one object on one thread. On the deployed object the cost shows in the lists: 8 concurrent lists of a 1,000-item project cost p95 216 ms against 77 ms for one (2.8 times), and 8 concurrent lists of a 10,000-item project cost 1,687 ms against 350 ms (4.8 times); p50 at c=8 is 3.2 and 5.2 times p50 at c=1. Locally the same shape appears (list of 1,000 items, p95 6.2 ms at c=1 and 39 ms at c=8 on `worker-do` under `wrangler dev`, against 1.4 and 5.1 ms on `native-sqlite`). On Postgres each request serialises its JSON in its own isolate in parallel, so the lists are not serialised by the engine; that is an architectural difference between the cells, not measured on the platform.

### 6.5 Cold start [tested-cf]

The object is evicted shortly after its last activity: its counters (`rows_read`, since construction) reset between two samples ten seconds apart after the queue drained. The every-minute cron wakes it each minute.
- First request that reaches the object after at least 40 s of idle (so after the cron's wake at :00 and the eviction), `GET /_info` (admin) over 5 rounds: 100, 98, 92, 71, 112 ms; the second request: 81, 186, 86, 79, 86 ms; `/healthz` (also answered by the object): 122, 183, 90, 114, 108 ms.
- The same with an authenticated list of an empty project over 5 rounds: first 91, 117, 132, 99, 112 ms; second 101, 97, 97, 127, 102 ms.
- Verified cold once: the first call after 40 s idle to `/_test/do/usage` returned `rows_read: 4` (the constructor's migration-ledger reads only), so that call constructed the object, and took 73 ms against 90 ms for the second.

**No measurable cold cost** at this precision and from this vantage: constructing the object, checking the migration ledger and serving the request fit inside the noise of the laptop's round trip. The first DB-backed request on a fresh isolate took 595-615 ms on stage 2's Postgres through Hyperdrive (`research/0004` F7); the object has no connection to open.

### 6.5b Cold start, `worker-pg` [tested-cf]

The Postgres cell has no object to construct: every request builds its `App` and opens its Hyperdrive connection (a socket and a start-up) in whichever isolate serves it. Stage 2 measured **595 and 615 ms** for the first database-backed request on a fresh isolate (`research/0004` F7). Here: redeploy the same build five times (a new Worker version each time), then straight away `GET /healthz`, the same authenticated list of an empty project three times (3 statements each):

| Round | `/healthz` | first list | second | third |
|---|---|---|---|---|
| 1 | 103 ms | 153 | 150 | 176 |
| 2 | 99 | 209 | 98 | 213 |
| 3 | 127 | 140 | 216 | 85 |
| 4 | 143 | 127 | 158 | 208 |
| 5 | 117 | 153 | 218 | 149 |

**No measurable cold cost on the first database-backed request**: 127 to 209 ms for the first list, against 85 to 218 ms for the second and third; nothing near stage 2's 595-615 ms. Two limits: whether the first request after a deploy always reached a new isolate was not observable (the response carries no isolate or version id), and the runs are about 10 s apart, so the Hyperdrive pool was warm throughout; a Hyperdrive pool that had been idle for its 10-minute timeout was not measured.

### 6.6 Local numbers for the twins [tested]

Loopback, release client, n=200, 2 runs each (these are not Cloudflare latency).

| Target | `POST create` p95 c=1 | c=8 | `GET list` p95 (c=1) |
|---|---|---|---|
| `native-memory` | 0.11 / 0.12 ms | 0.20 / 0.24 ms | 0.23 / 0.23 ms |
| `native-sqlite` | 0.29 / 0.20 ms | 1.72 / 1.87 ms | 0.36 / 0.41 ms |
| `worker-do` (`wrangler dev`) | 6.13 / 4.51 ms | 21.06 / 18.54 ms | 4.31 / 2.33 ms |

At 100k items, `scripts/contract.sh <target> scale`, release builds, one run each:

| Target | seed rows/s | list 1,000 items p95 c=1 / c=8 | list 10,000 p95 c=1 / c=8 | search p95 (Fix / phrase / miss / `ou`) | create p95 c=1 / c=8 |
|---|---|---|---|---|---|
| `native-sqlite` | 17,536 | 1.36 / 5.12 ms | 14.1 / 49.8 ms | 1.04 / 1.27 / 0.17 / 0.28 ms | 0.22 / 2.32 ms |
| `native-pg` (Docker) | 30,377 | 2.45 / 9.09 ms | 17.1 / 37.6 ms | 1.81 / 1.44 / 1.42 / 1.43 ms | 4.59 / 20.55 ms |
| `worker-do` (`wrangler dev`) | 11,843 | 6.16 / 39.38 ms | 44.0 / 310.9 ms | 2.76 / 2.35 / 1.25 / 1.52 ms | 5.87 / 14.50 ms |

### 6.7 Hyperdrive caching on versus off [tested-cf]

Two Hyperdrive configs on the same origin and role, `thrust-spike-pg` (caching disabled) and `thrust-spike-pg-cached` (enabled, Hyperdrive's defaults), the Worker deployed against each in turn, the same laptop, `bench` n=200 and three runs per concurrency, the delivery phase skipped on the cached config (it cannot finish there, below).

| Measure (p50 · p95, ms) | caching off c=1 | caching ON c=1 | caching off c=8 | caching ON c=8 |
|---|---|---|---|---|
| `GET list` (a few items) | 66-69 · 83-92 | **47-57** · 57-78 | 67-69 · 85-91 | **50-58** · 72-77 |
| `GET activity` | 56-57 · 71-79 | **41-51** · 54-68 | 56-58 · 69-78 | **43-52** · 53-64 |
| `POST create` | 125-135 · 177-198 | 116-140 · 160-237 | 174-190 · 300-381 | 176-187 · 315-614 |

- **Caching takes 10 to 20 ms (15 to 25 percent) off the p50 of a repeated read and nothing off a write.** The bench lists the same project and reads the same activity again and again, so this is the best case for a cache; a create's own statements are writes, which are not cached, and its p50 does not move (the one read before them that can repeat, the membership, could save a round trip at most).
- **It breaks read-after-write, as Hyperdrive's documentation says it does.** With caching enabled the contract suite's main pass failed **4 of 21** (the same build passed 21 of 21 with caching off): `outbox_event_reaches_the_queue_consumer_exactly_once` and `redelivered_event_is_processed_once` (the activity row never appears within 15 s: the empty `list_activity` result is served from the cache), the access-table matrix (the list after the matrix's writes is stale: two disagreements over 504 combinations) and the generated model test against the reference model (the same cause, `never delivered within 15 s`); and the bench's create-to-activity poll timed out at 20 s on every run, against 1.3 s with caching off. `a_revoked_session_is_rejected_on_the_next_request` still passed: `session` carries `now` as a parameter, so its parameters never repeat and its result is never reused. The model test wrote a shrunk counterexample into `tests/model/mod.proptest-regressions`; it is an effect of this experiment, not a defect of the code, so it was **not** committed (AGENTS.md rule 3: the failing test is the finding, and the finding is this paragraph).
- **Consequence:** caching stays disabled for the system of record. Where a read may be stale (a list that tolerates a minute), the way Hyperdrive offers is a second, cache-enabled config bound only to those reads; nothing in the slice has such a read today.

### 6.8 Placement [tested-cf]

`placement.region = "aws:us-east-1"` (the database's region) against no placement, caching off, the same build.

| Vantage | Measure | placement on (p50 · p95, ms) | no placement |
|---|---|---|---|
| laptop in Virginia (IAD edge, 8 ms) | `POST create` c=1 | 125-135 · 177-198 | 123-149 · 160-238 |
| | `GET list` c=1 | 66-69 · 83-92 | 54-68 · 65-80 |
| | `GET activity` c=1 | 56-57 · 71-79 | 49-59 · 58-84 |

From a vantage already in the origin's region, placement changes nothing measurable (the list is 0 to 14 ms faster without it, inside the spread between runs). The distant-region result, where it matters, is in section 6.9.

### 6.9 The distant region [tested-cf]

The same `bench` (n=200, c=1 then c=8, **two runs**, delivery phase included) from a `t4g.small` in **us-west-2** (Oregon), about 65 ms from `us-east-1`, reached through an EC2 Instance Connect Endpoint (no inbound port open; the ledger is in section 11). Its connect time to the Cloudflare edge was about 6.5 ms; a `GET /healthz` to the placed `worker-pg` cost 118-177 ms to first byte from it, because the placed Worker runs in `us-east-1` (a cross-country hop for every request, whatever it does). **This is a mid-distance vantage, not an antipodal one**: it says nothing about Europe or Asia, where the hop to `us-east-1` is longer.

| Cell | `POST create` p50 · p95, c=1 | `GET list` (a few items) p50 · p95, c=1 | `GET activity` p50 · p95, c=1 | `POST create` p50 · p95, c=8 |
|---|---|---|---|---|
| `worker-pg`, placed in `us-east-1` | 197 / 200 · 275 / 295 ms | 187 / 182 · 365 / 289 ms | 131 / 135 · 160 / 163 ms | 238 / 253 · 497 / 461 ms |
| `worker-pg`, **not** placed (the Worker runs beside the user) | 738 / 665 · 871 / 758 ms | 322 / 281 · 387 / 335 ms | 307 / 268 · 370 / 317 ms | 884 / 910 · 1,422 / 1,607 ms |
| `worker-do` (one object, hint `enam`) | 147 / 153 · 180 / 272 ms | 117 / 111 · 165 / 157 ms | 89 / 85 · 106 / 121 ms | 154 / 155 · 356 / 344 ms |

Create to activity visible (c=1, p50 · p95): placed `worker-pg` 1,549 / 1,465 · 2,272 / 1,738 ms; unplaced 2,321 / 2,135 · 5,154 / 5,446 ms; `worker-do` 1,417 / 1,340 · 2,297 / 1,513 ms.

- **Placement is what makes Postgres usable from a distance.** Unplaced, each statement and each connection set-up crosses the 65 ms to the database: a list costs 281-322 ms (about 3 to 4 round trips more than from Virginia, section 6.10) and a create 665-738 ms (about 8 to 9). Placed, the Worker and Hyperdrive sit beside the database and the user pays the one hop: create 3.3 to 3.8 times faster, list 1.5 to 1.7 times, activity 2.0 to 2.4 times. From the origin's own region placement changed nothing (6.8): it trades a hop for the user for the round trips to the database, which wins whenever a request makes more than one statement.
- **The object is 40 to 70 ms faster than placed Postgres at p50 from here**: one hop to the object against one hop plus 3 to 7 statements in the origin's region. The two cells' c=8 creates are close at p95 (344-356 against 461-497 ms).
- A list p95 of 289-365 ms at this vantage on `worker-pg` is **above** 300 ms for a project of a few items; the criterion is stated from the origin's region (section 6.2b), and this vantage is reported beside it, not as the criterion.
- Not measured from here: lists of 1,000 and 10,000 items (a 5 MB response over this path), search, and the cold request.

### 6.10 Round trips per request, by reading the code [derived]

One Worker request opens **one** shared Hyperdrive connection for its reads (a socket and a Postgres start-up through Hyperdrive) and a second, exclusive one for a transaction. `list` is 3 statements: the session (`authenticate`), the membership (`authorize`), the list. `create` is 7: the session and the membership on the shared connection, then on the transaction's own connection the counter upsert (autocommit, FR-22), `BEGIN`, the work item insert, the outbox insert and `COMMIT`; plus one statement after the response (`mark_outbox_dispatched` of the fast path) and a hop to the per-user rate limiter's Durable Object. Each statement is one round trip (the typed pipeline is one `Parse/Bind/Describe/Execute/Sync` flight). This was not enforced by a counting connector (issue #39); it is consistent with the distant measurements of section 6.9, where a list costs about 3 to 4 round trips of the 65 ms between the vantage and the origin region more than it does from Virginia, and a create about 8 to 9.

## 7. Criterion 4: the migration story [tested, tested-cf]

### 7.1 Mechanism

Versioned SQL files, applied the same way on both twins: each file once, as one transaction together with the row that records it in `schema_migrations`, one statement per `exec` (the Durable Object's `exec` is documented per statement; a splitter that understands `--` comments, quoting and `CREATE TRIGGER ... BEGIN ... END` does it, with a property test). The object runs the runner in its constructor inside `blockConcurrencyWhile`: no request is delivered until it is done. A database that records a version the build does not know is refused (a contract migration must not be run under older code).

### 7.2 Expand and contract on the platform [tested-cf]

On a copy (`_ms_*` tables, dropped afterwards) of the live `work_items` table, 124,081 rows, in the deployed object, one phase per request, timed by the client over HTTP (workerd freezes `Date.now()` during synchronous SQL, so the object cannot time itself):

| Phase | What | Time |
|---|---|---|
| copy | `CREATE TABLE ... AS SELECT` | 0.85 s |
| expand | add a nullable column, backfill it (`UPDATE` of every row), index it | 0.90 s |
| fts | build the FTS5 trigram index over the copy: what migration 0006 does to a seeded database | 3.81 s |
| contract | drop the index and the column | 0.67 s |
| fail | the real runner given a migration whose third statement fails | 0.14 s |
| cleanup | drop the scratch tables | 0.83 s |

The whole story read 869,641 rows and wrote 496,364 (billed rows). Each phase is far below the 30 s a `blockConcurrencyWhile` may run. Not measured: a contract migration that rewrites a table (`ALTER ... DROP COLUMN` was the contract used).

### 7.3 A migration that fails halfway [tested, tested-cf]

- **The runner:** the real runner, given a migration of statements 1 and 2 that succeed and a third that fails (`INSERT INTO no_such_table`), returns an error that names the migration and the statement, leaves no table behind and no ledger row, and the ledger still reads versions [1, 6] (probe `fail`, on the deployed object). Hermetically: for every position of the failing statement among 1 to 6 good ones, the database is exactly as it was, the next start fails the same way, and the same migration without the bad statement applies; a runner that did not use a transaction fails that property.
- **The object:** a build with `mutant_broken_migration` (a migration whose third statement fails) deployed over the object: **every request answered 500 from the first one on, `/healthz` included** (the front Worker forwards every path, so liveness and readiness are the same call), and so did the cron sweeps. Redeploying the good build restored service in one deploy with the data and the ledger intact (131 MB, [1, 6], an authenticated list answered 200). The object stays down until the migration is fixed; there is no degraded mode. Not independently queried: that the failed migration's marker table is absent on the deployed object (the `fail` phase shows it for the real runner).

### 7.4 The Postgres cell [tested-cf]

`thrust_adapters_postgres::migrate` applies each file with one `batch_execute`, idempotent by `IF NOT EXISTS`; a multi-statement simple query is one implicit transaction. Run on the real cluster, as the real per-PR role:

- **The migrations apply as the role.** Migrations 0001 to 0004 and 0006 applied over a password login to the primary as the NOSUPERUSER role `pr_1101` (connection limit 8 at that time, role-level `search_path = pr_1101`), the files fed to `psql` on stdin (the Rust `migrate` function was not the thing run). 0006 needs `pg_trgm` in `public` (installed once by an administrator; `pg_trgm` 1.6) and the operator class qualified as `public.gin_trgm_ops`: an on-cluster control in a rolled-back transaction showed the unqualified `gin_trgm_ops` failing under the role's `search_path` and the qualified form succeeding (section 5.4, commit 7968468).
- **Expand and contract on a copy of the live table**, 132,265 rows, `psql \timing` **on the primary, over the local socket of its pod, so these are database-side times with no network in them** (the object's times in 7.2 are client-timed over HTTP and carry about 40 ms each):

| Phase | What | Time |
|---|---|---|
| copy | `CREATE TABLE ... AS SELECT` | 160 ms |
| expand | one transaction: add a nullable column (0.27 ms), backfill it with an `UPDATE` of every row (358 ms), index it (97 ms), `COMMIT` (79 ms) | 535 ms |
| trigram | the two `pg_trgm` GIN indexes (`name`, then `description`) over the copy: what migration 0006 does to a seeded database | 446 ms + 1,113 ms |
| contract | one transaction: drop the index, drop the column, `COMMIT` | 1.5 ms (a catalog change; SQLite rewrites the table, 7.2) |
| cleanup | drop the scratch table | 8 ms |

- **A migration that fails halfway changes nothing.** A single simple-query string of the adapter's `batch_execute` shape, `CREATE TABLE _ms_fail (x int); INSERT INTO _ms_fail VALUES (1); INSERT INTO no_such_table VALUES (1);`, sent as one `psql -c`, answered `relation "no_such_table" does not exist`, and the catalog afterwards held **no** `_ms_fail` (0 rows in `pg_tables`): the implicit transaction rolled the first two statements back. Unlike the object, a failed migration does not take a running service down by itself: the Worker keeps serving the previous schema until the deploy order (ADR-0110) applies the migration, and `migrate` runs before the new build, not in its constructor. What this does not show is the **concurrent** case, a migration holding locks while requests run (the 535 ms expand above holds a row lock on every row its `UPDATE` rewrites, and an exclusive lock for the `ALTER`, however briefly); the order in which a deploy applies expand and contract is ADR-0042's, and was not exercised under load.


## 8. Criterion 5: cost, projected [derived]

### 8.1 Billed rows per operation [tested-cf]

Durable Object SQLite bills rows read, rows written and stored data, besides requests and duration. `DoSql` counts `SqlStorageCursor.rowsRead` and `rowsWritten` after each statement, which is what the platform bills (indexes, and the FTS5 shadow tables, count as rows written). `/_do/cost-probe` runs the slice's use-cases against a fixture of their own with the outbox relay off so nothing asynchronous lands in a window. On the deployed object (the local run read 7.0 rows per create instead of 8.0 and was otherwise identical):

| Operation | Rows written | Rows read | n |
|---|---|---|---|
| create (membership, counter, item, outbox row, commit) | 9.025 | 7.975 | 40 |
| relay: mark the outbox row dispatched | 1.0 | 1.0 | 40 |
| consumer: insert the activity row | 4.0 | 1.0 | 40 |
| soft delete (item update, outbox row) | 4.0 | 7.0 | 10 |
| bulk-seeded item (one call of 1,000, FTS5 and indexes included) | 5.001 | 0.002 | 1,000 |
| list of the 1,030 live items | 0 | 1,046 | 1 |
| search `Fix` (3+ characters, indexed, 30 hits) | 0 | 1,086 | 1 |
| search `zzzzqq` (indexed miss) | 0 | 1,046 | 1 |
| search `ou` (2 characters, a scan) | 0 | 1,046 | 1 |

A create's whole life (create + relay + consumer) writes 14.0 rows. Stored data: 949 to 965 bytes per item with every index and the FTS5 table (local and deployed); the database was 131,067,904 bytes at 124,081 rows, about 1.06 KB per `work_items` row including the activity, outbox and fixture rows around them.

### 8.2 The projection

Vendor prices read on 2026-10-24 (https://developers.cloudflare.com/durable-objects/platform/pricing/ and https://developers.cloudflare.com/workers/platform/pricing/ **[doc]**), Workers Paid: Durable Object requests 1 M included then $0.15 per million; duration 400,000 GB-s included then $12.50 per million GB-s, billed in wall-clock time at the full 128 MB while the object is active and cannot hibernate; SQLite rows read 25 billion included then $0.001 per million; rows written 50 million included then $1.00 per million; stored data 5 GB-month included then $0.20 per GB-month.

**Assumed workload** (the one `research/0004` section 9 states, so the cells compare): 50 users x 22 days x about 4,500 API requests per user per day, about **5 M requests a month**; 500 k queue messages and the same number of mutating events a month; cron every minute; the rate limiter object called on every request (pessimistic: the slice calls it only on creates); 100k items.

| Line | Arithmetic | $/month |
|---|---|---|
| Workers Paid plan | base | 5.00 |
| Workers requests (front Worker) | 5.0 M + cron 43 k + queue batches 50 k = 5.1 M < 10 M included | 0.00 |
| Workers CPU (front Worker, forwarding only; the work is in the object and is billed as duration) | 5 M x about 2 ms = 10 M ms < 30 M included **[assumed]** | 0.00 |
| Queues | 500 k x 3 operations = 1.5 M - 1 M included = 0.5 M x $0.40 | 0.20 |
| Durable Object requests | `AppDo`: 5.0 M API + 0.5 M consumer forwards + 43 k cron = 5.54 M; limiter 5 M; (10.54 M - 1 M) x $0.15 | 1.43 |
| Durable Object duration | one 128 MB object, at most always active: 30 days x 86,400 s x 0.125 GB = 324,000 GB-s < 400,000 included | 0.00 |
| SQLite rows written | 500 k creates x 14.0 = 7.0 M < 50 M included | 0.00 |
| SQLite rows read | even if all 5 M requests were lists of a 1,000-item project: 5.2 B < 25 B included | 0.00 |
| SQLite storage | 100k items x about 1 KB + an activity and outbox history of the same order (ADR-0040 sizes activity at about 10x the items) is about 0.5 GB < 5 GB included | 0.00 |
| R2 | as `research/0004` section 9 (20 GB of attachments) | 0.15 |
| KV, Hyperdrive, Workers Logs | none used by the cell (sessions are database rows; no Hyperdrive); logs as section 9 | 0.00 |
| **Total, Cloudflare side** | | **about 6.78** |

**Ten times the load** (50 M requests, 5 M queue messages): Workers requests (50.5 M - 10 M) x $0.30 = 12.15; Workers CPU (100 M ms - 30 M) x $0.02 = 1.40; Durable Object requests (105.4 M - 1 M) x $0.15 = 15.66; rows written 70 M - 50 M included = 20 M x $1.00 = **20.00**; duration still bounded by 324,000 GB-s, 0.00; rows read 52 B - 25 B included = 27 B x $0.001 per million = 27.00 if every request were a 1,000-item list, 0.00 for a more realistic mix **[assumed]**; queues 5.60; R2 2.85; plan 5.00. About **$62.7 without the all-lists case** and about **$89.7 with it**, under the $100 envelope either way. Rows written, not requests, become the largest line at that load. The indexes, the FTS5 table among them, are part of the 5.0 to 9.0 rows written per item; the FTS5 share was not isolated.

The projection is Cloudflare's side only. The Postgres host is outside the envelope for the `worker-pg` cell (ADR-0040, round 27), and the `worker-do` cell has no Postgres. Compared with `research/0004` section 9 ($6.38 for the Postgres cell's Cloudflare pieces) the cells differ by cents at the sizing target.

### 8.3 Why it is projected, not reconciled

Spike 0002 asks for a 24-hour run per cell reconciled against the account's usage, as `research/0004` F8 did not. The owner skipped it on 2026-10-24: there is no real traffic, so a day of cron-only load would reconcile the idle floor (one invocation a minute) and nothing about the billed rows, which are what matter, and the rows-per-operation above were measured directly from the platform's own counters. So criterion 5 is a **projection from measured billed rows per operation and the vendor prices, with an assumed workload**. It has not been reconciled against an invoice or the usage analytics. The cell was deployed from 2026-10-23T23:39Z to 2026-10-24T01:40Z (about two hours, the measurement load included); the account's analytics for that window were not read.

What the projection is not: a capacity figure. The binding constraint at this load is not the bill but the one object's thread (6.4): at 5 M requests a month an object sees about 6 requests a second on average if the load is spread over 10 working hours a day (**[assumed]**), which a 5 ms list handles (about 200 a second) but a 41 ms unpaged 10,000-item list (about 24 a second) would not at ten times the load.

## 9. FR-22: the sequence allocated outside the create transaction [tested, tested-cf]

Implemented in `thrust-adapters-postgres`: a `PgTx` checks its connection out at `begin` but opens no transaction until its first write; `allocate` runs before that, as one statement in autocommit, so the counter row's lock is held for one round trip instead of through the insert, the outbox insert and the commit. A second `allocate` in a transaction that is already open takes its number on another connection (on this one the upsert would join the transaction); a transaction that wrote nothing needs no `COMMIT`. Statements per create are unchanged.

The price, accepted by ADR-0043 round 29: a number taken by a create that then fails or is abandoned is not given back. Three Postgres adapter tests restated the old property ("a rollback leaves no gap") with the reason in the commit message (AGENTS.md, rule 3): `rollback_is_atomic_and_unique_violation_is_a_conflict` (the next number is 3, not 2), `an_abandoned_transaction_does_not_poison_the_pool` (the abandoned transactions now write an item and the next number is 41; it still kills `mutant_discard_noop`) and the generated `model_postgres_adapter_matches_the_reference_model` (counters advance on every end of a transaction; a new step class is required by the coverage control; the shrunk counterexample is pinned). The SQLite twin writes its counter at commit and keeps the no-gap property.

**Measured on `native-pg`, release bench, n=200, 3 runs per arm [tested], before the change then after:**

| Arm | `POST create` p95 c=1 | `POST create` p95 c=8 |
|---|---|---|
| loopback, before | 6.14 / 2.43 / 4.92 ms | 20.98 / 36.88 / 21.56 ms |
| loopback, after | 5.43 / 2.87 / 4.12 ms | 19.98 / 10.71 / 19.18 ms |
| 5 ms one-way delay proxy (RTT 10 ms), before | 106.2 / 92.1 / 93.0 ms | 343.7 / 360.1 / 357.6 ms |
| 5 ms one-way delay proxy, after | 93.1 / 93.0 / 96.2 ms | 99.6 / 96.6 / 104.5 ms |

p50 at c=8 with the proxy: 225 / 215 / 230 ms before, 90 / 88 / 91 ms after; the list and the rest did not move.
- On loopback the lock is held for microseconds and the spread inside an arm is larger than the difference between arms: **no effect is visible**.
- With the proxy, before the change c=8 was 3.7 times c=1 (stage 2's shape: 296 and 559 ms); after, c=8 is within about 10 percent of c=1. This confirms the mechanism ADR-0043 round 29 attributed the c=8 cost to.
- **The proxy is a stand-in, not the platform** (`scripts/pg-delay-proxy.py`, `THRUST_PG_LATENCY_MS=5`, a fixed delay each way with no jitter, no Hyperdrive pooling, no tunnel). The platform result is the next subsection.

### 9.1 On the platform: the same hour, before and after [tested-cf]

The cleanest comparison the run allowed: the pre-FR-22 shape was rebuilt for one deployment (a temporary two-line change to `thrust-adapters-postgres`, never committed: `begin` opens the transaction at once and `allocate` runs inside it, so the counter row is locked through the insert, the outbox insert and the `COMMIT`), deployed to the same Worker, Hyperdrive config and database (2026-10-24T03:49Z), and run three times with the same laptop and the same `bench`; the committed code (FR-22) was redeployed (03:59Z) and run twice more, so that the FR-22 arm brackets the pre-FR-22 arm in time (three runs at 02:43-02:52Z, two at 04:00-04:05Z).

| `POST create` | c=1 p50 | c=1 p95 | c=8 p50 | c=8 p95 | c=8 p99 |
|---|---|---|---|---|---|
| pre-FR-22 (3 runs) | 125.5 / 119.9 / 123.3 ms | 172.6 / 156.6 / 242.8 ms | 207.3 / 219.0 / 202.8 ms | 346.9 / 726.8 / 331.5 ms | 495 / 1,299 / 420 ms |
| FR-22 (5 runs, in time order) | 124.8 / 128.1 / 134.5 / 120.9 / 117.5 ms | 198.4 / 186.4 / 176.5 / 145.5 / 148.9 ms | 189.7 / 174.3 / 176.4 / 169.3 / 178.1 ms | 341.5 / 300.0 / 380.7 / 404.9 / 558.0 ms | 423 / 425 / 442 / 545 / 918 ms |

- **At c=1 there is no difference** (p50 about 123 ms both ways).
- **At c=8 the p50 is about 15 percent lower** (mean 210 ms before, 178 ms after; every FR-22 run is below every pre-FR-22 run) and the c=8 to c=1 ratio of the p50 falls from 1.7 to 1.4. **The p95 and p99 are not distinguishable**: the ranges overlap and one pre-FR-22 run (727 ms) and one FR-22 run (558 ms) are outliers.
- **The delay-proxy result does not reproduce.** On the proxy c=8 fell to within about 10 percent of c=1; on the platform it is still 40 percent above it. The mechanism ADR-0043 round 29 named (the counter lock held through the transaction) is real and removing it helps, but it is not the only cost of a concurrent create. Candidates, **not tested**: the eight concurrent creates of one user go through one per-user rate-limiter Durable Object; each create needs two Hyperdrive connections and the pool is 8, so eight concurrent creates ask for sixteen; the laptop's own link.
- **Next to stage 2's 296 and 559 ms (p95)** the Postgres cell reads 145-198 ms at c=1 and 300-558 ms at c=8 with the FR-22 order, and 157-243 ms and 332-727 ms without it, today. So most of the difference from stage 2 is not FR-22: it is the day, the build and the vantage.
- **What this says about ADR-0043's price.** The gap a failed create leaves in the sequence (accepted by the ADR) buys a p50 improvement of about 15 percent at c=8 on this vantage and nothing at c=1. The order is confirmed correct and harmless; the benefit is smaller than the proxy suggested. Whether it is worth its gaps is for the owner and ADR-0043 to weigh; this note changes neither.

## 10. The other findings

1. **The outbox fast path loses events when the object is evicted [tested-cf].** After the engine pass, 100 of about 10.5k outbox rows were undispatched (a constant count of exactly 100 for a minute), and were delivered by the every-minute cron sweeper within about a minute. `waitUntil` in a Durable Object does not keep an evicted object running, so a relay future in flight at eviction is lost; the row stays and the sweeper retries, which is the design of ADR-0046, and what the engine pass drained in 10.5k events was delivered. The delivery latency tail (create to activity visible p95 about 2 s, 10 s while a backlog drained) is the queue plus the consumer forwarding through the object.
2. **`/healthz` goes through the object [tested-cf].** The front Worker forwards every path; a failed migration makes health checks fail (7.3). That is readiness, not liveness.
3. **Cron wakes the object every minute [tested-cf].** Each wake costs a few seconds of wall-clock duration (the object is evicted within about 20 s), which is why the duration line of the projection is bounded by the always-active ceiling, not by traffic.
4. **The Durable Object refuses `sqlite_version()` and `PRAGMA compile_options` [tested-cf]**, so the SQLite version in the platform is unknown; capabilities are probed by use (5.1).
5. **A 3-character indexed search reads O(project) rows (5.4); SQLite's LIKE folds ASCII only (5.4); `pg_trgm` needs an administrator and a qualified operator class (5.4); a 100-connection Postgres failed 1 of 50 writers on `worker-cf` locally (4).**
6. **argon2id sign-in runs inside the one object and blocks it for the length of the hash [tested, local workerd; the platform figure is derived].** The auth slice (#38) put argon2id at OWASP's minimum (m = 19,456 KiB, t = 2, p = 1) in the sign-in path, and on `worker-do` every request goes through the one object. Measured under `wrangler dev` (local workerd, this laptop's CPU, the real cost, no override): one sign-in costs the object **26 ms** (p50 of 20; seeding one login, one hash, 46 ms); a burst of 8 concurrent sign-ins finishes its first at 27 ms and the other seven together at 194-200 ms (7 x 26 ms and a little); and a `/healthz` request issued while the burst runs, which the object also answers, **waited 192-198 ms against 1.5 ms idle**: the object serves nothing else while a hash runs. On the platform one sign-in costs 45 ms of CPU at p50 and 78 ms at p99 on a plain Worker (`research/0012` section 4, 3.4 to 3.9 times native), so in the object a burst of 8 would hold the instance for about 0.36 s at p50 **[derived, not measured on the platform]**; on Postgres the same burst runs in as many isolates as the platform gives it. It is a reported cost of the `worker-do` cell, not a pass or fail: 50 users signing in about once a day (`research/0012`) is far below it, and the contract suite runs the cell with a cheap cost (64 KiB, t = 1) so that its 37 tests do not spend minutes hashing.

7. **A per-PR role's connection limit and Hyperdrive's pool must be sized together [tested-cf].** `pr-schema.sh` creates `pr_<n>` with `CONNECTION LIMIT 8`, and the Hyperdrive config was created with an origin connection limit of 8 (Hyperdrive's minimum is 5). A second config on the same role (the caching comparison) found every slot taken: the database logged `FATAL 53300 too many connections for role "pr_1101"` for each connection it tried, and the Worker answered **500 to every request, the seed included**. The first config's pool had been idle for 2.5 minutes; Hyperdrive keeps an idle origin connection for 10 minutes (its documented idle timeout), so the slots would have come back at about the 10-minute mark, and the pool limit is a soft one (it may exceed its limit during network trouble). For the experiment the role's limit was raised from 8 to 20 (ledger, 02:56Z); the finding for the per-PR tooling is that a role's limit must be at least the sum of the limits of the Hyperdrive configs bound to it plus room for an administrator's `psql`, and that eight is exactly one config and nothing else. The 50-writer test and the c=8 bench ran with the limit at 8 and one config without a failure: Hyperdrive queued the statements.
8. **A server error through the Postgres adapter reads `db error` [tested-cf].** `DbError::Backend(e.to_string())` of a `tokio_postgres` server error prints only `db error`; the SQLSTATE and the message are in `e.as_db_error()` and are dropped, so the Worker's log said `internal error: db: db error` for a `53300`, and the cause was found in the primary's log. A product finding for the adapter (include the code and message in `Backend`, as `db_err` already does for the unique violation); not changed here, so that the deployed code stays the code that produced the numbers.

9. **The one real email send [tested-cf].** A Worker version with the `EMAIL` binding (`[[send_email]]`) and two variables set at deploy time, the sender (an address on the account's Email-Sending-enabled domain `<sender-domain>`) and the verified destination `<verified-destination>` (neither committed; the test prints their lengths, not the addresses), was asked **once** for a real send by the test `stage2_email_real_send_once` (`scripts/stage2-contract.sh email`). The control ran first and sent nothing: a sender on `example.com` was **refused** by the deployed binding with `email from example.com not allowed because domain is not owned by the same account` (the text `research/0004` F9 recorded), so the pass says something about domain ownership. The real send returned `sent: true` and the message id `<AM1srsfECkGwrNQBE1EOj7xJeMQ61JfYrMoz@<sender-domain>>` at 2026-10-24T03:40:01Z; exactly one message was sent (the ledger has one send; the control was refused, and the pass was not re-run). **Delivery to the mailbox was not verified**: the binding accepted the message and returned an id; whether it arrived is the owner's to confirm. The send went through the Postgres cell's Worker, since it does not depend on the engine.
10. **The earlier draft of this note said no client ran near the origin; that was wrong about the laptop.** `wrangler tail` showed this laptop on the IAD colo at 8 ms, in the origin's region, so its numbers are the origin-region numbers criterion 3 asks for (for `worker-pg`; for `worker-do` the same laptop and network, colo not recorded). The distant numbers are the new ones (section 6.9).

## 11. Teardown ledger

Everything the deployed runs created, with UTC times from the runs' own ledgers (`target/spike-do/ledger.txt` and `target/spike-pg/ledger.txt`, written by `scripts/spike-0002-do.sh` and `scripts/spike-0002-pg.sh`, and the bench host's ledger). Account ids, IP addresses, mailbox addresses and resource ids are not copied here.

### 11.1 `worker-do`, first deployment (2026-10-23 and 24)

| Time | Event |
|---|---|
| 2026-10-23T23:39:47Z | created R2 bucket `thrust-spike-do-blobs` |
| 23:39:49Z | created queue `thrust-spike-do-events-dlq` |
| 23:39:51Z | created queue `thrust-spike-do-events` |
| 23:40:04Z | deployed Worker `thrust-spike-do` (with its cron, the `APP` and `RATE_LIMITER` Durable Object bindings, the consumer of the queue) |
| 23:40:05Z | put the Worker secret `THRUST_ADMIN_TOKEN` (from a 0600 file; never printed) |
| 23:42:07Z | redeployed with `THRUST_CREATE_RATE_LIMIT=1000000` for the engine pass |
| 2026-10-24T00:12:31Z | deployed the broken-migration build (the failure demonstration) |
| 00:13:06Z | redeployed the good build |
| 01:40:09Z | removed the queue consumer `thrust-spike-do` of `thrust-spike-do-events` |
| 01:40:26Z | deleted the Worker (with its Durable Object storage, cron and secret) |
| 01:40:27Z / 01:40:28Z | deleted the queues `thrust-spike-do-events` and `thrust-spike-do-events-dlq` |
| 01:40:29Z | deleted the R2 bucket `thrust-spike-do-blobs` |

The order is the one stage 2 found (research/0004 section 10.7): consumer, Worker, queues, then the bucket. R2 deletes only an empty bucket, and this one was deleted; the contract tests remove the attachments they store, and whether `/_test/bindings/r2purge` (stage 2's helper) was needed was not recorded.

**Inventory before** (wrangler and the API, read-only, before anything was created): Workers `art-filestore`, `thrust-spike-auth`, `thrust-spike-auth-tail`, `workflows-starter-template`; Durable Object namespaces `workflows-starter-template_WorkflowStatusDO`; queues `mergify-foo`; no `thrust-spike-do-blobs` bucket (`wrangler r2 bucket list` is capped at 20 names, so the bucket was checked by name). The `thrust-spike-auth` Workers belong to issue #38 and were not touched.

**After**, on 2026-10-24: `scripts/spike-0002-do.sh status` shows no queue and no bucket under the prefix; `wrangler deployments list` answers "This Worker does not exist on your account" (code 10007) and the `workers.dev` URL answers 404. The owner's coordinator verified the account through the Cloudflare API and wrangler after the teardown and reported that only `art-filestore` and `workflows-starter-template` Workers, only the `mergify-foo` queue and the `workflows-starter-template` Durable Object namespace remain; that check was not repeated here.



### 11.2 `worker-pg` (2026-10-24), in the order research/0004 section 10.7 found for teardown

| Time | Event |
|---|---|
| 02:23:48Z | created the role and schema `pr_1101` on `thrust-pg-dev` (`pr-schema.sh create 1101`, NOSUPERUSER, connection limit 8, role-level `search_path`); `pg_trgm` 1.6 was already installed in `public` |
| 02:25:35Z to 02:25:38Z | applied migrations 0001, 0002, 0003, 0004 and 0006 as `pr_1101` over a password login |
| 02:26:15Z / 02:26:17Z | created the Hyperdrive configs `thrust-spike-pg` (caching disabled) and `thrust-spike-pg-cached` (caching enabled), origin connection limit 8, VPC service `thrust-pg-dev`; the first create was **not** refused (Hyperdrive Write present) |
| 02:26:53Z to 02:26:57Z | created the R2 bucket `thrust-spike-pg-blobs` and the queues `thrust-spike-pg-events-dlq` and `thrust-spike-pg-events` |
| 02:27:14Z | deployed the Worker `thrust-spike-pg` (placement `aws:us-east-1`, cron every minute, the queue consumer, the `RATE_LIMITER` Durable Object, the `EMAIL` binding) and put its admin secret from a 0600 file |
| 02:29Z to 04:07Z | redeployed it twelve times: the high create rate limit for the engine pass and the benches (02:29); bound to the caching-enabled config (02:53, and 03:06 with the default limit for the main pass); without placement (03:08); with the two email variables (03:39); with the **pre-FR-22 build** (03:49) and back to the committed code (03:59); five redeploys of the same build for the cold-start probe (04:07) |
| 02:56:29Z | raised the connection limit of `pr_1101` from 8 to 20 (`ALTER ROLE`, superuser): the second Hyperdrive config could not connect (section 10, item 7). The role is dropped below |
| 03:40:01Z | **the one real email send** (control refused first); section 10, item 9 |
| 04:08:15Z | emptied the R2 bucket through the Worker's binding (172 objects left by the contract tests) |
| 04:08:30Z | removed the queue consumer `thrust-spike-pg` of `thrust-spike-pg-events` |
| 04:08:32Z | deleted the Worker (with its Durable Object storage, cron and secret) |
| 04:08:33Z / 04:08:34Z | deleted the queues `thrust-spike-pg-events` and `thrust-spike-pg-events-dlq` |
| 04:08:36Z | deleted the R2 bucket `thrust-spike-pg-blobs` |
| 04:08:37Z / 04:08:39Z | deleted the Hyperdrive configs `thrust-spike-pg` and `thrust-spike-pg-cached` |
| 04:08:40Z | dropped the role and schema `pr_1101` (`pr-schema.sh drop 1101`) |

At the end the schema held 142,010 `work_items` rows (93 MB with its indexes), 34,217 `outbox` rows (24 MB) and 10 MB of `activity`.

### 11.3 `worker-do`, second deployment (2026-10-24), for the same-day laptop run and the distant run

| Time | Event |
|---|---|
| 03:30:35Z to 03:30:54Z | created the bucket and queues again, deployed `thrust-spike-do` with the high create rate limit, put the admin secret |
| 04:05:52Z | removed the queue consumer |
| 04:05:54Z | deleted the Worker |
| 04:05:55Z / 04:05:56Z | deleted the queues |
| 04:05:58Z | deleted the R2 bucket (the purge through the binding reported 0 objects: this deployment stored no attachments) |

### 11.4 The distant bench host (us-west-2)

One `t4g.small` (Amazon Linux 2023, arm64, `Project=thrust-spike`, `TTL=3h`, self-terminating at the end of 3 hours), reached through an EC2 Instance Connect Endpoint; a first attempt over a security group opened to the laptop's address timed out, so that rule was removed and the endpoint used (no inbound rule from any address remains). Created 02:02Z to 02:13Z: an ed25519 key pair, two security groups (the host's, SSH only from the endpoint's group, and the endpoint's), the instance, the endpoint. Binaries copied 02:44:59Z. **Torn down by this run at 04:06:38Z to 04:07:01Z**: instance terminated, endpoint deleted, both security groups deleted, key pair and local private key deleted. `bench-host.sh status` afterwards: the instance listed as `terminated`, no endpoint, no security group, no key pair.

### 11.5 Verification, 2026-10-24T04:09Z, after everything above [what was verified]

- **Cloudflare API** (the `thrust-spikes` token, read): Workers scripts `art-filestore` and `workflows-starter-template` only; Durable Object namespaces `workflows-starter-template_WorkflowStatusDO` only; queues `mergify-foo` only; Hyperdrive configs `thrust-pg` and `thrust-pg-dev` only (both pre-existing and not this run's: no `thrust-spike*` config remains).
- **wrangler** (OAuth): `queues list` shows `mergify-foo` only; `r2 bucket info` answers "bucket does not exist" for `thrust-spike-pg-blobs` and `thrust-spike-do-blobs` (`r2 bucket list` is capped at 20 names, so the buckets were checked by name); `deployments list` answers "This Worker does not exist" for `thrust-spike-pg` and `thrust-spike-do`; both `workers.dev` URLs answer 404 on `/healthz`.
- **`thrust-pg-dev`** (psql on the primary as the administrator): **0 roles and 0 schemas** named `pr_*`, 0 connections from them, extensions `pg_trgm 1.6` (kept, as asked) and `plpgsql`.
- **`thrust-spike-auth` and `thrust-spike-auth-tail`** (issue #38's Workers) were listed in the inventory before the first run and are **no longer present**; this run did not touch them.
- **Not verified:** that no Durable Object namespace of `thrust-spike-pg` or `thrust-spike-do` lingers beyond the namespaces listed (the listing shows only the Workflows one); that nothing was billed beyond the projection; the EC2 console (the CLI listings above are what was checked); that the one email reached its mailbox.

## 12. What was NOT measured

- **The 24-hour cost reconciliation**, both cells (skipped by the owner, 8.3). Criterion 5 is a projection for both.
- **Any vantage beyond us-west-2.** Europe, Asia and the antipodes are not measured; the distant numbers are two runs from one host. From that host the lists of 1,000 and 10,000 items (a 5 MB response over that path), search and the cold request were not run on either cell.
- **The Postgres migrations through the Rust `migrate` function** (the files were applied with `psql` as the role, and the failing migration was a `psql -c` of the same simple-query shape), **a migration under concurrent load** (the expand's row locks while requests run), and a contract migration that rewrites a table, on either engine.
- **Why a concurrent create on `worker-pg` still costs 1.4 times a sequential one** after FR-22 (9.1): the candidates (the per-user rate-limiter object, a Hyperdrive pool of 8 for sixteen requested connections, the laptop link) were not tested, and neither was a different origin connection limit.
- **Hyperdrive caching** other than on or off with the default `max_age`; and the arrangement Hyperdrive documents for tolerating stale reads, a second cache-enabled config bound only to those reads.
- **A Hyperdrive pool that had been idle for its 10-minute timeout** (the cold-start probe ran with a warm pool, 6.5b), and the order of magnitude of a fully cold first request.
- **The round-trip budget enforced** by a counting `Connector` in the contract suite (issue #39): the counts in 6.10 are from reading the code, checked only for consistency against the distant deltas.
- **That the email reached the mailbox.** The binding accepted one message and returned an id; bounce handling, volume and delivery latency were not looked at.
- **A project of 100k items in one list**, `--panic-unwind`, workers-rs behaviour under a Durable Object that is evicted mid-request, and any sharding of the object (one object per workspace was not built).
- **The 50-writer controls on the deployed Postgres Worker** (they ran locally, 4.1).
- **The Postgres host's cost and the Hyperdrive and VPC service costs**: outside the envelope by ADR-0040 round 27, and not accounted here.
- Anything beyond one client, one run at a time, 200 requests per bench cell (two runs on the distant vantage, three elsewhere).
- **The deployed numbers were not re-taken on the code after the reconcile onto the auth slice** (section 14): the cells were torn down before it.

## 13. Reproduce

On the merged tree (Postgres and engine-agnostic parts):

```
scripts/contract.sh <target> engine               # 4 tests; native-memory native-pg worker-memory worker-cf
scripts/contract.sh <target> scale                # 100k items: list, search, create
THRUST_PG_LATENCY_MS=5 scripts/contract.sh native-pg bench    # the delay proxy (needs uv)
DOCKER_HOST=unix://$HOME/.orbstack/run/docker.sock scripts/mutants.sh   # DOCKER_HOST only where the socket is not /var/run/docker.sock
# deployed Postgres cell (needs CLOUDFLARE_ACCOUNT_ID, HYPERDRIVE_API_TOKEN with Hyperdrive Write, THRUST_DEV_VPC_SERVICE_ID,
# an AWS_PROFILE that reaches the cluster, and wrangler's OAuth login):
scripts/spike-0002-pg.sh role && scripts/spike-0002-pg.sh migrate && scripts/spike-0002-pg.sh hyperdrive && scripts/spike-0002-pg.sh resources
scripts/spike-0002-pg.sh deploy --var THRUST_CREATE_RATE_LIMIT:1000000      # also --cached, --no-placement, --var THRUST_EMAIL_FROM:... --var THRUST_EMAIL_TO:...
THRUST_BASE_URL=https://thrust-spike-pg.<subdomain>.workers.dev THRUST_ADMIN_TOKEN_FILE=target/spike-pg/admin-token sh scripts/stage2-contract.sh main   # also engine, email (sends one mail)
THRUST_BENCH_SKIP_DELIVERY=1 THRUST_EXPECT_CREATE_LIMIT=1000000 THRUST_BASE_URL=... THRUST_ADMIN_TOKEN=... target/release/bench 200 8   # the skip is for a caching-enabled config
THRUST_SPIKE_PG_URL=https://thrust-spike-pg.<subdomain>.workers.dev scripts/spike-0002-pg.sh down
```

On the tag `spike/0002-worker-do` (the archived `worker-do` cell and SQLite adapter; `git checkout spike/0002-worker-do`):

```
scripts/contract.sh native-sqlite main            # 37 of 37 (21 before the auth slice, section 14)
scripts/contract.sh worker-do main                # 37 of 37 (wrangler dev)
scripts/contract.sh <target> engine               # also native-sqlite and worker-do
scripts/contract.sh worker-do do                  # 5 tests: Tx control, FTS5 probe, cost probe, migration story
scripts/mutants.sh                                # 17 mutants KILLED, 2 guard checks REFUSED (the SQLite rows are on the tag)
scripts/spike-0002-do.sh up                       # deployed (needs CLOUDFLARE_ACCOUNT_ID and wrangler's OAuth login)
THRUST_BASE_URL=https://thrust-spike-do.<subdomain>.workers.dev THRUST_ADMIN_TOKEN_FILE=target/spike-do/admin-token \
  THRUST_EXPECT_ADAPTERS=durable sh scripts/stage2-contract.sh main    # also do, engine
THRUST_SPIKE_DO_URL=https://thrust-spike-do.<subdomain>.workers.dev scripts/spike-0002-do.sh down
```

## 14. The reconcile onto the auth slice (2026-10-24, after the deployed runs and the teardown)

**The deployed numbers in this note were taken before this step and were not re-taken.** `origin/main` moved under the branch while the deployed runs were going: the auth slice (#38, PR #76) and its follow-ups (#78) merged (squashes `453226c` and `1de2dac`). Both cells were already torn down, so the numbers belong to the code of the branch's commits up to `c9d52da`, which had no sign-in, API keys or OIDC; the deployed `worker-do` and `worker-pg` never ran the 16 auth tests (the main pass was 21 tests then, 37 now).

**How.** `origin/main` was merged into `feat/engine-cells` with a merge commit, not rebased: the branch is pushed and the owner has said there is to be no history rewrite and no force-push (the repository is moving to squash-only merges, so the merge commit does not reach `main`). Seventeen files conflicted; every conflict was adjacency, resolved by keeping both sides: the Cargo manifests and `Cargo.lock` (regenerated by the build), the Postgres adapter (migrations 0005, 0006 and 0007 now apply in that order: main left 0006 free for the search migration, the numbering both sides had agreed on), the Postgres model test (main's 81 step classes plus the one this branch added: 82), `thrust-api` (this branch's search and bulk-seed routes beside main's auth routes), the contract-tests `User`, the Worker's adapter-set builder, `scripts/contract.sh`, `scripts/mutants.sh` and `scripts/stage2-contract.sh` (this branch's engine, do and scale passes and its adapters guard beside main's `csrfoff` pass and its `csrf_enforce` guard), the testing docs (defect ids X21 to X24 and X29 are main's, X25 to X28 and the new X30 are this branch's; gap ids G-12 and G-14 are main's, G-13 is this branch's), `PLAN.md` and the spikes README.

**Ported to the SQLite twin.** Migrations `0005_identity.sql` (users, accounts, api_keys, oidc_logins: same tables, columns and constraints, in the dialect of 0001; only `uuid`, `bytea`, `timestamptz` and `boolean` columns, so nothing needed a SQLite-specific spelling) and `0007_api_key_hash_version.sql` (a plain `ADD COLUMN` with a named CHECK, since this runner applies each file once); the `Database` methods `revoke_user_sessions`, `user_by_email`, `user_for_account`, `link_account`, `insert_api_key`, `api_key`, `touch_api_key`, `rehash_api_key` (a compare-and-set), `list_api_keys`, `revoke_api_key`, `insert_oidc_login`, `take_oidc_login`, `set_user_active` and `seed_users`; `session` now requires an existing, active user; `seed` creates the fixture's users by id. `AppDo` builds the new `Ports` fields (the `Rng`, the argon2id hasher, the optional OIDC provider) and reads the same `CoreConfig` as the Worker's other sets (CSRF enforcement, the argon2id cost, the sign-in limit). `scripts/check-deps.sh` passes with `app_do.rs` in the mint allowlist. The cell configs carry `THRUST_CSRF_ENFORCE` (off in the local `wrangler.do.toml`, as the production default; on in the two stage templates, as `wrangler.stage2.toml`).

**The test of the port.** `thrust-adapters-sqlite/tests/identity.rs` is the memory adapter's identity matrix (183 cases, exhaustive over its finite domains) run against `SqliteDatabase`: it passed on the first run, and with the `is_active` joins of `session` and `api_key` removed 4 of its cases fail (the control). The Postgres adapter is held to the same answers by main's generated sequences.

**What the merged tree does** (all local; nothing here was deployed):
- `scripts/contract.sh native-sqlite main` **37 of 37** (including the credential matrix, the CSRF tests, sign-in, API keys and the six OIDC tests against the mock provider); `worker-do main` (`wrangler dev`, the SQLite adapter inside the one Durable Object) **37 of 37**; `native-pg main` 37 of 37; `scripts/contract-all.sh` **18 of 18** cells; `worker-do` `csrfoff` 6 of 6, `do` 5 of 5 and `engine` 4 of 4.
- `cargo nextest run`: 1,256 tests passed, 4 skipped (the SQLite adapter's 238 among them); the Postgres adapter's integration tests (`pg_adapter`, Docker): 13 of 13, the model test and migration 0005's idempotence included; `cargo check` for `wasm32-unknown-unknown` of `thrust-worker`, with and without its default features.
- `scripts/mutants.sh` (with `DOCKER_HOST` set to OrbStack's socket, or the Postgres mutant is a vacuous kill): exit 0; **17 mutants KILLED** (this branch's eight: the memory and SQLite duplicate-number builds against the engine pass, the SQLite one also against the main pass, the removed transaction gate, the case-sensitive search, and the SQLite adapter's own non-atomic commit, gate leak and duplicate-number builds; main's eight identity and CSRF mutants; the Postgres discard) and **2 CSRF guard checks REFUSED** (exit 5, no test ran).
- **argon2id inside the one object** (section 10, item 6): at the real cost a sign-in costs the object 26 ms locally and the object serves nothing else while it runs; the contract suite runs the cell with a cheap cost, so its passes say nothing about that.

**Not verified, and affecting the earlier sections:** the SQLite adapter's identity code was never run on a deployed Worker (so the platform's rows-per-request counts of section 8.1 predate it: an authenticated request now also reads one `users` row through the session join [derived]; the local cost probe reads 6.975 and writes 9.025 rows per create, against 7.975 and 9.025 deployed, a difference of one read that was not chased); the `worker-do` and `worker-pg` latency numbers do not include the sign-in path; and the cost projection was not recomputed.

## 15. The decision (owner, 2026-10-24) and what was archived

**Decision.** Criterion 3 is read on the realistic paged list (a 1,000-item project in a database of about 100k items): both cells pass; the unpaged 10,000-item list is a slice artifact that both engines fail, and becomes a pagination requirement of the list endpoint (issue #58). A projected cost satisfies criterion 5: both cells pass as projected. Q-0040.6: **Postgres is the default**; ADR-0040 is `accepted` as Postgres through Hyperdrive (ADR-0040, "Decision — spike 0002", which carries the evidence, the case for `worker-do` as a viable fallback and why it was not chosen). Q-0040, Q-0040.1, Q-0040.6, Q-0006.6 and Q-0083.1 are answered in QUESTIONS.md.

**What Postgres staying the default means for the open work.** The authenticated-TLS item (FR-09, issue #41), the cluster coupling (FR-10, issues #42 and #57) and the CNPG infrastructure (ADR-0094) are **not moot**: they were the costs the Durable Object cell would have removed. The Postgres follow-ups the spike found stand and are recorded, not changed here: the 8-connection per-PR role cannot host two Hyperdrive configs (section 10, item 7); the adapter's error reads `db error` and drops the SQLSTATE and message (item 8); Hyperdrive caching stays off (section 6.7: 4 of the 21 contract tests then in the main pass fail on read-after-write); and `worker-pg` placement must be `us-east-1` (section 6.9: 665 to 738 ms for a create from a distant client without it, 197 to 200 ms with it).

**What was archived, and where.** The tag `spike/0002-worker-do` (an annotated tag on commit 021dc65 of branch `feat/engine-cells`, pushed to `aRustyDev/thrust`) holds the complete `worker-do` and SQLite state with these docs: `crates/thrust-adapters-sqlite` (the adapter, migrations 0001, 0005, 0006, 0007, tests, mutants), `AppDo` and the `durable` adapter set of `thrust-worker`, `DoSql` in `thrust-adapters-cloudflare`, `wrangler.do.toml` and `wrangler.do.stage.toml`, `scripts/spike-0002-do.sh`, the sqlite arm of `thrust-server`, the `native-sqlite` and `worker-do` targets and the `do` pass, and the SQLite rows of `scripts/mutants.sh`. The merged tree keeps what is engine-agnostic or Postgres: the engine pass and the 50-writer test (with the memory adapter's mutants), the search route and the bulk-seed route with Postgres migration 0006, the memory implementation, the FR-22 allocation order, `bench_scale`, `scripts/pg-delay-proxy.py`, `scripts/spike-0002-pg.sh` and the Email probes with their one-send test, this note, the spike record and the ADRs. The repository squash-merges, so the squash message lists the branch's commits, SQLite ones included: they are the archived work.

**What the archived cell's numbers are, and are not.** Its deployed figures (sections 4 to 8, 6.9 and the `worker-do` columns) were taken on 2026-10-23 and 24 **before the auth slice (#76, #78) was merged into the branch**, and its SQLite identity code (section 14) **never ran on a deployed Worker**; it ran only under `wrangler dev`. The `worker-pg` figures were taken on the same pre-merge code and were not re-taken either.

