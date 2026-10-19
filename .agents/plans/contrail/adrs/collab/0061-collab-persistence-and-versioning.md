---
status: proposed
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0061: Collab document persistence, snapshotting and page versioning

## Context and Problem Statement

Today `apps/live` loads a Yjs binary via the Django API and, on store, writes `description_binary` + `description_html` +
`description_json` together; if the binary is empty it converts existing `description_html` to Yjs and saves it back **[S]**.
Page versions are produced afterwards by Celery tasks (`page_version_task.track_page_version`,
`page_transaction_task.page_transaction`), pruned daily by `cleanup_task.delete_page_versions` **[S]**. In the new design the
`Collab` port (ADR-0060) needs a persistence strategy that works on both a DO adapter and a Hocuspocus adapter, and that keeps the three
representations consistent because other features read `description_html/json` (search, export, mentions, API v1).

## Decision Drivers

* Lossless compatibility with existing `description_binary` data (Q-G3 migration).
* Durability: no acknowledged edit lost on DO eviction or Hocuspocus crash.
* Triple-representation consistency (binary/html/json) for non-collab readers.
* Cost/limits: DO SQLite ≤ 10 GB per object; Workflows/Queues limits for derived work.
* Version history cadence and retention parity with Plane (daily prune).

## Considered Options

* Option 1: **DB is the system of record**; DO/Hocuspocus hold a hot copy, debounce-flush binary+html+json to `Database` via `CollabPersistence`
* Option 2: **DO storage is the system of record** for the Yjs doc (incremental updates in DO SQLite, compacted); async projection of html/json to `Database`
* Option 3: **Blob snapshots in R2/S3** (binary) with metadata in DB; DO keeps only a write-ahead log
* Option 4: Split: updates log in DO SQLite, periodic full snapshot to `BlobStore`, html/json projected on snapshot

## Decision Outcome

Decision pending — see **Q-0061**. Leading hypothesis: **Option 4** (confidence: low–medium) — DO SQLite as a short-lived
write-ahead update log for durability (strongly consistent, transactional), snapshot to `BlobStore` + `Database` on idle/interval,
html/json projected from the snapshot; the Hocuspocus adapter implements the same `CollabPersistence` against Postgres/S3. Needs the
spike in Confirmation.

### Consequences

* Good (if Option 4): durable acks without a DB round-trip per keystroke; bounded DO storage via compaction; DB remains the
  source for non-collab readers.
* Bad: projected html/json is eventually consistent (seconds) — API v1 readers may see stale `description_html` right after editing.
* Neutral: version creation becomes an event (`PageVersionDue`) on the outbox (ADR-0046), consumed by a Queue consumer.

### Confirmation

Spike (part of Q-0040.1 spike or separate): (a) load 1000 real Plane page binaries, assert round-trip identity; (b) kill a DO / Hocuspocus
mid-session and verify no acked update lost; (c) measure cold start for a 1 MB, 5 MB doc; (d) verify html/json projection equals what
`@plane/editor` produces server-side today.

## Pros and Cons of the Options

### Option 1: DB as system of record

* Good, because single source, simplest mental model, matches Plane today.
* Bad, because every flush is a DB write over Hyperdrive/D1; a crash between flushes loses unflushed edits unless the room keeps a WAL.

### Option 2: DO storage as system of record

* Good, because strong consistency and colocated compute; PITR available on SQLite-backed DOs (30 days, CF docs).
* Bad, because not portable — the Hocuspocus adapter has no equivalent; DO becomes the source for data other features need; a 10 GB per-DO cap and per-DO migration are hard to reason about for backup/export.

### Option 3: Blob snapshots only

* Good, because cheap and portable (S3 and R2 both fine).
* Bad, because R2 has per-key concurrent-write limits (HTTP 429 on high-rate writes to one key) and snapshot-only loses recent edits on failure without a WAL.

### Option 4: WAL in DO + snapshots

* Good, because durable and portable-ish (self-hosted WAL = Redis stream/Postgres table behind the same port).
* Bad, because two persistence paths to keep consistent; compaction logic to write and test.

## Update — interview round 11 (2026-10-17)

P0 storage (Q-F5.1): persist **Yjs binary + html + json from day one** with an **optimistic version check** (stale save returns 409) on plain API saves, so the P1 move to Durable Objects/Yjs needs no data migration.

## Update — interview round 18 (2026-10-18)

Page versions every **10 minutes of edits**; `description_html/json` staleness up to **5 s** is acceptable (Q-0061.3, Q-0061.4).

## Update — review decisions, round 25 (2026-10-18)

P0 persistence follows ADR-0012 Option B (round 25): forms stored verbatim, HTML sanitised in Rust, schema versioned; conversion arrives with P1 collab.

## Update — spike 0006 results, round 27 (2026-10-18)

Spike 0006 (`research/0008-editor-conversions.md`): **Confirmation (d) is met for the server flavour** — the Rust projection equals today's server output byte for byte on 263 documents and 6,000 fuzzed inputs. Caveat: server projections are the **zeed-dom flavour**, which differs from the client's `editor.getHTML()` in 37 % of corpus documents (`style` attributes, boolean attribute form, attribute-name case), so any equality test that mixes the two fails spuriously; and server JSON and HTML drop the client's block `id`s. **Q-0061.4** (round-18 answer: up to 5 s staleness is fine): the allowance is not needed for CPU reasons — a collab Durable Object in Rust can project html and json synchronously at store time (0.09 to 0.5 ms for typical pages, 118 ms for a 745 KB document), so strong consistency inside the store transaction is affordable; the owner's answer stands and nothing forces a change. Add an explicit **depth cap (about 200), a size cap and a schema-version column** before the P1 collab: the Rust Worker dies, an instance reset, at about 1,000 nesting levels.

## Open Questions

* Q-0061 — choose the strategy (decision-pending).
* Q-0061.1 — presence limits, max doc size, cold-start.
* Q-0061.2 — Hocuspocus adapter reuse.
* Q-0061.3 — version cadence: Plane's `track_page_version` triggers on each store; keep per-store, per-N-minutes, or on-explicit-save?
* Q-0061.4 — are `description_html/json` required to be strongly consistent with `binary` for API v1 reads, or is bounded staleness acceptable?

## More Information

* Plane: `apps/live/src/extensions/database.ts`, `apps/api/plane/bgtasks/page_version_task.py`, `page_transaction_task.py`, `cleanup_task.py`.
* CF: DO SQLite storage <https://developers.cloudflare.com/durable-objects/api/sqlite-storage-api/>; R2 limits <https://developers.cloudflare.com/r2/platform/limits/>.
* Related: [[ADR-0060]], [[ADR-0046]], [[ADR-0073]], [[ADR-0080]].
