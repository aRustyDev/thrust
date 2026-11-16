-- Issue #36 (FR-03): sessions are database rows, authoritative on every request (ADR-0052 round 29), so a
-- revocation is a row update and is effective at once, on every target. Idempotent like the files before it.
--
-- The table stores the SHA-256 of the token, never the token (ADR-0052 Tests, ADR-0141): there is no
-- plaintext column, and the length check keeps anything but a digest out of the key.

-- scope: directory (a session belongs to a user, not to a workspace; ADR-0041, Q-0041.1: no workspace_id)
CREATE TABLE IF NOT EXISTS sessions (
    token_hash    bytea PRIMARY KEY CHECK (octet_length(token_hash) = 32),
    user_id       uuid NOT NULL,
    created_at    timestamptz NOT NULL,
    expires_at    timestamptz NOT NULL,
    revoked_at    timestamptz,
    last_seen_at  timestamptz NOT NULL
);

-- Sign-out-everywhere (#38) revokes by user.
CREATE INDEX IF NOT EXISTS sessions_user ON sessions (user_id);
