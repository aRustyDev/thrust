-- Issue #44 (the compat slice, work item stage 6b): a work item has a history, the rows the activity consumer writes for the changes an
-- event names in its payload (the pin's `IssueActivity`): who changed what, from which value to which, when, and the words the app
-- puts before the new value.
--
-- A row is made once: the key is the event and the place of the row in it (`UNIQUE (event_id, ordinal)`), so an event the queue
-- delivers twice writes nothing the second time. `seq` is the order the rows were written in, which breaks a tie between rows of one
-- time (a work item's history is read by `created_at`, then `seq`; the consumer can move a row's time, never its place).
--
-- The row names the work item's own workspace and project (as `work_item_assignees` does), so a read needs no join; the adapter
-- refuses a row whose work item is not in the workspace and project it names (the foreign keys below cannot say it: the work item's
-- id is its key, and its tenant is two more columns). A work item is never deleted for good, so no row is left pointing at nothing.
--
-- Idempotent like the files before it (`migrate` applies every file on every run): the table is made whole or not at all
-- (`CREATE TABLE IF NOT EXISTS`, in one batch), so its constraints are written inline and are not touched again; dropping and adding
-- them by name on every start, as 0013 must for the columns it added to a table that exists, would check every row of a history that
-- only grows against each foreign key at every start.

CREATE TABLE IF NOT EXISTS work_item_activities (
    seq             bigint GENERATED ALWAYS AS IDENTITY,
    id              uuid PRIMARY KEY,
    event_id        uuid NOT NULL,
    ordinal         integer NOT NULL,
    workspace_id    uuid NOT NULL,
    project_id      uuid NOT NULL,
    work_item_id    uuid NOT NULL REFERENCES work_items (id),
    actor_id        uuid REFERENCES users (id),
    verb            text NOT NULL,
    field           text,
    old_value       text,
    new_value       text,
    comment         text NOT NULL,
    old_identifier  uuid,
    new_identifier  uuid,
    epoch           bigint NOT NULL,
    created_at      timestamptz NOT NULL,
    CONSTRAINT work_item_activities_event_key UNIQUE (event_id, ordinal),
    CONSTRAINT work_item_activities_tenant_fkey FOREIGN KEY (workspace_id, project_id) REFERENCES projects (workspace_id, id)
);

-- a work item's history is read in the order of its times
CREATE INDEX IF NOT EXISTS work_item_activities_by_item ON work_item_activities (work_item_id, created_at);
