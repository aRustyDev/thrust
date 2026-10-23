//! The synchronous SQL surface the adapter is written against.
//!
//! Two implementations exist: `NativeSql` (rusqlite, feature `native`) and `DoSql` in
//! `thrust-adapters-cloudflare` (workers-rs `SqlStorage` inside the Durable Object). Both are synchronous on purpose:
//! `SqlStorage::exec` is synchronous, and a Durable Object transaction (`transactionSync`) takes a callback that must
//! finish without yielding. The one thing the trait has to express is therefore [`Sql::atomically`], a callback run as
//! one transaction.
//!
//! No call here reports "rows changed": `SqlStorageCursor::rowsWritten` counts index writes too, so it is not a row
//! count. Anything that needs to know whether a row was touched uses `RETURNING` and counts the rows it gets back.

use std::fmt;

use thrust_contracts::chrono::DateTime;
use thrust_contracts::{Timestamp, Uuid};

/// A SQLite value. No floating point: nothing in the slice stores one, and a JS number cannot tell `1` from `1.0`.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Text(String),
    Blob(Vec<u8>),
}

impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Int(v as i64)
    }
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::Text(v.to_string())
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Text(v)
    }
}
impl From<&[u8]> for Value {
    fn from(v: &[u8]) -> Self {
        Value::Blob(v.to_vec())
    }
}
impl From<Uuid> for Value {
    /// Ids are stored as their hyphenated lower-case text: it sorts like the bytes, and it is readable in a dump.
    fn from(v: Uuid) -> Self {
        Value::Text(v.to_string())
    }
}
impl From<Timestamp> for Value {
    /// Timestamps are stored as microseconds since the epoch, what `timestamptz` keeps. That stays below 2^53 until
    /// the year 2255, so it survives the trip through a JS number in the Durable Object.
    fn from(v: Timestamp) -> Self {
        Value::Int(v.timestamp_micros())
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(v: Option<T>) -> Self {
        v.map_or(Value::Null, Into::into)
    }
}

/// One result row, by column position.
#[derive(Debug, Clone, PartialEq)]
pub struct Row(pub Vec<Value>);

impl Row {
    fn at(&self, i: usize) -> Result<&Value, SqlError> {
        self.0.get(i).ok_or_else(|| SqlError::decode(format!("column {i} is missing from a row of {}", self.0.len())))
    }

    pub fn int(&self, i: usize) -> Result<i64, SqlError> {
        match self.at(i)? {
            Value::Int(v) => Ok(*v),
            other => Err(SqlError::decode(format!("column {i}: want an integer, got {other:?}"))),
        }
    }

    pub fn opt_int(&self, i: usize) -> Result<Option<i64>, SqlError> {
        match self.at(i)? {
            Value::Null => Ok(None),
            Value::Int(v) => Ok(Some(*v)),
            other => Err(SqlError::decode(format!("column {i}: want an integer or NULL, got {other:?}"))),
        }
    }

    pub fn bool(&self, i: usize) -> Result<bool, SqlError> {
        Ok(self.int(i)? != 0)
    }

    pub fn text(&self, i: usize) -> Result<&str, SqlError> {
        match self.at(i)? {
            Value::Text(v) => Ok(v),
            other => Err(SqlError::decode(format!("column {i}: want text, got {other:?}"))),
        }
    }

    pub fn opt_text(&self, i: usize) -> Result<Option<String>, SqlError> {
        match self.at(i)? {
            Value::Null => Ok(None),
            Value::Text(v) => Ok(Some(v.clone())),
            other => Err(SqlError::decode(format!("column {i}: want text or NULL, got {other:?}"))),
        }
    }

    pub fn uuid(&self, i: usize) -> Result<Uuid, SqlError> {
        Uuid::parse_str(self.text(i)?).map_err(|e| SqlError::decode(format!("column {i}: {e}")))
    }

    pub fn opt_uuid(&self, i: usize) -> Result<Option<Uuid>, SqlError> {
        self.opt_text(i)?
            .map(|t| Uuid::parse_str(&t).map_err(|e| SqlError::decode(format!("column {i}: {e}"))))
            .transpose()
    }

    pub fn timestamp(&self, i: usize) -> Result<Timestamp, SqlError> {
        micros_to_timestamp(self.int(i)?)
    }

    pub fn opt_timestamp(&self, i: usize) -> Result<Option<Timestamp>, SqlError> {
        self.opt_int(i)?.map(micros_to_timestamp).transpose()
    }
}

fn micros_to_timestamp(us: i64) -> Result<Timestamp, SqlError> {
    DateTime::from_timestamp_micros(us).ok_or_else(|| SqlError::decode(format!("{us} is not a timestamp")))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SqlError {
    /// A UNIQUE or PRIMARY KEY constraint failed. It is the only constraint the ports report as `Conflict`.
    Unique(String),
    /// Anything else the engine refused, or a result the adapter could not read.
    Other(String),
}

impl SqlError {
    pub fn decode(m: impl Into<String>) -> Self {
        SqlError::Other(m.into())
    }

    /// Classify an engine error by its message. Both SQLite builds word a unique violation as
    /// `UNIQUE constraint failed: <table>.<column>, ...` (a primary key as `... failed: <table>.<column>`), and the
    /// Durable Object surfaces the same text, so one classifier serves both.
    pub fn from_message(m: impl Into<String>) -> Self {
        let m = m.into();
        if m.contains("UNIQUE constraint failed") {
            SqlError::Unique(m)
        } else {
            SqlError::Other(m)
        }
    }
}

impl fmt::Display for SqlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SqlError::Unique(m) | SqlError::Other(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for SqlError {}

/// Run statements. `execute` is for statements whose result nobody reads; everything that reads, including `INSERT ...
/// RETURNING`, goes through `query`.
pub trait Exec {
    fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>, SqlError>;
    fn execute(&self, sql: &str, params: &[Value]) -> Result<(), SqlError>;
}

/// A SQLite database that can run a callback as one transaction.
pub trait Sql: Exec + Send + Sync {
    /// Run `f` as one transaction: every statement it issues through the handle it is given commits together, or, if
    /// `f` returns an error, none does. `f` runs synchronously, start to finish, and nothing else touches the database
    /// meanwhile: that is `BEGIN IMMEDIATE` natively and `ctx.storage.transactionSync` in the Durable Object.
    fn atomically(&self, f: &mut dyn FnMut(&dyn Exec) -> Result<(), SqlError>) -> Result<(), SqlError>;
}
