//! Cloudflare (workers-rs) adapters.
//!
//! Every adapter that touches a JS object returns a `Send` future by wrapping the JS-backed part
//! in `worker::send::SendFuture`. That is sound because a Worker isolate is single-threaded
//! (workers-rs relies on the same argument for its own `unsafe impl Send` on bindings).

use std::sync::Arc;

use thrust_contracts::chrono::DateTime;
use thrust_contracts::{Timestamp, Uuid};
use thrust_ports::{Background, BoxFuture, Clock, IdGen};

/// Wall clock from the JS `Date` (workerd freezes it during pure CPU work; that is fine here).
pub struct JsClock;

impl Clock for JsClock {
    fn now(&self) -> Timestamp {
        let ms = worker::Date::now().as_millis() as i64;
        DateTime::from_timestamp_millis(ms).expect("valid epoch millis")
    }
}

/// UUID v7 from the JS clock + `crypto.getRandomValues` (uuid's `js` feature).
pub struct JsIdGen;

impl IdGen for JsIdGen {
    fn new_uuid(&self) -> Uuid {
        let ms = worker::Date::now().as_millis();
        let ts = uuid::Timestamp::from_unix(uuid::NoContext, ms / 1000, ((ms % 1000) * 1_000_000) as u32);
        Uuid::new_v7(ts)
    }
}

/// `Background` on `ctx.waitUntil`: the isolate stays alive until the future completes.
pub struct WaitUntilBackground(pub Arc<worker::Context>);

impl Background for WaitUntilBackground {
    fn spawn(&self, fut: BoxFuture<()>) {
        self.0.wait_until(fut);
    }
}
