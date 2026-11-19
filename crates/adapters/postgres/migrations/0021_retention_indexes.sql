-- Retention (issue #59, F-D10, deviation D-001): the daily job that deletes for good the work items that were soft-deleted long ago, and the
-- outbox rows that went out long ago, reads both tables by age. Without these two indexes each run would read every row of both.
--
-- Only indexes, so the files before it stay as they are (a file's checksum is in the ledger) and nothing is dropped or changed in type.
-- The other thing the job needs, "the work items under a work item", is `work_items_by_parent` (0015).
--
-- Both are partial. A work item is nearly always live, and the outbox is nearly all dispatched rows waiting out their 90 days, so the
-- index holds only what the job reads: the deleted work items, and the dispatched outbox rows. (The undispatched rows have
-- `outbox_undispatched`, 0001, which the sweeper reads.)
--
-- Safe to run again, like the files before it (`IF NOT EXISTS`): a database that has no ledger yet is given every file once more.

-- the job walks the deleted work items by (`deleted_at`, `id`), from the oldest, a page at a time
CREATE INDEX IF NOT EXISTS work_items_deleted_by_time ON work_items (deleted_at, id) WHERE deleted_at IS NOT NULL;

-- the job deletes the oldest dispatched rows first
CREATE INDEX IF NOT EXISTS outbox_dispatched_by_age ON outbox (created_at, event_id) WHERE dispatched_at IS NOT NULL;
