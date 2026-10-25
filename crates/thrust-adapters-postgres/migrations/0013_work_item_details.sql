-- Issue #44 (the compat slice, work item stage): a work item has a state, a priority, dates, a description as HTML, who last
-- changed it and when, the people it is assigned to and its labels. They are columns of the row the slice already makes (the slice
-- reads and writes the columns it had, so its JSON is what it was), and two join tables.
--
-- Rows that exist already (a test's seed, the slice's own) get what an item made with no options has: no state, priority `none`, no
-- dates, the empty paragraph the pin starts a description with, and the time it was made as the time it was last changed.
-- Idempotent like the files before it (`migrate` applies every file on every run): `ADD COLUMN IF NOT EXISTS` skips a column that is
-- there but NOT a CHECK or a foreign key written inline with it, so each is its own statement, dropped and added by name; a NOT NULL
-- column that needs a backfill is added nullable, backfilled, then made NOT NULL (Postgres checks NOT NULL before it looks for a
-- conflict, the trap 0003 and 0009 met).
--
-- Not here, because nothing reads or writes it yet: the parent of a work item, its estimate, its cycle and its modules, the counts of
-- its attachments, links and sub work items (the compat plane answers them null or empty, deviation D-028).

ALTER TABLE work_items ADD COLUMN IF NOT EXISTS state_id uuid;
ALTER TABLE work_items DROP CONSTRAINT IF EXISTS work_items_state_fkey;
ALTER TABLE work_items ADD CONSTRAINT work_items_state_fkey FOREIGN KEY (state_id) REFERENCES states (id);

ALTER TABLE work_items ADD COLUMN IF NOT EXISTS priority text NOT NULL DEFAULT 'none';
ALTER TABLE work_items DROP CONSTRAINT IF EXISTS work_items_priority_valid;
ALTER TABLE work_items ADD CONSTRAINT work_items_priority_valid CHECK (priority IN ('urgent', 'high', 'medium', 'low', 'none'));

ALTER TABLE work_items ADD COLUMN IF NOT EXISTS start_date date;
ALTER TABLE work_items ADD COLUMN IF NOT EXISTS target_date date;
ALTER TABLE work_items DROP CONSTRAINT IF EXISTS work_items_dates_ordered;
ALTER TABLE work_items ADD CONSTRAINT work_items_dates_ordered CHECK (start_date IS NULL OR target_date IS NULL OR start_date <= target_date);

ALTER TABLE work_items ADD COLUMN IF NOT EXISTS completed_at timestamptz;

ALTER TABLE work_items ADD COLUMN IF NOT EXISTS updated_at timestamptz;
UPDATE work_items SET updated_at = created_at WHERE updated_at IS NULL;
ALTER TABLE work_items ALTER COLUMN updated_at SET DEFAULT now();
ALTER TABLE work_items ALTER COLUMN updated_at SET NOT NULL;
ALTER TABLE work_items ADD COLUMN IF NOT EXISTS updated_by uuid;
ALTER TABLE work_items DROP CONSTRAINT IF EXISTS work_items_updated_by_fkey;
ALTER TABLE work_items ADD CONSTRAINT work_items_updated_by_fkey FOREIGN KEY (updated_by) REFERENCES users (id);

ALTER TABLE work_items ADD COLUMN IF NOT EXISTS description_html text NOT NULL DEFAULT '<p></p>';

-- a list is read by its project and one of its orders, and the page after another is a different offset into the same order
CREATE INDEX IF NOT EXISTS work_items_by_state ON work_items (state_id);
CREATE INDEX IF NOT EXISTS work_items_by_updated ON work_items (project_id, updated_at);

-- Who a work item is assigned to. The row names the work item's own workspace and project (so a join to the project's members
-- needs no second lookup), and a person is assigned once.
CREATE TABLE IF NOT EXISTS work_item_assignees (
    work_item_id  uuid NOT NULL REFERENCES work_items (id),
    workspace_id  uuid NOT NULL,
    project_id    uuid NOT NULL,
    user_id       uuid NOT NULL REFERENCES users (id),
    created_at    timestamptz NOT NULL,
    PRIMARY KEY (work_item_id, user_id),
    FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id)
);
CREATE INDEX IF NOT EXISTS work_item_assignees_by_user ON work_item_assignees (user_id, project_id);

-- The labels of a work item (the labels are 0012's), a label once.
CREATE TABLE IF NOT EXISTS work_item_labels (
    work_item_id  uuid NOT NULL REFERENCES work_items (id),
    workspace_id  uuid NOT NULL,
    project_id    uuid NOT NULL,
    label_id      uuid NOT NULL REFERENCES labels (id),
    created_at    timestamptz NOT NULL,
    PRIMARY KEY (work_item_id, label_id),
    FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id)
);
CREATE INDEX IF NOT EXISTS work_item_labels_by_label ON work_item_labels (label_id);
