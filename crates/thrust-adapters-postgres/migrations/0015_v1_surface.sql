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

-- Labels (issue #58): the key an integration gave a label (the pair is unique by the view's own check, as on the pin: no index), who
-- last changed one, and deletion. A label that is deleted stays as a row, marked with the time, together with the labels under it,
-- and the work items lose it; its name is free again, so the name's uniqueness holds among the live ones, as the pin's constraint
-- `unique_project_name_when_not_deleted` has it.
ALTER TABLE labels ADD COLUMN IF NOT EXISTS external_source text;
ALTER TABLE labels ADD COLUMN IF NOT EXISTS external_id text;
ALTER TABLE labels ADD COLUMN IF NOT EXISTS updated_by uuid;
ALTER TABLE labels DROP CONSTRAINT IF EXISTS labels_updated_by_fkey;
ALTER TABLE labels ADD CONSTRAINT labels_updated_by_fkey FOREIGN KEY (updated_by) REFERENCES users (id);
ALTER TABLE labels ADD COLUMN IF NOT EXISTS deleted_at timestamptz;

-- as for states: 0012's table constraint of this name is gone after the first start, and the index that replaces it keeps the name
ALTER TABLE labels DROP CONSTRAINT IF EXISTS labels_project_id_name_key;
CREATE UNIQUE INDEX IF NOT EXISTS labels_project_id_name_key ON labels (project_id, name) WHERE deleted_at IS NULL;
-- the labels under a label, which a delete walks
CREATE INDEX IF NOT EXISTS labels_by_parent ON labels (parent_id) WHERE parent_id IS NOT NULL;

-- Work items (issue #58): who made an item is optional (Plane's column is nullable, and the oracle's seeded items have no maker); an
-- item may be under another (a parent: the same project, which the use-case checks, as the pin's serializer does; the pin lets an
-- item be its own parent, and so does this), and may have a `point`, a whole number from 0 to 12. Each is idempotent: a start runs
-- the file again.
ALTER TABLE work_items ALTER COLUMN created_by DROP NOT NULL;
ALTER TABLE work_items ADD COLUMN IF NOT EXISTS parent_id uuid;
ALTER TABLE work_items DROP CONSTRAINT IF EXISTS work_items_parent_fkey;
ALTER TABLE work_items ADD CONSTRAINT work_items_parent_fkey FOREIGN KEY (parent_id) REFERENCES work_items (id);
ALTER TABLE work_items ADD COLUMN IF NOT EXISTS point integer;
ALTER TABLE work_items DROP CONSTRAINT IF EXISTS work_items_point_valid;
ALTER TABLE work_items ADD CONSTRAINT work_items_point_valid CHECK (point IS NULL OR point BETWEEN 0 AND 12);
-- the items under an item, which a delete walks
CREATE INDEX IF NOT EXISTS work_items_by_parent ON work_items (parent_id) WHERE parent_id IS NOT NULL;
