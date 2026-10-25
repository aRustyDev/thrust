-- Issue #44 (the compat slice, project stage): a project is made, renamed and opened. It has the rest of the columns the app
-- reads, its name and key are unique in its workspace, the people in it have rows of their own (view settings, a place in their
-- list), it has work-item states (the five Plane makes with a project, and a sixth for intake) and labels.
--
-- Rows that exist already (a test's seed) get what a project made with no options has, and the time the column was added.
-- Idempotent like the files before it (`migrate` applies every file on every run): `ADD COLUMN IF NOT EXISTS` skips a column
-- that is there but NOT a CHECK or a foreign key written inline with it, so each is its own statement, dropped and added by
-- name; a NOT NULL column that needs a backfill is added nullable, backfilled, then made NOT NULL (Postgres checks NOT NULL
-- before it looks for a conflict, the trap 0003 and 0009 met). The unique indexes on a project's name and key would fail on a
-- database that already holds two projects of one workspace with one name; only test seeds have written projects so far.
--
-- Not here, because nothing reads or writes it yet: soft deletion, the estimate, the cover image's asset (the asset stage adds
-- its foreign key), the active flag of a project member (a project member is always active until members can be removed).

ALTER TABLE projects ADD COLUMN IF NOT EXISTS description text NOT NULL DEFAULT '';
ALTER TABLE projects ADD COLUMN IF NOT EXISTS description_text jsonb;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS description_html jsonb;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS emoji text;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS icon_prop jsonb;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS is_time_tracking_enabled boolean NOT NULL DEFAULT false;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS is_issue_type_enabled boolean NOT NULL DEFAULT false;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS cover_image text;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS cover_image_asset uuid;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS estimate uuid;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS archive_in integer NOT NULL DEFAULT 0;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS close_in integer NOT NULL DEFAULT 0;
ALTER TABLE projects DROP CONSTRAINT IF EXISTS projects_archive_in_valid;
ALTER TABLE projects ADD CONSTRAINT projects_archive_in_valid CHECK (archive_in BETWEEN 0 AND 12);
ALTER TABLE projects DROP CONSTRAINT IF EXISTS projects_close_in_valid;
ALTER TABLE projects ADD CONSTRAINT projects_close_in_valid CHECK (close_in BETWEEN 0 AND 12);
ALTER TABLE projects ADD COLUMN IF NOT EXISTS default_state uuid;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS timezone text NOT NULL DEFAULT 'UTC';
ALTER TABLE projects ADD COLUMN IF NOT EXISTS external_source text;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS external_id text;
ALTER TABLE projects ADD COLUMN IF NOT EXISTS default_assignee uuid;

-- the users a project names are users (0010 left these columns without a foreign key until a writer existed)
ALTER TABLE projects DROP CONSTRAINT IF EXISTS projects_created_by_fkey;
ALTER TABLE projects ADD CONSTRAINT projects_created_by_fkey FOREIGN KEY (created_by) REFERENCES users (id);
ALTER TABLE projects DROP CONSTRAINT IF EXISTS projects_updated_by_fkey;
ALTER TABLE projects ADD CONSTRAINT projects_updated_by_fkey FOREIGN KEY (updated_by) REFERENCES users (id);
ALTER TABLE projects DROP CONSTRAINT IF EXISTS projects_project_lead_fkey;
ALTER TABLE projects ADD CONSTRAINT projects_project_lead_fkey FOREIGN KEY (project_lead) REFERENCES users (id);
ALTER TABLE projects DROP CONSTRAINT IF EXISTS projects_default_assignee_fkey;
ALTER TABLE projects ADD CONSTRAINT projects_default_assignee_fkey FOREIGN KEY (default_assignee) REFERENCES users (id);

-- a project's name and its key are each unique in its workspace (exact match: `Plane` and `plane` are two names)
CREATE UNIQUE INDEX IF NOT EXISTS projects_workspace_identifier_key ON projects (workspace_id, identifier);
CREATE UNIQUE INDEX IF NOT EXISTS projects_workspace_name_key ON projects (workspace_id, name);

-- A person's row in a project: an id of its own, their view settings, a place in the project's own member list, who made it and
-- when. The three settings are objects; the app reads the pin's defaults for a key a row does not set (the compat layer fills
-- them, as it does for a workspace member's `props`).
ALTER TABLE project_members
    ADD COLUMN IF NOT EXISTS id uuid NOT NULL DEFAULT md5(random()::text || clock_timestamp()::text)::uuid;
CREATE UNIQUE INDEX IF NOT EXISTS project_members_id_key ON project_members (id);
ALTER TABLE project_members ADD COLUMN IF NOT EXISTS comment text;
ALTER TABLE project_members ADD COLUMN IF NOT EXISTS view_props jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE project_members ADD COLUMN IF NOT EXISTS default_props jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE project_members ADD COLUMN IF NOT EXISTS preferences jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE project_members DROP CONSTRAINT IF EXISTS project_members_settings_are_objects;
ALTER TABLE project_members ADD CONSTRAINT project_members_settings_are_objects CHECK (
    jsonb_typeof(view_props) = 'object' AND jsonb_typeof(default_props) = 'object' AND jsonb_typeof(preferences) = 'object'
);
ALTER TABLE project_members ADD COLUMN IF NOT EXISTS sort_order double precision NOT NULL DEFAULT 65535;
ALTER TABLE project_members ADD COLUMN IF NOT EXISTS created_by uuid;
ALTER TABLE project_members ADD COLUMN IF NOT EXISTS created_at timestamptz NOT NULL DEFAULT now();
ALTER TABLE project_members ADD COLUMN IF NOT EXISTS updated_at timestamptz NOT NULL DEFAULT now();
ALTER TABLE project_members DROP CONSTRAINT IF EXISTS project_members_created_by_fkey;
ALTER TABLE project_members ADD CONSTRAINT project_members_created_by_fkey FOREIGN KEY (created_by) REFERENCES users (id);
-- the member list of a project, and the roles of a user, read these
CREATE INDEX IF NOT EXISTS project_members_by_user ON project_members (user_id, workspace_id);

-- A person's own settings for a project (0010 made the row with its sort order alone): an id, the workspace, and the four
-- work-item list settings and the preferences the pin keeps beside it.
ALTER TABLE project_user_properties
    ADD COLUMN IF NOT EXISTS id uuid NOT NULL DEFAULT md5(random()::text || clock_timestamp()::text)::uuid;
CREATE UNIQUE INDEX IF NOT EXISTS project_user_properties_id_key ON project_user_properties (id);
ALTER TABLE project_user_properties ADD COLUMN IF NOT EXISTS workspace_id uuid;
UPDATE project_user_properties up SET workspace_id = p.workspace_id FROM projects p WHERE p.id = up.project_id AND up.workspace_id IS NULL;
ALTER TABLE project_user_properties ALTER COLUMN workspace_id SET NOT NULL;
ALTER TABLE project_user_properties ADD COLUMN IF NOT EXISTS filters jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE project_user_properties ADD COLUMN IF NOT EXISTS display_filters jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE project_user_properties ADD COLUMN IF NOT EXISTS display_properties jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE project_user_properties ADD COLUMN IF NOT EXISTS rich_filters jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE project_user_properties ADD COLUMN IF NOT EXISTS preferences jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE project_user_properties DROP CONSTRAINT IF EXISTS project_user_properties_settings_are_objects;
ALTER TABLE project_user_properties ADD CONSTRAINT project_user_properties_settings_are_objects CHECK (
    jsonb_typeof(filters) = 'object' AND jsonb_typeof(display_filters) = 'object' AND jsonb_typeof(display_properties) = 'object'
    AND jsonb_typeof(rich_filters) = 'object' AND jsonb_typeof(preferences) = 'object'
);
-- the lowest place a person has in a workspace is what a new project's place is made from
CREATE INDEX IF NOT EXISTS project_user_properties_by_user ON project_user_properties (user_id, workspace_id);

-- A project's work-item states. Plane makes five with a project (backlog, unstarted, started, completed, cancelled) and a sixth,
-- in the group `triage`, that intake uses; the lists leave the sixth out. `sequence` is the state's place among its project's,
-- and a project's names are unique.
CREATE TABLE IF NOT EXISTS states (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL,
    project_id    uuid NOT NULL,
    name          text NOT NULL,
    description   text NOT NULL DEFAULT '',
    color         text NOT NULL,
    sequence      double precision NOT NULL,
    state_group   text NOT NULL,
    is_default    boolean NOT NULL DEFAULT false,
    created_by    uuid REFERENCES users (id),
    created_at    timestamptz NOT NULL,
    updated_at    timestamptz NOT NULL,
    UNIQUE (project_id, name),
    FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id)
);
ALTER TABLE states DROP CONSTRAINT IF EXISTS states_group_valid;
ALTER TABLE states ADD CONSTRAINT states_group_valid CHECK (state_group IN ('backlog', 'unstarted', 'started', 'completed', 'cancelled', 'triage'));
CREATE INDEX IF NOT EXISTS states_by_project ON states (project_id, sequence);
-- one default state at most per project
CREATE UNIQUE INDEX IF NOT EXISTS states_one_default_per_project ON states (project_id) WHERE is_default;

-- A project's labels: a name that is unique in its project, a colour, a place and, for a nested one, its parent.
CREATE TABLE IF NOT EXISTS labels (
    id            uuid PRIMARY KEY,
    workspace_id  uuid NOT NULL,
    project_id    uuid NOT NULL,
    parent_id     uuid REFERENCES labels (id),
    name          text NOT NULL,
    description   text NOT NULL DEFAULT '',
    color         text NOT NULL DEFAULT '',
    sort_order    double precision NOT NULL DEFAULT 65535,
    created_by    uuid REFERENCES users (id),
    created_at    timestamptz NOT NULL,
    updated_at    timestamptz NOT NULL,
    UNIQUE (project_id, name),
    FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id)
);
CREATE INDEX IF NOT EXISTS labels_by_project ON labels (project_id, sort_order);
