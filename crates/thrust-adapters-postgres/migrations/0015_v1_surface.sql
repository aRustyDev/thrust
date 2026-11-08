-- Issue #58 (the /api/v1 surface): what the public API reads and writes beyond what the compat slice (0010 to 0014) built.
-- One file, one section for each resource, added as each resource is served.
--
-- Idempotent like the files before it (`migrate` applies every file on every run): `ADD COLUMN IF NOT EXISTS` for a column,
-- and for an index a catalog check, so that a unique index is not dropped and made again at every start.

-- --- projects: deletion --------------------------------------------------------------------------------------------------------
-- A project that is deleted is not removed: its row stays, with the time, and every read leaves it out (as Plane's soft deletion
-- does), so its work items, states and history are kept and nothing that points at it is left pointing at nothing. Its name and its
-- key are free again, which is why the two unique indexes below hold only among the projects that are not deleted.
ALTER TABLE projects ADD COLUMN IF NOT EXISTS deleted_at timestamptz;

-- 0012 made these two as plain unique indexes on a database that has them from before this file; replace those (only those: the
-- check looks for the condition in the index's definition) and keep the names, which the conflict handling reports.
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_indexes WHERE schemaname = current_schema() AND tablename = 'projects'
               AND indexname = 'projects_workspace_identifier_key' AND indexdef NOT LIKE '%deleted_at%') THEN
        DROP INDEX projects_workspace_identifier_key;
    END IF;
    IF EXISTS (SELECT 1 FROM pg_indexes WHERE schemaname = current_schema() AND tablename = 'projects'
               AND indexname = 'projects_workspace_name_key' AND indexdef NOT LIKE '%deleted_at%') THEN
        DROP INDEX projects_workspace_name_key;
    END IF;
END $$;
CREATE UNIQUE INDEX IF NOT EXISTS projects_workspace_identifier_key ON projects (workspace_id, identifier) WHERE deleted_at IS NULL;
CREATE UNIQUE INDEX IF NOT EXISTS projects_workspace_name_key ON projects (workspace_id, name) WHERE deleted_at IS NULL;

-- --- states: what v1 reads and writes ---------------------------------------------------------------------------------------------
-- An integration's key (`external_source` and `external_id`, unique among a project's states by the view's own check, as on the pin: no
-- index), who last changed a state, and deletion: a state that is deleted stays as a row, marked with the time, and the three lists leave
-- it out, while the work items that name it still read it. Its name is free again, so the name's uniqueness holds among the live ones.
ALTER TABLE states ADD COLUMN IF NOT EXISTS external_source text;
ALTER TABLE states ADD COLUMN IF NOT EXISTS external_id text;
ALTER TABLE states ADD COLUMN IF NOT EXISTS updated_by uuid;
ALTER TABLE states DROP CONSTRAINT IF EXISTS states_updated_by_fkey;
ALTER TABLE states ADD CONSTRAINT states_updated_by_fkey FOREIGN KEY (updated_by) REFERENCES users (id);
ALTER TABLE states ADD COLUMN IF NOT EXISTS deleted_at timestamptz;

-- 0012 made the name unique as a table constraint of this name, in the CREATE TABLE that a start skips once the table is there: so
-- dropping it is a no-op after the first start, and the index that replaces it keeps the name (the conflict handling reports it).
ALTER TABLE states DROP CONSTRAINT IF EXISTS states_project_id_name_key;
CREATE UNIQUE INDEX IF NOT EXISTS states_project_id_name_key ON states (project_id, name) WHERE deleted_at IS NULL;
