# Research 0011: database engine cells (spike 0002, issue #37)

**Date:** 2026-10-24. **Spike:** `spikes/0002-database-engine.md`. **Gates:** ADR-0040 (engine), ADR-0041, ADR-0083; confirms ADR-0043's FR-22 order. **Code:** thrust PR #77 (branch `feat/engine-cells`, draft), beads `plane-byd.11.4.1` to `.9`.

## 1. Verdict: the `worker-do` cell is measured on the platform; the `worker-pg` cell is not. ADR-0040 stays `proposed`.

This note records what was built, what was measured and under what conditions, and what was **not** measured. It does not decide the engine. The decision rule is unchanged (spike 0002, "Decision rule"): Postgres stays the default if the `worker-do` cell fails any exit criterion; if both cells pass, Q-0040.6 is the owner's. The `worker-pg` cell has not run on the platform (section 12), so there is nothing to compare the `worker-do` numbers with except stage 2's pre-FR-22 numbers from a different day, and the owner has not been asked Q-0040.6.

Tags, as in `research/0004`: **[tested]** = ran locally (Apple-silicon macOS laptop, loopback, local workerd from wrangler 4.123.0 or a local Docker Postgres 16); **[tested-cf]** = ran against the Worker deployed on Cloudflare's network; **[derived]** = arithmetic on measured numbers and vendor prices; **[unverified]** = not run.

**Conditions that apply to every deployed latency below.** One client machine, a US laptop (the location and network path were not recorded), is the only vantage. It is not the origin region: the object was created with the location hint `enam`, and nothing in this note is a measurement from near it. Each figure carries the laptop's round trip to Cloudflare's edge (the 43 ms p95 of a list of a nearly empty project, and the 71-186 ms of `GET /_info` to an idle or warm object, are the floor of what this client can see). The Worker was `workers.dev` (`thrust-spike-do.woven-4d9.workers.dev`) on the Woven account (`<cloudflare-account-id>`), built with the cells' release profile, one deploy at a time, one run at a time. n is small (200 requests per bench cell, 20 to 50 for the list probes), three bench runs per arm; percentiles at that n are rough, and where runs disagree the spread is printed rather than averaged.

### Five exit criteria, as measured

| # | Criterion (spike 0002) | `worker-do` | `worker-pg` |
|---|---|---|---|
| 1 | 50-writer test: no duplicate `sequence_id` in 10k creates, no reuse after delete, `sort_order` with it | **passes** [tested-cf]: 10,500 creates + 1,428 deletes, every number once and contiguous from 1, no deleted number reused (section 4). Control shown failing | passes locally on `native-pg` and `worker-cf` (local Hyperdrive emulation) [tested]; **not run on the platform** |
| 2 | FTS plus trigram: substring and prefix queries return Plane's `icontains` results, 1-minute freshness | **passes** [tested-cf]: 400 generated queries identical to `icontains`; a created item is found by the very next search (section 5) | passes locally on `native-pg` and `worker-cf` [tested]; **not run on the platform**; the migration needs `pg_trgm`, installed by an administrator (section 5.4) |
| 3 | List p95 < 300 ms from the origin's region at 100k items, warm; cold first requests reported beside it | **passes for a 1,000-item project** (p95 77 ms at c=1, 216 ms at c=8, in a database of about 124k items); **fails for an unpaged 10,000-item project** (350 ms at c=1, 1,687 ms at c=8). **Not measured from the origin's region.** Cold: no measurable cost (section 6) | **not run** |
| 4 | Migration story, failure case included | **passes** [tested-cf]: expand 0.90 s, FTS5 build 3.8 s, contract 0.67 s on 124,081 rows; a failing migration changes nothing; a build whose migration fails takes the instance down until redeployed (section 7) | **not run** on the platform; the Postgres runner applies each file with one `batch_execute` (section 7.4) |
| 5 | Projected cost within the envelope, reconciled against the 24-hour run | **projected, not reconciled**: about $6.8 a month at the sizing target and about $63 at ten times it, Cloudflare side (section 8). The 24-hour run was skipped by the owner on 2026-10-24: with no real traffic it would only have measured the idle floor | not run; Cloudflare side as `research/0004` section 9 ($6.38) plus the Postgres host, which is outside the envelope (ADR-0040, round 27) |

Reported, not pass or fail: the FR-22 allocation-order check (a stand-in only; the platform numbers wait for `worker-pg`), the placement re-run, Hyperdrive caching on versus off, the distant-region bench, the round-trip counts, the email send, `--panic-unwind` (section 12 lists what was not run).

## 2. What was built

| Part | Where | Notes |
|---|---|---|
| SQLite adapter | `crates/thrust-adapters-sqlite` | the `Database` port written once over a synchronous `Sql` trait (`query`, `execute`, `atomically`); `NativeSql` on rusqlite 0.40 (bundled SQLite 3.53.2, never compiled for wasm32); `DoSql` in `thrust-adapters-cloudflare` over `SqlStorage` |
| SQL | `crates/thrust-adapters-sqlite/migrations/` | `0001_init.sql` = the end state of Postgres 0001-0004 (a SQLite database starts empty, so there is nothing to patch); `0006_search.sql` (FTS5). Ids are text, times are microseconds since the epoch (below 2^53 until 2255, so they survive a JS number), booleans 0/1 |
| Migration runner | `migrate.rs` | each file once, as one transaction with its ledger row (`schema_migrations`), one statement per `exec`; refuses a database that records a version the build does not know |
| The cell | `crates/thrust-worker/src/app_do.rs`, `wrangler.do.toml`, `wrangler.do.stage.toml` | one instance-level `AppDo` (`get_by_name_with_location_hint("instance", "enam")`): the `App` is built once in `new`, migrations run in `new` inside `blockConcurrencyWhile`, `Background` is `State::wait_until`; the front Worker forwards every API request, and the queue consumer and cron handler forward through it (`/_do/consume`, `/_do/sweep`). `/_do/*` is the Worker-to-object protocol: a client path that starts with it is a 404 |
| Targets | `scripts/contract.sh` | `native-sqlite` and `worker-do`, opt-in: **not** in `contract-all.sh`, so not required checks until ADR-0040 decides. New passes `engine`, `do` (worker-do), `scale` |
| Engine pass | `crates/thrust-contract-tests/tests/engine.rs` | the 50-writer test, search parity, freshness and scope, the bulk seed; own test binary, so `EXPECT_MAIN` is untouched (21) |
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

The create at c=8 is 1.5 to 3 times the create at c=1 on `worker-do`, against 1.9 times on stage 2's Postgres before FR-22. This is not a comparison of engines: it is two different days and different code, and `worker-pg` has not been re-measured with the FR-22 order on the platform.

### 6.4 R2: one object serialises the instance [tested, tested-cf]

Every request, every consumer message and every cron sweep goes through one object on one thread. On the deployed object the cost shows in the lists: 8 concurrent lists of a 1,000-item project cost p95 216 ms against 77 ms for one (2.8 times), and 8 concurrent lists of a 10,000-item project cost 1,687 ms against 350 ms (4.8 times); p50 at c=8 is 3.2 and 5.2 times p50 at c=1. Locally the same shape appears (list of 1,000 items, p95 6.2 ms at c=1 and 39 ms at c=8 on `worker-do` under `wrangler dev`, against 1.4 and 5.1 ms on `native-sqlite`). On Postgres each request serialises its JSON in its own isolate in parallel, so the lists are not serialised by the engine; that is an architectural difference between the cells, not measured on the platform.

### 6.5 Cold start [tested-cf]

The object is evicted shortly after its last activity: its counters (`rows_read`, since construction) reset between two samples ten seconds apart after the queue drained. The every-minute cron wakes it each minute.
- First request that reaches the object after at least 40 s of idle (so after the cron's wake at :00 and the eviction), `GET /_info` (admin) over 5 rounds: 100, 98, 92, 71, 112 ms; the second request: 81, 186, 86, 79, 86 ms; `/healthz` (also answered by the object): 122, 183, 90, 114, 108 ms.
- The same with an authenticated list of an empty project over 5 rounds: first 91, 117, 132, 99, 112 ms; second 101, 97, 97, 127, 102 ms.
- Verified cold once: the first call after 40 s idle to `/_test/do/usage` returned `rows_read: 4` (the constructor's migration-ledger reads only), so that call constructed the object, and took 73 ms against 90 ms for the second.

**No measurable cold cost** at this precision and from this vantage: constructing the object, checking the migration ledger and serving the request fit inside the noise of the laptop's round trip. The first DB-backed request on a fresh isolate took 595-615 ms on stage 2's Postgres through Hyperdrive (`research/0004` F7); the object has no connection to open.

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

### 7.4 What the Postgres cell does

`thrust_adapters_postgres::migrate` applies each file with one `batch_execute`, idempotent by `IF NOT EXISTS`; a multi-statement simple query is one implicit transaction. Not exercised on the platform: **not run** (section 12).

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

## 9. FR-22: the sequence allocated outside the create transaction [tested]

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
- **The proxy is a stand-in, not the platform** (`scripts/pg-delay-proxy.py`, `THRUST_PG_LATENCY_MS=5`, a fixed delay each way with no jitter, no Hyperdrive pooling, no tunnel). The FR-22 numbers next to 296 and 559 ms **have not been measured on the platform**; they wait for the `worker-pg` run.

## 10. The other findings

1. **The outbox fast path loses events when the object is evicted [tested-cf].** After the engine pass, 100 of about 10.5k outbox rows were undispatched (a constant count of exactly 100 for a minute), and were delivered by the every-minute cron sweeper within about a minute. `waitUntil` in a Durable Object does not keep an evicted object running, so a relay future in flight at eviction is lost; the row stays and the sweeper retries, which is the design of ADR-0046, and what the engine pass drained in 10.5k events was delivered. The delivery latency tail (create to activity visible p95 about 2 s, 10 s while a backlog drained) is the queue plus the consumer forwarding through the object.
2. **`/healthz` goes through the object [tested-cf].** The front Worker forwards every path; a failed migration makes health checks fail (7.3). That is readiness, not liveness.
3. **Cron wakes the object every minute [tested-cf].** Each wake costs a few seconds of wall-clock duration (the object is evicted within about 20 s), which is why the duration line of the projection is bounded by the always-active ceiling, not by traffic.
4. **The Durable Object refuses `sqlite_version()` and `PRAGMA compile_options` [tested-cf]**, so the SQLite version in the platform is unknown; capabilities are probed by use (5.1).
5. **A 3-character indexed search reads O(project) rows (5.4); SQLite's LIKE folds ASCII only (5.4); `pg_trgm` needs an administrator and a qualified operator class (5.4); a 100-connection Postgres failed 1 of 50 writers on `worker-cf` locally (4).**
6. **argon2id sign-in would run inside the one object** and serialise the instance for its CPU time. The numbers are `research/0012-auth-slice.md` (issue #38); they are a reported cost of the `worker-do` cell, not a pass or fail.

## 11. Teardown ledger

Everything the deployed run created, with UTC times from `target/spike-do/ledger.txt` (the run's own ledger, written by `scripts/spike-0002-do.sh`):

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

The Postgres side (a role and schema on `thrust-pg-dev`, a Hyperdrive config, queues) was never created: `worker-pg` was not run.

## 12. What was NOT measured

- **`worker-pg` on the platform**, entirely: no create, list, activity or cold-start number for Postgres through Hyperdrive, the Workers VPC service and the tunnel since stage 2; so the FR-22 allocation order is **not** confirmed on the platform (section 9 is a loopback and a delay-proxy result), the c=1 and c=8 numbers next to 296 and 559 ms do not exist, and the `worker-do` figures have no same-day comparison. Blocked until the owner's decision (the token, database administrator access and an unlocked 1Password were the stated preconditions; nothing was written).
- **Hyperdrive caching on versus off** (a separate Hyperdrive config).
- **The placement re-run** (`placement.region = "aws:us-east-1"` with and without).
- **The 24-hour cost reconciliation**, both cells (skipped by the owner, 8.3).
- **The distant-region bench**, both cells (the runner is an open owner decision). Every deployed latency here is from one US laptop.
- **A real Email send** (an owned sender domain and a verified destination exist on the account; the send needs a deployed Worker with the binding and an owner OK).
- **The origin-region measurements** for criterion 3: the object was placed with the hint `enam` and no client ran near it.
- **The round-trip counts** (issue #39's budget): a list request makes one Worker-to-object hop and, by reading the code, 3 SQL statements in the object (the session, the membership query, the list); not enforced by a counting connector for either cell.
- A project of 100k items in one list, a contract migration that rewrites a table, `--panic-unwind`, workers-rs behaviour under a Durable Object that is evicted mid-request.
- Anything beyond one client, one run at a time, 200 requests per bench cell.

## 13. Reproduce

```
scripts/contract.sh native-sqlite main            # 21 of 21
scripts/contract.sh worker-do main                # 21 of 21 (wrangler dev)
scripts/contract.sh <target> engine               # 4 tests; native-memory native-sqlite native-pg worker-do worker-cf
scripts/contract.sh worker-do do                  # 5 tests: Tx control, FTS5 probe, cost probe, migration story
scripts/contract.sh <target> scale                # 100k items: list, search, create
THRUST_PG_LATENCY_MS=5 scripts/contract.sh native-pg bench    # the delay proxy (needs uv)
scripts/mutants.sh                                # 13 mutants, all KILLED
# deployed (needs CLOUDFLARE_ACCOUNT_ID and wrangler's OAuth login):
scripts/spike-0002-do.sh up
THRUST_BASE_URL=https://thrust-spike-do.woven-4d9.workers.dev THRUST_ADMIN_TOKEN_FILE=target/spike-do/admin-token \
  THRUST_EXPECT_ADAPTERS=durable sh scripts/stage2-contract.sh main    # also do, engine
THRUST_SPIKE_DO_URL=https://thrust-spike-do.woven-4d9.workers.dev scripts/spike-0002-do.sh down
```
