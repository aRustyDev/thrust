//! `Database::ping` of the Postgres adapter (issue #42, ADR-0094), where no server is needed: what the adapter does when it cannot get a
//! connection. (That it succeeds against a real database, and costs exactly one statement, is `model_counting_...` in the selfhosted crate's
//! `tests/counting`, which has Docker.)
//!
//! This is in `tests/` and not in the crate's `#[cfg(test)]` module because `scripts/check-deps.sh` lets `SystemScope::mint` be called in a
//! member's own `tests/` directory and nowhere else under `src/`.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test code: a panic is how a test fails, and clippy.toml exempts only #[test] functions and #[cfg(test)] modules, not the helpers around them"
)]

mod common;

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use async_trait::async_trait;
use proptest::prelude::*;
use thrust_adapters_postgres::{ClientHandle, Connector, PgDatabase};
use thrust_ports::{Database, DbError, SystemScope};

/// A `Connector` that has no connection to give and says why.
struct NoConnection(DbError);

#[async_trait]
impl Connector for NoConnection {
    async fn checkout(&self, _exclusive: bool) -> Result<Box<dyn ClientHandle>, DbError> {
        Err(match &self.0 {
            DbError::Backend(m) => DbError::Backend(m.clone()),
            DbError::Conflict(m) => DbError::Conflict(m.clone()),
        })
    }
}

/// Runs a future that has nothing to wait for: the connector above answers at once.
fn ready<T>(f: impl Future<Output = T>) -> T {
    let mut f = std::pin::pin!(f);
    match f.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("the future was not ready"),
    }
}

proptest! {
    #![proptest_config(common::proptest_config(256))]

    // `ping` has no way to succeed without a connection: whatever the connector refuses with comes back, unchanged, and nothing panics.
    #[test]
    fn prop_ping_without_a_connection_is_the_connectors_error(message in "[a-zA-Z0-9 _.:'-]{0,60}", conflict in any::<bool>()) {
        let refused = if conflict { DbError::Conflict(message.clone()) } else { DbError::Backend(message.clone()) };
        let db = PgDatabase::new(Arc::new(NoConnection(refused)));
        match ready(db.ping(&SystemScope::mint())) {
            Err(DbError::Backend(m)) => prop_assert!(!conflict && m == message, "got Backend({m:?}) for {message:?}"),
            Err(DbError::Conflict(m)) => prop_assert!(conflict && m == message, "got Conflict({m:?}) for {message:?}"),
            Ok(()) => prop_assert!(false, "a ping with no connection must not be Ok"),
        }
    }
}
