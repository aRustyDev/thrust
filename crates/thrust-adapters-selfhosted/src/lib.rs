//! Self-hosted (native) adapters: clock, ids and the `Background` executor on tokio.
//! The Postgres connection pool lives here too (added with the Postgres stage).

use chrono::Utc;
use thrust_contracts::{Timestamp, Uuid};
use thrust_ports::{Background, BoxFuture, Clock, IdGen, Rng, RngError};

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

/// The operating system's generator (`getrandom`): the `Rng` port natively.
pub struct NativeRng;

impl Rng for NativeRng {
    fn fill(&self, buf: &mut [u8]) -> Result<(), RngError> {
        getrandom::fill(buf).map_err(|e| RngError(e.to_string()))
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
