//! The native twin: `Sql` on rusqlite (bundled SQLite, so the dialect does not depend on the host).
//!
//! One connection behind a mutex. Every call is short and synchronous, so holding the mutex never spans an await, and
//! `atomically` holds it for the whole transaction, which is what makes the callback exclusive.

use std::panic::{catch_unwind, resume_unwind, AssertUnwindSafe};
use std::sync::{Mutex, MutexGuard};

use rusqlite::types::{ToSqlOutput, ValueRef};
use rusqlite::{Connection, ToSql};

use crate::sql::{Exec, Row, Sql, SqlError, Value};

impl ToSql for Value {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(match self {
            Value::Null => ToSqlOutput::Owned(rusqlite::types::Value::Null),
            Value::Int(v) => ToSqlOutput::Owned(rusqlite::types::Value::Integer(*v)),
            Value::Text(v) => ToSqlOutput::Borrowed(ValueRef::Text(v.as_bytes())),
            Value::Blob(v) => ToSqlOutput::Borrowed(ValueRef::Blob(v)),
        })
    }
}

fn err(e: rusqlite::Error) -> SqlError {
    SqlError::from_message(e.to_string())
}

fn read(v: ValueRef<'_>) -> Result<Value, SqlError> {
    Ok(match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Int(i),
        ValueRef::Text(t) => Value::Text(
            std::str::from_utf8(t).map_err(|e| SqlError::decode(format!("text column is not UTF-8: {e}")))?.to_string(),
        ),
        ValueRef::Blob(b) => Value::Blob(b.to_vec()),
        ValueRef::Real(_) => return Err(SqlError::decode("a REAL column: the slice stores none")),
    })
}

fn query(conn: &Connection, sql: &str, params: &[Value]) -> Result<Vec<Row>, SqlError> {
    let mut stmt = conn.prepare_cached(sql).map_err(err)?;
    let cols = stmt.column_count();
    let mut rows = stmt.query(rusqlite::params_from_iter(params.iter())).map_err(err)?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(err)? {
        out.push(Row((0..cols).map(|i| read(r.get_ref(i).map_err(err)?)).collect::<Result<_, _>>()?));
    }
    Ok(out)
}

fn execute(conn: &Connection, sql: &str, params: &[Value]) -> Result<(), SqlError> {
    let mut stmt = conn.prepare_cached(sql).map_err(err)?;
    // `execute` rejects a statement that returns rows (an `INSERT ... RETURNING`); stepping it is what runs it.
    let mut rows = stmt.query(rusqlite::params_from_iter(params.iter())).map_err(err)?;
    while rows.next().map_err(err)?.is_some() {}
    Ok(())
}

/// The handle `atomically` gives its callback: the connection the transaction is open on.
struct ConnExec<'a>(&'a Connection);

impl Exec for ConnExec<'_> {
    fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>, SqlError> {
        query(self.0, sql, params)
    }
    fn execute(&self, sql: &str, params: &[Value]) -> Result<(), SqlError> {
        execute(self.0, sql, params)
    }
}

pub struct NativeSql {
    conn: Mutex<Connection>,
}

impl NativeSql {
    /// A private in-memory database (every `NativeSql::memory()` is a new, empty one).
    pub fn memory() -> Result<Self, SqlError> {
        Self::with(Connection::open_in_memory().map_err(err)?)
    }

    /// A database file, created if it is missing. Write-ahead logging, so readers do not wait for the writer.
    pub fn open(path: &str) -> Result<Self, SqlError> {
        let conn = Connection::open(path).map_err(err)?;
        conn.execute_batch("PRAGMA journal_mode = WAL; PRAGMA synchronous = NORMAL;").map_err(err)?;
        Self::with(conn)
    }

    fn with(conn: Connection) -> Result<Self, SqlError> {
        // The Durable Object's SQLite enforces foreign keys; the twin must too, or it accepts rows the object refuses.
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA busy_timeout = 5000;").map_err(err)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn lock(&self) -> MutexGuard<'_, Connection> {
        // A panic while the lock was held leaves a connection that `atomically` has already rolled back.
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Exec for NativeSql {
    fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>, SqlError> {
        query(&self.lock(), sql, params)
    }
    fn execute(&self, sql: &str, params: &[Value]) -> Result<(), SqlError> {
        execute(&self.lock(), sql, params)
    }
}

impl Sql for NativeSql {
    fn atomically(&self, f: &mut dyn FnMut(&dyn Exec) -> Result<(), SqlError>) -> Result<(), SqlError> {
        let conn = self.lock();
        conn.execute_batch("BEGIN IMMEDIATE").map_err(err)?;
        match catch_unwind(AssertUnwindSafe(|| f(&ConnExec(&conn)))) {
            Ok(Ok(())) => conn.execute_batch("COMMIT").map_err(|e| {
                let _ = conn.execute_batch("ROLLBACK");
                err(e)
            }),
            Ok(Err(e)) => {
                let _ = conn.execute_batch("ROLLBACK");
                Err(e)
            }
            Err(panic) => {
                let _ = conn.execute_batch("ROLLBACK");
                resume_unwind(panic)
            }
        }
    }
}
