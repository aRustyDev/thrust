-- Q-0052.3 (issue #38 follow-up): which digest scheme made an API key's `token_hash`.
--
-- API keys are stored as the plain SHA-256 of a token of at least 256 random bits, and that stays: a leaked digest
-- cannot be reversed with or without a pepper, and a pepper would add a secret and a rotation story to every target
-- for no gain. This column is the escape hatch for the day that changes: it lets a second scheme be introduced
-- WITHOUT invalidating a single key (a key is looked up under every supported version and moved to the current one
-- the next time it is used). Version 1 is plain SHA-256, the only scheme that ships; every key made before this
-- column existed is version 1, which is what the default says.
--
-- Idempotent like the files before it (`migrate` applies every file on every run). `ADD COLUMN IF NOT EXISTS` skips the
-- column when it is there, but NOT a CHECK written inline with it: Postgres adds that one again on every run (measured:
-- three runs, three identical checks), so the CHECK is its own statement, dropped and added by name, and there is one.
-- No `DO` block, no Postgres-only type: a smallint and a plain CHECK. Number 0006 is the search migration (`0006_search.sql`),
-- applied before this file; `migrate` applies the files in order. (The archived SQLite twin, tag `spike/0002-worker-do`,
-- carried this file over in its own spelling of "add the column once".)
ALTER TABLE api_keys ADD COLUMN IF NOT EXISTS hash_version smallint NOT NULL DEFAULT 1;
ALTER TABLE api_keys DROP CONSTRAINT IF EXISTS api_keys_hash_version_positive;
ALTER TABLE api_keys ADD CONSTRAINT api_keys_hash_version_positive CHECK (hash_version > 0);
