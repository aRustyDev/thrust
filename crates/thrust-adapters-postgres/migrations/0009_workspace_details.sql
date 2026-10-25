-- Issue #44 (the compat slice, workspace stage): a workspace has a name, an owner, a size, a time zone and a colour, and a
-- membership has an id of its own, a role, whether it is active, a company role and per-member view settings. Until now a
-- workspace was a slug and a membership was a pair of ids (migration 0003).
--
-- Rows that exist already (a test's seed) get the slug as their name, no owner, role `member`, active, and the time the
-- column was added. New rows state every one of these themselves: nothing here relies on the defaults but the backfill.
--
-- Idempotent like the files before it (`migrate` applies every file on every run). `ADD COLUMN IF NOT EXISTS` skips a column
-- that is there, but NOT a CHECK written inline with it, so each CHECK is its own statement, dropped and added by name.
-- The membership id's default is built from `md5` so that the file needs no extension and no minimum Postgres version
-- (`gen_random_uuid()` is only in core from 13); the default only serves rows that exist when the column is added.

ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS name text;
UPDATE workspaces SET name = slug WHERE name IS NULL;
ALTER TABLE workspaces ALTER COLUMN name SET NOT NULL;
ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS owner_id uuid REFERENCES users (id);
ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS organization_size text;
ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS timezone text NOT NULL DEFAULT 'UTC';
ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS background_color text NOT NULL DEFAULT '#808080';
ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS created_at timestamptz NOT NULL DEFAULT now();
ALTER TABLE workspaces ADD COLUMN IF NOT EXISTS updated_at timestamptz NOT NULL DEFAULT now();

ALTER TABLE workspace_members
    ADD COLUMN IF NOT EXISTS id uuid NOT NULL DEFAULT md5(random()::text || clock_timestamp()::text)::uuid;
CREATE UNIQUE INDEX IF NOT EXISTS workspace_members_id_key ON workspace_members (id);
ALTER TABLE workspace_members ADD COLUMN IF NOT EXISTS role text NOT NULL DEFAULT 'member';
ALTER TABLE workspace_members DROP CONSTRAINT IF EXISTS workspace_members_role_valid;
ALTER TABLE workspace_members ADD CONSTRAINT workspace_members_role_valid CHECK (role IN ('guest', 'member', 'admin'));
ALTER TABLE workspace_members ADD COLUMN IF NOT EXISTS company_role text;
ALTER TABLE workspace_members ADD COLUMN IF NOT EXISTS is_active boolean NOT NULL DEFAULT true;
ALTER TABLE workspace_members ADD COLUMN IF NOT EXISTS props jsonb NOT NULL DEFAULT '{}'::jsonb;
ALTER TABLE workspace_members DROP CONSTRAINT IF EXISTS workspace_members_props_is_object;
ALTER TABLE workspace_members ADD CONSTRAINT workspace_members_props_is_object CHECK (jsonb_typeof(props) = 'object');
ALTER TABLE workspace_members ADD COLUMN IF NOT EXISTS created_at timestamptz NOT NULL DEFAULT now();
ALTER TABLE workspace_members ADD COLUMN IF NOT EXISTS updated_at timestamptz NOT NULL DEFAULT now();

-- the member list and `workspaces_of` read a workspace's members and a user's workspaces
CREATE INDEX IF NOT EXISTS workspace_members_by_user ON workspace_members (user_id, workspace_id);
