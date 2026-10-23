-- Issue #36 (FR-03): a workspace has a slug and members, so `authorize` can resolve the route's slug and
-- check workspace membership in the same query as project membership (ADR-0051 round 29).
-- Each table says which side of ADR-0041's `scope: tenant|directory` line it is on.
-- Idempotent: `migrate` applies every file on every run. Rows written before this file existed have
-- a workspace id and no workspaces row, so they are backfilled (slug = the id as text) before the
-- foreign keys go on.

-- scope: tenant (resolved by slug before a `Scope` exists; only `membership` reads it that way)
CREATE TABLE IF NOT EXISTS workspaces (
    id    uuid PRIMARY KEY,
    slug  text NOT NULL UNIQUE
);

-- scope: tenant
CREATE TABLE IF NOT EXISTS workspace_members (
    workspace_id  uuid NOT NULL REFERENCES workspaces (id),
    user_id       uuid NOT NULL,
    PRIMARY KEY (workspace_id, user_id)
);

INSERT INTO workspaces (id, slug)
SELECT w, w::text
FROM (
    SELECT workspace_id AS w FROM projects
    UNION
    SELECT workspace_id AS w FROM project_members
) AS known
ON CONFLICT DO NOTHING;

INSERT INTO workspace_members (workspace_id, user_id)
SELECT DISTINCT workspace_id, user_id FROM project_members
ON CONFLICT DO NOTHING;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'projects_workspace_fkey') THEN
        ALTER TABLE projects ADD CONSTRAINT projects_workspace_fkey
            FOREIGN KEY (workspace_id) REFERENCES workspaces (id);
    END IF;
    -- A project member is a member of the workspace the member row names.
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'project_members_workspace_member_fkey') THEN
        ALTER TABLE project_members ADD CONSTRAINT project_members_workspace_member_fkey
            FOREIGN KEY (workspace_id, user_id) REFERENCES workspace_members (workspace_id, user_id);
    END IF;
END
$$;
