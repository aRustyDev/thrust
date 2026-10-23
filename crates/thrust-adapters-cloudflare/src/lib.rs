//! Cloudflare (workers-rs) adapters.
//!
//! Every adapter that touches a JS object returns a `Send` future by wrapping the JS-backed part
//! in `worker::send::SendFuture`. That is sound because a Worker isolate is single-threaded
//! (workers-rs relies on the same argument for its own `unsafe impl Send` on bindings). It is
//! what lets one `Send`-bounded set of ports serve both a shared axum router and the Worker.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use thrust_adapters_postgres::{ClientHandle, Connector};
use thrust_contracts::chrono::DateTime;
use thrust_contracts::*;
use thrust_ports::*;
use worker::send::SendFuture;

// ---------------------------------------------------------------------------------------------
// Ambient: clock, ids, background
// ---------------------------------------------------------------------------------------------

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

/// `Background` for entrypoints without a `fetch` context (queue consumer, cron): a local task.
/// Neither entrypoint relies on it today (the consumer and the sweeper await their I/O inline).
pub struct LocalBackground;

impl Background for LocalBackground {
    fn spawn(&self, fut: BoxFuture<()>) {
        wasm_bindgen_futures::spawn_local(fut);
    }
}

// ---------------------------------------------------------------------------------------------
// R2 -> BlobStore
// ---------------------------------------------------------------------------------------------

pub struct R2BlobStore(pub worker::Bucket);

fn r2_err(e: impl std::fmt::Display) -> BlobError {
    BlobError::Backend(e.to_string())
}

#[async_trait]
impl BlobStore for R2BlobStore {
    async fn put(&self, key: &str, bytes: Vec<u8>, content_type: Option<String>) -> Result<(), BlobError> {
        let key = key.to_string();
        SendFuture::new(async move {
            let mut b = self.0.put(key, worker::Data::Bytes(bytes));
            if let Some(ct) = content_type {
                b = b.http_metadata(worker::HttpMetadata { content_type: Some(ct), ..Default::default() });
            }
            b.execute().await.map(|_| ()).map_err(r2_err)
        })
        .await
    }

    async fn get(&self, key: &str) -> Result<Option<Blob>, BlobError> {
        let key = key.to_string();
        SendFuture::new(async move {
            let Some(obj) = self.0.get(key).execute().await.map_err(r2_err)? else {
                return Ok(None);
            };
            let content_type = obj.http_metadata().content_type;
            let Some(body) = obj.body() else {
                return Ok(None);
            };
            let bytes = body.bytes().await.map_err(r2_err)?;
            Ok(Some(Blob { bytes, content_type }))
        })
        .await
    }

    async fn delete(&self, key: &str) -> Result<(), BlobError> {
        let key = key.to_string();
        SendFuture::new(async move { self.0.delete(key).await.map_err(r2_err) }).await
    }

    async fn list(&self, prefix: &str) -> Result<Vec<String>, BlobError> {
        let prefix = prefix.to_string();
        SendFuture::new(async move {
            let objs = self.0.list().prefix(prefix).execute().await.map_err(r2_err)?;
            Ok(objs.objects().into_iter().map(|o| o.key()).collect())
        })
        .await
    }
}

// ---------------------------------------------------------------------------------------------
// Queues producer -> Queue
// ---------------------------------------------------------------------------------------------

pub struct CfQueue(pub worker::Queue);

fn q_err(e: impl std::fmt::Display) -> QueueError {
    QueueError::Backend(e.to_string())
}

/// The envelope is sent as a JSON *text* message. Sending the struct directly goes through
/// serde-wasm-bindgen, which turns `serde_json::Value` objects into JS `Map`s that
/// `JSON.stringify` then renders as `{}` (a silent payload loss); text content type avoids it.
pub fn encode_envelope(env: &Envelope) -> Result<String, QueueError> {
    serde_json::to_string(env).map_err(q_err)
}

#[async_trait]
impl Queue for CfQueue {
    async fn send(&self, _topic: &str, msg: &Envelope, opts: SendOpts) -> Result<(), QueueError> {
        let text = encode_envelope(msg)?;
        let delay = opts.delay;
        SendFuture::new(async move {
            let mut b = worker::RawMessageBuilder::new(worker::wasm_bindgen::JsValue::from_str(&text));
            if let Some(d) = delay {
                b = b.delay_seconds(d.as_secs().min(86_400) as u32);
            }
            self.0
                .send_raw(b.build_with_content_type(worker::QueueContentType::Text))
                .await
                .map_err(q_err)
        })
        .await
    }
}

// ---------------------------------------------------------------------------------------------
// Durable Object (SQLite storage) -> RateLimiter
// ---------------------------------------------------------------------------------------------

/// One DO instance per rate-limit key; the DO class lives in `thrust-worker` (the only crate
/// that exports DO classes). Wire protocol: `GET /hit?limit=&window=` -> JSON.
pub struct DoRateLimiter(pub worker::ObjectNamespace);

#[derive(serde::Deserialize)]
struct DoHit {
    allowed: bool,
    remaining: u32,
    retry_after_secs: u64,
}

#[async_trait]
impl RateLimiter for DoRateLimiter {
    async fn hit(&self, key: &str, limit: u32, window_secs: u64) -> Result<RateDecision, RateError> {
        let e = |x: worker::Error| RateError::Backend(x.to_string());
        SendFuture::new(async move {
            let stub = self.0.get_by_name(key).map_err(e)?;
            let url = format!("https://rate-limiter.internal/hit?limit={limit}&window={window_secs}");
            let req = worker::Request::new(&url, worker::Method::Get).map_err(e)?;
            let mut resp = stub.fetch_with_request(req).await.map_err(e)?;
            if resp.status_code() != 200 {
                return Err(RateError::Backend(format!("rate-limiter DO answered {}", resp.status_code())));
            }
            let hit: DoHit = resp.json().await.map_err(e)?;
            Ok(if hit.allowed {
                RateDecision::Allowed { remaining: hit.remaining }
            } else {
                RateDecision::Limited { retry_after_secs: hit.retry_after_secs }
            })
        })
        .await
    }
}

// ---------------------------------------------------------------------------------------------
// Hyperdrive -> Postgres connection strategy
// ---------------------------------------------------------------------------------------------

/// Opens `tokio_postgres` connections over the Hyperdrive binding's `Socket`.
///
/// workerd ties a socket to the request that opened it, so a `HyperdriveConnector` (and the
/// `App` holding it) is created per request. Reads share one lazily-opened connection;
/// transactions open their own (Hyperdrive pools the upstream side).
pub struct HyperdriveConnector {
    hd: worker::Hyperdrive,
    shared: Mutex<Option<Arc<tokio_postgres::Client>>>,
}

impl HyperdriveConnector {
    pub fn new(hd: worker::Hyperdrive) -> Self {
        Self { hd, shared: Mutex::new(None) }
    }

    async fn connect(&self) -> Result<tokio_postgres::Client, DbError> {
        let be = |m: String| DbError::Backend(m);
        let hd = &self.hd;
        SendFuture::new(async move {
            let socket = hd.connect().map_err(|e| be(format!("hyperdrive connect: {e}")))?;
            let mut cfg = tokio_postgres::Config::new();
            cfg.host(&hd.host()).port(hd.port()).user(&hd.user()).password(hd.password()).dbname(&hd.database());
            let (client, conn) = cfg
                .connect_raw(socket, tokio_postgres::NoTls)
                .await
                .map_err(|e| be(format!("postgres handshake: {e}")))?;
            // The connection driver runs until the last `Client` clone is dropped.
            wasm_bindgen_futures::spawn_local(async move {
                let _ = conn.await;
            });
            Ok(client)
        })
        .await
    }
}

#[async_trait]
impl Connector for HyperdriveConnector {
    async fn checkout(&self, exclusive: bool) -> Result<Box<dyn ClientHandle>, DbError> {
        if exclusive {
            return Ok(Box::new(Arc::new(self.connect().await?)));
        }
        if let Some(c) = self.shared.lock().unwrap().clone() {
            return Ok(Box::new(c));
        }
        let fresh = Arc::new(self.connect().await?);
        let mut slot = self.shared.lock().unwrap();
        // Two first callers can race; keep the first connection, the loser is dropped (closed).
        let keep = slot.get_or_insert_with(|| fresh.clone()).clone();
        Ok(Box::new(keep))
    }
}

// ---------------------------------------------------------------------------------------------
// Durable Object SQLite -> `Sql` (the storage of the instance-level object, spike 0002 cell `worker-do`)
// ---------------------------------------------------------------------------------------------

/// `thrust_adapters_sqlite::Sql` over a Durable Object's storage: `storage.sql.exec` for statements and
/// `storage.transactionSync` for `atomically`. workers-rs 0.8.7 binds the first and not the second, so the second is
/// called through `js_sys::Reflect` on `Storage::as_raw()`.
pub struct DoSql {
    storage: worker::Storage,
}

// SAFETY: a Worker isolate is single-threaded, so the JS handles inside are never touched from two threads; this is the
// argument workers-rs itself makes for its own `unsafe impl Send for SqlStorage`, and the one this crate's header makes
// for `SendFuture`. A `DoSql` must not be moved out of the isolate that made it (nothing here can).
unsafe impl Send for DoSql {}
unsafe impl Sync for DoSql {}

impl DoSql {
    pub fn new(storage: worker::Storage) -> Self {
        Self { storage }
    }
}

fn sql_err(e: worker::Error) -> thrust_adapters_sqlite::SqlError {
    thrust_adapters_sqlite::SqlError::from_message(e.to_string())
}

fn js_message(e: &worker::wasm_bindgen::JsValue) -> String {
    e.as_string().unwrap_or_else(|| {
        worker::js_sys::Reflect::get(e, &"message".into())
            .ok()
            .and_then(|m| m.as_string())
            .unwrap_or_else(|| format!("{e:?}"))
    })
}

impl thrust_adapters_sqlite::Exec for DoSql {
    fn query(
        &self,
        sql: &str,
        params: &[thrust_adapters_sqlite::Value],
    ) -> Result<Vec<thrust_adapters_sqlite::Row>, thrust_adapters_sqlite::SqlError> {
        use thrust_adapters_sqlite::{Row, SqlError, Value};
        use worker::SqlStorageValue as Sv;
        let bindings: Vec<Sv> = params
            .iter()
            .map(|p| match p {
                Value::Null => Sv::Null,
                Value::Int(i) => Sv::Integer(*i),
                Value::Text(t) => Sv::String(t.clone()),
                Value::Blob(b) => Sv::Blob(b.clone()),
            })
            .collect();
        // Consume the cursor to the end: that is what runs a write, and it surfaces a constraint failure raised
        // after the first row.
        let cursor = self.storage.sql().exec(sql, if bindings.is_empty() { None } else { Some(bindings) }).map_err(sql_err)?;
        cursor
            .raw()
            .map(|r| {
                let cols = r.map_err(sql_err)?;
                cols.into_iter()
                    .map(|c| match c {
                        Sv::Null => Ok(Value::Null),
                        Sv::Integer(i) => Ok(Value::Int(i)),
                        Sv::String(t) => Ok(Value::Text(t)),
                        Sv::Blob(b) => Ok(Value::Blob(b)),
                        other => Err(SqlError::decode(format!("unexpected column value {other:?}: the slice stores no floats or booleans"))),
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map(Row)
            })
            .collect()
    }

    fn execute(&self, sql: &str, params: &[thrust_adapters_sqlite::Value]) -> Result<(), thrust_adapters_sqlite::SqlError> {
        self.query(sql, params).map(|_| ())
    }
}

impl thrust_adapters_sqlite::Sql for DoSql {
    fn atomically(
        &self,
        f: &mut dyn FnMut(&dyn thrust_adapters_sqlite::Exec) -> Result<(), thrust_adapters_sqlite::SqlError>,
    ) -> Result<(), thrust_adapters_sqlite::SqlError> {
        use thrust_adapters_sqlite::SqlError;
        use worker::js_sys::{Function, Reflect};
        use worker::wasm_bindgen::closure::ScopedClosure;
        use worker::wasm_bindgen::{JsCast, JsValue};

        let storage: &JsValue = self.storage.as_raw().as_ref();
        let transaction_sync: Function = Reflect::get(storage, &"transactionSync".into())
            .map_err(|e| SqlError::Other(js_message(&e)))?
            .dyn_into()
            .map_err(|_| SqlError::Other("storage.transactionSync is not a function".into()))?;

        // A thrown exception is how `transactionSync` is told to roll back, and it rethrows it. The callback keeps the
        // real error aside, because what comes back through JS is only a string.
        let mut failure: Option<SqlError> = None;
        let mut callback = || -> Result<JsValue, JsValue> {
            match f(self) {
                Ok(()) => Ok(JsValue::UNDEFINED),
                Err(e) => {
                    failure = Some(e);
                    Err(JsValue::from_str("thrust: roll back the transaction"))
                }
            }
        };
        let closure = ScopedClosure::<dyn FnMut() -> Result<JsValue, JsValue>>::borrow_mut_aborting(&mut callback);
        let called = transaction_sync.call1(storage, closure.as_ref());
        drop(closure);
        match (failure, called) {
            (Some(e), _) => Err(e),
            (None, Ok(_)) => Ok(()),
            (None, Err(e)) => Err(SqlError::from_message(format!("transactionSync: {}", js_message(&e)))),
        }
    }
}
