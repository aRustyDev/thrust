-- Issue #48 (D-020, the work items' half): a work item keeps ONE form of its description, the cleaned HTML (`description_html`, 0013),
-- and beside it what is derived from that form: `description_stripped`, the text of it (Django's `strip_tags`, as the pin's column of
-- that name), and `description_schema`, the version of the policy the HTML was cleaned under (`thrust_html::SCHEMA_VERSION`, ADR-0012).
--
-- There is deliberately no `description_json` and no `description_binary` column: at P0 the editor's JSON and binary are not stored,
-- because a second stored form is a second source of truth, one of them not cleaned (FR-14). `thrust-editor-doc` derives them from the
-- HTML on the first collab open (P1), so adding them later needs no data migration.
--
-- Both are NULL on a row that was written before this file, and on a work item the slice made: nothing was derived for them (their
-- HTML was cleaned by Plane's plain allow-list, before the hardened one was the default), and nothing reads either column yet. A search
-- over the text, or a re-clean of the old rows, belongs to the change that needs it; an empty description is `''`, never NULL.
--
-- An expand migration (scripts/check-migration-lint.sh, issue #195): two nullable columns with no default, which an older version of the
-- app, still running on the same database during a deploy, neither reads nor trips over. Idempotent like the files before it
-- (`migrate` applies every file on every run).

ALTER TABLE work_items ADD COLUMN IF NOT EXISTS description_stripped text;
ALTER TABLE work_items ADD COLUMN IF NOT EXISTS description_schema smallint;
