-- Spike 0002 (issue #37, ADR-0083): substring search over a work item's name and description, Plane's `icontains`.
-- An FTS5 table with the `trigram` tokenizer indexes every three-character window of both columns, so `MATCH` answers a
-- substring query of three characters or more, in any case, from the index; the adapter scans with LIKE for a shorter one
-- (a trigram needs three characters). Its Postgres twin is `pg_trgm` (`thrust-adapters-postgres/migrations/0006_search.sql`).
--
-- The table is an external-content index over `work_items` (no second copy of the text), kept in step by triggers: one per
-- way a row's text can change. A NULL description is indexed as an empty string everywhere, so a delete can always remove
-- exactly what an insert added. `rebuild` indexes the rows that already exist: this file is also the expand migration of a
-- seeded database (research/0011 times it at 100k rows).

CREATE VIRTUAL TABLE work_items_fts USING fts5(
    name,
    description,
    content = 'work_items',
    content_rowid = 'rowid',
    tokenize = 'trigram'
);

CREATE TRIGGER work_items_fts_insert AFTER INSERT ON work_items BEGIN
    INSERT INTO work_items_fts (rowid, name, description) VALUES (new.rowid, new.name, coalesce(new.description, ''));
END;

CREATE TRIGGER work_items_fts_delete AFTER DELETE ON work_items BEGIN
    INSERT INTO work_items_fts (work_items_fts, rowid, name, description)
    VALUES ('delete', old.rowid, old.name, coalesce(old.description, ''));
END;

CREATE TRIGGER work_items_fts_update AFTER UPDATE OF name, description ON work_items BEGIN
    INSERT INTO work_items_fts (work_items_fts, rowid, name, description)
    VALUES ('delete', old.rowid, old.name, coalesce(old.description, ''));
    INSERT INTO work_items_fts (rowid, name, description) VALUES (new.rowid, new.name, coalesce(new.description, ''));
END;

INSERT INTO work_items_fts (work_items_fts) VALUES ('rebuild');
