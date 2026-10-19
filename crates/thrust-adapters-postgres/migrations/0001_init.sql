-- Spike 0001 schema: just enough for the vertical slice. Not the product schema (ADR-0042).
-- Every table carries workspace_id (tenant key, ADR-0041) except pure children of projects.

CREATE TABLE IF NOT EXISTS projects (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL,
    name          text NOT NULL
);

CREATE TABLE IF NOT EXISTS project_members (
    project_id    uuid NOT NULL REFERENCES projects(id),
    workspace_id  uuid NOT NULL,
    user_id       uuid NOT NULL,
    role          text NOT NULL CHECK (role IN ('guest', 'member', 'admin')),
    PRIMARY KEY (project_id, user_id)
);

-- ADR-0043 B1: counter row per project, one atomic UPSERT ... RETURNING inside the creating
-- transaction. Monotonic: nothing ever decrements it, so numbers are never reused.
CREATE TABLE IF NOT EXISTS project_sequences (
    project_id        uuid PRIMARY KEY REFERENCES projects(id),
    last_sequence_id  bigint NOT NULL DEFAULT 0,
    last_sort_order   bigint NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS work_items (
    id               uuid PRIMARY KEY,
    workspace_id     uuid NOT NULL,
    project_id       uuid NOT NULL REFERENCES projects(id),
    sequence_id      bigint NOT NULL,
    sort_order       bigint NOT NULL,
    name             text NOT NULL,
    description      text,
    external_source  text,
    external_id      text,
    created_by       uuid NOT NULL,
    created_at       timestamptz NOT NULL,
    archived_at      timestamptz,
    is_draft         boolean NOT NULL DEFAULT false,
    deleted_at       timestamptz
);

-- sequence_id is unique across live AND deleted rows (a deleted number is never handed out again)
CREATE UNIQUE INDEX IF NOT EXISTS work_items_project_seq ON work_items (project_id, sequence_id);

-- ADR-0043 C1: partial unique index; a soft-deleted row frees its external key.
CREATE UNIQUE INDEX IF NOT EXISTS work_items_external_live
    ON work_items (project_id, external_source, external_id)
    WHERE deleted_at IS NULL AND external_source IS NOT NULL AND external_id IS NOT NULL;

CREATE INDEX IF NOT EXISTS work_items_list ON work_items (workspace_id, project_id, sequence_id);

-- ADR-0046 transactional outbox.
CREATE TABLE IF NOT EXISTS outbox (
    event_id       uuid PRIMARY KEY,
    workspace_id   uuid NOT NULL,
    envelope       jsonb NOT NULL,
    created_at     timestamptz NOT NULL,
    dispatched_at  timestamptz,
    attempts       integer NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS outbox_undispatched ON outbox (created_at) WHERE dispatched_at IS NULL;

-- Projection written by the queue consumer; UNIQUE(event_id) is the idempotency key.
CREATE TABLE IF NOT EXISTS activity (
    id            uuid PRIMARY KEY,
    event_id      uuid NOT NULL UNIQUE,
    workspace_id  uuid NOT NULL,
    project_id    uuid NOT NULL,
    entity_id     uuid NOT NULL,
    verb          text NOT NULL,
    actor_id      uuid,
    created_at    timestamptz NOT NULL
);
CREATE INDEX IF NOT EXISTS activity_lookup ON activity (workspace_id, project_id, entity_id, created_at);
