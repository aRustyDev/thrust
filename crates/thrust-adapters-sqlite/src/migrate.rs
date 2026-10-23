//! Versioned SQL migrations, applied the same way on both twins.
//!
//! The Postgres side applies its files with one `batch_execute` each (`thrust-adapters-postgres::migrate`) and is
//! idempotent by `IF NOT EXISTS`. Here each file runs once, as one transaction, together with the row that records it
//! in `schema_migrations`: a migration that fails halfway leaves the database as it was, version row included, and
//! the next start tries it again. DDL is transactional in SQLite, which is what makes that cheap.
//!
//! The runner is synchronous (a Durable Object runs it in its constructor, inside `blockConcurrencyWhile`). It
//! executes one statement per `exec`: the Durable Object's `sql.exec` is documented per statement, and splitting here
//! means a file behaves the same whichever twin runs it.

use crate::sql::{Sql, SqlError, Value};

/// A migration file: `version` orders it and is the number in its name (`0001_init.sql`).
#[derive(Debug, Clone, Copy)]
pub struct Migration {
    pub version: u32,
    pub name: &'static str,
    pub sql: &'static str,
}

/// Every migration this build knows, in order. Numbers match the Postgres files they translate or sit beside:
/// 0001 is the end state of Postgres 0001 to 0004; 0005 is identity (issue #38); 0006 is search.
pub const MIGRATIONS: &[Migration] = &[
    Migration { version: 1, name: "init", sql: include_str!("../migrations/0001_init.sql") },
    Migration { version: 6, name: "search", sql: include_str!("../migrations/0006_search.sql") },
    // MUTANT (build for the migration-failure demonstration only, feature `mutant_broken_migration`): a migration whose
    // third statement fails, after two that succeed. An object built with it must fail to start, and must roll the first
    // two back (research/0011, the migration story's failure case).
    #[cfg(feature = "mutant_broken_migration")]
    Migration {
        version: 98,
        name: "broken",
        sql: "CREATE TABLE broken_migration_marker (x); INSERT INTO broken_migration_marker VALUES (1); INSERT INTO no_such_table VALUES (1);",
    },
];

const CREATE_LEDGER: &str = "CREATE TABLE IF NOT EXISTS schema_migrations (\
    version INTEGER PRIMARY KEY, name TEXT NOT NULL, applied_at INTEGER NOT NULL)";

/// Apply, in order, every migration in `migrations` that the database has not recorded. Returns the versions applied.
///
/// A database that records a version this build does not know was migrated by a newer build; running older code on it
/// is how a contract migration breaks, so it is an error, not a silent skip.
pub fn migrate(sql: &dyn Sql, migrations: &[Migration]) -> Result<Vec<u32>, SqlError> {
    assert!(
        migrations.windows(2).all(|w| w[0].version < w[1].version),
        "migrations must be in strictly increasing version order"
    );
    sql.execute(CREATE_LEDGER, &[])?;
    let recorded = sql
        .query("SELECT version FROM schema_migrations ORDER BY version", &[])?
        .iter()
        .map(|r| r.int(0))
        .collect::<Result<Vec<i64>, _>>()?;
    if let Some(unknown) = recorded.iter().find(|v| !migrations.iter().any(|m| i64::from(m.version) == **v)) {
        return Err(SqlError::Other(format!(
            "the database records migration {unknown}, which this build does not know: it was migrated by a newer build"
        )));
    }
    let mut applied = Vec::new();
    for m in migrations.iter().filter(|m| !recorded.contains(&i64::from(m.version))) {
        let statements = split_statements(m.sql);
        sql.atomically(&mut |x| {
            for s in &statements {
                x.execute(s, &[]).map_err(|e| SqlError::Other(format!("migration {:04}_{}: {e} in `{}`", m.version, m.name, first_line(s))))?;
            }
            x.execute(
                "INSERT INTO schema_migrations (version, name, applied_at) VALUES (?, ?, CAST(strftime('%s', 'now') AS INTEGER) * 1000000)",
                &[Value::Int(i64::from(m.version)), Value::from(m.name)],
            )
        })?;
        applied.push(m.version);
    }
    Ok(applied)
}

fn first_line(s: &str) -> &str {
    s.lines().next().unwrap_or(s)
}

/// Split a SQL file into statements, without the terminating `;`, comments and blank statements dropped.
///
/// It understands what the migration files use: `--` line comments, `'...'` and `"..."` quoting, and the one construct
/// that carries a `;` inside a statement, `CREATE TRIGGER ... BEGIN ...; END`, where `BEGIN` and `CASE` open a block and
/// `END` closes one.
pub fn split_statements(sql: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut depth = 0i32;
    let mut chars = sql.chars().peekable();
    let mut word = String::new();

    fn close_word(word: &mut String, depth: &mut i32) {
        match word.to_ascii_uppercase().as_str() {
            "BEGIN" | "CASE" => *depth += 1,
            "END" => *depth -= 1,
            _ => {}
        }
        word.clear();
    }

    while let Some(c) = chars.next() {
        if c.is_ascii_alphanumeric() || c == '_' {
            word.push(c);
            cur.push(c);
            continue;
        }
        close_word(&mut word, &mut depth);
        match c {
            '-' if chars.peek() == Some(&'-') => {
                // line comment: dropped, up to (not including) the newline
                for n in chars.by_ref() {
                    if n == '\n' {
                        cur.push('\n');
                        break;
                    }
                }
            }
            '\'' | '"' => {
                cur.push(c);
                while let Some(n) = chars.next() {
                    cur.push(n);
                    if n == c {
                        // a doubled quote is an escaped quote and the literal goes on
                        if chars.peek() == Some(&c) {
                            cur.push(chars.next().unwrap());
                        } else {
                            break;
                        }
                    }
                }
            }
            ';' if depth <= 0 => {
                push(&mut out, &mut cur);
                depth = 0;
            }
            _ => cur.push(c),
        }
    }
    close_word(&mut word, &mut depth);
    push(&mut out, &mut cur);
    out
}

fn push(out: &mut Vec<String>, cur: &mut String) {
    let s = cur.trim();
    if !s.is_empty() {
        out.push(s.to_string());
    }
    cur.clear();
}
