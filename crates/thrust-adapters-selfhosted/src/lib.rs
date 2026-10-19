//! Self-hosted (native) adapters: clock, ids and the `Background` executor on tokio.
//! The Postgres connection pool lives here too (added with the Postgres stage).

use chrono::Utc;
use thrust_contracts::{Timestamp, Uuid};
use thrust_ports::{Background, BoxFuture, Clock, IdGen};

pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        Utc::now()
    }
}

pub struct NativeIdGen;

impl IdGen for NativeIdGen {
    fn new_uuid(&self) -> Uuid {
        Uuid::now_v7()
    }
}

pub mod pg;

/// `Background` on the tokio runtime.
pub struct TokioBackground;

impl Background for TokioBackground {
    fn spawn(&self, fut: BoxFuture<()>) {
        tokio::spawn(fut);
    }
}
