//! The deep health check (issue #42, ADR-0094): `ping_database`, over a database that answers and over one that refuses.
//!
//! The memory adapters can never fail, so on their own they cannot show that the use-case reports a failure at all. The Postgres adapter
//! over a `Connector` that always refuses is the one `Database` that fails with no server, and it is what a stopped database looks like
//! from here: the connect fails, and the adapter says so as `DbError::Backend`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::sync::Arc;

use async_trait::async_trait;
use common::*;
use futures_executor::block_on;
use proptest::prelude::*;
use thrust_adapters_postgres::{ClientHandle, Connector, PgDatabase};
use thrust_core::*;
use thrust_ports::DbError;

/// A `Connector` that refuses every checkout, with the cause it was given.
struct Refusing(String);

#[async_trait]
impl Connector for Refusing {
    async fn checkout(&self, _exclusive: bool) -> Result<Box<dyn ClientHandle>, DbError> {
        Err(DbError::Backend(self.0.clone()))
    }
}

proptest! {
    #![proptest_config(proptest_config(64))]

    // Over the memory adapters the check is `Ok`; over a database that refuses, with any cause, it is an error that carries the cause
    // for the log (the handler turns it into a body that does not).
    #[test]
    fn prop_the_database_check_reports_what_the_database_refused_with(cause in "[a-zA-Z0-9 _.:-]{1,40}") {
        let mut w = world();
        prop_assert!(block_on(ping_database(&w.app)).is_ok());

        w.app.ports.db = Arc::new(PgDatabase::new(Arc::new(Refusing(cause.clone()))));
        match block_on(ping_database(&w.app)) {
            Err(CoreError::Internal(m)) => prop_assert!(m.contains(&cause), "the cause is lost: {m}"),
            other => prop_assert!(false, "a refusing database must be an Internal error, got {other:?}"),
        }
    }
}
