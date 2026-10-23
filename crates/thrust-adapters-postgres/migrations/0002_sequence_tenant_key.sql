-- Issue #35 (FR-02): the sequence counter carries the tenant key, and a counter row can exist only
-- for a project in its own workspace, so an allocation that names the wrong workspace is rejected
-- (a foreign-key violation) instead of starting a second counter.
-- Idempotent: `migrate` applies every file on every run, and 0001 is CREATE ... IF NOT EXISTS, so a
-- database created before this file still has the old table and is altered here.

ALTER TABLE project_sequences ADD COLUMN IF NOT EXISTS workspace_id uuid;

UPDATE project_sequences s SET workspace_id = p.workspace_id
FROM projects p
WHERE p.id = s.project_id AND s.workspace_id IS NULL;

ALTER TABLE project_sequences ALTER COLUMN workspace_id SET NOT NULL;

DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'projects_workspace_id_id_key') THEN
        ALTER TABLE projects ADD CONSTRAINT projects_workspace_id_id_key UNIQUE (workspace_id, id);
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_constraint WHERE conname = 'project_sequences_tenant_fkey') THEN
        ALTER TABLE project_sequences ADD CONSTRAINT project_sequences_tenant_fkey
            FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id);
    END IF;
    -- Re-key from (project_id) to (workspace_id, project_id); `allocate` upserts on the new key.
    IF EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'project_sequences_pkey' AND cardinality(conkey) = 1
    ) THEN
        ALTER TABLE project_sequences DROP CONSTRAINT project_sequences_pkey;
        ALTER TABLE project_sequences ADD CONSTRAINT project_sequences_pkey PRIMARY KEY (workspace_id, project_id);
    END IF;
END
$$;
