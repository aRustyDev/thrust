-- The SQLite translation of the Postgres migrations 0001 to 0004 (crates/thrust-adapters-postgres/migrations), as one
-- file in their END state. The Postgres files are idempotent patches over databases that already exist (they add a
-- column, re-key a table, backfill rows); a SQLite database starts empty, so there is nothing to patch and the end
-- state is the whole translation. Postgres migration 0005 (identity, issue #38) and 0006 (search, `0006_search.sql`
-- here) keep their numbers on both sides.
--
-- Dialect, so that a later migration can be written once for both:
--   uuid -> TEXT (hyphenated, lower case: it sorts like the bytes);  timestamptz -> INTEGER (microseconds since the
--   epoch: below 2^53 until 2255, so it survives a JS number in the Durable Object);  bytea -> BLOB;
--   boolean -> INTEGER 0/1;  jsonb -> TEXT holding JSON;  bigint/integer -> INTEGER.
-- Everything else is unchanged: partial unique indexes, composite foreign keys, CHECK constraints.
-- No `IF NOT EXISTS`: `migrate` records what it has applied and runs each file once, inside one transaction.

-- scope: tenant (resolved by slug before a `Scope` exists; only `membership` reads it that way)
CREATE TABLE workspaces (
    id    TEXT PRIMARY KEY,
    slug  TEXT NOT NULL UNIQUE
);

-- scope: tenant
CREATE TABLE workspace_members (
    workspace_id  TEXT NOT NULL REFERENCES workspaces (id),
    user_id       TEXT NOT NULL,
    PRIMARY KEY (workspace_id, user_id)
);

CREATE TABLE projects (
    id            TEXT PRIMARY KEY,
    workspace_id  TEXT NOT NULL REFERENCES workspaces (id),
    name          TEXT NOT NULL,
    UNIQUE (workspace_id, id)
);

CREATE TABLE project_members (
    project_id    TEXT NOT NULL REFERENCES projects (id),
    workspace_id  TEXT NOT NULL,
    user_id       TEXT NOT NULL,
    role          TEXT NOT NULL CHECK (role IN ('guest', 'member', 'admin')),
    PRIMARY KEY (project_id, user_id),
    -- A project member is a member of the workspace the member row names.
    FOREIGN KEY (workspace_id, user_id) REFERENCES workspace_members (workspace_id, user_id)
);

-- ADR-0043 B1: counter row per project. Monotonic: nothing ever decrements it, so numbers are never reused. The row
-- can exist only for a project in its own workspace, so an allocation naming the wrong workspace is refused.
CREATE TABLE project_sequences (
    workspace_id      TEXT NOT NULL,
    project_id        TEXT NOT NULL,
    last_sequence_id  INTEGER NOT NULL DEFAULT 0,
    last_sort_order   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (workspace_id, project_id),
    FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id)
);

CREATE TABLE work_items (
    id               TEXT PRIMARY KEY,
    workspace_id     TEXT NOT NULL,
    project_id       TEXT NOT NULL REFERENCES projects (id),
    sequence_id      INTEGER NOT NULL,
    sort_order       INTEGER NOT NULL,
    name             TEXT NOT NULL,
    description      TEXT,
    external_source  TEXT,
    external_id      TEXT,
    created_by       TEXT NOT NULL,
    created_at       INTEGER NOT NULL,
    archived_at      INTEGER,
    is_draft         INTEGER NOT NULL DEFAULT 0,
    deleted_at       INTEGER
);

-- sequence_id is unique across live AND deleted rows (a deleted number is never handed out again)
CREATE UNIQUE INDEX work_items_project_seq ON work_items (project_id, sequence_id);

-- ADR-0043 C1: partial unique index; a soft-deleted row frees its external key.
CREATE UNIQUE INDEX work_items_external_live
    ON work_items (project_id, external_source, external_id)
    WHERE deleted_at IS NULL AND external_source IS NOT NULL AND external_id IS NOT NULL;

CREATE INDEX work_items_list ON work_items (workspace_id, project_id, sequence_id);

-- ADR-0046 transactional outbox.
CREATE TABLE outbox (
    event_id       TEXT PRIMARY KEY,
    workspace_id   TEXT NOT NULL,
    envelope       TEXT NOT NULL,
    created_at     INTEGER NOT NULL,
    dispatched_at  INTEGER,
    attempts       INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX outbox_undispatched ON outbox (created_at) WHERE dispatched_at IS NULL;

-- Projection written by the queue consumer; UNIQUE(event_id) is the idempotency key.
CREATE TABLE activity (
    id            TEXT PRIMARY KEY,
    event_id      TEXT NOT NULL UNIQUE,
    workspace_id  TEXT NOT NULL,
    project_id    TEXT NOT NULL,
    entity_id     TEXT NOT NULL,
    verb          TEXT NOT NULL,
    actor_id      TEXT,
    created_at    INTEGER NOT NULL
);
CREATE INDEX activity_lookup ON activity (workspace_id, project_id, entity_id, created_at);

-- Sessions are database rows, authoritative on every request (ADR-0052 round 29). The table stores the SHA-256 of the
-- token, never the token: there is no plaintext column, and the length check keeps anything but a digest out of the key.
-- scope: directory (a session belongs to a user, not to a workspace; ADR-0041, Q-0041.1: no workspace_id)
CREATE TABLE sessions (
    token_hash    BLOB PRIMARY KEY CHECK (length(token_hash) = 32),
    user_id       TEXT NOT NULL,
    created_at    INTEGER NOT NULL,
    expires_at    INTEGER NOT NULL,
    revoked_at    INTEGER,
    last_seen_at  INTEGER NOT NULL
);

-- Sign-out-everywhere (#38) revokes by user.
CREATE INDEX sessions_user ON sessions (user_id);
