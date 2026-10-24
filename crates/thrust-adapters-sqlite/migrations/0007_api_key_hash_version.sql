-- The SQLite spelling of Postgres migration 0007 (Q-0052.3, issue #38 follow-up): which digest scheme made an API key's
-- `token_hash`. Version 1 is plain SHA-256, the only scheme that ships; every key made before this column existed is
-- version 1, which is what the default says. The column lets a second scheme be introduced WITHOUT invalidating a key (a
-- key is looked up under every supported version and moved to the current one the next time it is used).
--
-- Postgres needs `ADD COLUMN IF NOT EXISTS` and a CHECK dropped and added by name, because its runner applies every file
-- on every run. This runner applies each file once, inside one transaction with the row that records it, so "add the
-- column once" is a plain `ADD COLUMN` (SQLite allows a NOT NULL column with a default, and a named CHECK, in an ADD COLUMN).
ALTER TABLE api_keys ADD COLUMN hash_version INTEGER NOT NULL DEFAULT 1 CONSTRAINT api_keys_hash_version_positive CHECK (hash_version > 0);
