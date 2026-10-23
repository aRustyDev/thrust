-- Spike 0002 (issue #37, ADR-0083): substring search over a work item's name and description, Plane's `icontains`
-- (`name ILIKE '%q%' OR description ILIKE '%q%'`). A trigram GIN index answers `ILIKE '%q%'` for a query of three
-- characters or more; a shorter one scans the project. The SQLite adapters do the same with an FTS5 table that uses the
-- `trigram` tokenizer (`thrust-adapters-sqlite/migrations/0006_search.sql`).
-- Idempotent like the files before it: `migrate` applies every file on every run.
--
-- `pg_trgm` is a trusted extension (PostgreSQL 13 and later): a role with CREATE on the database may install it, and a
-- schema-only role such as the per-PR roles of the shared dev database may not, so there it must be installed once by an
-- administrator (research/0011).
CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE INDEX IF NOT EXISTS work_items_name_trgm ON work_items USING gin (name gin_trgm_ops);
CREATE INDEX IF NOT EXISTS work_items_description_trgm ON work_items USING gin (description gin_trgm_ops);
