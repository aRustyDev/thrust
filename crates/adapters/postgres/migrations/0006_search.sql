-- Spike 0002 (issue #37, ADR-0083): substring search over a work item's name and description, Plane's `icontains`
-- (`name ILIKE '%q%' OR description ILIKE '%q%'`). A trigram GIN index answers `ILIKE '%q%'` for a query of three
-- characters or more; a shorter one scans the project. (A SQLite twin did the same with an FTS5 table that uses the
-- `trigram` tokenizer; it is archived on the tag `spike/0002-worker-do`.)
-- Idempotent like the files before it: `migrate` applies every file on every run.
--
-- `pg_trgm` is a trusted extension (PostgreSQL 13 and later): a role with CREATE on the database may install it, and a
-- schema-only role such as the per-PR roles of the shared dev database may not, so there it must be installed once by an
-- administrator, in schema `public` (research/0011).
--
-- The operator class is qualified (`public.gin_trgm_ops`) on purpose. The per-PR roles of the shared dev database
-- (products/thrust/kube/scripts/pr-schema.sh) have a ROLE-LEVEL `search_path = pr_<n>` and nothing else, so an unqualified
-- `gin_trgm_ops` is not found when the extension lives in `public`: `operator class "gin_trgm_ops" does not exist for
-- access method "gin"` (reproduced on a throwaway Postgres as such a role, research/0011). `WITH SCHEMA public` makes a
-- superuser's `CREATE EXTENSION` put it where the qualified name looks, whatever that session's search_path is; where the
-- extension already exists, IF NOT EXISTS skips it.
CREATE EXTENSION IF NOT EXISTS pg_trgm WITH SCHEMA public;

CREATE INDEX IF NOT EXISTS work_items_name_trgm ON work_items USING gin (name public.gin_trgm_ops);
CREATE INDEX IF NOT EXISTS work_items_description_trgm ON work_items USING gin (description public.gin_trgm_ops);
