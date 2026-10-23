//! Fault injection for the commit-atomicity control (spike 0002, risk R1).
//!
//! `FaultSql` wraps any [`Sql`] and makes the n-th statement that contains a given text fail, once, with an error a
//! real engine could raise. The control for the `Tx` mapping is: inject the failure into each statement a commit issues,
//! in turn, and check that neither the work item nor its outbox row nor the counter survives. It runs hermetically on
//! rusqlite (`tests/commit.rs`) and, through a probe route, on the real `SqlStorage` inside the Durable Object.
//!
//! Test support: no composition root arms a fault except the probe behind the test-routes flag and the admin token.

use std::sync::{Arc, Mutex};

use crate::sql::{Exec, Row, Sql, SqlError, Value};

/// Fail the `nth` (0-based) statement whose text contains `contains`, once.
#[derive(Debug, Clone)]
pub struct Fault {
    pub contains: String,
    pub nth: usize,
}

#[derive(Default)]
struct Plan {
    fault: Option<Fault>,
    seen: usize,
    fired: usize,
}

impl Plan {
    /// Count a statement; true if this is the one to fail.
    fn hit(&mut self, sql: &str) -> bool {
        let Some(f) = &self.fault else { return false };
        if !sql.contains(&f.contains) {
            return false;
        }
        let this = self.seen;
        self.seen += 1;
        if this == f.nth {
            self.fired += 1;
            return true;
        }
        false
    }
}

fn injected(sql: &str) -> SqlError {
    SqlError::Other(format!("injected fault at `{}`", sql.lines().next().unwrap_or(sql).trim()))
}

pub struct FaultSql {
    inner: Arc<dyn Sql>,
    plan: Mutex<Plan>,
}

impl FaultSql {
    pub fn new(inner: Arc<dyn Sql>) -> Self {
        Self { inner, plan: Mutex::new(Plan::default()) }
    }

    /// Arm a fault and reset the counters.
    pub fn arm(&self, fault: Fault) {
        *self.plan.lock().unwrap() = Plan { fault: Some(fault), seen: 0, fired: 0 };
    }

    /// Disarm; the statements keep passing through.
    pub fn disarm(&self) {
        self.plan.lock().unwrap().fault = None;
    }

    /// How many times the armed fault has fired.
    pub fn fired(&self) -> usize {
        self.plan.lock().unwrap().fired
    }

    /// How many statements matched the armed text so far (so a test knows how many it could have failed).
    pub fn matched(&self) -> usize {
        self.plan.lock().unwrap().seen
    }

    fn check(&self, sql: &str) -> Result<(), SqlError> {
        if self.plan.lock().unwrap().hit(sql) {
            Err(injected(sql))
        } else {
            Ok(())
        }
    }
}

impl Exec for FaultSql {
    fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>, SqlError> {
        self.check(sql)?;
        self.inner.query(sql, params)
    }
    fn execute(&self, sql: &str, params: &[Value]) -> Result<(), SqlError> {
        self.check(sql)?;
        self.inner.execute(sql, params)
    }
}

struct FaultExec<'a> {
    owner: &'a FaultSql,
    inner: &'a dyn Exec,
}

impl Exec for FaultExec<'_> {
    fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>, SqlError> {
        self.owner.check(sql)?;
        self.inner.query(sql, params)
    }
    fn execute(&self, sql: &str, params: &[Value]) -> Result<(), SqlError> {
        self.owner.check(sql)?;
        self.inner.execute(sql, params)
    }
}

impl Sql for FaultSql {
    fn atomically(&self, f: &mut dyn FnMut(&dyn Exec) -> Result<(), SqlError>) -> Result<(), SqlError> {
        self.inner.atomically(&mut |x| f(&FaultExec { owner: self, inner: x }))
    }
}

/// A `Sql` whose `atomically` is no transaction at all: the callback just runs, each statement on its own. It is the
/// naive mapping of `Tx::commit` onto an engine, kept so the control can be shown failing against it on the real
/// runtime (the probe's `naive` mode), not only against a mutant build.
pub struct Unatomic(pub Arc<dyn Sql>);

impl Exec for Unatomic {
    fn query(&self, sql: &str, params: &[Value]) -> Result<Vec<Row>, SqlError> {
        self.0.query(sql, params)
    }
    fn execute(&self, sql: &str, params: &[Value]) -> Result<(), SqlError> {
        self.0.execute(sql, params)
    }
}

impl Sql for Unatomic {
    fn atomically(&self, f: &mut dyn FnMut(&dyn Exec) -> Result<(), SqlError>) -> Result<(), SqlError> {
        f(self)
    }
}
