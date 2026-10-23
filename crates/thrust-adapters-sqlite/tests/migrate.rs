//! The migration runner: the statement splitter, the ledger, and what a migration that fails halfway leaves behind.
//!
//! Spike 0002 step 11 asks each cell for a migration story, the failure case included. This is the hermetic part for
//! the SQLite twin; the Durable Object runs the same runner (`migrate`) over `SqlStorage`.

use proptest::prelude::*;
use thrust_adapters_sqlite::{migrate, split_statements, Exec, Migration, NativeSql, SqlError, MIGRATIONS};

fn db() -> NativeSql {
    NativeSql::memory().unwrap()
}

fn tables(sql: &dyn Exec) -> Vec<String> {
    sql.query("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' ORDER BY name", &[])
        .unwrap()
        .iter()
        .map(|r| r.text(0).unwrap().to_string())
        .collect()
}

// --- the splitter -------------------------------------------------------------------------------

/// A statement the migration files could contain, and what the splitter must return for it.
fn statement() -> impl Strategy<Value = String> {
    prop_oneof![
        (0..1000u32).prop_map(|n| format!("SELECT {n}")),
        // a literal with a `;` and an escaped quote inside
        "[a-z; ]{0,8}".prop_map(|s| format!("SELECT '{s}''{s}'")),
        // a quoted identifier with a `;` in it
        "[a-z;]{1,6}".prop_map(|s| format!("CREATE TABLE \"{s}\" (x)")),
        // a trigger: `;` inside BEGIN ... END, and a CASE ... END that is not the trigger's END
        (0..100u32).prop_map(|n| format!(
            "CREATE TRIGGER t{n} AFTER INSERT ON x BEGIN INSERT INTO y VALUES (1); \
             UPDATE y SET a = CASE WHEN 1 THEN 2 ELSE 3 END; END"
        )),
        // identifiers that merely contain the block words must not open or close a block
        Just("CREATE TABLE begin_end (end_at, case_id, beginning)".to_string()),
    ]
}

proptest! {
    /// Joining statements with `;`, under line comments (which may themselves contain `;`), and splitting again gives
    /// the statements back.
    #[test]
    fn prop_split_statements_inverts_joining(
        parts in prop::collection::vec((prop::option::of("[a-z ;]{0,10}"), statement()), 0..8),
        trailing_semicolon in any::<bool>(),
    ) {
        let mut text = String::new();
        for (comment, body) in &parts {
            if let Some(c) = comment {
                text.push_str(&format!("-- {c}\n"));
            }
            text.push_str(body);
            text.push_str(";\n");
        }
        if !trailing_semicolon {
            text = text.trim_end().trim_end_matches(';').to_string();
        }
        let want: Vec<String> = parts.iter().map(|(_, b)| b.clone()).collect();
        prop_assert_eq!(split_statements(&text), want);
    }
}

#[test]
fn the_shipped_migrations_split_into_statements_that_run() {
    // Control for the splitter on the real files: every statement is complete SQL (a split in the wrong place would
    // leave a fragment SQLite refuses) and the whole file creates what it says.
    let sql = db();
    for m in MIGRATIONS {
        assert!(!split_statements(m.sql).is_empty(), "{} split into nothing", m.name);
    }
    assert_eq!(migrate(&sql, MIGRATIONS).unwrap(), MIGRATIONS.iter().map(|m| m.version).collect::<Vec<_>>());
    let t = tables(&sql);
    for want in ["workspaces", "workspace_members", "projects", "project_members", "project_sequences", "work_items", "outbox", "activity", "sessions"] {
        assert!(t.iter().any(|n| n == want), "no table {want} in {t:?}");
    }
}

// --- the ledger and the failure case --------------------------------------------------------------

// Static examples (rule 9 (b)): the expand step of spike 0002 step 11, run over an existing row; the generative
// counterparts are `prop_split_statements_inverts_joining` and the failure property below.
const A: Migration = Migration { version: 1, name: "a", sql: "CREATE TABLE a (x INTEGER); INSERT INTO a VALUES (1);" };
const B_EXPAND: Migration = Migration { version: 2, name: "b_expand", sql: "ALTER TABLE a ADD COLUMN y INTEGER; UPDATE a SET y = x;" };

#[test]
fn a_migration_runs_once_and_is_recorded() {
    let sql = db();
    assert_eq!(migrate(&sql, &[A, B_EXPAND]).unwrap(), [1, 2]);
    assert_eq!(migrate(&sql, &[A, B_EXPAND]).unwrap(), Vec::<u32>::new(), "a second run applied something");
    let versions: Vec<i64> =
        sql.query("SELECT version FROM schema_migrations ORDER BY version", &[]).unwrap().iter().map(|r| r.int(0).unwrap()).collect();
    assert_eq!(versions, [1, 2]);
    // the expand step ran over the existing row
    assert_eq!(sql.query("SELECT x, y FROM a", &[]).unwrap()[0].int(1).unwrap(), 1);
}

proptest! {
    /// A migration of `n` good statements with one that fails in any position: the database is as it was (no table
    /// of the failed migration, no ledger row), the error names the migration, and the same migration without the
    /// failing statement applies to the same database (so the failure was the statement, not the state).
    #[test]
    fn prop_a_migration_that_fails_at_any_statement_leaves_the_database_as_it_was(n in 1..6usize, fail_at in 0..6usize) {
        let sql = db();
        migrate(&sql, &[A, B_EXPAND]).unwrap();
        let before = tables(&sql);

        let mut statements: Vec<String> = (0..n).map(|i| format!("CREATE TABLE t{i} (x); INSERT INTO t{i} VALUES ({i})")).collect();
        let good = statements.join("; ");
        statements.insert(fail_at.min(n), "INSERT INTO no_such_table VALUES (1)".to_string());
        let bad_sql: &'static str = Box::leak(statements.join("; ").into_boxed_str());
        let good_sql: &'static str = Box::leak(good.into_boxed_str());
        let bad = Migration { version: 3, name: "c_fails", sql: bad_sql };

        let e = migrate(&sql, &[A, B_EXPAND, bad]).unwrap_err();
        prop_assert!(e.to_string().contains("c_fails") && e.to_string().contains("no_such_table"), "the error does not say where: {e}");
        prop_assert_eq!(tables(&sql), before, "a failed migration left a table behind");
        prop_assert_eq!(sql.query("SELECT count(*) FROM schema_migrations WHERE version = 3", &[]).unwrap()[0].int(0).unwrap(), 0);
        // the next start tries it again and fails the same way: the object stays down until the file is fixed
        prop_assert!(migrate(&sql, &[A, B_EXPAND, bad]).is_err());
        // control: the same runner and database take the migration without the failing statement
        let ok = Migration { sql: good_sql, ..bad };
        prop_assert_eq!(migrate(&sql, &[A, B_EXPAND, ok]).unwrap(), [3]);
    }
}

#[test]
fn a_database_from_a_newer_build_is_refused() {
    let sql = db();
    migrate(&sql, &[A, B_EXPAND]).unwrap();
    // an older build (only knows version 1) on a database at version 2: dropping or renaming columns in a later
    // migration is exactly what breaks it, so the runner refuses rather than skipping
    let e = migrate(&sql, &[A]).unwrap_err();
    assert!(matches!(e, SqlError::Other(m) if m.contains("migration 2") && m.contains("newer build")));
}

#[test]
fn the_runner_wants_versions_in_order() {
    let sql = db();
    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| migrate(&sql, &[B_EXPAND, A])));
    assert!(r.is_err(), "out-of-order migrations were accepted");
}
