-- Issue #44 (the compat slice, workspace-load stage): a project has what the workspace's project list shows about it, and a
-- member has a place for it in their own list. Until now a project was an id, a workspace and a name (migration 0001).
--
-- Rows that exist already (a test's seed) get an identifier made from their id (the last twelve hex digits, upper-cased, as
-- `default_project_identifier` makes it), a public network, no archive time and no lead or creator, and the time the column
-- was added. New rows state every one of these themselves: nothing here relies on the defaults but the backfill.
--
-- Idempotent like the files before it (`migrate` applies every file on every run), and with the trap 0009 met: a NOT NULL
-- column is added nullable, backfilled, and then made NOT NULL, because Postgres checks NOT NULL before it looks for a
-- conflict. `ADD COLUMN IF NOT EXISTS` skips a column that is there but NOT a CHECK written inline with it, so each CHECK is
-- its own statement, dropped and added by name.
--
-- `created_by`, `updated_by` and `project_lead` name users and have no foreign key yet: nothing writes them until the stage
-- that makes projects (S30, S32), which adds the constraints with its writers.

ALTER TABLE projects ADD COLUMN IF NOT EXISTS identifier text;
UPDATE projects SET identifier = upper(split_part(id::text, '-', 5)) WHERE identifier IS NULL;
ALTER TABLE projects ALTER COLUMN identifier SET NOT NULL;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS network text NOT NULL DEFAULT 'public';
ALTER TABLE projects DROP CONSTRAINT IF EXISTS projects_network_valid;
ALTER TABLE projects ADD CONSTRAINT projects_network_valid CHECK (network IN ('secret', 'public'));
ALTER TABLE projects ADD COLUMN IF NOT EXISTS logo_props jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE projects DROP CONSTRAINT IF EXISTS projects_logo_props_is_object;
ALTER TABLE projects ADD CONSTRAINT projects_logo_props_is_object CHECK (jsonb_typeof(logo_props) = 'object');
ALTER TABLE projects ADD COLUMN IF NOT EXISTS archived_at timestamptz;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS cycle_view boolean NOT NULL DEFAULT false;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS issue_views_view boolean NOT NULL DEFAULT false;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS module_view boolean NOT NULL DEFAULT false;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS page_view boolean NOT NULL DEFAULT true;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS intake_view boolean NOT NULL DEFAULT false;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS guest_view_all_features boolean NOT NULL DEFAULT false;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS project_lead uuid;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS created_by uuid;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS updated_by uuid;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS created_at timestamptz NOT NULL DEFAULT now();
ALTER TABLE projects ADD COLUMN IF NOT EXISTS updated_at timestamptz NOT NULL DEFAULT now();

-- the project list reads a workspace's projects newest first
CREATE INDEX IF NOT EXISTS projects_by_workspace ON projects (workspace_id, created_at DESC, id DESC);

-- A member's own row for a project: where their list puts it. Plane makes it when it makes the project (and for a member
-- added later); the stage that makes projects adds the rest of its columns (the member's list settings).
CREATE TABLE IF NOT EXISTS project_user_properties (
    project_id  uuid NOT NULL REFERENCES projects (id),
    user_id     uuid NOT NULL REFERENCES users (id),
    sort_order  double precision NOT NULL DEFAULT 65535,
    created_at  timestamptz NOT NULL DEFAULT now(),
    updated_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (project_id, user_id)
);
