---
status: accepted
date: 2026-10-17
decision-makers: asmith
consulted: []
informed: []
---

# ADR-0083: `Search` port — backend strategy for global search

## Context and Problem Statement

Plane CE's global search is served by `plane/app/views/search` against Postgres **[S]** (the survey found no OpenSearch client in the backend requirements, although the fork's
`features.md` lists "Search: Typesense, Meilisearch, OpenSearch" as desired options **[S]**). The new backend needs a `Search` port with a Cloudflare adapter and a self-hosted adapter (ADR-0002),
fed by the event stream (ADR-0046/0073) so indexes follow writes. Open: which backend(s), and whether semantic/hybrid search is in scope (links to AI, ADR-0084).

## Decision Drivers

* Parity with Plane CE search results for work items, projects, cycles, modules, pages, views (F-D08).
* Tenant- and permission-aware results (never return items the user cannot see; permission seam, ADR-0051).
* Works on both adapter sets with comparable relevance.
* Index freshness (seconds) and reindex ability (backfill from DB).
* Cost and operational burden.

## Considered Options

* Option 1: **Database-native search** behind the port (Postgres FTS/`pg_trgm`, or SQLite FTS if D1/DO-SQLite) — no extra system
* Option 2: **Cloudflare AI Search / Vectorize** on CF (managed hybrid vector+keyword), external engine on self-hosted
* Option 3: **External engine on both sets** (Typesense / Meilisearch / OpenSearch), CF adapter calls it over HTTP
* Option 4: Phase it: DB-native for P0, add an engine adapter later via the same port

## Decision Outcome

Chosen option: **Option 4 — database-native full-text search as the P0 adapter**, selected by the owner in interview round 7 (2026-10-17) as the starting backend for the Search port (F-D08 is P0). The port keeps the door open for Typesense/Meilisearch (named in `features.md`) or Cloudflare AI Search later, as P1+ adapters.

**Caveat that is still open:** FTS capabilities differ between Postgres, D1 and DO-SQLite, and D1/SQLite FTS support was not confirmed by the ADR author (Q-0083.1). The choice of DB engine (ADR-0040) therefore must include "FTS meets F-D08 at ≈100k work items" as a spike criterion; if it fails on a candidate engine, that engine needs a different Search adapter, not a different decision here.

Port sketch: `index(docs: SearchDoc[])`, `delete(ids)`, `search(query, {tenant, filters, viewerScope, limit, cursor})`, `reindex(entity)`. `viewerScope` carries ACL terms so adapters can filter server-side.

### Consequences

* Good: Option 4 minimises new infrastructure for first release and lets relevance parity be measured against Plane.
* Bad: DB-native relevance is weaker than a dedicated engine; ACL filtering in-DB may be costly for large workspaces.
* Neutral: Cloudflare Vectorize supports up to 20,000,000 vectors per index, 1,536 dimensions, and AI Search (managed; formerly AutoRAG) provides vector and keyword (hybrid) retrieval with metadata filtering
  — a candidate for semantic search, not required for parity (CF docs).

### Confirmation

* Relevance harness: a fixed corpus + query set run against current Plane and the adapter; report recall@10 and rank deltas.
* ACL test: results never include items the viewer cannot read (property test over generated memberships).
* Freshness test: write → searchable within target (Q-0083.2) on both adapters.

## Pros and Cons of the Options

### Option 1: DB-native

* Good: no extra system, transactional consistency possible. * Bad: engine-specific SQL (pg vs SQLite FTS), limited ranking/typo tolerance. D1 FTS support was not verified in this pass (see Q-0083.1).

### Option 2: AI Search / Vectorize on CF

* Good: managed hybrid retrieval, semantic search, continuous sync from R2 sources. * Bad: CF-only (needs a different self-hosted adapter → two relevance profiles); content must be shaped into files/vectors; per-tenant permission filtering relies on metadata filters (limited custom metadata fields per instance).

### Option 3: External engine both sets

* Good: one relevance profile, rich features (typo tolerance, facets). * Bad: extra service to run on both; CF adapter egress to an external host; cost.

### Option 4: Phased

* Good: fastest to parity; defers commitment. * Bad: a later engine adds a migration/reindex project.

## Update — interview round 18 (2026-10-18)

Index freshness target **within 1 minute**; **no semantic search** (Q-0083.2, Q-0083.3).

## Update — review fixes, round 25 (2026-10-18)

**Parity finding:** Plane's search (`app/views/search/base.py`) is **substring matching** (`Q(field__icontains=query)` per entity), not full-text. Plain FTS is word/token based and would change results (no substring or prefix matching). **Owner decision: FTS with a trigram index for substring/prefix parity** — Postgres `tsvector` + `pg_trgm`, or SQLite/D1 FTS5 with the `trigram` tokenizer — verified in spike 0002; plain `ILIKE` over the same indexes is the fallback at this scale. The port sketch in Rust: `index(&[SearchDoc])`, `delete(&[Id])`, `search(&Query { tenant, filters, viewer_scope, limit, cursor })`, `reindex(entity)`. The relevance harness (Confirmation) compares against the pinned Plane oracle's substring results, not just recall@10 on word queries.

## Open Questions

* Q-0083 — **answered (round 7, 18, 25)**: DB-native FTS + trigram at P0; engines later via the same port.
* Q-0083.1 — verify full-text **and trigram** availability and limits for the chosen DB (D1 FTS5 trigram tokenizer unverified, Postgres `pg_trgm`) in spike 0002.
* Q-0083.2 — freshness SLO for index updates (seconds vs minutes).
* Q-0083.3 — is semantic search (embeddings) a feature, or only keyword parity? (ties to Q-0084)

## More Information

* Plane: `apps/api/plane/app/views/search`; fork `features.md`.
* CF: Vectorize limits <https://developers.cloudflare.com/vectorize/platform/limits/>; AI Search <https://developers.cloudflare.com/ai-search/concepts/how-ai-search-works/>.
* Related: [[ADR-0040]], [[ADR-0046]], [[ADR-0051]], [[ADR-0073]], [[ADR-0084]].
